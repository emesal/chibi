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
//! `Handle::current().block_on()`. The registry and call context are stashed in
//! `BRIDGE_CALL_CTX` before each invocation and cleared after via a guard.
//! The registry is embedded in `ToolImpl::Synthesised` and flows through
//! `execute_synthesised` → `CallContextGuard`, so concurrent calls never
//! overwrite each other's registry.
//!
//! Each synthesised tool gets its own sandboxed tein context, shared via
//! `Arc` for concurrent dispatch. All access goes through `ThreadLocalContext`,
//! which is `Send + Sync`.

#[cfg(feature = "synthesised-tools")]
use std::sync::{Arc, Mutex, RwLock};

#[cfg(feature = "synthesised-tools")]
use tein::{Context, ThreadLocalContext, Value, sandbox::Modules};

use std::io;

use crate::tools::registry::{ToolCall, ToolRegistry};
use crate::tools::{Tool, ToolCategory, ToolImpl, ToolMetadata};
use crate::vfs::{Vfs, VfsCaller, VfsEntryKind, VfsPath}; // Vfs+VfsCaller used in scan_and_register

// --- output capture types ----------------------------------------------------

/// Shared write buffer for capturing scheme output port data.
/// Cloned `Arc` handles go into tein's custom output ports; the same
/// `Arc` is held by `TeinSession` for reading after evaluation.
#[cfg(feature = "synthesised-tools")]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

#[cfg(feature = "synthesised-tools")]
impl std::io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Result of a scheme evaluation with captured output streams.
///
/// Holds the raw `Result<Value>` so callers can stringify as needed.
/// `format_eval()` uses `to_string()` (for scheme_eval).
/// `format_tool()` uses `as_string()` unwrap (for synthesised tools that
/// conventionally return scheme strings).
///
/// Both format methods produce:
/// ```text
/// result: <value or "error: ...">
/// stdout: <captured stdout or "(empty)">
/// stderr: <captured stderr or "(empty)">
/// ```
#[cfg(feature = "synthesised-tools")]
pub(crate) struct CapturedOutput {
    value: tein::Result<tein::Value>,
    pub stdout: String,
    pub stderr: String,
}

/// Shared formatting for structured output. Produces:
/// `"result: <value_str>\nstdout: <stdout or "(empty)">\nstderr: <stderr or "(empty)">"`
#[cfg(feature = "synthesised-tools")]
fn format_output(value_str: &str, stdout: &str, stderr: &str) -> String {
    format!(
        "result: {}\nstdout: {}\nstderr: {}",
        value_str,
        if stdout.is_empty() { "(empty)" } else { stdout },
        if stderr.is_empty() { "(empty)" } else { stderr },
    )
}

#[cfg(feature = "synthesised-tools")]
impl CapturedOutput {
    /// Format for `scheme_eval` -- stringifies value with `to_string()`.
    pub fn format_eval(&self) -> String {
        let value_str = match &self.value {
            Ok(val) => val.to_string(),
            Err(e) => format!("error: {e}"),
        };
        format_output(&value_str, &self.stdout, &self.stderr)
    }

    /// Format for synthesised tools -- unwraps scheme strings via `as_string()`,
    /// falling back to `to_string()` for non-string values.
    pub fn format_tool(&self) -> String {
        let value_str = match &self.value {
            Ok(val) => match val.as_string() {
                Some(s) => s.to_string(),
                None => val.to_string(),
            },
            Err(e) => format!("error: {e}"),
        };
        format_output(&value_str, &self.stdout, &self.stderr)
    }
}

/// Wrapper around `ThreadLocalContext` that captures stdout/stderr output.
///
/// Created by `build_tein_context`. The init closure wires `SharedWriter`-backed
/// output ports as `current-output-port` and `current-error-port`. The ports
/// survive across evaluations (tein's `set_current_output_port` persists).
///
/// Thread safety: `ThreadLocalContext` serialises all calls via its internal
/// channel mutex, so no interleaving is possible between drain and read in
/// `with_capture`.
#[cfg(feature = "synthesised-tools")]
pub struct TeinSession {
    ctx: ThreadLocalContext,
    stdout_buf: Arc<Mutex<Vec<u8>>>,
    stderr_buf: Arc<Mutex<Vec<u8>>>,
}

#[cfg(feature = "synthesised-tools")]
impl TeinSession {
    /// Run a closure with stdout/stderr capture.
    ///
    /// Drain-eval-flush-read cycle:
    /// 1. Clear buffers (drain stale output from prior calls)
    /// 2. Run the closure (evaluate or call)
    /// 3. Flush scheme ports to ensure buffers are complete
    /// 4. Read captured output
    ///
    /// The closure receives `&ThreadLocalContext` for `evaluate` or `call`.
    pub(crate) fn with_capture<F>(&self, f: F) -> CapturedOutput
    where
        F: FnOnce(&ThreadLocalContext) -> tein::Result<tein::Value>,
    {
        // 1. flush-then-drain: flush chibi's internal port buffers first (emitting
        //    any stale buffered data into SharedWriter), then discard. This ensures
        //    the capture window starts clean — no bleed from previous calls.
        //    flush-output-port is R7RS (scheme base); flush-output is chibi-only and
        //    unavailable in sandboxed contexts.
        //    Flush errors are non-fatal but mean the drain may be incomplete; stale
        //    output could bleed into this call's capture. No tracing dep is available,
        //    so errors are silently discarded — investigate if bleed is observed.
        let _ = self
            .ctx
            .evaluate("(flush-output-port (current-output-port))");
        let _ = self
            .ctx
            .evaluate("(flush-output-port (current-error-port))");
        self.stdout_buf.lock().unwrap().clear();
        self.stderr_buf.lock().unwrap().clear();

        // 2. run
        let value = f(&self.ctx);

        // 3. flush -- push any remaining buffered output into SharedWriter.
        //    Same caveat: flush errors mean captured output may be truncated.
        let _ = self
            .ctx
            .evaluate("(flush-output-port (current-output-port))");
        let _ = self
            .ctx
            .evaluate("(flush-output-port (current-error-port))");

        // 4. read
        let stdout = String::from_utf8_lossy(&self.stdout_buf.lock().unwrap()).to_string();
        let stderr = String::from_utf8_lossy(&self.stderr_buf.lock().unwrap()).to_string();

        CapturedOutput {
            value,
            stdout,
            stderr,
        }
    }

    /// Delegate to inner context for internal calls (e.g. setting `%context-name%`,
    /// resolving bindings) that don't need output capture.
    pub(crate) fn evaluate(&self, code: &str) -> tein::Result<tein::Value> {
        self.ctx.evaluate(code)
    }

    /// Delegate to inner context for calling procedures without capture.
    /// Use when stdout/stderr output is not needed (e.g. internal resolution,
    /// hook dispatch where only the return value matters).
    pub(crate) fn call(
        &self,
        proc: &tein::Value,
        args: &[tein::Value],
    ) -> tein::Result<tein::Value> {
        self.ctx.call(proc, args)
    }
}

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
pub(crate) const HARNESS_TOOLS_MODULE: &str = r#"
(define-library (harness tools)
  (import (scheme base))
  (export call-tool)
  (begin
    ;; call-tool is injected as a foreign fn before this module loads.
    ;; re-export it so (import (harness tools)) provides it.
    #t))
"#;

/// Scheme source for the `(harness io)` module (privileged IO, unsandboxed only).
///
/// The five `io-*` foreign functions are registered before this module so that
/// `(import (harness io))` re-exports them cleanly. Only available when
/// `build_tein_context` is called with `SandboxTier::Unsandboxed`.
///
/// Mutation site: if the exported function set changes, update `build_tein_context`
/// registration block and `docs/plugins.md`.
#[cfg(feature = "synthesised-tools")]
const HARNESS_IO_MODULE: &str = r#"
(define-library (harness io)
  (import (scheme base))
  (export io-read io-write io-append io-list io-exists? io-delete)
  (begin #t))
"#;

/// Scheme source for the `(harness hooks)` module.
///
/// `register-hook` is defined at top level in `HARNESS_PREAMBLE` so it can
/// mutate `%hook-registry%`. This library re-exports it for clean imports.
///
/// Mutation site: if `register-hook` signature changes, update
/// `extract_hook_registrations` which reads `%hook-registry%` entries.
#[cfg(feature = "synthesised-tools")]
pub(crate) const HARNESS_HOOKS_MODULE: &str = r#"
(define-library (harness hooks)
  (import (scheme base))
  (export register-hook)
  (begin
    ;; register-hook is defined in HARNESS_PREAMBLE (top-level).
    ;; re-export it so (import (harness hooks)) provides it.
    #t))
"#;

/// Scheme source for the `(harness docs)` module.
///
/// Re-exports `hooks-docs` and `harness-tools-docs` from their top-level
/// preamble bindings. Both bindings are defined in `HARNESS_PREAMBLE` at the
/// top level (not inside a library) so they are accessible before any imports,
/// and this module provides the canonical named import path.
///
/// `hooks-docs` covers all hook points with payload/return contracts.
/// `harness-tools-docs` covers define-tool, call-tool, register-hook, etc.
///
/// Canonical usage: `(import (harness docs))` then `(describe hooks-docs)`.
/// Note: `(module-exports '(harness docs))` will error — runtime-registered
/// modules are absent from tein's build-time MODULE_EXPORTS table.
#[cfg(feature = "synthesised-tools")]
pub(crate) const HARNESS_DOCS_MODULE: &str = r#"
(define-library (harness docs)
  (import (scheme base))
  (export hooks-docs harness-tools-docs)
  ;; Both bindings are pre-defined at top level by HARNESS_PREAMBLE (evaluated
  ;; before module registration), so this library intentionally re-exports
  ;; top-level bindings without defining them locally. Same pattern as
  ;; HARNESS_HOOKS_MODULE re-exporting the top-level `register-hook`.
  (begin #t))
"#;

/// Top-level scheme preamble evaluated in every synthesised tool context.
///
/// Defines `%tool-registry%`, `%hook-registry%`, `%context-name%`, `define-tool`,
/// `register-hook`, `harness-tools-docs`, and `hooks-docs` at the top level.
///
/// `hooks-docs` is generated from `HOOK_METADATA` — the canonical single source
/// of truth for hook contracts. `harness-tools-docs` covers the harness API.
/// Both are re-exported via `(harness docs)` as the canonical access path.
///
/// Call `(import (harness docs))` then `(describe hooks-docs)` to list all hooks,
/// or `(module-doc hooks-docs 'pre_message)` for a specific hook's contract.
/// Note: `(describe X)` takes an alist directly, not a symbol.
///
/// `define-tool` must be top-level (not inside a library) so its `set!` of
/// `%tool-registry%` affects the top-level binding that rust reads post-evaluation.
///
/// Mutation site: if `define-tool` syntax changes, update `extract_multi_tools`
/// which parses `%tool-registry%` entries. If `harness-tools-docs` entries change,
/// update `chibi.md` and `AGENTS.md` accordingly.
///
/// Built once via `LazyLock` since `hooks-docs` is generated at runtime from
/// `HOOK_METADATA`. The allocation is reused across all context builds.
#[cfg(feature = "synthesised-tools")]
pub(crate) static HARNESS_PREAMBLE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let hooks_docs_alist = crate::tools::hooks::generate_hooks_docs_alist();
    format!(
        r#"
(import (scheme base))

;; accumulates define-tool entries. each entry is a list:
;; (name-string description-string params-value execute-procedure)
(define %tool-registry% '())

;; accumulates hook registrations. each entry is a list:
;; (hook-name-string handler-procedure)
;; rust reads %hook-registry% after evaluation to populate Tool.hooks.
(define %hook-registry% '())

;; name of the calling context — mutated by execute_synthesised before each call.
;; plugins read this to resolve /home/<ctx>/... VFS paths.
(define %context-name% "")

;; docs alist for public harness APIs — use (import (harness docs)) then
;; (describe harness-tools-docs) or (module-doc harness-tools-docs 'define-tool).
;; follows the same convention as introspect-docs, json-docs, etc.
;; note: (describe X) takes an alist directly, NOT a symbol.
(define harness-tools-docs
  '((__module__ . "harness tools")
    (define-tool . "macro: (define-tool name (description DESC) (parameters PARAMS-ALIST) (execute (lambda (args) ...))) — registers a persistent tool; args is ((\"key\" . val) ...) alist")
    (call-tool . "procedure: (call-tool NAME ARGS-ALIST) -> string — invoke another registered tool; NAME is a string, ARGS-ALIST is ((\"key\" . \"val\") ...)")
    (register-hook . "procedure: (register-hook HOOK-SYMBOL HANDLER) — register a hook callback; HOOK-SYMBOL e.g. 'pre_vfs_write, HANDLER is (lambda (payload) ...)")
    (generate-id . "procedure: (generate-id) -> string — returns an 8-hex-char random identifier (uuid v4 prefix)")
    (current-timestamp . "procedure: (current-timestamp) -> string — returns current UTC time as \"YYYYMMDD-HHMMz\"")))

;; docs alist for all hook points — use (import (harness docs)) then
;; (describe hooks-docs) to list all hooks, or (module-doc hooks-docs 'pre_message).
;; generated from HOOK_METADATA (hooks.rs) — single source of truth.
(define hooks-docs
  {hooks_docs_alist})

;; registers a tool: appends to %tool-registry% in definition order (LIFO via cons).
;; rust reads %tool-registry% after evaluation; non-empty → multi-tool mode.
(define-syntax define-tool
  (syntax-rules (description category summary-params parameters execute)
    ;; pattern 1: baseline (no category, no summary-params)
    ((define-tool name
       (description desc)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler #f #f)
             %tool-registry%)))
    ;; pattern 2: category only
    ((define-tool name
       (description desc)
       (category cat)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler cat #f)
             %tool-registry%)))
    ;; pattern 3: summary-params only
    ((define-tool name
       (description desc)
       (summary-params sp)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler #f sp)
             %tool-registry%)))
    ;; pattern 4: category + summary-params
    ((define-tool name
       (description desc)
       (category cat)
       (summary-params sp)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler cat sp)
             %tool-registry%)))))

;; registers a hook handler for a given hook point.
;; hook-name is a symbol (e.g. 'pre_vfs_write).
;; handler is a procedure taking one argument (the hook payload as an alist)
;; and returning an alist (or '() for no-op).
(define (register-hook hook-name handler)
  (set! %hook-registry%
    (cons (list (symbol->string hook-name) handler)
          %hook-registry%)))
"#,
        hooks_docs_alist = hooks_docs_alist,
    )
});

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
    /// Tokio runtime handle from the caller thread. Used by `call_tool_fn` to
    /// schedule async work from the tein worker thread (which is not a tokio thread
    /// and cannot use `block_in_place`).
    runtime_handle: tokio::runtime::Handle,
    /// Shared tool registry. Used by `call_tool_fn` for per-call dispatch.
    /// Embedded per-call so concurrent tests never overwrite each other's registry.
    registry: Arc<RwLock<ToolRegistry>>,
}

// SAFETY: the pointers in `ActiveCallContext` are only dereferenced on the
// tein worker thread while `CallContextGuard` is alive (i.e. within a single
// `execute_synthesised` call on that thread). No other thread accesses them.
#[cfg(feature = "synthesised-tools")]
unsafe impl Send for ActiveCallContext {}

/// Per-tein-worker-thread call context. Keyed by the tein worker thread's `ThreadId`
/// so concurrent synthesised tool calls from different contexts never collide.
///
/// `call_tool_fn` runs on the tein worker thread, so `std::thread::current().id()`
/// gives the correct lookup key. `CallContextGuard` inserts on set and removes on drop.
#[cfg(feature = "synthesised-tools")]
static BRIDGE_CALL_CTX: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<std::thread::ThreadId, ActiveCallContext>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// RAII guard that inserts/removes an entry in the thread-ID-keyed `BRIDGE_CALL_CTX` map.
/// Used in `execute_synthesised` to make the context available to the `call-tool`
/// bridge without threading it through tein's FFI boundary.
#[cfg(feature = "synthesised-tools")]
pub(crate) struct CallContextGuard {
    thread_id: std::thread::ThreadId,
}

#[cfg(feature = "synthesised-tools")]
impl CallContextGuard {
    pub(crate) fn set(
        ctx: &crate::tools::registry::ToolCallContext<'_>,
        registry: Arc<RwLock<ToolRegistry>>,
        thread_id: std::thread::ThreadId,
    ) -> Self {
        BRIDGE_CALL_CTX.lock().unwrap().insert(
            thread_id,
            ActiveCallContext {
                app: ctx.app as *const _,
                context_name: ctx.context_name.to_string(),
                config: ctx.config as *const _,
                project_root: ctx.project_root.to_path_buf(),
                vfs: ctx.vfs as *const _,
                vfs_caller_context: match ctx.vfs_caller {
                    VfsCaller::Context(name) => name.to_string(),
                    VfsCaller::System => String::new(),
                },
                runtime_handle: tokio::runtime::Handle::current(),
                registry,
            },
        );
        CallContextGuard { thread_id }
    }

    /// Set from a `TeinHookContext` — used during hook dispatch to enable
    /// `call-tool` and `(harness io)` from tein hook callbacks.
    ///
    /// Must be called from a tokio thread (uses `Handle::current()`).
    pub(crate) fn set_from_hook_ctx(
        ctx: &crate::tools::hooks::TeinHookContext<'_>,
        worker_thread_id: std::thread::ThreadId,
    ) -> Self {
        BRIDGE_CALL_CTX.lock().unwrap().insert(
            worker_thread_id,
            ActiveCallContext {
                app: ctx.app as *const _,
                context_name: ctx.context_name.to_string(),
                config: ctx.config as *const _,
                project_root: ctx.project_root.to_path_buf(),
                vfs: ctx.vfs as *const _,
                vfs_caller_context: String::new(), // System caller for hook dispatch
                runtime_handle: tokio::runtime::Handle::current(),
                registry: Arc::clone(&ctx.registry),
            },
        );
        CallContextGuard {
            thread_id: worker_thread_id,
        }
    }
}

#[cfg(feature = "synthesised-tools")]
impl Drop for CallContextGuard {
    fn drop(&mut self) {
        BRIDGE_CALL_CTX.lock().unwrap().remove(&self.thread_id);
    }
}

// --- call-tool foreign function bridge ---------------------------------------

/// The `call-tool` foreign function: `(call-tool name args-alist) → string`
///
/// Reads `BRIDGE_CALL_CTX` (which carries the registry for the current call).
/// Converts the scheme alist args to JSON, looks up the tool, and dispatches
/// via `ToolRegistry::dispatch_impl` on the current tokio runtime.
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

    // Extract all needed data from the per-thread context while holding the lock,
    // then drop the lock before dispatching (which may block_on async code).
    let (
        app_ptr,
        context_name,
        config_ptr,
        project_root,
        vfs_ptr,
        vfs_caller_str,
        runtime_handle,
        registry,
    ) = {
        let tid = std::thread::current().id();
        let guard = BRIDGE_CALL_CTX.lock().unwrap();
        let active = guard.get(&tid).ok_or_else(|| {
            "call-tool: no active call context for this thread (called outside tool execute?)"
                .to_string()
        })?;
        (
            active.app,
            active.context_name.clone(),
            active.config,
            active.project_root.clone(),
            active.vfs,
            active.vfs_caller_context.clone(),
            active.runtime_handle.clone(),
            Arc::clone(&active.registry),
        )
        // guard drops here, releasing the lock
    };

    // SAFETY: pointers are valid for the duration of execute_synthesised,
    // which holds CallContextGuard that set them. We reconstruct a
    // ToolCallContext only for dispatch — no storage beyond this fn.
    let call_ctx = unsafe {
        crate::tools::registry::ToolCallContext {
            app: &*app_ptr,
            context_name: &context_name,
            config: &*config_ptr,
            project_root: &project_root,
            vfs: &*vfs_ptr,
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

    // Use the captured runtime handle directly rather than `vfs_block_on`:
    // the tein worker thread is not a tokio thread, so `block_in_place`
    // (which `vfs_block_on` uses) would panic. The captured handle from
    // `CallContextGuard::set` (which runs on a tokio thread) is safe to
    // `block_on` from this non-tokio worker thread. See commit 2016b193
    // for the original `vfs_block_on` change and why this context differs.
    runtime_handle
        .block_on(ToolRegistry::dispatch_impl(
            tool_impl, &name, &json_args, &call_ctx,
        ))
        .map_err(|e| format!("tool error: {e}"))
}

/// `(generate-id)` harness helper — returns an 8-hex-char string from uuid v4.
/// Provides ~4 billion possible values; sufficient for task IDs within a
/// workspace. Not a cryptographic identifier.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "generate-id")]
fn generate_id_fn() -> String {
    let id = uuid::Uuid::new_v4();
    id.simple().to_string()[..8].to_string()
}

/// `(current-timestamp)` harness helper — returns `"YYYYMMDD-HHMMz"` UTC.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "current-timestamp")]
fn current_timestamp_fn() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi) = secs_to_ymdhmz(secs);
    format!("{:04}{:02}{:02}-{:02}{:02}z", y, mo, d, h, mi)
}

// --- harness io foreign function bridge --------------------------------------

/// Helper: extract VFS handle and runtime handle from `BRIDGE_CALL_CTX` for the
/// current thread. Returns `(runtime_handle, vfs_ptr)` after dropping the lock.
///
/// Called from IO FFI functions running on the tein worker thread.
#[cfg(feature = "synthesised-tools")]
fn io_bridge_ctx() -> Result<(tokio::runtime::Handle, *const Vfs), String> {
    let tid = std::thread::current().id();
    let guard = BRIDGE_CALL_CTX.lock().unwrap();
    let active = guard.get(&tid).ok_or_else(|| {
        "harness io: no active call context (called outside tool execute or hook dispatch?)"
            .to_string()
    })?;
    let handle = active.runtime_handle.clone();
    let vfs = active.vfs;
    drop(guard);
    Ok((handle, vfs))
}

/// `(io-read path)` — read a file. Returns string content or `#f` if not found.
///
/// Path dispatch: `"vfs://..."` → VFS with `VfsCaller::System`;
/// bare absolute path → `tokio::fs::read_to_string`.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-read")]
fn io_read_fn(path: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: vfs_ptr comes from BRIDGE_CALL_CTX, set by CallContextGuard for the
        // duration of execute_synthesised or hook dispatch. The Vfs lives in AppState
        // (Arc-owned, session lifetime), so the reference is valid as long as the guard
        // is alive.
        let vfs = unsafe { &*vfs_ptr };
        match handle.block_on(vfs.read(VfsCaller::System, &vp)) {
            Ok(bytes) => Ok(Value::String(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::Boolean(false)),
            Err(e) => Err(e.to_string()),
        }
    } else {
        match handle.block_on(tokio::fs::read_to_string(&path)) {
            Ok(s) => Ok(Value::String(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::Boolean(false)),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// `(io-write path data)` — write a file. Returns `#t` on success, raises on error.
///
/// VFS writes via `VfsCaller::System` (goes through `LocalBackend::write` →
/// `safe_io::atomic_write`). Bare FS writes also use `safe_io::atomic_write`
/// via `spawn_blocking`, matching the VFS backend's safety guarantees.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-write")]
fn io_write_fn(path: String, data: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: see io_read_fn — same pointer provenance and lifetime contract.
        let vfs = unsafe { &*vfs_ptr };
        handle
            .block_on(vfs.write(VfsCaller::System, &vp, data.as_bytes()))
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    } else {
        // The tein worker thread is a plain OS thread (not a tokio task), so a
        // direct blocking call to safe_io::atomic_write is correct here.
        // handle is only needed for VFS operations; bare FS writes are sync.
        let _ = handle; // suppress unused warning in non-VFS branch
        crate::safe_io::atomic_write(std::path::Path::new(&path), data.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    }
}

/// `(io-append path data)` — append to a file. Returns `#t` on success, raises on error.
///
/// VFS append via `VfsCaller::System`. Bare FS appends match `LocalBackend::append`:
/// `create(true) + append(true)` — creates the file if missing, no fsync required
/// (appends are idempotent and not crash-atomic by design, same as VFS).
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-append")]
fn io_append_fn(path: String, data: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: see io_read_fn — same pointer provenance and lifetime contract.
        let vfs = unsafe { &*vfs_ptr };
        handle
            .block_on(vfs.append(VfsCaller::System, &vp, data.as_bytes()))
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    } else {
        use tokio::io::AsyncWriteExt;
        handle
            .block_on(async {
                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await?;
                file.write_all(data.as_bytes()).await
            })
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    }
}

/// `(io-list path)` — list directory entries. Returns list of name strings sorted
/// lexicographically, or empty list if path not found.
///
/// Both VFS and bare FS results are sorted to guarantee consistent ordering across
/// backends (VFS via `LocalBackend` uses `read_dir` which is OS-order; bare FS
/// likewise).
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-list")]
fn io_list_fn(path: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: see io_read_fn — same pointer provenance and lifetime contract.
        let vfs = unsafe { &*vfs_ptr };
        match handle.block_on(vfs.list(VfsCaller::System, &vp)) {
            Ok(mut entries) => {
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                let names: Vec<Value> =
                    entries.into_iter().map(|e| Value::String(e.name)).collect();
                Ok(Value::List(names))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::List(vec![])),
            Err(e) => Err(e.to_string()),
        }
    } else {
        match handle.block_on(async {
            let mut entries = Vec::new();
            let mut dir = tokio::fs::read_dir(&path).await?;
            while let Some(entry) = dir.next_entry().await? {
                if let Some(name) = entry.file_name().to_str() {
                    entries.push(name.to_string());
                }
            }
            entries.sort();
            Ok::<_, io::Error>(entries)
        }) {
            Ok(entries) => Ok(Value::List(
                entries.into_iter().map(Value::String).collect(),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::List(vec![])),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// `(io-exists? path)` — check if a path exists. Returns `#t` or `#f`.
///
/// VFS check via `VfsCaller::System`. Filesystem check via `tokio::fs::metadata`.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-exists?")]
fn io_exists_fn(path: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: see io_read_fn — same pointer provenance and lifetime contract.
        let vfs = unsafe { &*vfs_ptr };
        let exists = handle
            .block_on(vfs.exists(VfsCaller::System, &vp))
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(exists))
    } else {
        let exists = handle.block_on(tokio::fs::metadata(&path)).is_ok();
        Ok(Value::Boolean(exists))
    }
}

/// `(io-delete path)` — delete a file. Returns `#t` on success, raises on error.
///
/// VFS paths use `Vfs::delete(VfsCaller::System)`. Bare filesystem paths use
/// `tokio::fs::remove_file`. Unsandboxed tier only.
#[cfg(feature = "synthesised-tools")]
#[tein::tein_fn(name = "io-delete")]
fn io_delete_fn(path: String) -> Result<Value, String> {
    let (handle, vfs_ptr) = io_bridge_ctx()?;
    if let Some(vfs_path_str) = path.strip_prefix("vfs://") {
        let vp = VfsPath::new(vfs_path_str).map_err(|e| e.to_string())?;
        // SAFETY: see io_read_fn — same pointer provenance and lifetime contract.
        let vfs = unsafe { &*vfs_ptr };
        handle
            .block_on(vfs.delete(VfsCaller::System, &vp))
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    } else {
        handle
            .block_on(async { tokio::fs::remove_file(&path).await })
            .map_err(|e| e.to_string())?;
        Ok(Value::Boolean(true))
    }
}

// --- loader ------------------------------------------------------------------

/// Decompose Unix epoch seconds into (year, month, day, hour, minute) UTC.
///
/// Used by the `current-timestamp` harness helper to avoid a chrono dependency
/// in the synthesised-tools module path.
#[cfg(feature = "synthesised-tools")]
fn secs_to_ymdhmz(mut s: u64) -> (u32, u32, u32, u32, u32) {
    s /= 60; // discard seconds
    let mi = (s % 60) as u32;
    s /= 60;
    let h = (s % 24) as u32;
    s /= 24;
    // Gregorian calendar decomposition
    let z = s + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if mo <= 2 { 1 } else { 0 }) as u32;
    (y, mo, d, h, mi)
}

/// Build a `TeinSession` for a synthesised tool, registering
/// `call-tool`, the harness preamble, and `(harness tools)` module.
///
/// Returns the session and the tein worker thread's `ThreadId` (captured
/// during init). The thread ID is the key for `BRIDGE_CALL_CTX` lookups.
///
/// Sandbox behaviour depends on `tier`:
/// - `Sandboxed`: safe modules only, 10M step limit
/// - `Unsandboxed`: full R7RS, no step limit (trusted tools only)
#[cfg(feature = "synthesised-tools")]
fn build_tein_context(
    source: String,
    tier: crate::config::SandboxTier,
) -> io::Result<(TeinSession, std::thread::ThreadId)> {
    let worker_thread_id = Arc::new(std::sync::Mutex::new(None::<std::thread::ThreadId>));
    let tid_capture = Arc::clone(&worker_thread_id);

    // Shared buffers for stdout/stderr capture. Arc clones go into the init
    // closure (for wiring output ports) and into TeinSession (for reading).
    let stdout_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stdout_for_init = Arc::clone(&stdout_buf);
    let stderr_for_init = Arc::clone(&stderr_buf);

    let init = move |ctx: &Context| -> tein::Result<()> {
        // Capture the tein worker thread's ID for BRIDGE_CALL_CTX keying.
        *tid_capture.lock().unwrap() = Some(std::thread::current().id());
        ctx.define_fn_variadic("call-tool", __tein_call_tool_fn)?;
        ctx.define_fn_variadic("generate-id", __tein_generate_id_fn)?;
        ctx.define_fn_variadic("current-timestamp", __tein_current_timestamp_fn)?;
        ctx.evaluate(&HARNESS_PREAMBLE)?;
        ctx.register_module(HARNESS_TOOLS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness module: {e}")))?;
        ctx.register_module(HARNESS_HOOKS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness hooks module: {e}")))?;
        ctx.register_module(HARNESS_DOCS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness docs module: {e}")))?;
        // (harness io) — privileged direct IO, available at Unsandboxed tier only.
        // Sandboxed contexts will get a module-not-found error on (import (harness io)).
        if tier == crate::config::SandboxTier::Unsandboxed {
            ctx.define_fn_variadic("io-read", __tein_io_read_fn)?;
            ctx.define_fn_variadic("io-write", __tein_io_write_fn)?;
            ctx.define_fn_variadic("io-append", __tein_io_append_fn)?;
            ctx.define_fn_variadic("io-list", __tein_io_list_fn)?;
            ctx.define_fn_variadic("io-exists?", __tein_io_exists_fn)?;
            ctx.define_fn_variadic("io-delete", __tein_io_delete_fn)?;
            ctx.register_module(HARNESS_IO_MODULE)
                .map_err(|e| tein::Error::EvalError(format!("harness io module: {e}")))?;
        }
        // Standard prelude — must run after all modules are registered (EVAL_PRELUDE
        // imports (harness tools), which is only available post-register_module).
        ctx.evaluate(EVAL_PRELUDE)?;
        // Wire stdout/stderr capture ports. The SharedWriter clones share the
        // same Arc buffers that TeinSession reads from. Ports persist across
        // evaluations (confirmed by tein test suite).
        let out_port = ctx.open_output_port(SharedWriter(stdout_for_init.clone()))?;
        ctx.set_current_output_port(&out_port)?;
        let err_port = ctx.open_output_port(SharedWriter(stderr_for_init.clone()))?;
        ctx.set_current_error_port(&err_port)?;

        ctx.evaluate(&source)?;
        Ok(())
    };

    let ctx = match tier {
        crate::config::SandboxTier::Sandboxed => Context::builder()
            .standard_env()
            .sandboxed(Modules::Safe)
            .step_limit(10_000_000)
            .build_managed(init),
        crate::config::SandboxTier::Unsandboxed => {
            // with_vfs_shadows() enables shadow modules (e.g. scheme/process-context,
            // scheme/file) in non-sandboxed contexts. Required for (chibi diff) and
            // other library modules that depend on scheme/process-context.
            Context::builder()
                .standard_env()
                .with_vfs_shadows()
                .build_managed(init)
        }
    }
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tein init: {e}")))?;

    let tid = worker_thread_id
        .lock()
        .unwrap()
        .expect("init closure must have run and captured thread ID");

    let session = TeinSession {
        ctx,
        stdout_buf,
        stderr_buf,
    };
    Ok((session, tid))
}

/// Build a sandboxed tein harness context with no user source.
///
/// Exposed for `eval.rs` which needs the same FFI/harness setup as synthesised
/// tools but manages its own prelude and persistence. Passes an empty source so
/// the caller can call `ctx.evaluate(prelude)` after receiving the context.
#[cfg(feature = "synthesised-tools")]
pub(crate) fn build_sandboxed_harness_context() -> io::Result<(TeinSession, std::thread::ThreadId)>
{
    build_tein_context(String::new(), crate::config::SandboxTier::Sandboxed)
}

/// Standard prelude evaluated in every tein context (synthesised tools and `scheme_eval`).
///
/// Auto-imports the standard module set so the LLM can use them without explicit
/// `(import ...)` statements. Evaluated after `HARNESS_PREAMBLE` and before user source.
///
/// Mutation site: `eval.rs` `EVAL_PRELUDE` has been removed — this is the single source
/// of truth. Update tests in `eval.rs` if the module set changes.
#[cfg(feature = "synthesised-tools")]
pub(crate) const EVAL_PRELUDE: &str = r#"
(import (scheme base)
        (scheme write)
        (scheme read)
        (scheme char)
        (scheme case-lambda)
        (scheme inexact)
        (scheme complex)
        (tein json)
        (tein safe-regexp)
        (tein docs)
        (tein introspect)
        (srfi 1)
        (srfi 27)
        (srfi 69)
        (srfi 95)
        (srfi 125)
        (srfi 128)
        (srfi 130)
        (srfi 132)
        (srfi 133)
        (chibi match)
        (harness tools)
        (harness docs))

;; R5RS aliases — LLMs reach for these instinctively
(define exact->inexact inexact)
(define inexact->exact exact)
"#;

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
    load_tools_from_source_with_tier(
        source,
        vfs_path,
        registry,
        crate::config::SandboxTier::Sandboxed,
    )
}

/// Like `load_tools_from_source` but with an explicit sandbox tier.
#[cfg(feature = "synthesised-tools")]
pub fn load_tools_from_source_with_tier(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    tier: crate::config::SandboxTier,
) -> io::Result<Vec<Tool>> {
    let source_owned = source.to_string();

    let (session, worker_thread_id) = build_tein_context(source_owned, tier)?;

    // check if define-tool was used (%tool-registry% is non-empty list)
    let multi = session.evaluate("%tool-registry%").ok();
    let is_multi = matches!(
        &multi,
        Some(Value::List(items)) if !items.is_empty()
    );

    if is_multi {
        extract_multi_tools(session, vfs_path, registry, worker_thread_id)
    } else {
        extract_single_tool(session, vfs_path, registry, worker_thread_id).map(|t| vec![t])
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
fn extract_single_tool(
    session: TeinSession,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    worker_thread_id: std::thread::ThreadId,
) -> io::Result<Tool> {
    let name = extract_string(&session, "tool-name")?;
    let description = extract_string(&session, "tool-description")?;
    let params_val = session.evaluate("tool-parameters").map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing tool-parameters: {e}"),
        )
    })?;
    let parameters = params_alist_to_json_schema(&params_val)?;

    let exec_val = session.evaluate("tool-execute").map_err(|e| {
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

    let (hooks, hook_bindings) = extract_hook_registrations(&session)?;
    let context = Arc::new(session);
    Ok(Tool {
        name,
        description,
        parameters,
        hooks,
        metadata: ToolMetadata::new(),
        summary_params: vec![],
        r#impl: ToolImpl::Synthesised {
            vfs_path: vfs_path.clone(),
            exec_binding: "tool-execute".to_string(),
            context,
            registry: Arc::clone(registry),
            worker_thread_id,
            hook_bindings,
        },
        category: ToolCategory::Synthesised,
    })
}

/// Escape a string for safe embedding inside a Scheme string literal.
/// Replaces `\` with `\\` and `"` with `\"`.
#[cfg(feature = "synthesised-tools")]
pub(crate) fn scheme_escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Reads `%tool-registry%` (a LIFO list built via `cons`) and produces one
/// `Tool` per entry. All tools share the same tein context via `Arc`.
#[cfg(feature = "synthesised-tools")]
fn extract_multi_tools(
    session: TeinSession,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    worker_thread_id: std::thread::ThreadId,
) -> io::Result<Vec<Tool>> {
    let registry_val = session
        .evaluate("%tool-registry%")
        .map_err(|e| io::Error::other(format!("reading %tool-registry%: {e}")))?;

    let entries = match registry_val {
        Value::List(items) => items,
        other => {
            return Err(io::Error::other(format!(
                "%%tool-registry%% is not a list: {other}"
            )));
        }
    };

    let (hooks, hook_bindings) = extract_hook_registrations(&session)?;
    let context = Arc::new(session);
    let mut tools = Vec::with_capacity(entries.len());

    // entries are in LIFO order (built via cons); reverse to get definition order
    for entry in entries.iter().rev() {
        let fields = match entry {
            Value::List(f) if f.len() >= 4 => f,
            other => {
                return Err(io::Error::other(format!(
                    "define-tool entry has unexpected shape: {other}"
                )));
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

        // reject names that would produce invalid Scheme identifiers when
        // embedded in `%tool-execute-{name}%` (whitespace or parentheses break
        // the symbol syntax)
        if name
            .chars()
            .any(|c| c.is_whitespace() || c == '(' || c == ')')
        {
            return Err(io::Error::other(format!(
                "define-tool: name {name:?} contains invalid characters (whitespace or parens)"
            )));
        }

        // bind the execute handler to a per-tool name so execute_synthesised
        // can find it by name. the context is shared across all tools in this file.
        let exec_binding = format!("%tool-execute-{name}%");
        // scheme-escape name before interpolating into a string literal
        let name_escaped = scheme_escape_string(&name);
        // %tool-registry% entries are (name desc params handler).
        // use list-ref to extract the handler (index 3) for this tool by name.
        // list-ref is in (scheme base) which the preamble already imports.
        context
            .evaluate(&format!(
                "(define {exec_binding} \
                 (list-ref \
                   (let loop ((reg %tool-registry%)) \
                     (if (string=? (car (car reg)) \"{name_escaped}\") \
                         (car reg) \
                         (loop (cdr reg)))) \
                   3))"
            ))
            .map_err(|e| io::Error::other(format!("binding {exec_binding}: {e}")))?;

        tools.push(Tool {
            name,
            description,
            parameters,
            hooks: hooks.clone(),
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: ToolImpl::Synthesised {
                vfs_path: vfs_path.clone(),
                exec_binding: exec_binding.clone(),
                context: Arc::clone(&context),
                registry: Arc::clone(registry),
                worker_thread_id,
                hook_bindings: hook_bindings.clone(),
            },
            category: ToolCategory::Synthesised,
        });
    }

    Ok(tools)
}

/// Execute a synthesised tool by calling its bound execute procedure.
///
/// Converts JSON args to a scheme alist, resolves the `exec_binding` in the
/// session, and calls it with stdout/stderr capture. Returns structured output:
/// `"result: <value>\nstdout: <output>\nstderr: <output>"`.
///
/// Sets `BRIDGE_CALL_CTX` (with the per-call registry) via `CallContextGuard`
/// before calling into scheme so that `call-tool` can access the runtime context.
///
/// `%context-name%` is injected via `set!` on the top-level binding from
/// `HARNESS_PREAMBLE`. Concurrent calls to different `TeinSession`s are
/// safe because each session has its own `BRIDGE_CALL_CTX` slot (keyed by
/// `worker_thread_id`). Calls to the same session are serialised by
/// `ThreadLocalContext`'s internal mutex, so no interleaving can occur.
#[cfg(feature = "synthesised-tools")]
pub async fn execute_synthesised(
    session: &TeinSession,
    exec_binding: &str,
    call: &ToolCall<'_>,
    registry: Arc<RwLock<ToolRegistry>>,
    worker_thread_id: std::thread::ThreadId,
) -> io::Result<String> {
    let _guard = CallContextGuard::set(call.context, registry, worker_thread_id);

    // Inject per-call context name before invoking the tool handler.
    // %context-name% mutates the top-level binding defined in HARNESS_PREAMBLE.
    // Safe because ThreadLocalContext serialises all calls to this context.
    let ctx_name_escaped = scheme_escape_string(call.context.context_name);
    session
        .evaluate(&format!("(set! %context-name% \"{ctx_name_escaped}\")"))
        .map_err(|e| io::Error::other(format!("inject %context-name%: {e}")))?;

    let args_alist = json_args_to_scheme_alist(call.args)?;
    let exec_fn = session
        .evaluate(exec_binding)
        .map_err(|e| io::Error::other(format!("resolve {exec_binding}: {e}")))?;

    // NOTE: similar capture pattern exists in run_scheme (eval.rs).
    // Changes here may need to be assessed for that callsite too.
    let captured = session.with_capture(|ctx| ctx.call(&exec_fn, &[args_alist]));
    Ok(captured.format_tool())
}

/// No-op stub so the module compiles without the feature. Unreachable at
/// runtime since `ToolImpl::Synthesised` only exists behind the same cfg.
#[cfg(not(feature = "synthesised-tools"))]
pub async fn execute_synthesised(
    _session: &(),
    _exec_binding: &str,
    _call: &ToolCall<'_>,
    _registry: std::sync::Arc<std::sync::RwLock<ToolRegistry>>,
    _worker_thread_id: std::thread::ThreadId,
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
        if entry.kind == VfsEntryKind::Directory {
            // recurse into subdirectories — ignore errors (zone boundary)
            let subzone = format!("{zone}/{}", entry.name);
            let _ = Box::pin(scan_zone(vfs, registry, &subzone, tools_config)).await;
            continue;
        }
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
        if let Ok(tools) = load_tools_from_source_with_tier(&source_str, &file_path, registry, tier)
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
pub fn unregister_tool_at_path(registry: &Arc<RwLock<ToolRegistry>>, path: &VfsPath) {
    let mut reg = registry.write().unwrap();
    let names = reg.find_all_by_vfs_path(path);
    for name in names {
        reg.unregister(&name);
    }
}

// --- helpers -----------------------------------------------------------------

/// Extract a scheme string binding from a `TeinSession`.
#[cfg(feature = "synthesised-tools")]
fn extract_string(session: &TeinSession, name: &str) -> io::Result<String> {
    let val = session
        .evaluate(name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}: {e}")))?;
    val.as_string().map(str::to_string).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} is not a string"),
        )
    })
}

/// Read `%hook-registry%` from a tein context and return (hooks, hook_bindings).
///
/// Each entry in `%hook-registry%` is `(hook-name-string handler-procedure)`.
/// For each valid entry, we:
/// 1. Parse the hook name string into a `HookPoint`.
/// 2. Bind the handler to `%hook-{hook_name}%` in the context.
/// 3. Record the mapping in `hook_bindings`.
///
/// Invalid hook names are warned and skipped (same as plugin hook parsing).
///
/// Mutation site: if `register-hook` alist shape changes, update this function
/// and `HARNESS_PREAMBLE` accordingly.
#[cfg(feature = "synthesised-tools")]
fn extract_hook_registrations(
    session: &TeinSession,
) -> io::Result<(
    Vec<super::hooks::HookPoint>,
    std::collections::HashMap<super::hooks::HookPoint, String>,
)> {
    let registry_val = session
        .evaluate("%hook-registry%")
        .map_err(|e| io::Error::other(format!("reading %hook-registry%: {e}")))?;

    let entries = match registry_val {
        Value::List(items) if !items.is_empty() => items,
        _ => return Ok((vec![], std::collections::HashMap::new())),
    };

    let mut hooks = Vec::new();
    let mut hook_bindings = std::collections::HashMap::new();

    // entries are LIFO (via cons); reverse for definition order
    for entry in entries.iter().rev() {
        let fields = match entry {
            Value::List(f) if f.len() >= 2 => f,
            other => {
                eprintln!("[WARN] register-hook entry has unexpected shape: {other}");
                continue;
            }
        };

        let hook_name = match fields[0].as_string() {
            Some(s) => s.to_string(),
            None => {
                eprintln!("[WARN] register-hook: hook name not a string");
                continue;
            }
        };

        let hook_point = match hook_name.parse::<super::hooks::HookPoint>() {
            Ok(hp) => hp,
            Err(_) => {
                eprintln!("[WARN] Unknown hook '{hook_name}' in register-hook");
                continue;
            }
        };

        if !fields[1].is_procedure() {
            eprintln!("[WARN] register-hook {hook_name}: handler is not a procedure");
            continue;
        }

        // bind handler to a well-known name so execute_hook can find it
        let binding = format!("%hook-{hook_name}%");
        let hook_name_escaped = scheme_escape_string(&hook_name);
        // look up handler from %hook-registry% by name (first match in LIFO list = newest).
        // when the same hook point is registered multiple times, `define` overwrites on
        // each iteration (oldest-first), so the last-defined handler wins.
        session
            .evaluate(&format!(
                "(define {binding} \
             (cadr \
               (let loop ((reg %hook-registry%)) \
                 (if (string=? (car (car reg)) \"{hook_name_escaped}\") \
                     (car reg) \
                     (loop (cdr reg))))))"
            ))
            .map_err(|e| io::Error::other(format!("binding {binding}: {e}")))?;

        // deduplicate hook points: only push to `hooks` once per hook point.
        // the binding name is the same regardless (`%hook-{hook_name}%`), so
        // or_insert_with is used purely to avoid duplicate entries in `hooks`.
        hook_bindings.entry(hook_point).or_insert_with(|| {
            hooks.push(hook_point);
            binding
        });
    }

    Ok((hooks, hook_bindings))
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
pub(crate) fn scheme_value_to_json(val: &Value) -> io::Result<serde_json::Value> {
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
pub(crate) fn json_args_to_scheme_alist(args: &serde_json::Value) -> io::Result<Value> {
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

    #[test]
    fn test_tein_session_stdout_capture() {
        // Direct TeinSession capture test — verifies both tiers.
        for tier in [
            crate::config::SandboxTier::Unsandboxed,
            crate::config::SandboxTier::Sandboxed,
        ] {
            let (session, _) =
                build_tein_context(String::new(), tier).expect("session should build");
            let cap = session.with_capture(|ctx| ctx.evaluate("(display 42)"));
            assert!(
                cap.value.is_ok(),
                "display should not error ({tier:?}): {:?}",
                cap.value
            );
            assert_eq!(cap.stdout, "42", "stdout should be 42 ({tier:?})");
            assert!(cap.stderr.is_empty(), "stderr should be empty ({tier:?})");
        }
    }

    #[test]
    fn test_harness_docs_module_available_both_tiers() {
        // (import (harness docs)) must succeed in both sandboxed and unsandboxed contexts,
        // and both hooks-docs and harness-tools-docs must be bound and non-empty.
        for tier in [
            crate::config::SandboxTier::Sandboxed,
            crate::config::SandboxTier::Unsandboxed,
        ] {
            let (session, _) =
                build_tein_context(String::new(), tier).expect("session should build");

            let hooks_docs_ok = session
                .evaluate("(and (pair? hooks-docs) (pair? harness-tools-docs))")
                .expect("evaluate pair checks");
            assert_eq!(
                hooks_docs_ok,
                tein::Value::Boolean(true),
                "hooks-docs and harness-tools-docs must be pairs in {tier:?} context"
            );
        }
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

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default())
            .await
            .unwrap();

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

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default())
            .await
            .unwrap();
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
        let result =
            scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await;
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
        let result =
            scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default()).await;
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

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default())
            .await
            .unwrap();
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

        scan_and_register(&vfs, &registry, &crate::config::ToolsConfig::default())
            .await
            .unwrap();
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

        reload_tool_from_content(
            &registry,
            &path,
            SCAN_TOOL.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );

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
        reload_tool_from_content(
            &registry,
            &path,
            SCAN_TOOL.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );
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
        reload_tool_from_content(
            &registry,
            &path,
            updated.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );
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
        reload_tool_from_content(
            &registry,
            &path,
            SCAN_TOOL.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );
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
        reload_tool_from_content(
            &registry,
            &path,
            SCAN_TOOL.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );
        assert!(registry.read().unwrap().get("scan_hello").is_some());

        // Try to reload with invalid source — should not remove the existing tool
        reload_tool_from_content(
            &registry,
            &path,
            b"(define tool-name \"bad\")",
            &crate::config::ToolsConfig::default(),
        );
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
                    reload_tool_from_content(
                        &reg,
                        path,
                        bytes,
                        &crate::config::ToolsConfig::default(),
                    );
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
        reload_tool_from_content(
            &registry,
            &path,
            source_v1.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );

        assert!(registry.read().unwrap().get("tool_a").is_some());
        assert!(registry.read().unwrap().get("tool_b").is_some());

        // update: remove tool_b, add tool_c
        let source_v2 = r#"
(import (scheme base))
(import (harness tools))
(define-tool tool_a (description "a v2") (parameters '()) (execute (lambda (args) "a2")))
(define-tool tool_c (description "c") (parameters '()) (execute (lambda (args) "c")))
"#;
        reload_tool_from_content(
            &registry,
            &path,
            source_v2.as_bytes(),
            &crate::config::ToolsConfig::default(),
        );

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
        assert!(
            result.is_err(),
            "sandboxed tier should reject (scheme regex)"
        );
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

    // --- task plugin integration tests ---------------------------------------

    const TASKS_PLUGIN: &str = include_str!("../../../../plugins/tasks.scm");

    /// Load the tasks.scm plugin and verify all five tools are registered.
    #[test]
    fn test_tasks_plugin_loads() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/tasks.scm").unwrap();
        let tools = load_tools_from_source(TASKS_PLUGIN, &path, &registry).unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for expected in &[
            "task_create",
            "task_update",
            "task_view",
            "task_list",
            "task_delete",
        ] {
            assert!(
                names.contains(expected),
                "expected tool '{}' to be registered, got: {:?}",
                expected,
                names
            );
        }
        assert_eq!(tools.len(), 5, "expected exactly 5 task tools");
    }

    const HISTORY_PLUGIN: &str = include_str!("../../../../plugins/history.scm");

    /// Load history.scm and verify all four tools are registered.
    #[test]
    fn test_history_plugin_loads() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/history.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            HISTORY_PLUGIN,
            &path,
            &registry,
            crate::config::SandboxTier::Unsandboxed,
        )
        .unwrap();

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"file_history_log"),
            "missing file_history_log: {:?}",
            names
        );
        assert!(
            names.contains(&"file_history_show"),
            "missing file_history_show: {:?}",
            names
        );
        assert!(
            names.contains(&"file_history_diff"),
            "missing file_history_diff: {:?}",
            names
        );
        assert!(
            names.contains(&"file_history_revert"),
            "missing file_history_revert: {:?}",
            names
        );
        assert_eq!(tools.len(), 4, "expected exactly 4 tools: {:?}", names);
    }

    /// Verify history.scm registers the pre_vfs_write hook binding.
    #[test]
    fn test_history_plugin_registers_hook() {
        let registry = make_registry();
        let path = VfsPath::new("/tools/shared/history.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            HISTORY_PLUGIN,
            &path,
            &registry,
            crate::config::SandboxTier::Unsandboxed,
        )
        .unwrap();

        // At least one tool should carry the pre_vfs_write hook binding.
        let has_hook = tools.iter().any(|t| {
            if let ToolImpl::Synthesised { hook_bindings, .. } = &t.r#impl {
                hook_bindings.contains_key(&crate::tools::hooks::HookPoint::PreVfsWrite)
            } else {
                false
            }
        });
        assert!(
            has_hook,
            "history plugin should register pre_vfs_write hook"
        );
    }

    /// Verify generate-id and current-timestamp harness helpers work.
    #[test]
    fn test_harness_helpers_generate_id_and_timestamp() {
        let registry = make_registry();
        let source = r#"
(import (scheme base))
(import (harness tools))
(define tool-name "helper_test")
(define tool-description "test generate-id and current-timestamp")
(define tool-parameters '())
(define (tool-execute args)
  (string-append (generate-id) ":" (current-timestamp)))
"#;
        let path = VfsPath::new("/tools/shared/helper_test.scm").unwrap();
        let tool = load_tool_from_source(source, &path, &registry).unwrap();
        if let ToolImpl::Synthesised {
            ref context,
            ref exec_binding,
            ..
        } = tool.r#impl
        {
            let exec_fn = context.evaluate(exec_binding).unwrap();
            let alist = json_args_to_scheme_alist(&serde_json::json!({})).unwrap();
            let result = context.call(&exec_fn, &[alist]).unwrap();
            let s = result.as_string().unwrap().to_string();
            // format: "XXXXXXXX:YYYYMMDD-HHMMz"
            assert!(s.contains(':'), "expected id:timestamp, got: {}", s);
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            assert_eq!(parts[0].len(), 8, "id should be 8 hex chars: {}", parts[0]);
            assert!(
                parts[1].ends_with('z'),
                "timestamp should end with 'z': {}",
                parts[1]
            );
        } else {
            panic!("expected Synthesised impl");
        }
    }

    /// Verify %context-name% injection via execute_synthesised.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_context_name_injection() {
        use crate::test_support::create_test_chibi;

        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();
        let source = r#"
(import (scheme base))
(import (harness tools))
(define tool-name "ctx_test")
(define tool-description "return context name")
(define tool-parameters '())
(define (tool-execute args) %context-name%)
"#;
        let path = VfsPath::new("/tools/shared/ctx_test.scm").unwrap();
        let tools = load_tools_from_source(source, &path, &registry).unwrap();
        {
            let mut reg = registry.write().unwrap();
            for t in tools {
                reg.register(t);
            }
        }
        let result = chibi
            .execute_tool("default", "ctx_test", serde_json::json!({}))
            .await
            .unwrap();
        let ctx_name = extract_result_field(&result);
        assert_eq!(
            ctx_name, "default",
            "context name should be injected as %%context-name%%: {result}"
        );
    }

    /// Full CRUD integration: create → list → view → update → delete.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_task_crud_integration() {
        use crate::test_support::create_test_chibi;

        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();
        let path = VfsPath::new("/tools/shared/tasks.scm").unwrap();
        let tools = load_tools_from_source(TASKS_PLUGIN, &path, &registry).unwrap();
        {
            let mut reg = registry.write().unwrap();
            for t in tools {
                reg.register(t);
            }
        }

        // Create a task
        let create_result = chibi
            .execute_tool(
                "default",
                "task_create",
                serde_json::json!({
                    "path": "test/my-task",
                    "body": "do the thing",
                    "priority": "high"
                }),
            )
            .await
            .unwrap();
        let create_body = extract_result_field(&create_result);
        assert!(
            create_body.contains("created task"),
            "unexpected: {create_result}"
        );
        // Extract ID from "created task XXXX at ..."
        let id = create_body
            .split_whitespace()
            .nth(2)
            .expect("expected id in create result")
            .to_string();

        // List tasks — should include our task
        let list_result = chibi
            .execute_tool("default", "task_list", serde_json::json!({}))
            .await
            .unwrap();
        let list_body = extract_result_field(&list_result);
        assert!(
            list_body.contains(&id),
            "id should appear in list: {list_result}"
        );

        // View the task
        let view_result = chibi
            .execute_tool("default", "task_view", serde_json::json!({"id": id}))
            .await
            .unwrap();
        let view_body = extract_result_field(&view_result);
        assert!(
            view_body.contains(&id),
            "id should appear in view: {view_result}"
        );
        assert!(
            view_body.contains("do the thing"),
            "body should appear in view: {view_result}"
        );

        // Update status to in-progress
        let update_result = chibi
            .execute_tool(
                "default",
                "task_update",
                serde_json::json!({
                    "id": id,
                    "status": "in-progress"
                }),
            )
            .await
            .unwrap();
        let update_body = extract_result_field(&update_result);
        assert!(
            update_body.contains("updated task"),
            "unexpected: {update_result}"
        );

        // View again — status should be in-progress
        let view2 = chibi
            .execute_tool("default", "task_view", serde_json::json!({"id": id}))
            .await
            .unwrap();
        let view2_body = extract_result_field(&view2);
        assert!(
            view2_body.contains("in-progress"),
            "status should be in-progress: {view2}"
        );

        // Delete the task
        let delete_result = chibi
            .execute_tool("default", "task_delete", serde_json::json!({"id": id}))
            .await
            .unwrap();
        let delete_body = extract_result_field(&delete_result);
        assert!(
            delete_body.contains("deleted task"),
            "unexpected: {delete_result}"
        );

        // List again — should be gone
        let list2 = chibi
            .execute_tool("default", "task_list", serde_json::json!({}))
            .await
            .unwrap();
        let list2_body = extract_result_field(&list2);
        assert!(
            !list2_body.contains(&id),
            "id should be gone after delete: {list2}"
        );
    }

    // --- hook registration tests ---

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_hook_registration_populates_tool_hooks() {
        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) '()))
(register-hook 'pre_message (lambda (payload) '()))

(define tool-name "test-hook-tool")
(define tool-description "A tool that registers hooks")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/test-hooks.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        assert_eq!(tools.len(), 1);
        assert!(
            tools[0]
                .hooks
                .contains(&crate::tools::hooks::HookPoint::OnStart),
            "expected OnStart in hooks: {:?}",
            tools[0].hooks
        );
        assert!(
            tools[0]
                .hooks
                .contains(&crate::tools::hooks::HookPoint::PreMessage),
            "expected PreMessage in hooks: {:?}",
            tools[0].hooks
        );

        if let ToolImpl::Synthesised { hook_bindings, .. } = &tools[0].r#impl {
            assert!(hook_bindings.contains_key(&crate::tools::hooks::HookPoint::OnStart));
            assert!(hook_bindings.contains_key(&crate::tools::hooks::HookPoint::PreMessage));
        } else {
            panic!("expected Synthesised");
        }
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_hook_registration_invalid_hook_name_skipped() {
        let source = r#"
(import (harness hooks))
(register-hook 'nonexistent_hook (lambda (payload) '()))
(register-hook 'on_start (lambda (payload) '()))

(define tool-name "test-invalid-hook")
(define tool-description "Tool with invalid hook")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/test-invalid.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        assert_eq!(tools.len(), 1);
        // only valid hook should be registered
        assert_eq!(
            tools[0].hooks.len(),
            1,
            "invalid hook should be skipped: {:?}",
            tools[0].hooks
        );
        assert!(
            tools[0]
                .hooks
                .contains(&crate::tools::hooks::HookPoint::OnStart)
        );
    }

    // --- structured output tests ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_synthesised_tool_error_captured() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();
        let source = r#"
(import (scheme base))
(define tool-name "error_test")
(define tool-description "tool that errors")
(define tool-parameters '())
(define (tool-execute args) (error "intentional boom"))
"#;
        let path = VfsPath::new("/tools/shared/error_test.scm").unwrap();
        let tools = load_tools_from_source(source, &path, &registry).unwrap();
        {
            let mut reg = registry.write().unwrap();
            for t in tools {
                reg.register(t);
            }
        }
        let result = chibi
            .execute_tool("default", "error_test", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            result.contains("result: error:"),
            "scheme error should appear in result field: {result}"
        );
        assert!(
            result.contains("intentional boom"),
            "error message should be preserved: {result}"
        );
    }

    // --- harness io tests ---

    /// Extract the `result: ` field from structured output.
    /// Structured format: "result: <value>\nstdout: ...\nstderr: ..."
    /// The result value may itself contain newlines, so we split on "\nstdout: ".
    fn extract_result_field(output: &str) -> &str {
        output
            .strip_prefix("result: ")
            .and_then(|s| s.split_once("\nstdout: "))
            .map(|(result, _)| result)
            .unwrap_or(output)
    }

    /// Build an unsandboxed tein tool with `(harness io)` and execute it via
    /// `chibi.execute_tool`, which sets `BRIDGE_CALL_CTX` on the tein worker thread.
    /// Returns only the `result:` field from structured output.
    async fn run_io_tool(
        chibi: &crate::Chibi,
        registry: &Arc<RwLock<ToolRegistry>>,
        source: &str,
    ) -> String {
        let path = VfsPath::new("/tools/shared/io_test.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            registry,
            crate::config::SandboxTier::Unsandboxed,
        )
        .unwrap();
        {
            let mut reg = registry.write().unwrap();
            for t in tools {
                reg.register(t);
            }
        }
        let raw = chibi
            .execute_tool("default", "io_test", serde_json::json!({}))
            .await
            .unwrap();
        extract_result_field(&raw).to_string()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_read_write_roundtrip() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        // write a known file into the VFS
        let path = VfsPath::new("/shared/io-roundtrip.txt").unwrap();
        chibi
            .app
            .vfs
            .write(VfsCaller::System, &path, b"hello from io")
            .await
            .unwrap();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "read vfs file")
(define tool-parameters '())
(define (tool-execute args)
  (let ((content (io-read "vfs:///shared/io-roundtrip.txt")))
    (if (string? content) content "not-a-string")))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "hello from io");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_write_then_read() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "write then read vfs file")
(define tool-parameters '())
(define (tool-execute args)
  (io-write "vfs:///shared/written.txt" "scheme wrote this")
  (io-read "vfs:///shared/written.txt"))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "scheme wrote this");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_not_found_returns_false() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "missing file returns #f")
(define tool-parameters '())
(define (tool-execute args)
  (let ((result (io-read "vfs:///shared/nonexistent-xyzzy.txt")))
    (if (boolean? result) "got-false" "got-something-else")))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "got-false");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_append() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "append to vfs file")
(define tool-parameters '())
(define (tool-execute args)
  (io-write "vfs:///shared/append-test.txt" "line1\n")
  (io-append "vfs:///shared/append-test.txt" "line2\n")
  (io-read "vfs:///shared/append-test.txt"))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "line1\nline2\n");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_exists() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let path = VfsPath::new("/shared/exists-check.txt").unwrap();
        chibi
            .app
            .vfs
            .write(VfsCaller::System, &path, b"exists")
            .await
            .unwrap();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "exists check")
(define tool-parameters '())
(define (tool-execute args)
  (let ((e1 (io-exists? "vfs:///shared/exists-check.txt"))
        (e2 (io-exists? "vfs:///shared/does-not-exist.txt")))
    (if (and e1 (not e2)) "correct" "wrong")))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "correct");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_list() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        chibi
            .app
            .vfs
            .write(
                VfsCaller::System,
                &VfsPath::new("/shared/list-dir/a.txt").unwrap(),
                b"a",
            )
            .await
            .unwrap();
        chibi
            .app
            .vfs
            .write(
                VfsCaller::System,
                &VfsPath::new("/shared/list-dir/b.txt").unwrap(),
                b"b",
            )
            .await
            .unwrap();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "list dir")
(define tool-parameters '())
(define (tool-execute args)
  (let ((entries (io-list "vfs:///shared/list-dir")))
    (number->string (length entries))))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "2");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_list_nonexistent_returns_empty() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "list nonexistent")
(define tool-parameters '())
(define (tool-execute args)
  (let ((entries (io-list "vfs:///shared/nonexistent-dir-xyzzy")))
    (if (null? entries) "empty" "not-empty")))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "empty");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_fs_write_read_roundtrip() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        // Use a temp file path
        let tmp_path = std::env::temp_dir().join("chibi-io-test-roundtrip.txt");
        let path_str = tmp_path.to_str().unwrap().to_string();
        // Cleanup before test
        let _ = std::fs::remove_file(&tmp_path);

        let source = format!(
            r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "fs write read roundtrip")
(define tool-parameters '())
(define (tool-execute args)
  (io-write "{path}" "fs content")
  (io-read "{path}"))
"#,
            path = path_str
        );
        let result = run_io_tool(&chibi, &registry, &source).await;
        let _ = std::fs::remove_file(&tmp_path);
        assert_eq!(result, "fs content");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_fs_not_found_returns_false() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "fs not found")
(define tool-parameters '())
(define (tool-execute args)
  (let ((result (io-read "/tmp/chibi-io-test-nonexistent-xyzzy-9999.txt")))
    (if (boolean? result) "got-false" "got-something-else")))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "got-false");
    }

    #[test]
    fn test_harness_io_blocked_at_sandboxed_tier() {
        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "blocked at sandboxed")
(define tool-parameters '())
(define (tool-execute args) "should not load")
"#;
        let vfs_path = VfsPath::new("/tools/shared/io_test.scm").unwrap();
        let registry = make_registry();
        let result = load_tools_from_source_with_tier(
            source,
            &vfs_path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        );
        assert!(
            result.is_err(),
            "sandboxed tier should not have (harness io)"
        );
    }

    #[test]
    fn test_harness_io_available_at_unsandboxed_tier() {
        // Just loading the source (no execution needed — import resolves at load time)
        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "available at unsandboxed")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let vfs_path = VfsPath::new("/tools/shared/io_test.scm").unwrap();
        let registry = make_registry();
        let result = load_tools_from_source_with_tier(
            source,
            &vfs_path,
            &registry,
            crate::config::SandboxTier::Unsandboxed,
        );
        assert!(
            result.is_ok(),
            "unsandboxed tier should have (harness io): {:?}",
            result.err()
        );
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_hook_registration_multi_tool_file() {
        // all tools in a multi-tool file share the same hooks
        let source = r#"
(import (harness hooks))
(import (harness tools))
(register-hook 'on_start (lambda (payload) '()))

(define-tool tool-a
  (description "first tool")
  (parameters '())
  (execute (lambda (args) "a")))

(define-tool tool-b
  (description "second tool")
  (parameters '())
  (execute (lambda (args) "b")))
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/multi-hooks.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        assert_eq!(tools.len(), 2);
        for tool in &tools {
            assert!(
                tool.hooks
                    .contains(&crate::tools::hooks::HookPoint::OnStart),
                "tool {} should have OnStart hook",
                tool.name
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_harness_io_vfs_delete() {
        use crate::test_support::create_test_chibi;
        let (chibi, _tmp) = create_test_chibi();
        let registry = chibi.registry.clone();

        // Pre-populate a file to delete
        let path = VfsPath::new("/shared/delete-me.txt").unwrap();
        chibi
            .app
            .vfs
            .write(VfsCaller::System, &path, b"doomed")
            .await
            .unwrap();

        let source = r#"
(import (scheme base))
(import (harness io))
(define tool-name "io_test")
(define tool-description "delete a vfs file")
(define tool-parameters '())
(define (tool-execute args)
  (io-delete "vfs:///shared/delete-me.txt")
  (if (io-exists? "vfs:///shared/delete-me.txt")
      "still-exists"
      "deleted"))
"#;
        let result = run_io_tool(&chibi, &registry, source).await;
        assert_eq!(result, "deleted");
    }
}
