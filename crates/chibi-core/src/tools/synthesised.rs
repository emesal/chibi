//! Synthesised tool loader and executor for tein-backed scheme tools.
//!
//! A synthesised tool is a scheme source file (loaded from VFS) that defines
//! five bindings:
//!
//! ```scheme
//! (import (scheme base))   ; required — sandbox starts with null env
//! (define tool-name        "my_tool")
//! (define tool-description "what the tool does")
//! (define tool-parameters
//!   '((param-name . ((type . "string") (description . "what it is")))))
//! (define (tool-execute args)
//!   (cdr (assoc "param-name" args)))
//! ```
//!
//! Alternatively, use `define-tool` for multi-tool files:
//!
//! ```scheme
//! (import (scheme base))
//! (import (harness tools))
//!
//! (define-tool greet
//!   (description "greets someone")
//!   (parameters '((name . ((type . "string") (description . "who to greet")))))
//!   (execute (lambda (args)
//!     (string-append "hello " (cdr (assoc "name" args))))))
//! ```
//!
//! **Sandbox note:** synthesised tools run in a null environment — only `import`
//! is available without an explicit import. You must `(import (scheme base))` to
//! access `assoc`, `cons`, `car`, `cdr`, `number->string`, etc. Import additional
//! modules (`(scheme list)`, `(scheme write)`, etc.) as needed.
//!
//! `tool-parameters` is a scheme alist mapping parameter names (symbols) to
//! inner alists describing the JSON Schema properties (`type`, `description`,
//! `required` — optional, defaults to true).
//!
//! `tool-execute` receives an alist built from the JSON tool-call args:
//! `{"key": "val"}` → `(("key" . val) ...)`. Use `(assoc "key" args)` to
//! extract values. Keys are scheme strings (not symbols).
//!
//! ## `(harness tools)` module
//!
//! Every synthesised tool context has `(harness tools)` available. It exports:
//!
//! - `call-tool` — invoke another tool by name: `(call-tool "name" args-alist)`
//! - `define-tool` — macro for declaring multiple tools in one file
//!
//! ## `call-tool` bridge
//!
//! `call-tool` bridges sync tein → async tokio dispatch via
//! `Handle::current().block_on()`. The tool registry and call context are
//! stashed in thread-locals (`BRIDGE_REGISTRY`, `BRIDGE_CALL_CTX`) before
//! each invocation and cleared after via a guard. All mutations to these
//! thread-locals happen on the tein worker thread.
//!
//! Each synthesised tool gets its own sandboxed tein context, shared via
//! `Arc` for concurrent dispatch. All access goes through `ThreadLocalContext`,
//! which is `Send + Sync`.

#[cfg(feature = "synthesised-tools")]
use std::sync::{Arc, RwLock};

#[cfg(feature = "synthesised-tools")]
use tein::{Context, ThreadLocalContext, Value, sandbox::Modules};

use std::io;

use crate::tools::registry::{ToolCall, ToolRegistry};
use crate::tools::{Tool, ToolCategory, ToolImpl, ToolMetadata};
use crate::vfs::{Vfs, VfsCaller, VfsPath}; // Vfs+VfsCaller used in scan_and_register

// --- (harness tools) module source -------------------------------------------

/// Scheme source for the `(harness tools)` module.
///
/// `call-tool` is registered as a foreign function before this module is
/// evaluated and re-exported here so synthesised tools can access it via
/// `(import (harness tools))`.
///
/// `define-tool` is NOT part of this library module — it is injected as a
/// top-level syntax form alongside `%tool-registry%` before the user's source
/// is evaluated. This allows `define-tool` to mutate the top-level
/// `%tool-registry%` binding, which is readable by rust after evaluation.
///
/// Mutation site: if `call-tool` signature changes, update `call_tool_bridge`.
#[cfg(feature = "synthesised-tools")]
const HARNESS_TOOLS_MODULE: &str = r#"
(define-library (harness tools)
  (import (scheme base))
  (export call-tool)
  (begin
    ;; call-tool is injected as a foreign fn before this module loads.
    ;; re-export it so (import (harness tools)) provides it.
    #t))
"#;

/// Top-level scheme preamble evaluated in every synthesised tool context.
///
/// Defines `%tool-registry%` and the `define-tool` syntax at the top level
/// so user source can call `(define-tool ...)` and rust can read the result
/// via `ctx.evaluate("%tool-registry%")` after evaluation.
///
/// `define-tool` also re-exports itself so `(import (harness tools))` provides
/// it — the module re-export is handled by this preamble's `define-tool` being
/// in scope before the import.
///
/// Mutation site: if `define-tool` syntax changes, update `extract_multi_tools`
/// which parses `%tool-registry%` entries.
#[cfg(feature = "synthesised-tools")]
const HARNESS_PREAMBLE: &str = r#"
(import (scheme base))

;; accumulates define-tool entries. each entry is a list:
;; (name-string description-string params-value execute-procedure)
(define %tool-registry% '())

;; registers a tool: appends to %tool-registry% in definition order (LIFO via cons).
;; rust reads %tool-registry% after evaluation; non-empty → multi-tool mode.
(define-syntax define-tool
  (syntax-rules (description parameters execute)
    ((define-tool name
       (description desc)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler)
             %tool-registry%)))))
"#;

// --- thread-local bridge state -----------------------------------------------

/// Subset of ToolCallContext that is owned (no lifetimes) for thread-local storage.
///
/// Raw pointers are used because `ToolCallContext` has lifetimes. They are valid
/// for the duration of `execute_synthesised`, which holds `CallContextGuard`.
///
/// Mutation sites: if `ToolCallContext` fields change, update this struct and
/// `CallContextGuard::set`. Also update `call_tool_bridge` reconstruction.
#[cfg(feature = "synthesised-tools")]
struct ActiveCallContext {
    app: *const crate::state::AppState,
    context_name: String,
    config: *const crate::config::ResolvedConfig,
    project_root: std::path::PathBuf,
    vfs: *const Vfs,
    /// Non-empty string = `VfsCaller::Context(name)`, empty = `VfsCaller::System`.
    vfs_caller_context: String,
}

// SAFETY: the pointers in `ActiveCallContext` are only dereferenced on the
// tein worker thread while `CallContextGuard` is alive (i.e. within a single
// `execute_synthesised` call on that thread). No other thread accesses them.
#[cfg(feature = "synthesised-tools")]
unsafe impl Send for ActiveCallContext {}

thread_local! {
    /// Arc to the tool registry, set once per `load_tool_from_source` call
    /// and retained for the lifetime of the tein worker thread. All tools on
    /// the same thread share the same registry.
    #[cfg(feature = "synthesised-tools")]
    static BRIDGE_REGISTRY: std::cell::RefCell<Option<Arc<RwLock<ToolRegistry>>>> =
        const { std::cell::RefCell::new(None) };

    /// Per-call context for `call-tool`. Set/cleared via `CallContextGuard`.
    #[cfg(feature = "synthesised-tools")]
    static BRIDGE_CALL_CTX: std::cell::RefCell<Option<ActiveCallContext>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard that stashes a `ToolCallContext` in `BRIDGE_CALL_CTX` and clears
/// it on drop. Used in `execute_synthesised` to make the context available to
/// the `call-tool` bridge without threading it through tein's FFI boundary.
#[cfg(feature = "synthesised-tools")]
struct CallContextGuard;

#[cfg(feature = "synthesised-tools")]
impl CallContextGuard {
    fn set(ctx: &crate::tools::registry::ToolCallContext<'_>) -> Self {
        BRIDGE_CALL_CTX.with(|cell| {
            *cell.borrow_mut() = Some(ActiveCallContext {
                app: ctx.app as *const _,
                context_name: ctx.context_name.to_string(),
                config: ctx.config as *const _,
                project_root: ctx.project_root.to_path_buf(),
                vfs: ctx.vfs as *const _,
                vfs_caller_context: match ctx.vfs_caller {
                    VfsCaller::Context(name) => name.to_string(),
                    VfsCaller::System => String::new(),
                },
            });
        });
        CallContextGuard
    }
}

#[cfg(feature = "synthesised-tools")]
impl Drop for CallContextGuard {
    fn drop(&mut self) {
        BRIDGE_CALL_CTX.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

// --- call-tool foreign function bridge ---------------------------------------

/// The `call-tool` foreign function: `(call-tool name args-alist) → string`
///
/// Reads `BRIDGE_REGISTRY` and `BRIDGE_CALL_CTX` from thread-locals. Converts
/// the scheme alist args to JSON, looks up the tool, and dispatches via
/// `ToolRegistry::dispatch_impl` on the current tokio runtime.
///
/// Error handling: returns a scheme error string on failure (tein surfacees it
/// as a scheme error condition). Does not panic.
///
/// Must be registered with `ctx.define_fn_variadic("call-tool", call_tool_bridge)`
/// before `HARNESS_TOOLS_MODULE` is evaluated, so the re-export resolves.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "call-tool")]
fn call_tool_fn(name: String, args: Value) -> Result<String, String> {
    let json_args = scheme_value_to_json(&args).map_err(|e| format!("args conversion: {e}"))?;

    BRIDGE_CALL_CTX.with(|ctx_cell| {
        let ctx_borrow = ctx_cell.borrow();
        let active = ctx_borrow
            .as_ref()
            .ok_or_else(|| "call-tool: no active call context (called outside tool execute?)"
                .to_string())?;

        let registry = BRIDGE_REGISTRY.with(|reg_cell| {
            reg_cell
                .borrow()
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| "call-tool: no registry available".to_string())
        })?;

        // SAFETY: pointers are valid for the duration of execute_synthesised,
        // which holds CallContextGuard that set them. We reconstruct a
        // ToolCallContext only for dispatch — no storage beyond this fn.
        let vfs_caller_str = active.vfs_caller_context.clone();
        let call_ctx = unsafe {
            crate::tools::registry::ToolCallContext {
                app: &*active.app,
                context_name: &active.context_name,
                config: &*active.config,
                project_root: &active.project_root,
                vfs: &*active.vfs,
                vfs_caller: if vfs_caller_str.is_empty() {
                    VfsCaller::System
                } else {
                    VfsCaller::Context(&vfs_caller_str)
                },
            }
        };

        let tool_impl = {
            let reg = registry.read().map_err(|e| format!("registry lock: {e}"))?;
            let tool = reg
                .get(&name)
                .ok_or_else(|| format!("unknown tool: {name}"))?;
            tool.r#impl.clone()
        };

        // bridge sync tein → async tokio
        let handle = tokio::runtime::Handle::current();
        handle
            .block_on(ToolRegistry::dispatch_impl(
                tool_impl,
                &name,
                &json_args,
                &call_ctx,
            ))
            .map_err(|e| format!("tool error: {e}"))
    })
}

// --- loader ------------------------------------------------------------------

/// Build a tein `ThreadLocalContext` for a synthesised tool, registering
/// `call-tool`, the harness preamble, and `(harness tools)` module.
///
/// Sandbox behaviour depends on `tier`:
/// - `Sandboxed`: safe modules only, 10M step limit
/// - `Unsandboxed`: full R7RS, no step limit (trusted tools only)
#[cfg(feature = "synthesised-tools")]
fn build_tein_context(
    source: String,
    tier: crate::config::SandboxTier,
) -> io::Result<ThreadLocalContext> {
    let init = move |ctx: &Context| -> tein::Result<()> {
        ctx.define_fn_variadic("call-tool", __tein_call_tool_fn)?;
        ctx.evaluate(HARNESS_PREAMBLE)?;
        ctx.register_module(HARNESS_TOOLS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness module: {e}")))?;
        ctx.evaluate(&source)?;
        Ok(())
    };

    match tier {
        crate::config::SandboxTier::Sandboxed => Context::builder()
            .standard_env()
            .sandboxed(Modules::Safe)
            .step_limit(10_000_000)
            .build_managed(init),
        crate::config::SandboxTier::Unsandboxed => {
            Context::builder().standard_env().build_managed(init)
        }
    }
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tein init: {e}")))
}

/// Load one or more synthesised tools from scheme source.
///
/// If the source uses `(define-tool ...)` macro, returns all defined tools.
/// If it uses the convention format (`tool-name`, `tool-description`, etc.),
/// returns a single tool. Backwards-compatible with both formats.
///
/// Evaluates `source` in a tein context configured by `tier`:
/// - `SandboxTier::Sandboxed` (default): safe module subset, step limit.
/// - `SandboxTier::Unsandboxed`: full R7RS, no step limit (trusted tools).
///
/// Registers `(harness tools)` and `call-tool` in every context.
#[cfg(feature = "synthesised-tools")]
pub fn load_tools_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<Vec<Tool>> {
    load_tools_from_source_with_tier(source, vfs_path, registry, crate::config::SandboxTier::Sandboxed)
}

/// Like `load_tools_from_source` but with an explicit sandbox tier.
#[cfg(feature = "synthesised-tools")]
pub fn load_tools_from_source_with_tier(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    tier: crate::config::SandboxTier,
) -> io::Result<Vec<Tool>> {
    // stash registry in thread-local so call-tool bridge can access it
    BRIDGE_REGISTRY.with(|cell| {
        *cell.borrow_mut() = Some(Arc::clone(registry));
    });

    let source_owned = source.to_string();

    let ctx = build_tein_context(source_owned, tier)?;

    // check if define-tool was used (%tool-registry% is non-empty list)
    let multi = ctx.evaluate("%tool-registry%").ok();
    let is_multi = matches!(
        &multi,
        Some(Value::List(items)) if !items.is_empty()
    );

    if is_multi {
        extract_multi_tools(ctx, vfs_path)
    } else {
        extract_single_tool(ctx, vfs_path).map(|t| vec![t])
    }
}

/// Load a single synthesised tool from scheme source (convenience wrapper).
///
/// Calls `load_tools_from_source` and expects exactly one tool. Returns an
/// error if the source defines multiple tools via `define-tool`.
#[cfg(feature = "synthesised-tools")]
pub fn load_tool_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<Tool> {
    let mut tools = load_tools_from_source(source, vfs_path, registry)?;
    match tools.len() {
        1 => Ok(tools.remove(0)),
        n => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected 1 tool, got {n} (use load_tools_from_source for multi-tool files)"),
        )),
    }
}

/// Extract a single tool from a context using the convention-based format.
#[cfg(feature = "synthesised-tools")]
fn extract_single_tool(ctx: ThreadLocalContext, vfs_path: &VfsPath) -> io::Result<Tool> {
    let name = extract_string(&ctx, "tool-name")?;
    let description = extract_string(&ctx, "tool-description")?;
    let params_val = ctx.evaluate("tool-parameters").map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing tool-parameters: {e}"),
        )
    })?;
    let parameters = params_alist_to_json_schema(&params_val)?;

    let exec_val = ctx.evaluate("tool-execute").map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing tool-execute: {e}"),
        )
    })?;
    if !exec_val.is_procedure() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tool-execute is not a procedure",
        ));
    }

    let context = Arc::new(ctx);
    Ok(Tool {
        name,
        description,
        parameters,
        hooks: vec![],
        metadata: ToolMetadata::new(),
        summary_params: vec![],
        r#impl: ToolImpl::Synthesised {
            vfs_path: vfs_path.clone(),
            exec_binding: "tool-execute".to_string(),
            context,
        },
        category: ToolCategory::Synthesised,
    })
}

/// Extract multiple tools from a context that used `(define-tool ...)`.
///
/// Reads `%tool-registry%` (a LIFO list built via `cons`) and produces one
/// `Tool` per entry. All tools share the same tein context via `Arc`.
#[cfg(feature = "synthesised-tools")]
fn extract_multi_tools(ctx: ThreadLocalContext, vfs_path: &VfsPath) -> io::Result<Vec<Tool>> {
    let registry_val = ctx
        .evaluate("%tool-registry%")
        .map_err(|e| io::Error::other(format!("reading %tool-registry%: {e}")))?;

    let entries = match registry_val {
        Value::List(items) => items,
        other => {
            return Err(io::Error::other(format!(
                "%%tool-registry%% is not a list: {other}"
            )))
        }
    };

    let context = Arc::new(ctx);
    let mut tools = Vec::with_capacity(entries.len());

    // entries are in LIFO order (built via cons); reverse to get definition order
    for entry in entries.iter().rev() {
        let fields = match entry {
            Value::List(f) if f.len() >= 4 => f,
            other => {
                return Err(io::Error::other(format!(
                    "define-tool entry has unexpected shape: {other}"
                )))
            }
        };

        let name = fields[0]
            .as_string()
            .ok_or_else(|| io::Error::other("define-tool: name not a string"))?
            .to_string();
        let description = fields[1]
            .as_string()
            .ok_or_else(|| io::Error::other("define-tool: description not a string"))?
            .to_string();
        let parameters = params_alist_to_json_schema(&fields[2])?;

        if !fields[3].is_procedure() {
            return Err(io::Error::other(format!(
                "define-tool {name}: execute is not a procedure"
            )));
        }

        // bind the execute handler to a per-tool name so execute_synthesised
        // can find it by name. the context is shared across all tools in this file.
        let exec_binding = format!("%tool-execute-{name}%");
        // %tool-registry% entries are (name desc params handler).
        // use list-ref to extract the handler (index 3) for this tool by name.
        // list-ref is in (scheme base) which the preamble already imports.
        context
            .evaluate(&format!(
                "(define {exec_binding} \
                 (list-ref \
                   (let loop ((reg %tool-registry%)) \
                     (if (string=? (car (car reg)) \"{name}\") \
                         (car reg) \
                         (loop (cdr reg)))) \
                   3))"
            ))
            .map_err(|e| io::Error::other(format!("binding {exec_binding}: {e}")))?;

        tools.push(Tool {
            name,
            description,
            parameters,
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: ToolImpl::Synthesised {
                vfs_path: vfs_path.clone(),
                exec_binding: exec_binding.clone(),
                context: Arc::clone(&context),
            },
            category: ToolCategory::Synthesised,
        });
    }

    Ok(tools)
}

/// Execute a synthesised tool by calling its bound execute procedure.
///
/// Converts JSON args to a scheme alist, resolves the `exec_binding` in the
/// context, and calls it. The result is coerced to a string via
/// `Value::as_string()` (for scheme strings) or `Display` (for other values).
///
/// Sets `BRIDGE_CALL_CTX` via `CallContextGuard` before calling into scheme
/// so that `call-tool` can access the runtime context.
#[cfg(feature = "synthesised-tools")]
pub async fn execute_synthesised(
    context: &ThreadLocalContext,
    exec_binding: &str,
    call: &ToolCall<'_>,
) -> io::Result<String> {
    let _guard = CallContextGuard::set(call.context);
    let args_alist = json_args_to_scheme_alist(call.args)?;
    let exec_fn = context
        .evaluate(exec_binding)
        .map_err(|e| io::Error::other(format!("resolve {exec_binding}: {e}")))?;
    let result = context
        .call(&exec_fn, &[args_alist])
        .map_err(|e| io::Error::other(format!("tool execution: {e}")))?;
    match result.as_string() {
        Some(s) => Ok(s.to_string()),
        None => Ok(result.to_string()),
    }
}

/// No-op stub so the module compiles without the feature. Unreachable at
/// runtime since `ToolImpl::Synthesised` only exists behind the same cfg.
#[cfg(not(feature = "synthesised-tools"))]
pub async fn execute_synthesised(
    _context: &(),
    _exec_binding: &str,
    _call: &ToolCall<'_>,
) -> io::Result<String> {
    unreachable!("synthesised-tools feature not enabled")
}

// --- startup scan ------------------------------------------------------------

/// Scan writable VFS zones for `.scm` tool files and register them.
///
/// Called once at startup after the VFS and registry are fully constructed.
/// Silently skips zones that don't exist yet, unreadable files, and files
/// whose source fails to parse/evaluate. Non-`.scm` entries are ignored.
///
/// `tools_config` is used to resolve the sandbox tier for each tool file.
/// Pass `&ToolsConfig::default()` if no tier overrides are needed.
///
/// **Zones scanned:**
/// - `/tools/shared` — globally shared tools
/// - `/tools/home/<ctx>/` — per-context tools (one dir per context name)
/// - `/tools/flocks/<name>/` — flock-scoped tools
#[cfg(feature = "synthesised-tools")]
pub async fn scan_and_register(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
    tools_config: &crate::config::ToolsConfig,
) -> io::Result<()> {
    let mut zones = vec!["/tools/shared".to_string()];

    // discover /tools/home/<ctx>/ directories
    if let Ok(entries) = vfs
        .list(VfsCaller::System, &VfsPath::new("/tools/home")?)
        .await
    {
        for entry in entries {
            zones.push(format!("/tools/home/{}", entry.name));
        }
    }

    // discover /tools/flocks/<name>/ directories
    if let Ok(entries) = vfs
        .list(VfsCaller::System, &VfsPath::new("/tools/flocks")?)
        .await
    {
        for entry in entries {
            zones.push(format!("/tools/flocks/{}", entry.name));
        }
    }

    for zone in &zones {
        scan_zone(vfs, registry, zone, tools_config).await?;
    }
    Ok(())
}

/// Scan a single VFS zone directory and register all `.scm` tool files found.
#[cfg(feature = "synthesised-tools")]
async fn scan_zone(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
    zone: &str,
    tools_config: &crate::config::ToolsConfig,
) -> io::Result<()> {
    let Ok(zone_path) = VfsPath::new(zone) else {
        return Ok(());
    };
    if !vfs
        .exists(VfsCaller::System, &zone_path)
        .await
        .unwrap_or(false)
    {
        return Ok(());
    }
    let entries = match vfs.list(VfsCaller::System, &zone_path).await {
        Ok(e) => e,
        Err(_) => return Ok(()), // zone unreadable — skip silently
    };
    for entry in entries {
        if !entry.name.ends_with(".scm") {
            continue;
        }
        let Ok(file_path) = VfsPath::new(&format!("{zone}/{}", entry.name)) else {
            continue;
        };
        let source = match vfs.read(VfsCaller::System, &file_path).await {
            Ok(b) => b,
            Err(_) => continue, // unreadable — skip silently
        };
        let Ok(source_str) = String::from_utf8(source) else {
            continue;
        };
        let tier = tools_config.resolve_tier(file_path.as_str());
        if let Ok(tools) =
            load_tools_from_source_with_tier(&source_str, &file_path, registry, tier)
        {
            let mut reg = registry.write().unwrap();
            for tool in tools {
                reg.register(tool);
            }
        }
        // Err(_): invalid source — skip silently (caller can inspect via VFS)
    }
    Ok(())
}

// --- hot-reload callbacks ----------------------------------------------------

/// Reload (or register for the first time) synthesised tools from source bytes.
///
/// Called synchronously from the `on_scm_change` callback after a successful
/// write. The `content` bytes are the data that was just written to the VFS,
/// so no re-read is needed. Handles multi-tool files: unregisters all previous
/// tools from this path before registering new ones.
///
/// `tools_config` is used to resolve the sandbox tier for the tool file.
/// Pass `&ToolsConfig::default()` if no tier overrides are needed.
///
/// On parse/eval error, leaves previous versions registered (safe degradation).
#[cfg(feature = "synthesised-tools")]
pub fn reload_tool_from_content(
    registry: &Arc<RwLock<ToolRegistry>>,
    path: &VfsPath,
    content: &[u8],
    tools_config: &crate::config::ToolsConfig,
) {
    let Ok(source_str) = std::str::from_utf8(content) else {
        return;
    };
    let tier = tools_config.resolve_tier(path.as_str());
    if let Ok(tools) = load_tools_from_source_with_tier(source_str, path, registry, tier) {
        let mut reg = registry.write().unwrap();
        // unregister all previous tools from this path
        let old_names = reg.find_all_by_vfs_path(path);
        for name in &old_names {
            reg.unregister(name);
        }
        for tool in tools {
            reg.register(tool);
        }
    }
    // invalid source — leave previous versions registered
}

/// Unregister all synthesised tools whose VFS path matches `path`.
///
/// Called synchronously from the `on_scm_change` callback after a successful
/// delete. Handles multi-tool files (unregisters all tools from that file).
#[cfg(feature = "synthesised-tools")]
pub fn unregister_tool_at_path(
    registry: &Arc<RwLock<ToolRegistry>>,
    path: &VfsPath,
) {
    let mut reg = registry.write().unwrap();
    let names = reg.find_all_by_vfs_path(path);
    for name in names {
        reg.unregister(&name);
    }
}

// --- helpers -----------------------------------------------------------------

/// Extract a scheme string binding from a `ThreadLocalContext`.
#[cfg(feature = "synthesised-tools")]
fn extract_string(ctx: &ThreadLocalContext, name: &str) -> io::Result<String> {
    let val = ctx
        .evaluate(name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}: {e}")))?;
    val.as_string().map(str::to_string).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} is not a string"),
        )
    })
}

/// Convert a scheme params alist to a JSON Schema object.
///
/// Input (scheme): `((name . ((type . "string") (description . "..."))) ...)`
///
/// In tein::Value terms, `(name . attrs-list)` where `attrs-list` is a proper
/// list becomes `List([Symbol("name"), Pair("type","string"), ...])` — tein
/// flattens `(head . proper-list)` into a proper list at the Value level.
/// Each entry is `List([name-sym, attr-pair, ...])` where head is the param
/// name and the rest are attribute pairs.
///
/// Output: `{"type": "object", "properties": {"name": {"type": "string", "description": "..."}}, "required": [...]}`
///
/// All parameters are required unless the inner alist contains `(required . #f)`.
#[cfg(feature = "synthesised-tools")]
fn params_alist_to_json_schema(val: &Value) -> io::Result<serde_json::Value> {
    let items = match val {
        Value::List(items) => items,
        Value::Nil => {
            return Ok(serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }));
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tool-parameters must be a list, got: {other}"),
            ));
        }
    };

    let mut properties = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();

    for item in items {
        // Each entry is List([Symbol/String(name), attr-pair, ...]) — tein
        // flattens (name . attr-list) into a proper list when attr-list is proper.
        let elems = match item {
            Value::List(e) if !e.is_empty() => e,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("tool-parameters entry must be a non-empty list, got: {other}"),
                ));
            }
        };
        let name = match &elems[0] {
            Value::Symbol(s) | Value::String(s) => s.clone(),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("parameter name must be symbol or string, got: {other}"),
                ));
            }
        };
        // attrs: the tail of the list entry — pairs like (type . "string")
        let attrs = &elems[1..];

        let mut prop = serde_json::Map::new();
        let mut is_required = true;
        for attr in attrs {
            let (akey, aval) = match attr {
                Value::Pair(k, v) => {
                    let k = match k.as_ref() {
                        Value::Symbol(s) | Value::String(s) => s.as_str(),
                        _ => continue,
                    };
                    (k.to_string(), v.as_ref())
                }
                _ => continue,
            };
            match akey.as_str() {
                "required" => {
                    if let Value::Boolean(b) = aval {
                        is_required = *b;
                    }
                }
                "type" => {
                    if let Some(s) = aval.as_string() {
                        prop.insert("type".into(), serde_json::Value::String(s.to_string()));
                    }
                }
                "description" => {
                    if let Some(s) = aval.as_string() {
                        prop.insert(
                            "description".into(),
                            serde_json::Value::String(s.to_string()),
                        );
                    }
                }
                _ => {} // forward-compat: ignore unknown attrs
            }
        }

        if is_required {
            required.push(serde_json::Value::String(name.clone()));
        }
        properties.insert(name, serde_json::Value::Object(prop));
    }

    Ok(serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    }))
}

/// Convert a scheme value (from `call-tool` args) to a `serde_json::Value`.
///
/// Handles:
/// - alists (list of pairs) → JSON object
/// - plain lists → JSON array
/// - atoms → scalar JSON values
/// - `Nil` → `null`
///
/// Mutation site: if scheme value representation changes in tein, update here.
#[cfg(feature = "synthesised-tools")]
fn scheme_value_to_json(val: &Value) -> io::Result<serde_json::Value> {
    match val {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(n) => Ok(serde_json::json!(*n)),
        Value::Float(f) => Ok(serde_json::json!(*f)),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Symbol(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Pair(k, v) => {
            // single pair → two-element array (or alist-like handling by caller)
            Ok(serde_json::Value::Array(vec![
                scheme_value_to_json(k)?,
                scheme_value_to_json(v)?,
            ]))
        }
        Value::List(items) => {
            // if every item is a pair → alist → JSON object
            let all_pairs = items
                .iter()
                .all(|item| matches!(item, Value::Pair(_, _) | Value::List(_)));
            if all_pairs && !items.is_empty() {
                // attempt alist → object conversion
                let mut map = serde_json::Map::new();
                let mut is_alist = true;
                for item in items {
                    match item {
                        Value::Pair(k, v) => {
                            let key = match k.as_ref() {
                                Value::String(s) | Value::Symbol(s) => s.clone(),
                                _ => {
                                    is_alist = false;
                                    break;
                                }
                            };
                            map.insert(key, scheme_value_to_json(v)?);
                        }
                        Value::List(pair_items) if pair_items.len() == 2 => {
                            let key = match &pair_items[0] {
                                Value::String(s) | Value::Symbol(s) => s.clone(),
                                _ => {
                                    is_alist = false;
                                    break;
                                }
                            };
                            map.insert(key, scheme_value_to_json(&pair_items[1])?);
                        }
                        _ => {
                            is_alist = false;
                            break;
                        }
                    }
                }
                if is_alist {
                    return Ok(serde_json::Value::Object(map));
                }
            }
            // plain list → array
            let arr: io::Result<Vec<_>> = items.iter().map(scheme_value_to_json).collect();
            Ok(serde_json::Value::Array(arr?))
        }
        other => Err(io::Error::other(format!(
            "unsupported scheme value type: {other}"
        ))),
    }
}

/// Convert JSON tool-call args to a scheme alist for `tool-execute`.
///
/// `{"key": "val", ...}` → `(("key" . val) ...)` using `tein::json_value_to_value`.
/// Keys are scheme strings. Use `(assoc "key" args)` in scheme to extract.
#[cfg(feature = "synthesised-tools")]
fn json_args_to_scheme_alist(args: &serde_json::Value) -> io::Result<Value> {
    tein::json_value_to_value(args.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("args conversion: {e}")))
}

// --- tests -------------------------------------------------------------------

#[cfg(all(test, feature = "synthesised-tools"))]
mod tests {
    use super::*;
    use crate::vfs::{LocalBackend, Vfs, VfsCaller};
    use tempfile::TempDir;

    fn make_test_vfs() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let vfs = Vfs::new(Box::new(backend), "test-site-0000");
        (dir, vfs)
    }

    fn make_registry() -> Arc<RwLock<ToolRegistry>> {
        Arc::new(RwLock::new(ToolRegistry::new()))
    }

    const SCAN_TOOL: &str = r#"
(import (scheme base))
(define tool-name "scan_hello")
(define tool-description "says hello")
(define tool-parameters '())
(define (tool-execute args) "hello")
"#;

    #[tokio::test]
    async fn test_scan_and_register_loads_scm_file() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        // Write a .scm tool to /tools/shared/
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();
        vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes())
            .await
            .unwrap();

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await.unwrap();

        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "scan_hello should be registered after scan"
        );
    }

    #[tokio::test]
    async fn test_scan_and_register_ignores_non_scm() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        let path = VfsPath::new("/tools/shared/readme.txt").unwrap();
        vfs.write(VfsCaller::System, &path, b"not a tool")
            .await
            .unwrap();

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await.unwrap();
        assert_eq!(
            registry.read().unwrap().all().count(),
            0,
            "non-.scm file should not register"
        );
    }

    #[tokio::test]
    async fn test_scan_and_register_skips_missing_zone() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        // /tools/shared does not exist — should not error
        let result = scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(registry.read().unwrap().all().count(), 0);
    }

    #[tokio::test]
    async fn test_scan_and_register_skips_bad_source() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        // Write an invalid .scm file
        let path = VfsPath::new("/tools/shared/bad_tool.scm").unwrap();
        vfs.write(VfsCaller::System, &path, b"(define tool-name \"bad\")")
            .await
            .unwrap();

        // Should complete without error (bad file is silently skipped)
        let result = scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(
            registry.read().unwrap().all().count(),
            0,
            "invalid tool should not register"
        );
    }

    #[tokio::test]
    async fn test_scan_registers_from_home_zone() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        // write a tool to /tools/home/alice/
        let path = VfsPath::new("/tools/home/alice/my_tool.scm").unwrap();
        vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes())
            .await
            .unwrap();

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await.unwrap();
        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "home zone tool should be registered"
        );
    }

    #[tokio::test]
    async fn test_scan_registers_from_flocks_zone() {
        let (_dir, vfs) = make_test_vfs();
        let registry = make_registry();

        // write a tool to /tools/flocks/dev-team/
        let path = VfsPath::new("/tools/flocks/dev-team/helper.scm").unwrap();
        vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes())
            .await
            .unwrap();

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await.unwrap();
        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "flock zone tool should be registered"
        );
    }

    // --- hot-reload tests ---

    #[test]
    fn test_reload_tool_from_content_registers() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes(), &crate::config::ToolsConfig::default());

        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "tool should be registered after reload"
        );
    }

    #[test]
    fn test_reload_tool_from_content_updates_existing() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register first version
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes(), &crate::config::ToolsConfig::default());
        assert_eq!(
            registry
                .read()
                .unwrap()
                .get("scan_hello")
                .unwrap()
                .description,
            "says hello"
        );

        // Overwrite with updated description
        let updated = SCAN_TOOL.replace("says hello", "waves hello");
        reload_tool_from_content(&registry, &path, updated.as_bytes(), &crate::config::ToolsConfig::default());
        assert_eq!(
            registry
                .read()
                .unwrap()
                .get("scan_hello")
                .unwrap()
                .description,
            "waves hello",
            "description should be updated after reload"
        );
    }

    #[test]
    fn test_unregister_tool_at_path() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register first
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes(), &crate::config::ToolsConfig::default());
        assert!(registry.read().unwrap().get("scan_hello").is_some());

        // Unregister via path
        unregister_tool_at_path(&registry, &path);
        assert!(
            registry.read().unwrap().get("scan_hello").is_none(),
            "tool should be removed after unregister"
        );
    }

    #[test]
    fn test_unregister_nonexistent_path_is_noop() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/ghost.scm").unwrap();
        // Should not panic or error
        unregister_tool_at_path(&registry, &path);
        assert_eq!(registry.read().unwrap().all().count(), 0);
    }

    #[test]
    fn test_reload_invalid_source_preserves_existing() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register valid version first
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes(), &crate::config::ToolsConfig::default());
        assert!(registry.read().unwrap().get("scan_hello").is_some());

        // Try to reload with invalid source — should not remove the existing tool
        reload_tool_from_content(&registry, &path, b"(define tool-name \"bad\")", &crate::config::ToolsConfig::default());
        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "invalid reload should not remove existing valid tool"
        );
    }

    #[tokio::test]
    async fn test_vfs_write_triggers_hot_reload() {
        use crate::vfs::ScmChangeKind;

        let (_dir, mut vfs) = make_test_vfs();
        let registry = make_registry();

        let reg = Arc::clone(&registry);
        vfs.set_scm_change_callback(Arc::new(move |path, kind, content| match kind {
            ScmChangeKind::Write => {
                if let Some(bytes) = content {
                    reload_tool_from_content(&reg, path, bytes, &crate::config::ToolsConfig::default());
                }
            }
            ScmChangeKind::Delete => unregister_tool_at_path(&reg, path),
        }));

        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Write triggers registration
        vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes())
            .await
            .unwrap();
        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "tool should be registered after VFS write"
        );

        // Delete triggers unregistration
        vfs.delete(VfsCaller::System, &path).await.unwrap();
        assert!(
            registry.read().unwrap().get("scan_hello").is_none(),
            "tool should be removed after VFS delete"
        );
    }

    #[tokio::test]
    async fn test_vfs_write_non_tools_path_does_not_trigger() {
        let (_dir, mut vfs) = make_test_vfs();
        let triggered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&triggered);
        vfs.set_scm_change_callback(Arc::new(move |_, _, _| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        // Write to /shared/ (not /tools/) — callback must NOT fire
        let path = VfsPath::new("/shared/something.scm").unwrap();
        vfs.write(VfsCaller::System, &path, b"content")
            .await
            .unwrap();

        assert!(
            !triggered.load(std::sync::atomic::Ordering::SeqCst),
            "callback should not fire for non-tools paths"
        );
    }

    const SIMPLE_TOOL: &str = r#"
(import (scheme base))
(define tool-name "word_count")
(define tool-description "count words in text")
(define tool-parameters
  '((text . ((type . "string") (description . "text to count")))))
(define (count-words str)
  ; count space-separated words without string-split
  (let loop ((i 0) (in-word #f) (count 0))
    (if (>= i (string-length str))
        (if in-word (+ count 1) count)
        (let ((ch (string-ref str i)))
          (if (char=? ch #\space)
              (loop (+ i 1) #f (if in-word (+ count 1) count))
              (loop (+ i 1) #t count))))))
(define (tool-execute args)
  (let ((text (cdr (assoc "text" args))))
    (number->string (count-words text))))
"#;

    #[test]
    fn test_load_synthesised_tool_schema() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/word_count.scm").unwrap();
        let tool = load_tool_from_source(SIMPLE_TOOL, &path, &registry).unwrap();
        assert_eq!(tool.name, "word_count");
        assert_eq!(tool.description, "count words in text");
        assert_eq!(tool.category, ToolCategory::Synthesised);
        // parameters JSON schema
        let props = &tool.parameters["properties"];
        assert!(props["text"]["type"].as_str() == Some("string"));
        let req = tool.parameters["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("text")));
    }

    #[test]
    fn test_sandbox_import_scheme_base() {
        // sandboxed context starts with a null env — (import (scheme base)) is required
        // for even basic operations like cons/car/cdr/assoc/number->string
        let ctx = Context::builder()
            .standard_env()
            .sandboxed(Modules::Safe)
            .step_limit(10_000_000)
            .build_managed(|ctx| {
                ctx.evaluate("(import (scheme base))(define p (cons 1 2))")?;
                Ok(())
            })
            .unwrap();
        let r = ctx.evaluate("(car p)").unwrap();
        assert_eq!(r, Value::Integer(1));
    }

    #[test]
    fn test_assoc_with_string_keys() {
        // assoc is in (scheme base) — requires (import (scheme base)) in sandboxed context
        let ctx = Context::builder()
            .standard_env()
            .sandboxed(Modules::Safe)
            .step_limit(10_000_000)
            .build_managed(|ctx| {
                ctx.evaluate(
                    r#"
(import (scheme base))
(define r (assoc "text" '(("text" . "hello"))))
"#,
                )?;
                Ok(())
            })
            .unwrap();
        let r = ctx.evaluate("r").unwrap();
        // tein flattens ("text" . "hello") with a proper-list cdr: result is a pair
        assert!(
            matches!(r, Value::Pair(..) | Value::List(..)),
            "assoc returned: {r:?}"
        );
    }

    #[tokio::test]
    async fn test_execute_synthesised_tool() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/word_count.scm").unwrap();
        let tool = load_tool_from_source(SIMPLE_TOOL, &path, &registry).unwrap();
        let args = serde_json::json!({"text": "hello world foo"});

        if let ToolImpl::Synthesised {
            ref context,
            ref exec_binding,
            ..
        } = tool.r#impl
        {
            // Call json_args_to_scheme_alist + execute directly without a
            // ToolCallContext (this tool's tool-execute only uses args).
            let args_alist = json_args_to_scheme_alist(&args).unwrap();
            let exec_fn = context.evaluate(exec_binding).unwrap();
            let result = context.call(&exec_fn, &[args_alist]).unwrap();
            assert_eq!(result.as_string(), Some("3"));
        } else {
            panic!("expected Synthesised impl");
        }
    }

    #[test]
    fn test_load_tool_missing_bindings() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/bad.scm").unwrap();
        let bad = "(define tool-name \"oops\")"; // missing other bindings
        let result = load_tool_from_source(bad, &path, &registry);
        assert!(result.is_err());
    }

    #[test]
    fn test_params_alist_empty() {
        let schema = params_alist_to_json_schema(&Value::Nil).unwrap();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_params_alist_optional_param() {
        // tein flattens (name . attr-list) into List([name, attr1, attr2, ...])
        // param with (required . #f) should not appear in required list
        let val = Value::List(vec![Value::List(vec![
            Value::Symbol("opt".into()),
            Value::Pair(
                Box::new(Value::Symbol("type".into())),
                Box::new(Value::String("string".into())),
            ),
            Value::Pair(
                Box::new(Value::Symbol("required".into())),
                Box::new(Value::Boolean(false)),
            ),
        ])]);
        let schema = params_alist_to_json_schema(&val).unwrap();
        let req = schema["required"].as_array().unwrap();
        assert!(req.is_empty());
        assert!(schema["properties"]["opt"]["type"].as_str() == Some("string"));
    }

    #[test]
    fn test_json_args_to_scheme_alist_object() {
        let args = serde_json::json!({"key": "val"});
        let v = json_args_to_scheme_alist(&args).unwrap();
        // should be List([Pair(String("key"), String("val"))])
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    Value::Pair(k, v) => {
                        assert_eq!(*k.as_ref(), Value::String("key".into()));
                        assert_eq!(*v.as_ref(), Value::String("val".into()));
                    }
                    other => panic!("expected pair, got {other:?}"),
                }
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn test_scheme_value_to_json_alist() {
        let alist = Value::List(vec![Value::Pair(
            Box::new(Value::String("cmd".into())),
            Box::new(Value::String("ls".into())),
        )]);
        let json = scheme_value_to_json(&alist).unwrap();
        assert_eq!(json, serde_json::json!({"cmd": "ls"}));
    }

    #[test]
    fn test_scheme_value_to_json_nil() {
        assert_eq!(
            scheme_value_to_json(&Value::Nil).unwrap(),
            serde_json::Value::Null
        );
    }

    #[test]
    fn test_scheme_value_to_json_scalars() {
        assert_eq!(
            scheme_value_to_json(&Value::Boolean(true)).unwrap(),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            scheme_value_to_json(&Value::Integer(42)).unwrap(),
            serde_json::json!(42i64)
        );
        assert_eq!(
            scheme_value_to_json(&Value::String("hi".into())).unwrap(),
            serde_json::Value::String("hi".into())
        );
    }

    // --- define-tool / multi-tool tests ---

    const MULTI_TOOL_SOURCE: &str = r#"
(import (scheme base))
(import (harness tools))

(define-tool greet
  (description "greets someone")
  (parameters '((name . ((type . "string") (description . "who to greet")))))
  (execute (lambda (args)
    (string-append "hello " (cdr (assoc "name" args))))))

(define-tool farewell
  (description "says goodbye")
  (parameters '((name . ((type . "string") (description . "who to farewell")))))
  (execute (lambda (args)
    (string-append "bye " (cdr (assoc "name" args))))))
"#;

    #[test]
    fn test_load_multiple_tools_from_define_tool() {
        let registry = make_registry();
        let vfs_path = VfsPath::new("/tools/shared/multi.scm").unwrap();
        let tools = load_tools_from_source(MULTI_TOOL_SOURCE, &vfs_path, &registry).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "greet");
        assert_eq!(tools[1].name, "farewell");
    }

    #[test]
    fn test_load_tools_backwards_compat_single_tool() {
        let registry = make_registry();
        let vfs_path = VfsPath::new("/tools/shared/old.scm").unwrap();
        let tools = load_tools_from_source(SCAN_TOOL, &vfs_path, &registry).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "scan_hello");
    }

    #[test]
    fn test_hot_reload_multi_tool_file() {
        let registry = make_registry();

        let source_v1 = r#"
(import (scheme base))
(import (harness tools))
(define-tool tool_a (description "a") (parameters '()) (execute (lambda (args) "a")))
(define-tool tool_b (description "b") (parameters '()) (execute (lambda (args) "b")))
"#;
        let path = VfsPath::new("/tools/shared/multi.scm").unwrap();
        reload_tool_from_content(&registry, &path, source_v1.as_bytes(), &crate::config::ToolsConfig::default());

        assert!(registry.read().unwrap().get("tool_a").is_some());
        assert!(registry.read().unwrap().get("tool_b").is_some());

        // update: remove tool_b, add tool_c
        let source_v2 = r#"
(import (scheme base))
(import (harness tools))
(define-tool tool_a (description "a v2") (parameters '()) (execute (lambda (args) "a2")))
(define-tool tool_c (description "c") (parameters '()) (execute (lambda (args) "c")))
"#;
        reload_tool_from_content(&registry, &path, source_v2.as_bytes(), &crate::config::ToolsConfig::default());

        let reg = registry.read().unwrap();
        assert!(reg.get("tool_a").is_some());
        assert!(reg.get("tool_b").is_none(), "tool_b should be unregistered");
        assert!(reg.get("tool_c").is_some());
    }

    // --- sandbox tier tests ---

    #[test]
    fn test_tier1_rejects_unsafe_imports() {
        // (scheme regex) has default_safe: false — not in Modules::Safe allowlist
        let source = r#"
(import (scheme base))
(import (scheme regex))  ; blocked by Modules::Safe (default_safe: false)
(define tool-name "bad_tool")
(define tool-description "tries unsafe import")
(define tool-parameters '())
(define (tool-execute args) "should not load")
"#;
        let vfs_path = VfsPath::new("/tools/shared/bad.scm").unwrap();
        let registry = make_registry();
        let result = load_tools_from_source_with_tier(
            source,
            &vfs_path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        );
        assert!(result.is_err(), "sandboxed tier should reject (scheme regex)");
    }

    #[test]
    fn test_tier2_allows_full_scheme() {
        let source = r#"
(import (scheme base))
(define tool-name "tier2_tool")
(define tool-description "uses full scheme")
(define tool-parameters '())
(define (tool-execute args) "full scheme works")
"#;
        let vfs_path = VfsPath::new("/tools/shared/full.scm").unwrap();
        let registry = make_registry();
        // tier 2 — no sandboxing; just verify it loads without error
        let result = load_tools_from_source_with_tier(
            source,
            &vfs_path,
            &registry,
            crate::config::SandboxTier::Unsandboxed,
        );
        assert!(result.is_ok(), "unsandboxed tier should allow loading");
    }

    // --- integration test: harness import ---

    #[test]
    fn test_integration_harness_import_works() {
        // verify (import (harness tools)) succeeds and call-tool is available
        let registry = make_registry();
        let source = r#"
(import (scheme base))
(import (harness tools))
(define tool-name "harness_test")
(define tool-description "tests harness import")
(define tool-parameters '())
(define (tool-execute args) "harness ok")
"#;
        let vfs_path = VfsPath::new("/tools/shared/harness_test.scm").unwrap();
        let tool = load_tool_from_source(source, &vfs_path, &registry).unwrap();
        assert_eq!(tool.name, "harness_test");
    }
}
