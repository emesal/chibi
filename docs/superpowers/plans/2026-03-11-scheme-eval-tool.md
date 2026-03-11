# scheme_eval Tool Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `scheme_eval` builtin tool that lets the LLM evaluate Scheme expressions in a persistent, sandboxed tein environment with `call-tool` support.

**Architecture:** New `eval.rs` module in `tools/`, following the `fs_read.rs` pattern. Shares the tein FFI bridge (`CallContextGuard`, `BRIDGE_CALL_CTX`, harness constants) from `synthesised.rs` by making those items `pub(crate)`. Persistent per-context tein environments stored in a module-global `LazyLock<Mutex<HashMap<String, (Arc<ThreadLocalContext>, ThreadId)>>>`. `ThreadLocalContext` is not `Clone` — wrapped in `Arc` for cheap sharing. Registration happens after the registry `Arc<RwLock<ToolRegistry>>` is created in `chibi.rs` (line 226), not with the other `register_*_tools` calls.

**Tech Stack:** Rust, tein (R7RS Scheme), existing chibi-core tool infrastructure.

**Spec:** `docs/superpowers/specs/2026-03-11-scheme-eval-tool-design.md`

---

## Chunk 1: Expose Shared Tein Infrastructure

### Task 1: Make `synthesised.rs` internals `pub(crate)`

The new module needs access to the harness constants, FFI functions, and `CallContextGuard::set()`. These are currently private to `synthesised.rs`. Make the minimum set `pub(crate)`.

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Make harness constants `pub(crate)`**

Change visibility of these three constants (all behind `#[cfg(feature = "synthesised-tools")]`):

```rust
// line 89 — was: const HARNESS_TOOLS_MODULE
pub(crate) const HARNESS_TOOLS_MODULE: &str = ...

// line 123 — was: const HARNESS_HOOKS_MODULE
pub(crate) const HARNESS_HOOKS_MODULE: &str = ...

// line 146 — was: const HARNESS_PREAMBLE
pub(crate) const HARNESS_PREAMBLE: &str = ...
```

Do NOT change `HARNESS_IO_MODULE` — the scheme_eval tool is sandboxed and doesn't need it.

Also make `scheme_escape_string` (line 827) `pub(crate)`:

```rust
// line 827 — was: fn scheme_escape_string
pub(crate) fn scheme_escape_string(s: &str) -> String {
```

- [ ] **Step 2: Make `CallContextGuard::set` `pub(crate)`**

At line 237, change `fn set(` to `pub(crate) fn set(`. The struct is already `pub(crate)`.

- [ ] **Step 3: Make FFI function symbols `pub(crate)`**

The `#[tein::tein_fn]` macro generates `__tein_call_tool_fn`, `__tein_generate_id_fn`, `__tein_current_timestamp_fn`. These need to be accessible from `eval.rs`. Check whether the macro-generated symbols respect a visibility modifier on the original `fn`. If not, wrap them in `pub(crate)` re-exports:

```rust
// after the tein_fn definitions, around line 405:
pub(crate) use self::{
    __tein_call_tool_fn,
    __tein_generate_id_fn,
    __tein_current_timestamp_fn,
};
```

If `use self::` doesn't work for macro-generated items, define thin `pub(crate)` wrapper functions that delegate to the private symbols. The key constraint: `define_fn_variadic` expects a specific function pointer type — check tein's docs for the exact signature.

**Alternative approach:** If exposing the FFI symbols is awkward, extract a `pub(crate) fn build_harness_context(source: &str) -> io::Result<(ThreadLocalContext, ThreadId)>` that wraps `build_tein_context` with `SandboxTier::Sandboxed` and a caller-supplied source string. This avoids exposing internals at the cost of less flexibility.

Prefer the direct approach (exposing symbols) if it works cleanly — the new module needs slightly different init logic (no user source, custom prelude).

- [ ] **Step 4: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compile. No visibility errors.

- [ ] **Step 5: Commit**

```
refactor: make tein harness internals pub(crate) for eval tool
```

---

## Chunk 2: Add `ToolCategory::Eval` and Module Scaffolding

### Task 2: Add `ToolCategory::Eval` variant

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

- [ ] **Step 1: Add `Eval` to `ToolCategory` enum**

At line 137, before the closing `}`:
```rust
    Synthesised,
    Eval,
}
```

- [ ] **Step 2: Add `as_str()` arm**

At line 154, before the closing `}`:
```rust
            ToolCategory::Synthesised => "synthesised",
            ToolCategory::Eval => "eval",
```

- [ ] **Step 3: Add to `test_tool_category_debug`**

At line 359, add `ToolCategory::Eval,` to the array.

- [ ] **Step 4: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compile.

- [ ] **Step 5: Commit**

```
feat: add ToolCategory::Eval variant
```

### Task 3: Add `regex` feature to tein dependency

**Files:**
- Modify: `crates/chibi-core/Cargo.toml`

- [ ] **Step 1: Add `regex` to tein features**

Line 30, change:
```toml
tein = { git = "https://github.com/emesal/tein", branch = "main", features = ["json"], optional = true }
```
to:
```toml
tein = { git = "https://github.com/emesal/tein", branch = "main", features = ["json", "regex"], optional = true }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compile (may fetch tein with new feature).

- [ ] **Step 3: Commit**

```
build: enable tein regex feature for (tein safe-regexp)
```

---

## Chunk 3: Implement `eval.rs`

### Task 4: Create `eval.rs` with tool definition and context builder

**Files:**
- Create: `crates/chibi-core/src/tools/eval.rs`

- [ ] **Step 1: Write the test for tool definition**

Create `eval.rs` with imports, constants, `EVAL_PRELUDE`, `EVAL_CONTEXTS` store, and the test module:

```rust
//! `scheme_eval` builtin tool — evaluates Scheme (R7RS) expressions in a
//! persistent sandboxed tein environment with `call-tool` bridge support.
//!
//! Environment persists for the process lifetime, keyed by context name.
//! Pre-imports core modules so the LLM can compute immediately.

#[cfg(feature = "synthesised-tools")]
use std::collections::HashMap;
#[cfg(feature = "synthesised-tools")]
use std::io;
#[cfg(feature = "synthesised-tools")]
use std::sync::{Arc, LazyLock, Mutex};

#[cfg(feature = "synthesised-tools")]
use tein::{Context, ThreadLocalContext, sandbox::Modules};

#[cfg(feature = "synthesised-tools")]
use super::registry::{ToolCategory, ToolRegistry};
#[cfg(feature = "synthesised-tools")]
use super::{BuiltinToolDef, Tool, ToolMetadata, ToolPropertyDef};

/// Scheme prelude evaluated once when a context is created.
/// Auto-imports the standard module set so the LLM can use them immediately.
#[cfg(feature = "synthesised-tools")]
const EVAL_PRELUDE: &str = r#"
(import (scheme base)
        (scheme write)
        (scheme read)
        (scheme char)
        (scheme case-lambda)
        (tein json)
        (tein safe-regexp)
        (srfi 1)
        (srfi 130)
        (chibi match)
        (harness tools))
"#;

/// Process-global store of persistent tein contexts, keyed by chibi context name.
/// Each entry is (Arc<ThreadLocalContext>, worker_thread_id).
/// `ThreadLocalContext` is not Clone — Arc provides cheap sharing.
#[cfg(feature = "synthesised-tools")]
static EVAL_CONTEXTS: LazyLock<Mutex<HashMap<String, (Arc<ThreadLocalContext>, std::thread::ThreadId)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub const SCHEME_EVAL_TOOL_NAME: &str = "scheme_eval";

pub static EVAL_TOOL_DEFS: &[BuiltinToolDef] = &[BuiltinToolDef {
    name: SCHEME_EVAL_TOOL_NAME,
    description: "Evaluate a Scheme (R7RS) expression in a persistent sandboxed environment. \
                  State persists across calls — define variables, build data structures, compose \
                  computations. Returns the result of the last expression. Additional safe modules \
                  can be imported with (import ...).",
    properties: &[ToolPropertyDef {
        name: "code",
        prop_type: "string",
        description: "Scheme expression(s) to evaluate",
        default: None,
    }],
    required: &["code"],
    summary_params: &["code"],
}];

#[cfg(all(test, feature = "synthesised-tools"))]
mod tests {
    #[test]
    fn test_tool_def() {
        assert_eq!(super::EVAL_TOOL_DEFS[0].name, "scheme_eval");
        assert_eq!(super::EVAL_TOOL_DEFS[0].required, &["code"]);
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p chibi-core eval::tests::test_tool_def -- --nocapture 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 3: Add `build_eval_context` function**

```rust
/// Build a sandboxed tein context for scheme_eval.
///
/// Registers the harness FFI functions and modules, then evaluates the prelude.
/// Returns the context (wrapped in Arc — ThreadLocalContext is not Clone)
/// and its worker thread ID (for `CallContextGuard` keying).
#[cfg(feature = "synthesised-tools")]
fn build_eval_context() -> io::Result<(Arc<ThreadLocalContext>, std::thread::ThreadId)> {
    use super::synthesised::{
        HARNESS_PREAMBLE, HARNESS_TOOLS_MODULE, HARNESS_HOOKS_MODULE,
        __tein_call_tool_fn, __tein_generate_id_fn, __tein_current_timestamp_fn,
    };

    let worker_thread_id = Arc::new(Mutex::new(None::<std::thread::ThreadId>));
    let tid_capture = Arc::clone(&worker_thread_id);

    let init = move |ctx: &Context| -> tein::Result<()> {
        *tid_capture.lock().unwrap() = Some(std::thread::current().id());
        ctx.define_fn_variadic("call-tool", __tein_call_tool_fn)?;
        ctx.define_fn_variadic("generate-id", __tein_generate_id_fn)?;
        ctx.define_fn_variadic("current-timestamp", __tein_current_timestamp_fn)?;
        ctx.evaluate(HARNESS_PREAMBLE)?;
        ctx.register_module(HARNESS_TOOLS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness module: {e}")))?;
        ctx.register_module(HARNESS_HOOKS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness hooks module: {e}")))?;
        ctx.evaluate(EVAL_PRELUDE)?;
        Ok(())
    };

    let ctx = Context::builder()
        .standard_env()
        .sandboxed(Modules::Safe)
        .step_limit(10_000_000)
        .build_managed(init)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tein init: {e}")))?;

    let tid = worker_thread_id
        .lock()
        .unwrap()
        .expect("init closure must have run and captured thread ID");
    Ok((Arc::new(ctx), tid))
}
```

- [ ] **Step 4: Write context builder tests**

Add to the `tests` module:

```rust
    #[test]
    fn test_build_context_basic() {
        let (ctx, tid) = super::build_eval_context().expect("context should build");
        let result = ctx.evaluate("(+ 1 2)").expect("eval should succeed");
        assert_eq!(result.to_string(), "3");
        // Worker thread should differ from test thread
        assert_ne!(tid, std::thread::current().id());
    }

    #[test]
    fn test_context_persistence() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        ctx.evaluate("(define x 42)").expect("define should work");
        let result = ctx.evaluate("x").expect("x should be defined");
        assert_eq!(result.to_string(), "42");
    }

    #[test]
    fn test_prelude_srfi_1() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx.evaluate("(fold + 0 '(1 2 3 4 5))").expect("fold from srfi-1");
        assert_eq!(result.to_string(), "15");
    }

    #[test]
    fn test_prelude_srfi_130() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate(r#"(string-contains "hello world" "world")"#)
            .expect("string-contains from srfi-130");
        // Returns cursor index, not #f
        assert_ne!(result.to_string(), "#f");
    }

    #[test]
    fn test_prelude_tein_json() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate(r#"(json-read-string "{\"a\":1}")"#)
            .expect("json-read-string from tein json");
        assert!(result.to_string().contains("a"));
    }

    /// Note: check `(tein safe-regexp)` export names at implementation time.
    /// tein may use `rx`, `regexp`, `make-regexp`, etc. Adjust if needed.
    #[test]
    fn test_prelude_safe_regexp() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate(r#"(regexp-match? (regexp "^hello") "hello world")"#)
            .expect("safe-regexp should work");
        assert_eq!(result.to_string(), "#t");
    }

    #[test]
    fn test_prelude_chibi_match() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate("(match '(1 2 3) ((a b c) (+ a b c)))")
            .expect("chibi match");
        assert_eq!(result.to_string(), "6");
    }

    #[test]
    fn test_error_reporting() {
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx.evaluate("undefined-var");
        assert!(result.is_err());
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p chibi-core eval::tests -- --nocapture 2>&1 | tail -20`
Expected: All PASS. Adjust assertions if tein's output format differs.

- [ ] **Step 6: Commit**

```
feat(eval): tein context builder with prelude and module tests
```

### Task 5: Implement `execute_scheme_eval` and `register_eval_tools`

**Files:**
- Modify: `crates/chibi-core/src/tools/eval.rs`

- [ ] **Step 1: Registry Arc access — two-phase registration**

The `CallContextGuard::set` needs `Arc<RwLock<ToolRegistry>>`. The other `register_*_tools` functions take `&mut ToolRegistry` before it's wrapped in Arc (chibi.rs line 168-176). The Arc is created at line 226. The eval tool's handler needs the Arc at call time.

**Solution:** `register_eval_tools` takes `&Arc<RwLock<ToolRegistry>>` instead of `&mut ToolRegistry`. It's called from `chibi.rs` AFTER line 226 (in the `#[cfg(feature = "synthesised-tools")]` block, alongside `scan_and_register`). The handler closure captures `Arc::clone(&registry)`.

Change `register_eval_tools` signature:

```rust
#[cfg(feature = "synthesised-tools")]
pub fn register_eval_tools(registry: &Arc<std::sync::RwLock<ToolRegistry>>) {
```

This means the function uses `registry.write().unwrap().register(tool)` instead of `registry.register(tool)`.

- [ ] **Step 2: Add the execute function**

```rust
/// Execute a `scheme_eval` tool call.
///
/// Retrieves or creates a persistent tein context for the calling chibi context,
/// sets `CallContextGuard` for `call-tool` support, injects `%context-name%`,
/// evaluates the code, and returns the display representation.
///
/// Errors from scheme evaluation are returned as `Ok("error: ...")` —
/// they don't abort the prompt cycle.
#[cfg(feature = "synthesised-tools")]
fn execute_scheme_eval(
    context_name: &str,
    code: &str,
    call_ctx: &super::registry::ToolCallContext<'_>,
    registry: Arc<std::sync::RwLock<ToolRegistry>>,
) -> io::Result<String> {
    use super::synthesised::CallContextGuard;

    if code.is_empty() {
        return Ok(String::new());
    }

    // Get or create the persistent tein context for this chibi context.
    let (tein_ctx, worker_tid) = {
        let mut contexts = EVAL_CONTEXTS.lock().unwrap();
        if let Some(entry) = contexts.get(context_name) {
            (Arc::clone(&entry.0), entry.1)
        } else {
            let (ctx, tid) = build_eval_context()?;
            let shared = Arc::clone(&ctx);
            contexts.insert(context_name.to_string(), (ctx, tid));
            (shared, tid)
        }
    };

    // Set the call-tool bridge context for this evaluation.
    let _guard = CallContextGuard::set(call_ctx, registry, worker_tid);

    // Inject %context-name% so call-tool resolves VFS paths correctly.
    let ctx_name_escaped = super::synthesised::scheme_escape_string(context_name);
    tein_ctx
        .evaluate(&format!("(set! %context-name% \"{ctx_name_escaped}\")"))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("context-name: {e}")))?;

    // Evaluate the user's code.
    match tein_ctx.evaluate(code) {
        Ok(val) => Ok(val.to_string()),
        Err(e) => Ok(format!("error: {e}")),
    }
}
```

**Important:** Check if `val.to_string()` uses `display` or `write` semantics. `write` adds quotes around strings (`"hello"` → `"\"hello\""`), which degrades usability. If tein's `Display` impl uses `write` semantics, look for a `display_string()` or `display()` method, or use `(let ((r CODE)) (display r) (newline))` wrapper to capture display output. This matters for string results the LLM will consume.

- [ ] **Step 3: Implement `register_eval_tools`**

The exact shape depends on step 1's findings. Template:

```rust
#[cfg(feature = "synthesised-tools")]
pub fn register_eval_tools(registry: &Arc<std::sync::RwLock<ToolRegistry>>) {
    let registry_for_handler = Arc::clone(registry);
    let handler: super::registry::ToolHandler = Arc::new(move |call| {
        let context_name = call.context.context_name.to_string();
        let code = call.args.get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reg = Arc::clone(&registry_for_handler);
        Box::pin(async move {
            execute_scheme_eval(&context_name, &code, &call.context, reg)
        })
    });

    let mut tool = Tool::from_builtin_def(&EVAL_TOOL_DEFS[0], handler, ToolCategory::Eval);
    tool.metadata.parallel = false;
    registry.write().unwrap().register(tool);
}

/// Stub when synthesised-tools feature is disabled.
#[cfg(not(feature = "synthesised-tools"))]
pub fn register_eval_tools(_registry: &Arc<std::sync::RwLock<ToolRegistry>>) {}
```

- [ ] **Step 4: Write context store test**

```rust
    #[test]
    fn test_contexts_isolation() {
        // Two contexts should have independent state
        let (ctx_a, _) = super::build_eval_context().expect("build a");
        let (ctx_b, _) = super::build_eval_context().expect("build b");
        ctx_a.evaluate("(define x 1)").unwrap();
        ctx_b.evaluate("(define x 2)").unwrap();
        assert_eq!(ctx_a.evaluate("x").unwrap().to_string(), "1");
        assert_eq!(ctx_b.evaluate("x").unwrap().to_string(), "2");
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p chibi-core eval::tests -- --nocapture 2>&1 | tail -20`
Expected: All PASS.

- [ ] **Step 6: Commit**

```
feat(eval): implement execute_scheme_eval and register_eval_tools
```

---

## Chunk 4: Integration

### Task 6: Wire eval.rs into the module system

**Files:**
- Modify: `crates/chibi-core/src/tools/mod.rs`

- [ ] **Step 1: Add module declaration**

At line 22 (between `index` and `pub mod mcp`), add:

```rust
mod eval;
```

- [ ] **Step 2: Add re-exports**

After the VFS re-exports (line 174):

```rust
// Re-export eval tool constants and functions
pub use eval::{EVAL_TOOL_DEFS, SCHEME_EVAL_TOOL_NAME, register_eval_tools};
```

- [ ] **Step 3: Add to `builtin_tool_names()` chain**

At line 300 (after `.chain(vfs_tools::VFS_TOOL_DEFS.iter())`):

```rust
        .chain(eval::EVAL_TOOL_DEFS.iter())
```

- [ ] **Step 4: Add to `builtin_summary_params()` chain**

At line 329 (after `.chain(vfs_tools::VFS_TOOL_DEFS.iter())`):

```rust
        .chain(eval::EVAL_TOOL_DEFS.iter())
```

- [ ] **Step 5: Update `test_builtin_tool_names_includes_all_registries`**

At line 598-605 in `mod.rs`, add `eval::EVAL_TOOL_DEFS.len()` to `expected_count`:

```rust
        let expected_count = memory::MEMORY_TOOL_DEFS.len()
            + flow::FLOW_TOOL_DEFS.len()
            + fs_read::FS_READ_TOOL_DEFS.len()
            + fs_write::FS_WRITE_TOOL_DEFS.len()
            + shell::SHELL_TOOL_DEFS.len()
            + network::NETWORK_TOOL_DEFS.len()
            + index::INDEX_TOOL_DEFS.len()
            + vfs_tools::VFS_TOOL_DEFS.len()
            + eval::EVAL_TOOL_DEFS.len();
```

Also add a spot-check:

```rust
        assert!(names.contains(&"scheme_eval")); // eval tool
```

- [ ] **Step 6: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compile.

- [ ] **Step 7: Commit**

```
feat(eval): wire eval module into tools/mod.rs
```

### Task 7: Register in `chibi.rs`

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs`

- [ ] **Step 1: Add registration call**

`register_eval_tools` takes `&Arc<RwLock<ToolRegistry>>`, so it's called AFTER `Arc::new(RwLock::new(reg))` at line 226. Add it in the existing `#[cfg(feature = "synthesised-tools")]` block (lines 246-253), after `scan_and_register`:

```rust
    #[cfg(feature = "synthesised-tools")]
    {
        crate::tools::vfs_block_on(crate::tools::synthesised::scan_and_register(
            &app.vfs,
            &registry,
            &app.config.tools,
        ))?;
        crate::tools::register_eval_tools(&registry);
    }
```

Do NOT add it with the other `register_*_tools(&mut reg)` calls at lines 169-176 — the registry is still a bare `ToolRegistry` there.

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compile.

- [ ] **Step 3: Commit**

```
feat(eval): register scheme_eval tool in chibi init
```

### Task 8: Update tests in `registry.rs`

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

- [ ] **Step 1: Add to `test_register_all_builtins`**

At line 560 (imports), add `register_eval_tools`:

```rust
use super::super::{
    register_eval_tools, register_flow_tools, register_fs_read_tools,
    register_fs_write_tools, register_index_tools, register_memory_tools,
    register_network_tools, register_shell_tools, register_vfs_tools,
};
```

At line 571 (after `register_vfs_tools`), wrap `reg` in Arc and register eval:

```rust
        register_vfs_tools(&mut reg);

        let reg_arc = std::sync::Arc::new(std::sync::RwLock::new(reg));
        register_eval_tools(&reg_arc);
        let reg = reg_arc.read().unwrap();
```

Then adjust the rest of the test to work with the `RwLockReadGuard` (the `reg.all()`, `reg.get()` calls should still work since they take `&self`).

Add spot-check:

```rust
        assert_eq!(
            reg.get("scheme_eval").unwrap().category,
            ToolCategory::Eval
        );
        assert!(
            !reg.get("scheme_eval").unwrap().metadata.parallel,
            "scheme_eval must not be parallel"
        );
```

- [ ] **Step 2: Run all registry tests**

Run: `cargo test -p chibi-core registry -- --nocapture 2>&1 | tail -20`
Expected: All PASS.

- [ ] **Step 3: Commit**

```
test(eval): update registry tests for scheme_eval
```

---

## Chunk 5: Full Test Suite, Lint, and Docs

### Task 9: Run the full test suite and lint

- [ ] **Step 1: Run all chibi-core tests**

Run: `cargo test -p chibi-core 2>&1 | tail -30`
Expected: All PASS. Fix any failures.

- [ ] **Step 2: Run lint**

Run: `just lint 2>&1 | tail -30`
Expected: Clean. Fix any warnings.

- [ ] **Step 3: Commit if lint changed anything**

```
fmt: lint fixes
```

### Task 10: Update documentation

**Files:**
- Modify: `docs/architecture.md` — add `eval.rs` to the tools file listing
- Modify: `AGENTS.md` — add any quirks/gotchas discovered during implementation

- [ ] **Step 1: Update `docs/architecture.md`**

Add `eval.rs` to the file listing under the tools section with a one-liner description:
`eval.rs — scheme_eval builtin tool: sandboxed R7RS expression evaluation with call-tool bridge`

- [ ] **Step 2: Collect AGENTS.md notes**

Add gotchas discovered during implementation. Expected candidates:
- `EVAL_CONTEXTS` is process-global, keyed by context name. Contexts are never evicted (process lifetime).
- `scheme_eval` has `parallel: false` — concurrent calls for the same context would collide on `BRIDGE_CALL_CTX`.
- FFI symbols `__tein_*_fn` are shared between `synthesised.rs` and `eval.rs` via `pub(crate)`.
- `(tein safe-regexp)` requires `regex` cargo feature on tein dep.

- [ ] **Step 3: Commit**

```
docs: document scheme_eval tool and add AGENTS.md notes
```
