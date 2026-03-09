# Tein Hook Registration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow synthesised tein tools to register for hook points, making them first-class hook participants alongside subprocess plugins.

**Architecture:** Extend `register-hook` from scheme (via `%hook-registry%` binding) → rust reads it after eval → populates `Tool.hooks` + new `hook_bindings` map in `ToolImpl::Synthesised` → `execute_hook` dispatches to tein callbacks via `ThreadLocalContext::call()` alongside subprocess plugins. Re-entrancy guard prevents recursive tein hook invocation.

**Tech Stack:** Rust, tein (scheme), existing hook/synthesised tool infrastructure in `crates/chibi-core/src/tools/`

**Branch:** `just feature tein-hook-registration-2603`

**Closes:** #220

---

### Task 1: Add `hook_bindings` field to `ToolImpl::Synthesised`

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs:76-86` (ToolImpl::Synthesised variant)

**Step 1: Add the field**

At `registry.rs:76-86`, the `Synthesised` variant currently has `vfs_path`, `exec_binding`, `context`, `registry`, `worker_thread_id`. Add `hook_bindings`:

```rust
#[cfg(feature = "synthesised-tools")]
Synthesised {
    vfs_path: crate::vfs::VfsPath,
    exec_binding: String,
    context: std::sync::Arc<tein::ThreadLocalContext>,
    registry: Arc<RwLock<ToolRegistry>>,
    /// The tein worker thread's `ThreadId`, captured at context init time.
    /// Used as the key in `BRIDGE_CALL_CTX` so concurrent synthesised tool
    /// calls from different tein contexts never overwrite each other's entry.
    worker_thread_id: std::thread::ThreadId,
    /// Maps hook points to scheme binding names for hook callbacks.
    /// Populated from `%hook-registry%` during tool loading.
    hook_bindings: std::collections::HashMap<super::hooks::HookPoint, String>,
},
```

**Step 2: Update all construction sites to include `hook_bindings: HashMap::new()`**

There are 3 places that construct `ToolImpl::Synthesised`:
- `synthesised.rs:517-523` (extract_single_tool)
- `synthesised.rs:626-632` (extract_multi_tools)
- Any test code constructing this variant

Add `hook_bindings: std::collections::HashMap::new()` to each.

**Step 3: Verify it compiles**

Run: `cargo build -p chibi-core 2>&1 | tail -5`
Expected: compiles clean (or clippy warnings only).

**Step 4: Commit**

```
feat(hooks): add hook_bindings field to ToolImpl::Synthesised

preparatory field for tein hook registration (#220). maps hook points
to scheme binding names so execute_hook can dispatch to tein callbacks.
```

---

### Task 2: Add `register-hook` to harness preamble and `(harness hooks)` module

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs:88-134` (HARNESS_TOOLS_MODULE, HARNESS_PREAMBLE)

**Step 1: Add `%hook-registry%` to HARNESS_PREAMBLE**

At `synthesised.rs:112-134`, add after `%tool-registry%` definition:

```scheme
;; accumulates hook registrations. each entry is a list:
;; (hook-name-string handler-procedure)
;; rust reads %hook-registry% after evaluation to populate Tool.hooks.
(define %hook-registry% '())
```

And add `register-hook` procedure:

```scheme
;; registers a hook handler for a given hook point.
;; hook-name is a symbol (e.g. 'pre_vfs_write).
;; handler is a procedure taking one argument (the hook payload as an alist)
;; and returning an alist (or '() for no-op).
(define (register-hook hook-name handler)
  (set! %hook-registry%
    (cons (list (symbol->string hook-name) handler)
          %hook-registry%)))
```

The full updated HARNESS_PREAMBLE should be:

```rust
const HARNESS_PREAMBLE: &str = r#"
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

;; registers a hook handler for a given hook point.
;; hook-name is a symbol (e.g. 'pre_vfs_write).
;; handler is a procedure taking one argument (the hook payload as an alist)
;; and returning an alist (or '() for no-op).
(define (register-hook hook-name handler)
  (set! %hook-registry%
    (cons (list (symbol->string hook-name) handler)
          %hook-registry%)))
"#;
```

**Step 2: Add `(harness hooks)` module**

After HARNESS_TOOLS_MODULE (`synthesised.rs:88-97`), add a new constant:

```rust
#[cfg(feature = "synthesised-tools")]
const HARNESS_HOOKS_MODULE: &str = r#"
(define-library (harness hooks)
  (import (scheme base))
  (export register-hook)
  (begin
    ;; register-hook is defined in HARNESS_PREAMBLE (top-level).
    ;; re-export it so (import (harness hooks)) provides it.
    #t))
"#;
```

**Step 3: Register the module in `build_tein_context`**

At `synthesised.rs:382-383`, after registering HARNESS_TOOLS_MODULE, add:

```rust
ctx.register_module(HARNESS_HOOKS_MODULE)
    .map_err(|e| tein::Error::EvalError(format!("harness hooks module: {e}")))?;
```

**Step 4: Verify it compiles**

Run: `cargo build -p chibi-core 2>&1 | tail -5`
Expected: compiles clean.

**Step 5: Commit**

```
feat(hooks): add register-hook and (harness hooks) module

tein tools can now call (register-hook 'hook_name handler) to register
for hook points. %hook-registry% accumulates registrations for rust to
read after evaluation. (#220)
```

---

### Task 3: Read `%hook-registry%` during tool extraction and populate hooks

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs:458-526` (extract_single_tool)
- Modify: `crates/chibi-core/src/tools/synthesised.rs:538-638` (extract_multi_tools)

**Step 1: Write a helper to extract hook registrations**

Add a new helper function after `extract_string` (around line 870):

```rust
/// Read `%hook-registry%` from a tein context and return (hooks, hook_bindings).
///
/// Each entry in `%hook-registry%` is `(hook-name-string handler-procedure)`.
/// For each valid entry, we:
/// 1. Parse the hook name string into a `HookPoint`.
/// 2. Bind the handler to `%hook-{hook_name}%` in the context.
/// 3. Record the mapping in `hook_bindings`.
///
/// Invalid hook names are warned and skipped (same as plugin hook parsing).
#[cfg(feature = "synthesised-tools")]
fn extract_hook_registrations(
    ctx: &ThreadLocalContext,
) -> io::Result<(Vec<HookPoint>, std::collections::HashMap<HookPoint, String>)> {
    let registry_val = ctx
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
        // look up handler from %hook-registry% by name (first match)
        ctx.evaluate(&format!(
            "(define {binding} \
             (cadr \
               (let loop ((reg %hook-registry%)) \
                 (if (string=? (car (car reg)) \"{hook_name_escaped}\") \
                     (car reg) \
                     (loop (cdr reg))))))"
        ))
        .map_err(|e| io::Error::other(format!("binding {binding}: {e}")))?;

        hooks.push(hook_point);
        hook_bindings.insert(hook_point, binding);
    }

    Ok((hooks, hook_bindings))
}
```

**Step 2: Call it in `extract_single_tool`**

In `extract_single_tool` (around line 509), before constructing the `Tool`, add:

```rust
let (hooks, hook_bindings) = extract_hook_registrations(&ctx)?;
```

Then update the Tool construction at line 510-526 to use `hooks` and `hook_bindings`:

```rust
let context = Arc::new(ctx);
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
```

**Step 3: Call it in `extract_multi_tools`**

In `extract_multi_tools` (around line 557), after `let context = Arc::new(ctx);`:

```rust
let (hooks, hook_bindings) = extract_hook_registrations(&context)?;
```

Note: `extract_hook_registrations` takes `&ThreadLocalContext`, and `context` is now `Arc<ThreadLocalContext>`. We need to call it *before* the `Arc::new(ctx)` wrapping, so move it to just before line 557:

```rust
let (hooks, hook_bindings) = extract_hook_registrations(&ctx)?;
let context = Arc::new(ctx);
```

Then update each Tool construction in the loop (line 619-634) to use `hooks: hooks.clone()` and `hook_bindings: hook_bindings.clone()`:

```rust
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
```

Note: all tools from the same `.scm` file share the same hooks. This is correct — the hooks are file-level, not per-tool.

**Step 4: Write a test**

Add a test in `synthesised.rs` tests (or a new test module):

```rust
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
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    assert_eq!(tools.len(), 1);
    assert!(tools[0].hooks.contains(&super::hooks::HookPoint::OnStart));
    assert!(tools[0].hooks.contains(&super::hooks::HookPoint::PreMessage));

    if let ToolImpl::Synthesised { hook_bindings, .. } = &tools[0].r#impl {
        assert!(hook_bindings.contains_key(&super::hooks::HookPoint::OnStart));
        assert!(hook_bindings.contains_key(&super::hooks::HookPoint::PreMessage));
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
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    assert_eq!(tools.len(), 1);
    // only valid hook should be registered
    assert_eq!(tools[0].hooks.len(), 1);
    assert!(tools[0].hooks.contains(&super::hooks::HookPoint::OnStart));
}
```

**Step 5: Run tests**

Run: `cargo test -p chibi-core test_hook_registration -- --nocapture 2>&1 | tail -20`
Expected: both tests pass.

**Step 6: Commit**

```
feat(hooks): extract hook registrations from %hook-registry% into Tool.hooks

register-hook calls in scheme now populate Tool.hooks and
ToolImpl::Synthesised.hook_bindings at load time. invalid hook names
are warned and skipped. (#220)
```

---

### Task 4: Extend `execute_hook` to dispatch tein callbacks

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs:52-125` (execute_hook)
- Modify: `crates/chibi-core/src/api/send.rs:1873-1881` (plugin_tools filter)

**Step 1: Write the failing test**

Add to `hooks.rs` tests:

```rust
#[test]
#[cfg(feature = "synthesised-tools")]
fn test_execute_hook_dispatches_to_synthesised() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::{ToolRegistry, ToolImpl};
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    let source = r#"
(import (harness hooks))
(register-hook 'on_start
  (lambda (payload)
    (list (cons "saw_event" (cdr (assoc "event" payload))))))

(define tool-name "tein-hook-test")
(define tool-description "Hook tester")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/hook-test.scm").unwrap();
    let tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    let data = serde_json::json!({"event": "start"});
    let results = execute_hook(&tools, HookPoint::OnStart, &data).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "tein-hook-test");
    assert_eq!(results[0].1["saw_event"], "start");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core test_execute_hook_dispatches_to_synthesised -- --nocapture 2>&1 | tail -20`
Expected: fails — `execute_hook` currently skips non-Plugin tools.

**Step 3: Add tein dispatch branch to `execute_hook`**

In `hooks.rs`, add imports at the top:

```rust
#[cfg(feature = "synthesised-tools")]
use std::collections::HashSet;
#[cfg(feature = "synthesised-tools")]
use std::cell::RefCell;
```

Add re-entrancy guard (thread-local):

```rust
#[cfg(feature = "synthesised-tools")]
thread_local! {
    /// Tracks which hook points are currently being dispatched to tein callbacks.
    /// Prevents re-entrancy: if a tein hook callback triggers an action that
    /// fires the same hook point, tein callbacks are skipped on the recursive call.
    static TEIN_HOOK_GUARD: RefCell<HashSet<HookPoint>> = RefCell::new(HashSet::new());
}
```

In `execute_hook`, after the existing `for tool in tools` loop (line 122, before `Ok(results)`), add the tein dispatch:

```rust
    // --- synthesised tein hooks ---
    #[cfg(feature = "synthesised-tools")]
    {
        let should_dispatch = TEIN_HOOK_GUARD.with(|guard| {
            let set = guard.borrow();
            !set.contains(&hook)
        });

        if should_dispatch {
            // Mark this hook point as in-flight for re-entrancy guard
            TEIN_HOOK_GUARD.with(|guard| {
                guard.borrow_mut().insert(hook);
            });

            for tool in tools {
                if !tool.hooks.contains(&hook) {
                    continue;
                }
                let (context, hook_bindings) = match &tool.r#impl {
                    super::ToolImpl::Synthesised {
                        context,
                        hook_bindings,
                        ..
                    } => (context, hook_bindings),
                    _ => continue,
                };

                let Some(binding) = hook_bindings.get(&hook) else {
                    continue;
                };

                let payload =
                    match super::synthesised::json_args_to_scheme_alist(data) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!(
                                "[WARN] tein hook {}: payload conversion: {e}",
                                hook.as_ref()
                            );
                            continue;
                        }
                    };

                let hook_fn = match context.evaluate(binding) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {} on {}: resolve {binding}: {e}",
                            hook.as_ref(),
                            tool.name
                        );
                        continue;
                    }
                };

                let result = match context.call(&hook_fn, &[payload]) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {} on {}: {e}",
                            hook.as_ref(),
                            tool.name
                        );
                        continue;
                    }
                };

                // Convert scheme result to JSON. Skip empty list (no-op).
                if matches!(&result, tein::Value::List(items) if items.is_empty()) {
                    continue;
                }
                if result.is_nil() {
                    continue;
                }

                match super::synthesised::scheme_value_to_json(&result) {
                    Ok(value) => results.push((tool.name.clone(), value)),
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {} on {}: result conversion: {e}",
                            hook.as_ref(),
                            tool.name
                        );
                    }
                }
            }

            // Clear re-entrancy guard for this hook point
            TEIN_HOOK_GUARD.with(|guard| {
                guard.borrow_mut().remove(&hook);
            });
        }
    }
```

**Step 4: Make `json_args_to_scheme_alist` and `scheme_value_to_json` pub(crate)**

In `synthesised.rs`, change visibility of these two functions:
- `fn json_args_to_scheme_alist` → `pub(crate) fn json_args_to_scheme_alist`
- `fn scheme_value_to_json` → `pub(crate) fn scheme_value_to_json`

**Step 5: Update `plugin_tools` filter in send.rs**

At `send.rs:1873-1881`, the filter currently only includes `ToolCategory::Plugin`. Update to also include synthesised tools that have hooks:

```rust
    // Tools for hook execution: plugin tools + synthesised tools with hooks.
    let plugin_tools: Vec<Tool> = registry
        .read()
        .unwrap()
        .filter(|t| {
            t.category == ToolCategory::Plugin
                || (t.category == ToolCategory::Synthesised && !t.hooks.is_empty())
        })
        .into_iter()
        .cloned()
        .collect();
```

Also update the comment above it (line 1873).

Check for other `execute_hook` call sites that filter tools similarly — grep for `execute_hook` in `chibi.rs`, `compact.rs`, `flow.rs`, `indexer.rs` and ensure they pass a tool list that could include synthesised tools. Most use `&plugin_tools` passed from `send.rs`, but `chibi.rs` has its own filter:

```rust
// chibi.rs — find where plugin_tools is constructed and apply same update
```

**Step 6: Run the test**

Run: `cargo test -p chibi-core test_execute_hook_dispatches_to_synthesised -- --nocapture 2>&1 | tail -20`
Expected: passes.

**Step 7: Commit**

```
feat(hooks): dispatch tein callbacks in execute_hook with re-entrancy guard

execute_hook now evaluates synthesised tool hook callbacks alongside
subprocess plugins. a thread-local re-entrancy guard prevents recursive
tein hook dispatch for the same hook point. (#220)
```

---

### Task 5: Write comprehensive tests for tein hook dispatch

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs` (test module)

**Step 1: Add tests**

```rust
#[test]
#[cfg(feature = "synthesised-tools")]
fn test_tein_hook_empty_list_return_is_noop() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::ToolRegistry;
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) '()))
(define tool-name "noop-hook")
(define tool-description "Returns empty list")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/noop.scm").unwrap();
    let tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    let results = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();
    assert_eq!(results.len(), 0, "empty list return should be treated as no-op");
}

#[test]
#[cfg(feature = "synthesised-tools")]
fn test_tein_hook_json_object_return() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::ToolRegistry;
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    let source = r#"
(import (harness hooks))
(register-hook 'pre_message
  (lambda (payload)
    (list (cons "prompt" "modified prompt"))))
(define tool-name "modify-hook")
(define tool-description "Modifies prompt")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/modify.scm").unwrap();
    let tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    let results = execute_hook(
        &tools,
        HookPoint::PreMessage,
        &serde_json::json!({"prompt": "hello"}),
    ).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1["prompt"], "modified prompt");
}

#[test]
#[cfg(feature = "synthesised-tools")]
fn test_tein_hook_skips_unregistered_hook_point() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::ToolRegistry;
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "selective-hook")
(define tool-description "Only fires on on_start")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/selective.scm").unwrap();
    let tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    // fire on_end — tool is registered for on_start only
    let results = execute_hook(&tools, HookPoint::OnEnd, &serde_json::json!({})).unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
#[cfg(feature = "synthesised-tools")]
fn test_tein_hook_error_in_callback_skipped() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::ToolRegistry;
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (error "boom")))
(define tool-name "error-hook")
(define tool-description "Errors in hook")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/error.scm").unwrap();
    let tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();

    // should not error — failed hooks are skipped silently
    let results = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();
    assert_eq!(results.len(), 0);
}

#[test]
#[cfg(feature = "synthesised-tools")]
fn test_mixed_plugin_and_tein_hooks() {
    use crate::tools::synthesised::load_tools_from_source_with_tier;
    use crate::tools::registry::ToolRegistry;
    use crate::vfs::VfsPath;
    use std::sync::{Arc, RwLock};

    // subprocess plugin
    let dir = tempfile::tempdir().unwrap();
    let script = create_test_script(
        dir.path(),
        "plugin.sh",
        b"#!/bin/bash\ncat > /dev/null\necho '{\"from\": \"plugin\"}'",
    );
    let plugin_tool = Tool {
        name: "plugin-hook".to_string(),
        description: "Plugin".to_string(),
        parameters: serde_json::json!({}),
        hooks: vec![HookPoint::OnStart],
        metadata: ToolMetadata::new(),
        summary_params: vec![],
        r#impl: crate::tools::ToolImpl::Plugin(script),
        category: crate::tools::ToolCategory::Plugin,
    };

    // tein tool
    let source = r#"
(import (harness hooks))
(register-hook 'on_start
  (lambda (payload) (list (cons "from" "tein"))))
(define tool-name "tein-hook")
(define tool-description "Tein hook")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let path = VfsPath::new("/tools/shared/tein.scm").unwrap();
    let mut tools = load_tools_from_source_with_tier(
        source, &path, &registry, crate::config::SandboxTier::Sandboxed,
    ).unwrap();
    tools.insert(0, plugin_tool);

    let results = execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

    // Both should fire: plugin first, then tein
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].1["from"], "plugin");
    assert_eq!(results[1].1["from"], "tein");
}
```

**Step 2: Run all tests**

Run: `cargo test -p chibi-core -- hooks --nocapture 2>&1 | tail -30`
Expected: all pass.

**Step 3: Commit**

```
test(hooks): comprehensive tests for tein hook dispatch

covers no-op returns, json object returns, unregistered hook points,
error handling, and mixed plugin+tein hook ordering. (#220)
```

---

### Task 6: Lint, full test suite, docs update, final commit

**Files:**
- Modify: `docs/hooks.md` (document tein hook registration)
- Modify: `AGENTS.md` (add quirks if needed)

**Step 1: Run full test suite**

Run: `cargo test -p chibi-core 2>&1 | tail -10`
Expected: all pass.

**Step 2: Lint**

Run: `just lint`
Expected: clean.

**Step 3: Update docs/hooks.md**

Add a section about tein hook registration. Find the appropriate place in the doc and add:

```markdown
### Tein Hook Registration

Synthesised tools (`.scm` files) can register for hooks using the `(harness hooks)` module:

\`\`\`scheme
(import (harness hooks))

(register-hook 'pre_message
  (lambda (payload)
    ;; payload is an alist parsed from the hook's JSON data.
    ;; return an alist to modify behaviour, or '() for no-op.
    (list (cons "prompt" "modified prompt"))))
\`\`\`

Tein hooks follow the same contract as subprocess plugin hooks:
- They receive the hook payload (converted from JSON to a scheme alist).
- They return a scheme value (converted back to JSON).
- Returning `'()` (empty list) is treated as a no-op.
- Errors in callbacks are caught and skipped silently (same as subprocess hook failures).

**Re-entrancy:** If a tein hook callback triggers an action that fires the same hook point, tein callbacks are skipped on the recursive call to prevent infinite loops. Subprocess hooks still fire normally.

**Lifecycle:** Hook registrations are tied to the `.scm` file. When a file is hot-reloaded or deleted, its hooks are automatically cleared and re-created from the fresh evaluation.
```

**Step 4: Check if AGENTS.md needs a quirk**

Search AGENTS.md for hook-related quirks. Add one if the re-entrancy guard or `execute_hook` dual-dispatch is non-obvious.

Suggested quirk:
```
- `execute_hook` dispatches to both subprocess plugins and tein callbacks. Tein dispatch uses a thread-local `TEIN_HOOK_GUARD` (`HashSet<HookPoint>`) for re-entrancy prevention — if a tein hook callback triggers the same hook point, tein callbacks are skipped on the recursive call.
```

**Step 5: Commit**

```
docs: document tein hook registration in hooks.md and AGENTS.md (#220)
```

**Step 6: Run full suite one more time**

Run: `cargo test -p chibi-core`
Expected: all pass.

**Step 7: Verify all commits reference #220**

Run: `git log --oneline dev..HEAD`
Expected: all commits mention #220, last one says "closes #220".

Amend last commit message to include `closes #220` if not present.
