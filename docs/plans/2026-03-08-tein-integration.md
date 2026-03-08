# Tein Integration — Remaining Items

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete the synthesised tool system so scheme tools can call other tools, all VFS zones are scanned, visibility is scoped per-context, multi-tool files are supported, and sandbox tiers are configurable.

**Architecture:** Six items in three layers. Layer 1 (call-tool bridge) is the enabling primitive. Layer 2 (multi-zone scan + visibility) makes tools context-aware. Layer 3 (define-tool macro + tier config) adds ergonomics and security policy. Each layer builds on the previous.

**Tech Stack:** Rust, tein (scheme), tokio (async runtime), serde_json

**Issue:** #193

---

## Layer 1: `(harness tools)` module & `call-tool` bridge

### Task 1: register `(harness tools)` scheme module in tein context

The `(harness tools)` module needs to be available in every synthesised tool's tein context. tein supports runtime module registration via `Context::register_module()`, which automatically allowlists the module for sandboxed contexts.

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs:51-100` (the `load_tool_from_source` function)

**Step 1: write the `(harness tools)` module source as a const**

Add a const string containing the scheme library definition. For now it exports nothing — `call-tool` will be added as a foreign function in task 2.

```rust
/// Scheme source for the `(harness tools)` module.
///
/// `call-tool` is registered as a foreign function (see `register_call_tool_bridge`)
/// and re-exported here so synthesised tools can `(import (harness tools))`.
const HARNESS_TOOLS_MODULE: &str = r#"
(define-library (harness tools)
  (import (scheme base))
  (export call-tool)
  (begin
    ;; call-tool is injected as a foreign fn before this module loads.
    ;; re-export it so (import (harness tools)) provides it.
    #t))
"#;
```

**Step 2: register the module in `load_tool_from_source`**

After `build_managed` but before extracting bindings, register the module:

```rust
ctx.register_module(HARNESS_TOOLS_MODULE)
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("harness module: {e}")))?;
```

**Step 3: run existing tests to verify nothing breaks**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all existing tests pass (module registration is additive)

**Step 4: commit**

```
feat(tein): register (harness tools) module in synthesised tool contexts
```

### Task 2: implement `call-tool` foreign function bridge

The `call-tool` bridge lets scheme code invoke any tool in chibi's registry. It bridges sync tein → async tokio dispatch via `Handle::current().block_on()`.

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

**Step 1: write a failing test**

```rust
#[tokio::test]
async fn test_call_tool_bridge_invokes_registry() {
    // register a simple builtin tool that echoes its arg
    let mut registry = ToolRegistry::new();
    let handler: ToolHandler = Arc::new(|call| {
        let msg = call.args.get("msg").and_then(|v| v.as_str()).unwrap_or("?");
        Box::pin(async move { Ok(format!("echo: {msg}")) })
    });
    registry.register(Tool {
        name: "test_echo".into(),
        description: "echo".into(),
        parameters: serde_json::json!({"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}),
        hooks: vec![],
        metadata: ToolMetadata::new(),
        summary_params: vec![],
        r#impl: ToolImpl::Builtin(handler),
        category: ToolCategory::Shell,
    });
    let registry = Arc::new(RwLock::new(registry));

    // load a scheme tool that calls test_echo via call-tool
    let source = r#"
(import (scheme base))
(import (harness tools))
(define tool-name "caller_tool")
(define tool-description "calls another tool")
(define tool-parameters '())
(define (tool-execute args)
  (call-tool "test_echo" '(("msg" . "hello from scheme"))))
"#;
    let vfs_path = VfsPath::new("/tools/shared/caller.scm").unwrap();
    let tool = load_tool_from_source(source, &vfs_path, &registry).unwrap();

    // execute the tool — it should call test_echo and return the result
    let ctx = match &tool.r#impl {
        ToolImpl::Synthesised { context, .. } => context,
        _ => panic!("expected Synthesised"),
    };
    let call_ctx = make_test_call_context();  // helper — see step 3
    let call = ToolCall {
        name: "caller_tool",
        args: &serde_json::json!({}),
        context: &call_ctx,
    };
    let result = execute_synthesised(ctx, &call).await.unwrap();
    assert_eq!(result, "echo: hello from scheme");
}
```

**Step 2: run test to verify it fails**

Run: `cargo test -p chibi-core test_call_tool_bridge --features synthesised-tools`
Expected: FAIL — `load_tool_from_source` doesn't accept registry, `call-tool` is not defined

**Step 3: implement the bridge**

The key challenge: `call-tool` needs access to the `ToolRegistry` and a `ToolCallContext` at call time. The registry is known at load time (passed to `load_tool_from_source`). The `ToolCallContext` is known at execute time.

Strategy:
- At load time: register a `call-tool` foreign fn that captures `Arc<RwLock<ToolRegistry>>`
- At execute time: before calling `tool-execute`, stash the `ToolCallContext` in a thread-local so the bridge can access it
- The bridge function reads the thread-local to get the context, then dispatches

Add a thread-local for the active call context:

```rust
use std::cell::RefCell;

/// Thread-local storage for the active ToolCallContext during synthesised tool execution.
/// Set before calling tool-execute, cleared after. Lets call-tool access the context
/// without threading it through tein's FFI boundary.
thread_local! {
    static ACTIVE_CALL_CONTEXT: RefCell<Option<ActiveCallContext>> = RefCell::new(None);
}

/// Subset of ToolCallContext that is owned (no lifetimes) for thread-local storage.
struct ActiveCallContext {
    app: *const AppState,
    context_name: String,
    config: *const ResolvedConfig,
    project_root: PathBuf,
    vfs: *const Vfs,
    vfs_caller_context: String,
}
```

**Note:** using raw pointers is necessary because `ToolCallContext` has lifetimes. The pointers are valid for the duration of the `execute_synthesised` call (they point into the caller's stack frame). The thread-local is set/cleared in a guard pattern:

```rust
struct CallContextGuard;

impl CallContextGuard {
    fn set(ctx: &ToolCallContext<'_>) -> Self {
        ACTIVE_CALL_CONTEXT.with(|cell| {
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

impl Drop for CallContextGuard {
    fn drop(&mut self) {
        ACTIVE_CALL_CONTEXT.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}
```

The `call-tool` bridge itself:

```rust
#[tein_fn(name = "call-tool")]
fn call_tool_bridge(name: String, args: tein::Value) -> Result<String, String> {
    // convert scheme alist args to JSON
    let json_args = tein::value_to_json_value(args)
        .map_err(|e| format!("args conversion: {e}"))?;

    ACTIVE_CALL_CONTEXT.with(|cell| {
        let active = cell.borrow();
        let active = active.as_ref()
            .ok_or_else(|| "call-tool: no active call context".to_string())?;

        // SAFETY: pointers are valid for the duration of execute_synthesised,
        // which owns the CallContextGuard that set them.
        let ctx = unsafe {
            ToolCallContext {
                app: &*active.app,
                context_name: &active.context_name,
                config: &*active.config,
                project_root: &active.project_root,
                vfs: &*active.vfs,
                vfs_caller: if active.vfs_caller_context.is_empty() {
                    VfsCaller::System
                } else {
                    VfsCaller::Context(&active.vfs_caller_context)
                },
            }
        };

        // bridge sync → async
        let handle = tokio::runtime::Handle::current();
        let registry = /* captured Arc<RwLock<ToolRegistry>> */;
        let reg = registry.read().map_err(|e| format!("registry lock: {e}"))?;
        let tool_impl = reg.get(&name)
            .ok_or_else(|| format!("unknown tool: {name}"))?
            .r#impl.clone();
        drop(reg);  // release read lock before dispatch

        handle.block_on(async {
            ToolRegistry::dispatch_impl(tool_impl, &name, &json_args, &ctx).await
        }).map_err(|e| format!("tool error: {e}"))
    })
}
```

Update `load_tool_from_source` signature to accept registry:

```rust
pub fn load_tool_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<Tool>
```

Register the foreign fn after building the context:

```rust
ctx.define_fn_variadic("call-tool", __tein_call_tool_bridge)?;
```

Wait — `#[tein_fn]` generates an `unsafe extern "C"` wrapper, but it can't capture the `registry` Arc. We need a different approach. Instead:

- Register `call-tool` as a scheme procedure that wraps a foreign function
- The foreign function reads from a thread-local that holds both the registry AND the call context
- Set the registry in the thread-local at load time (it's static), and the call context at execute time

Revised thread-local:

```rust
thread_local! {
    static CALL_TOOL_STATE: RefCell<Option<CallToolState>> = const { RefCell::new(None) };
}

struct CallToolState {
    registry: Arc<RwLock<ToolRegistry>>,
    call_context: Option<ActiveCallContext>,
}
```

Set the registry once at load time. The guard only updates `call_context`.

Actually, the cleanest approach: the `#[tein_fn]`-generated wrapper is a static function. We can't capture per-context state. So instead, use a thread-local for the entire bridge state. The registry is `Arc` (cheap to clone), set before each tein context creation and cleared after.

Simplest working approach:

```rust
thread_local! {
    static BRIDGE_REGISTRY: RefCell<Option<Arc<RwLock<ToolRegistry>>>> = const { RefCell::new(None) };
    static BRIDGE_CALL_CTX: RefCell<Option<ActiveCallContext>> = const { RefCell::new(None) };
}
```

- `BRIDGE_REGISTRY` is set in `load_tool_from_source` before building the context (so `(harness tools)` can resolve). It stays set — all tools on this thread share the same registry.
- `BRIDGE_CALL_CTX` is set/cleared per `execute_synthesised` call.

The `#[tein_fn]` for `call-tool` reads both thread-locals.

**Step 4: update `execute_synthesised` to set the call context guard**

```rust
pub async fn execute_synthesised(
    context: &ThreadLocalContext,
    call: &ToolCall<'_>,
) -> io::Result<String> {
    let _guard = CallContextGuard::set(call.context);
    let args_alist = json_args_to_scheme_alist(call.args)?;
    // ... rest unchanged
}
```

**Step 5: update all callers of `load_tool_from_source`**

Callers that need updating:
- `scan_and_register` (synthesised.rs:143) — needs registry passed in
- `reload_tool_from_content` (synthesised.rs:191) — already has `Arc<RwLock<ToolRegistry>>`

Update `scan_and_register` signature:

```rust
pub async fn scan_and_register(
    vfs: &Vfs,
    registry: &mut ToolRegistry,
    registry_arc: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<()>
```

Wait — at scan time, we have `&mut ToolRegistry` (write lock held). But `load_tool_from_source` needs `Arc<RwLock<ToolRegistry>>` for the bridge. This is fine: the thread-local holds the Arc, but the bridge won't be called during loading (only during execution). The registry lock contention only matters at execution time.

Actually, simpler: `scan_and_register` already receives `&mut ToolRegistry` (caller holds write lock). Change it to receive `&Arc<RwLock<ToolRegistry>>` instead, acquire the write lock internally after loading.

```rust
pub async fn scan_and_register(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<()>
```

**Step 6: run test to verify it passes**

Run: `cargo test -p chibi-core test_call_tool_bridge --features synthesised-tools`
Expected: PASS

**Step 7: run all synthesised tests**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all pass (existing tests updated for new signature)

**Step 8: commit**

```
feat(tein): call-tool bridge for synthesised tools (#193)

synthesised tools can now call other tools via (call-tool "name" args).
dispatch goes through the full permission + hook stack. blocking bridge
using tokio Handle::current().block_on().
```

### Task 3: update `chibi.rs` callers for new signatures

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs:246-268`

**Step 1: update `scan_and_register` call**

Change from:
```rust
let mut reg = registry.write().unwrap();
crate::tools::vfs_block_on(crate::tools::synthesised::scan_and_register(
    &app.vfs, &mut reg,
))?;
```

To:
```rust
crate::tools::vfs_block_on(crate::tools::synthesised::scan_and_register(
    &app.vfs, &registry,
))?;
```

**Step 2: update `reload_tool_from_content` to pass registry for `load_tool_from_source`**

The hot-reload callback already has `Arc<RwLock<ToolRegistry>>`. Update `reload_tool_from_content` to pass it through to `load_tool_from_source`.

**Step 3: build and run tests**

Run: `cargo build -p chibi-core && cargo test -p chibi-core --features synthesised-tools`
Expected: compiles and all tests pass

**Step 4: commit**

```
refactor(tein): update callers for registry-aware tool loading
```

### Task 4: add `tein::value_to_json_value` or equivalent

The `call-tool` bridge needs to convert the scheme alist args back to `serde_json::Value`. Check if tein provides `value_to_json_value` (the reverse of `json_value_to_value`).

**Files:**
- Possibly modify: tein crate (if conversion doesn't exist)
- Or: `crates/chibi-core/src/tools/synthesised.rs` (local conversion function)

**Step 1: check tein for existing reverse conversion**

Look for `value_to_json_value` or similar in tein's json module.

**Step 2: if missing, implement a local helper**

```rust
fn scheme_value_to_json(val: &Value) -> io::Result<serde_json::Value> {
    match val {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(n) => Ok(serde_json::json!(*n)),
        Value::Float(f) => Ok(serde_json::json!(*f)),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::List(items) => {
            // check if it's an alist (list of pairs) → JSON object
            // or a plain list → JSON array
            if items.iter().all(|item| matches!(item, Value::Pair(_, _) | Value::List(ref l) if l.len() == 2)) {
                // alist → object
                let mut map = serde_json::Map::new();
                for item in items {
                    match item {
                        Value::Pair(k, v) => {
                            let key = k.as_string()
                                .or_else(|| k.as_symbol())
                                .ok_or_else(|| io::Error::other("non-string alist key"))?;
                            map.insert(key.to_string(), scheme_value_to_json(v)?);
                        }
                        _ => {} // skip non-pair items
                    }
                }
                Ok(serde_json::Value::Object(map))
            } else {
                // plain list → array
                let arr: io::Result<Vec<_>> = items.iter().map(scheme_value_to_json).collect();
                Ok(serde_json::Value::Array(arr?))
            }
        }
        Value::Pair(k, v) => {
            // single pair → two-element array or special handling
            let arr = vec![scheme_value_to_json(k)?, scheme_value_to_json(v)?];
            Ok(serde_json::Value::Array(arr))
        }
        _ => Err(io::Error::other(format!("unsupported scheme value type: {val}"))),
    }
}
```

**Step 3: write test for conversion**

```rust
#[test]
fn test_scheme_value_to_json_alist() {
    let alist = Value::List(vec![
        Value::Pair(Box::new(Value::String("cmd".into())), Box::new(Value::String("ls".into()))),
    ]);
    let json = scheme_value_to_json(&alist).unwrap();
    assert_eq!(json, serde_json::json!({"cmd": "ls"}));
}
```

**Step 4: run test**

Run: `cargo test -p chibi-core scheme_value_to_json --features synthesised-tools`
Expected: PASS

**Step 5: commit**

```
feat(tein): scheme value to JSON conversion for call-tool args
```

---

## Layer 2: multi-zone scanning & visibility scoping

### Task 5: extend startup scan to all tool zones

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs:143-181`

**Step 1: write failing test**

```rust
#[tokio::test]
async fn test_scan_registers_from_home_zone() {
    let (_dir, vfs) = make_test_vfs();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));

    // write a tool to /tools/home/alice/
    let path = VfsPath::new("/tools/home/alice/my_tool.scm").unwrap();
    vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes()).await.unwrap();

    scan_and_register(&vfs, &registry).await.unwrap();
    assert!(registry.read().unwrap().get("scan_hello").is_some());
}

#[tokio::test]
async fn test_scan_registers_from_flock_zone() {
    let (_dir, vfs) = make_test_vfs();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));

    // create flock and write tool
    vfs.flock_join("dev-team", "alice").await.unwrap();
    let path = VfsPath::new("/tools/flocks/dev-team/helper.scm").unwrap();
    vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes()).await.unwrap();

    scan_and_register(&vfs, &registry).await.unwrap();
    assert!(registry.read().unwrap().get("scan_hello").is_some());
}
```

**Step 2: run tests to verify they fail**

Run: `cargo test -p chibi-core test_scan_registers_from --features synthesised-tools`
Expected: FAIL — scan only covers `/tools/shared`

**Step 3: extend `scan_and_register` to scan additional zones**

Replace the hardcoded `zones` array with dynamic discovery:

```rust
pub async fn scan_and_register(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<()> {
    let mut zones = vec!["/tools/shared".to_string()];

    // discover /tools/home/<ctx>/ directories
    if let Ok(entries) = vfs.list(VfsCaller::System, &VfsPath::new("/tools/home")?).await {
        for entry in entries {
            zones.push(format!("/tools/home/{}", entry.name));
        }
    }

    // discover /tools/flocks/<name>/ directories
    if let Ok(entries) = vfs.list(VfsCaller::System, &VfsPath::new("/tools/flocks")?).await {
        for entry in entries {
            zones.push(format!("/tools/flocks/{}", entry.name));
        }
    }

    for zone in &zones {
        scan_zone(vfs, registry, zone).await?;
    }
    Ok(())
}

async fn scan_zone(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
    zone: &str,
) -> io::Result<()> {
    let Ok(zone_path) = VfsPath::new(zone) else {
        return Ok(());
    };
    if !vfs.exists(VfsCaller::System, &zone_path).await.unwrap_or(false) {
        return Ok(());
    }
    let entries = match vfs.list(VfsCaller::System, &zone_path).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
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
            Err(_) => continue,
        };
        let Ok(source_str) = String::from_utf8(source) else {
            continue;
        };

        // set bridge registry thread-local before loading
        BRIDGE_REGISTRY.with(|cell| {
            *cell.borrow_mut() = Some(Arc::clone(registry));
        });

        if let Ok(tool) = load_tool_from_source(&source_str, &file_path, registry) {
            registry.write().unwrap().register(tool);
        }
    }
    Ok(())
}
```

**Step 4: run tests to verify they pass**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all pass

**Step 5: commit**

```
feat(tein): scan /tools/home/ and /tools/flocks/ zones at startup (#193)
```

### Task 6: visibility scoping in send.rs

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (near the `filter_tools_by_config` function)
- Modify: `crates/chibi-core/src/tools/registry.rs` (add helper method)

**Step 1: add `is_visible_to` helper on `ToolRegistry`**

```rust
/// Check if a synthesised tool is visible to a given context.
///
/// Visibility rules:
/// - Non-synthesised tools: always visible
/// - /tools/shared/*: visible to all
/// - /tools/home/<ctx>/*: visible only to <ctx>
/// - /tools/flocks/<name>/*: visible to members of <name>
#[cfg(feature = "synthesised-tools")]
pub fn is_tool_visible(&self, tool_name: &str, context_name: &str, flock_memberships: &[String]) -> bool {
    let tool = match self.get(tool_name) {
        Some(t) => t,
        None => return false,
    };
    match &tool.r#impl {
        ToolImpl::Synthesised { vfs_path, .. } => {
            let path = vfs_path.as_str();
            if path.starts_with("/tools/shared/") {
                true
            } else if let Some(rest) = path.strip_prefix("/tools/home/") {
                // /tools/home/alice/foo.scm → owner is "alice"
                rest.split('/').next() == Some(context_name)
            } else if let Some(rest) = path.strip_prefix("/tools/flocks/") {
                // /tools/flocks/dev-team/foo.scm → flock is "dev-team"
                rest.split('/').next()
                    .map(|flock| flock_memberships.iter().any(|f| f == flock))
                    .unwrap_or(false)
            } else {
                true // unknown zone — visible by default
            }
        }
        _ => true, // non-synthesised tools always visible
    }
}
```

**Step 2: write test for visibility**

```rust
#[test]
fn test_visibility_shared_visible_to_all() {
    let reg = registry_with_synth_tool("shared_tool", "/tools/shared/tool.scm");
    assert!(reg.is_tool_visible("shared_tool", "alice", &[]));
    assert!(reg.is_tool_visible("shared_tool", "bob", &[]));
}

#[test]
fn test_visibility_home_only_owner() {
    let reg = registry_with_synth_tool("alice_tool", "/tools/home/alice/tool.scm");
    assert!(reg.is_tool_visible("alice_tool", "alice", &[]));
    assert!(!reg.is_tool_visible("alice_tool", "bob", &[]));
}

#[test]
fn test_visibility_flock_members_only() {
    let reg = registry_with_synth_tool("flock_tool", "/tools/flocks/dev/tool.scm");
    assert!(reg.is_tool_visible("flock_tool", "alice", &["dev".into()]));
    assert!(!reg.is_tool_visible("flock_tool", "bob", &[]));
}

#[test]
fn test_visibility_builtin_always_visible() {
    let mut reg = ToolRegistry::new();
    // ... register a builtin tool ...
    assert!(reg.is_tool_visible("builtin_tool", "anyone", &[]));
}
```

**Step 3: run tests**

Run: `cargo test -p chibi-core visibility --features synthesised-tools`
Expected: PASS

**Step 4: apply visibility filter in send.rs**

In the tool filtering pipeline (near line 1994 in `send.rs`), after `filter_tools_by_config`, add visibility filtering:

```rust
// filter synthesised tools by visibility
let context_flocks = crate::tools::vfs_block_on(
    app.vfs.flock_list_for(context_name)
).unwrap_or_default();

let registry_guard = registry.read().unwrap();
all_tools.retain(|tool| {
    let name = tool.get("function")
        .and_then(|f| f.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    registry_guard.is_tool_visible(name, context_name, &context_flocks)
});
drop(registry_guard);
```

**Step 5: run full test suite**

Run: `cargo test -p chibi-core --features synthesised-tools`
Expected: all pass

**Step 6: commit**

```
feat(tein): visibility scoping for synthesised tools (#193)

tools in /tools/home/<ctx>/ visible only to owner, /tools/flocks/<name>/
visible only to flock members. /tools/shared/ and builtins always visible.
```

---

## Layer 3: `define-tool` macro & tier configuration

### Task 7: implement `define-tool` scheme macro

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

**Step 1: write failing test**

```rust
#[test]
fn test_load_multiple_tools_from_define_tool() {
    let source = r#"
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
    let vfs_path = VfsPath::new("/tools/shared/multi.scm").unwrap();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let tools = load_tools_from_source(source, &vfs_path, &registry).unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "greet");
    assert_eq!(tools[1].name, "farewell");
}
```

**Step 2: run test to verify it fails**

Run: `cargo test -p chibi-core test_load_multiple_tools --features synthesised-tools`
Expected: FAIL — `load_tools_from_source` doesn't exist

**Step 3: update `(harness tools)` module to include `define-tool` macro**

The macro expands `define-tool` into a call that appends to a module-level `%tool-registry%` list:

```scheme
(define-library (harness tools)
  (import (scheme base))
  (export call-tool define-tool)
  (begin
    (define %tool-registry% '())

    (define-syntax define-tool
      (syntax-rules (description parameters execute)
        ((define-tool name
           (description desc)
           (parameters params)
           (execute handler))
         (set! %tool-registry%
           (cons (list (symbol->string 'name) desc params handler)
                 %tool-registry%)))))))
```

After evaluation, rust reads `%tool-registry%` to extract tool definitions.

**Step 4: implement `load_tools_from_source`**

```rust
pub fn load_tools_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<Vec<Tool>> {
    // set bridge registry for call-tool
    BRIDGE_REGISTRY.with(|cell| {
        *cell.borrow_mut() = Some(Arc::clone(registry));
    });

    let source = source.to_string();
    let ctx = Context::builder()
        .standard_env()
        .sandboxed(Modules::Safe)
        .step_limit(10_000_000)
        .build_managed(move |ctx| {
            ctx.evaluate(&source)?;
            Ok(())
        })
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tein init: {e}")))?;

    ctx.register_module(HARNESS_TOOLS_MODULE)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("harness module: {e}")))?;

    // re-evaluate source now that (harness tools) is available
    // (the build_managed closure ran before module registration)
    // Actually: we need to restructure — register module BEFORE evaluating source.
    // This requires registering it in the build_managed closure.
    // But register_module needs a built context...
    //
    // Solution: register the module source in the VFS before building,
    // OR use a two-phase approach where we register the module in the
    // build_managed closure itself.

    // Try: check if %tool-registry% exists (define-tool was used)
    let multi_tools = ctx.evaluate("%tool-registry%").ok();

    if let Some(Value::List(entries)) = multi_tools.as_ref().filter(|v| !matches!(v, Value::Nil)) {
        // multi-tool mode: extract from %tool-registry%
        let mut tools = Vec::new();
        for entry in entries.iter().rev() {  // reverse: registry is built via cons (LIFO)
            if let Value::List(fields) = entry {
                if fields.len() >= 4 {
                    let name = fields[0].as_string()
                        .ok_or_else(|| io::Error::other("define-tool: name not a string"))?
                        .to_string();
                    let description = fields[1].as_string()
                        .ok_or_else(|| io::Error::other("define-tool: description not a string"))?
                        .to_string();
                    let parameters = params_alist_to_json_schema(&fields[2])?;
                    if !fields[3].is_procedure() {
                        return Err(io::Error::other(format!("define-tool {name}: execute is not a procedure")));
                    }

                    // Store the handler in the context for later execution.
                    // Bind it to a known name so execute_synthesised can find it.
                    let exec_binding = format!("%tool-execute-{name}%");
                    ctx.evaluate(&format!("(define {exec_binding} (list-ref (car %tool-registry%) 3))"))
                        .map_err(|e| io::Error::other(format!("binding {exec_binding}: {e}")))?;

                    let context = Arc::new(/* need a per-tool context or shared context */);
                    // Problem: all tools share one tein context but execute_synthesised
                    // looks up "tool-execute" by name. For multi-tool, each tool needs
                    // its own execute binding.
                    //
                    // Solution: change execute_synthesised to accept a binding name,
                    // or store it in the ToolImpl::Synthesised variant.

                    tools.push(Tool {
                        name,
                        description,
                        parameters,
                        hooks: vec![],
                        metadata: ToolMetadata::new(),
                        summary_params: vec![],
                        r#impl: ToolImpl::Synthesised {
                            vfs_path: vfs_path.clone(),
                            context: context.clone(),
                        },
                        category: ToolCategory::Synthesised,
                    });
                }
            }
        }
        Ok(tools)
    } else {
        // single-tool mode: fall back to convention-based format
        load_tool_from_source_inner(&ctx, vfs_path).map(|t| vec![t])
    }
}
```

**Design decision for multi-tool execution:** add an `exec_binding` field to `ToolImpl::Synthesised`:

```rust
ToolImpl::Synthesised {
    vfs_path: VfsPath,
    context: Arc<ThreadLocalContext>,
    exec_binding: String,  // "tool-execute" for single, "%tool-execute-{name}%" for multi
}
```

Update `execute_synthesised` to use `exec_binding` instead of hardcoded `"tool-execute"`.

**Step 5: run tests**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all pass

**Step 6: write test for backwards compat (single-tool convention still works)**

```rust
#[test]
fn test_load_tools_backwards_compat_single_tool() {
    let source = r#"
(import (scheme base))
(define tool-name "old_style")
(define tool-description "uses convention format")
(define tool-parameters '())
(define (tool-execute args) "works")
"#;
    let vfs_path = VfsPath::new("/tools/shared/old.scm").unwrap();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let tools = load_tools_from_source(source, &vfs_path, &registry).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "old_style");
}
```

**Step 7: run test**

Run: `cargo test -p chibi-core test_load_tools_backwards --features synthesised-tools`
Expected: PASS

**Step 8: commit**

```
feat(tein): define-tool macro for multi-tool files (#193)

(define-tool name (description ...) (parameters ...) (execute ...))
allows multiple tools per .scm file. single-tool convention format
still works for backwards compatibility.
```

### Task 8: update hot-reload for multi-tool files

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (reload/unregister functions)
- Modify: `crates/chibi-core/src/tools/registry.rs` (add `find_all_by_vfs_path`)

**Step 1: add `find_all_by_vfs_path` to registry**

```rust
#[cfg(feature = "synthesised-tools")]
pub fn find_all_by_vfs_path(&self, path: &VfsPath) -> Vec<String> {
    self.tools.values()
        .filter_map(|t| match &t.r#impl {
            ToolImpl::Synthesised { vfs_path, .. } if vfs_path == path => Some(t.name.clone()),
            _ => None,
        })
        .collect()
}
```

**Step 2: update `reload_tool_from_content` for multi-tool**

```rust
pub fn reload_tool_from_content(
    registry: &Arc<RwLock<ToolRegistry>>,
    path: &VfsPath,
    content: &[u8],
) {
    let Ok(source_str) = std::str::from_utf8(content) else {
        return;
    };
    match load_tools_from_source(source_str, path, registry) {
        Ok(tools) => {
            let mut reg = registry.write().unwrap();
            // unregister old tools from this path
            let old_names = reg.find_all_by_vfs_path(path);
            for name in &old_names {
                reg.unregister(name);
            }
            // register new tools
            for tool in tools {
                reg.register(tool);
            }
        }
        Err(_) => {
            // invalid source — leave previous versions registered
        }
    }
}
```

**Step 3: update `unregister_tool_at_path` for multi-tool**

```rust
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
```

**Step 4: write test for multi-tool hot-reload**

```rust
#[tokio::test]
async fn test_hot_reload_multi_tool_file() {
    let (_dir, vfs) = make_test_vfs();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));

    let source_v1 = r#"
(import (scheme base))
(import (harness tools))
(define-tool tool_a (description "a") (parameters '()) (execute (lambda (args) "a")))
(define-tool tool_b (description "b") (parameters '()) (execute (lambda (args) "b")))
"#;

    let path = VfsPath::new("/tools/shared/multi.scm").unwrap();
    reload_tool_from_content(&registry, &path, source_v1.as_bytes());

    assert!(registry.read().unwrap().get("tool_a").is_some());
    assert!(registry.read().unwrap().get("tool_b").is_some());

    // update: remove tool_b, add tool_c
    let source_v2 = r#"
(import (scheme base))
(import (harness tools))
(define-tool tool_a (description "a v2") (parameters '()) (execute (lambda (args) "a2")))
(define-tool tool_c (description "c") (parameters '()) (execute (lambda (args) "c")))
"#;

    reload_tool_from_content(&registry, &path, source_v2.as_bytes());

    let reg = registry.read().unwrap();
    assert!(reg.get("tool_a").is_some());
    assert!(reg.get("tool_b").is_none(), "tool_b should be unregistered");
    assert!(reg.get("tool_c").is_some());
}
```

**Step 5: run tests**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all pass

**Step 6: commit**

```
feat(tein): multi-tool hot-reload — unregister old, register new (#193)
```

### Task 9: tier configuration

**Files:**
- Modify: `crates/chibi-core/src/config.rs` (add `TiersConfig`)
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (accept tier param)

**Step 1: add `SandboxTier` enum and config**

In `config.rs`:

```rust
/// Sandbox tier for synthesised tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxTier {
    #[default]
    Sandboxed,     // tier 1: Modules::Safe, step limit
    Unsandboxed,   // tier 2: full r7rs, no step limit
}
```

Add tier config to `ToolsConfig`:

```rust
pub struct ToolsConfig {
    // ... existing fields ...

    /// Sandbox tier overrides for synthesised tools.
    /// Keys are VFS paths (tool or zone), values are tier numbers (1 or 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<std::collections::HashMap<String, u8>>,
}
```

Add tier resolution:

```rust
impl ToolsConfig {
    /// Resolve the sandbox tier for a tool at the given VFS path.
    /// Most specific path wins. Absent = tier 1 (sandboxed).
    pub fn resolve_tier(&self, vfs_path: &str) -> SandboxTier {
        let tiers = match &self.tiers {
            Some(t) => t,
            None => return SandboxTier::Sandboxed,
        };
        // find most specific (longest) matching path
        let mut best_match: Option<(&str, u8)> = None;
        for (pattern, tier) in tiers {
            if vfs_path.starts_with(pattern.as_str()) {
                match best_match {
                    None => best_match = Some((pattern, *tier)),
                    Some((prev, _)) if pattern.len() > prev.len() => {
                        best_match = Some((pattern, *tier));
                    }
                    _ => {}
                }
            }
        }
        match best_match {
            Some((_, 2)) => SandboxTier::Unsandboxed,
            _ => SandboxTier::Sandboxed,
        }
    }
}
```

**Step 2: write test for tier resolution**

```rust
#[test]
fn test_tier_resolution_default_sandboxed() {
    let config = ToolsConfig::default();
    assert_eq!(config.resolve_tier("/tools/shared/foo.scm"), SandboxTier::Sandboxed);
}

#[test]
fn test_tier_resolution_specific_path_wins() {
    let mut tiers = std::collections::HashMap::new();
    tiers.insert("/tools/home/admin/".into(), 2);
    tiers.insert("/tools/home/admin/safe.scm".into(), 1);
    let config = ToolsConfig { tiers: Some(tiers), ..Default::default() };

    assert_eq!(config.resolve_tier("/tools/home/admin/danger.scm"), SandboxTier::Unsandboxed);
    assert_eq!(config.resolve_tier("/tools/home/admin/safe.scm"), SandboxTier::Sandboxed);
}
```

**Step 3: run tests**

Run: `cargo test -p chibi-core tier_resolution`
Expected: PASS

**Step 4: update `load_tools_from_source` to accept tier**

```rust
pub fn load_tools_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    tier: SandboxTier,
) -> io::Result<Vec<Tool>>
```

Use tier to control context building:

```rust
let ctx = match tier {
    SandboxTier::Sandboxed => Context::builder()
        .standard_env()
        .sandboxed(Modules::Safe)
        .step_limit(10_000_000)
        .build_managed(/* ... */)?,
    SandboxTier::Unsandboxed => Context::builder()
        .standard_env()
        .build_managed(/* ... */)?,
};
```

**Step 5: update all callers to pass tier**

- `scan_and_register` → needs access to config for tier resolution
- `reload_tool_from_content` → needs config access
- Add config parameter or pass tier resolver

**Step 6: write test for tier 2 execution**

```rust
#[test]
fn test_tier2_allows_full_scheme() {
    let source = r#"
(import (scheme base))
(import (scheme file))  ; not available in sandboxed mode
(define tool-name "tier2_tool")
(define tool-description "uses full scheme")
(define tool-parameters '())
(define (tool-execute args) "full scheme works")
"#;
    let vfs_path = VfsPath::new("/tools/shared/full.scm").unwrap();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let result = load_tools_from_source(source, &vfs_path, &registry, SandboxTier::Unsandboxed);
    assert!(result.is_ok());
}

#[test]
fn test_tier1_rejects_unsafe_imports() {
    let source = r#"
(import (scheme base))
(import (scheme file))  ; should fail in sandboxed
(define tool-name "bad_tool")
(define tool-description "tries unsafe import")
(define tool-parameters '())
(define (tool-execute args) "should not load")
"#;
    let vfs_path = VfsPath::new("/tools/shared/bad.scm").unwrap();
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let result = load_tools_from_source(source, &vfs_path, &registry, SandboxTier::Sandboxed);
    assert!(result.is_err());
}
```

**Step 7: run all tests**

Run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: all pass

**Step 8: commit**

```
feat(tein): configurable sandbox tiers for synthesised tools (#193)

tier 1 (default): sandboxed, safe modules only, step limit.
tier 2 (opt-in via [tools.tiers] config): full r7rs, no limits.
most specific path wins in tier resolution.
```

---

## Finalization

### Task 10: update documentation

**Files:**
- Modify: `docs/plugins.md` — document `(harness tools)` module, `call-tool`, `define-tool`
- Modify: `docs/configuration.md` — document `[tools.tiers]` config
- Modify: `docs/vfs.md` — document `/tools/home/` and `/tools/flocks/` scanning
- Modify: `AGENTS.md` — add any quirks discovered during implementation

**Step 1: update docs with new features**

Document:
- `(harness tools)` module: `call-tool` and `define-tool` usage
- `define-tool` macro syntax with examples
- tier configuration in `chibi.toml`
- visibility scoping rules
- backwards compatibility with convention-based format

**Step 2: commit**

```
docs: document harness tools module, tiers, and visibility scoping (#193)
```

### Task 11: integration test

**Files:**
- Create: `crates/chibi-core/src/tools/synthesised.rs` (add integration test)

**Step 1: write end-to-end test**

```rust
#[tokio::test]
async fn test_integration_define_tool_with_call_tool() {
    let (_dir, vfs) = make_test_vfs();
    let mut base_registry = ToolRegistry::new();

    // register a builtin echo tool
    let handler: ToolHandler = Arc::new(|call| {
        let msg = call.args.get("msg").and_then(|v| v.as_str()).unwrap_or("?");
        Box::pin(async move { Ok(format!("echoed: {msg}")) })
    });
    base_registry.register(Tool {
        name: "echo".into(),
        description: "echo".into(),
        parameters: serde_json::json!({"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}),
        hooks: vec![],
        metadata: ToolMetadata::new(),
        summary_params: vec![],
        r#impl: ToolImpl::Builtin(handler),
        category: ToolCategory::Shell,
    });

    let registry = Arc::new(RwLock::new(base_registry));

    // write a multi-tool .scm file that uses call-tool
    let source = r#"
(import (scheme base))
(import (harness tools))

(define-tool shout
  (description "shout a message")
  (parameters '((msg . ((type . "string") (description . "message")))))
  (execute (lambda (args)
    (call-tool "echo" `(("msg" . ,(string-append "SHOUT: " (cdr (assoc "msg" args)))))))))

(define-tool whisper
  (description "whisper a message")
  (parameters '((msg . ((type . "string") (description . "message")))))
  (execute (lambda (args)
    (call-tool "echo" `(("msg" . ,(string-append "whisper: " (cdr (assoc "msg" args)))))))))
"#;

    let path = VfsPath::new("/tools/shared/voice.scm").unwrap();
    vfs.write(VfsCaller::System, &path, source.as_bytes()).await.unwrap();

    scan_and_register(&vfs, &registry).await.unwrap();

    let reg = registry.read().unwrap();
    assert!(reg.get("shout").is_some());
    assert!(reg.get("whisper").is_some());

    // execute shout — should call echo via call-tool
    let shout = reg.get("shout").unwrap();
    let ctx = match &shout.r#impl {
        ToolImpl::Synthesised { context, .. } => context.clone(),
        _ => panic!("expected Synthesised"),
    };
    drop(reg);

    let call_ctx = make_test_call_context();
    let call = ToolCall {
        name: "shout",
        args: &serde_json::json!({"msg": "hello"}),
        context: &call_ctx,
    };
    let result = execute_synthesised(&ctx, &call).await.unwrap();
    assert_eq!(result, "echoed: SHOUT: hello");
}
```

**Step 2: run test**

Run: `cargo test -p chibi-core test_integration_define_tool --features synthesised-tools`
Expected: PASS

**Step 3: run full test suite**

Run: `cargo test --workspace`
Expected: all pass

**Step 4: commit**

```
test(tein): integration test for define-tool + call-tool chain (#193)
```

### Task 12: final lint and cleanup

**Step 1: run lint**

Run: `just lint`

**Step 2: fix any warnings**

**Step 3: collect AGENTS.md notes**

Add any quirks discovered during implementation.

**Step 4: commit**

```
chore: lint fixes and AGENTS.md updates for tein integration (#193)
```
