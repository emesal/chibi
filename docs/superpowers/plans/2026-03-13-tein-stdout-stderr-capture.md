# Tein Stdout/Stderr Capture Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture stdout and stderr from scheme evaluation alongside the expression value, so `display` output reaches the LLM.

**Architecture:** `TeinSession` wraps `ThreadLocalContext` + shared stdout/stderr buffers. `with_capture()` encapsulates drain-eval-flush-read. Both `scheme_eval` and `execute_synthesised` use this struct, producing structured `result + stdout + stderr` responses.

**Tech Stack:** Rust, tein (chibi-scheme wrapper), tokio

**Spec:** `docs/superpowers/specs/2026-03-13-tein-stdout-stderr-capture-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/chibi-core/src/tools/synthesised.rs` | Modify | Add `SharedWriter`, `TeinSession`, `CapturedOutput`; update `build_tein_context`, `build_sandboxed_harness_context`, `extract_single_tool`, `extract_multi_tools`, `execute_synthesised`, `extract_string` |
| `crates/chibi-core/src/tools/eval.rs` | Modify | Update types to `TeinSession`, update `run_scheme` to use `with_capture`, update tool description, update tests |
| `crates/chibi-core/src/tools/registry.rs` | Modify | Change `ToolImpl::Synthesised.context` type from `Arc<ThreadLocalContext>` to `Arc<TeinSession>`, update `Clone` impl and `dispatch_impl` |
| `crates/chibi-core/src/tools/hooks.rs` | Modify | Update destructuring to use `TeinSession` delegate methods |

---

## Chunk 1: Core Types and Infrastructure

### Task 1: Add `SharedWriter` and `CapturedOutput` to `synthesised.rs`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

These are pure data types with no dependencies on existing code, so they can be added first.

- [ ] **Step 1: Add `SharedWriter` struct**

First, add `Mutex` to the existing import at line 63:
```rust
use std::sync::{Arc, Mutex, RwLock};
```

Then add below the `use` block, inside the `#[cfg(feature = "synthesised-tools")]` section:

```rust
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
```

- [ ] **Step 2: Add `CapturedOutput` struct**

Add immediately after `SharedWriter`:

```rust
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
    pub value: tein::Result<tein::Value>,
    pub stdout: String,
    pub stderr: String,
}

#[cfg(feature = "synthesised-tools")]
impl CapturedOutput {
    /// Format for `scheme_eval` -- stringifies value with `to_string()`.
    ///
    /// NOTE: similar formatting exists in `format_tool()`.
    /// Changes here may need to be assessed for that method too.
    pub fn format_eval(&self) -> String {
        let value_str = match &self.value {
            Ok(val) => val.to_string(),
            Err(e) => format!("error: {e}"),
        };
        format!(
            "result: {}\nstdout: {}\nstderr: {}",
            value_str,
            if self.stdout.is_empty() { "(empty)" } else { &self.stdout },
            if self.stderr.is_empty() { "(empty)" } else { &self.stderr },
        )
    }

    /// Format for synthesised tools -- unwraps scheme strings via `as_string()`,
    /// falling back to `to_string()` for non-string values.
    ///
    /// NOTE: similar formatting exists in `format_eval()`.
    /// Changes here may need to be assessed for that method too.
    pub fn format_tool(&self) -> String {
        let value_str = match &self.value {
            Ok(val) => match val.as_string() {
                Some(s) => s.to_string(),
                None => val.to_string(),
            },
            Err(e) => format!("error: {e}"),
        };
        format!(
            "result: {}\nstdout: {}\nstderr: {}",
            value_str,
            if self.stdout.is_empty() { "(empty)" } else { &self.stdout },
            if self.stderr.is_empty() { "(empty)" } else { &self.stderr },
        )
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -5`
Expected: compiles (new types are unused but that's fine -- we'll wire them next)

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat: add SharedWriter and CapturedOutput types for tein output capture"
```

### Task 2: Add `TeinSession` struct and `with_capture`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Add `TeinSession` struct**

Add after `CapturedOutput`:

```rust
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
pub(crate) struct TeinSession {
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
        // 1. drain
        self.stdout_buf.lock().unwrap().clear();
        self.stderr_buf.lock().unwrap().clear();

        // 2. run
        let value = f(&self.ctx);

        // 3. flush -- ignore errors (port should always be valid)
        let _ = self.ctx.evaluate("(flush-output (current-output-port))");
        let _ = self.ctx.evaluate("(flush-output (current-error-port))");

        // 4. read
        let stdout = String::from_utf8_lossy(&self.stdout_buf.lock().unwrap()).to_string();
        let stderr = String::from_utf8_lossy(&self.stderr_buf.lock().unwrap()).to_string();

        CapturedOutput { value, stdout, stderr }
    }

    /// Delegate to inner context for internal calls (e.g. setting `%context-name%`,
    /// resolving bindings) that don't need output capture.
    pub(crate) fn evaluate(&self, code: &str) -> tein::Result<tein::Value> {
        self.ctx.evaluate(code)
    }

    /// Delegate to inner context for calling procedures without capture.
    /// Used by hook dispatch (`hooks.rs`) which doesn't need stdout/stderr capture.
    pub(crate) fn call(
        &self,
        proc: &tein::Value,
        args: &[tein::Value],
    ) -> tein::Result<tein::Value> {
        self.ctx.call(proc, args)
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -5`
Expected: compiles (unused struct warnings are fine)

- [ ] **Step 3: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat: add TeinSession with stdout/stderr capture via with_capture"
```

### Task 3: Update `build_tein_context` to create `TeinSession`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Update `build_tein_context` signature and body**

Change the return type from `(ThreadLocalContext, ThreadId)` to `(TeinSession, ThreadId)`.

Create the stdout/stderr buffers before the init closure. Clone `Arc`s into the closure so it can wire output ports on each init call. Bundle the buffers into `TeinSession` after context creation.

The current function (line ~642-699) becomes:

```rust
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
        ctx.evaluate(HARNESS_PREAMBLE)?;
        ctx.register_module(HARNESS_TOOLS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness module: {e}")))?;
        ctx.register_module(HARNESS_HOOKS_MODULE)
            .map_err(|e| tein::Error::EvalError(format!("harness hooks module: {e}")))?;
        // (harness io) -- privileged direct IO, available at Unsandboxed tier only.
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
```

- [ ] **Step 2: Update `build_sandboxed_harness_context` return type**

Change the return type to match (line ~707-709):

```rust
pub(crate) fn build_sandboxed_harness_context()
-> io::Result<(TeinSession, std::thread::ThreadId)> {
    build_tein_context(String::new(), crate::config::SandboxTier::Sandboxed)
}
```

- [ ] **Step 3: Verify it compiles (expect downstream errors)**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -20`
Expected: compile errors in downstream code (`extract_single_tool`, `extract_multi_tools`, etc.) -- these expect `ThreadLocalContext`, not `TeinSession`. That's expected; we fix them in the next tasks.

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat: build_tein_context returns TeinSession with captured output ports"
```

### Task 4: Update tool extraction and helper functions

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

These functions take the context by value from `build_tein_context` and wrap it in `Arc`. Update them to take `TeinSession` instead of `ThreadLocalContext`.

- [ ] **Step 1: Update `extract_string` parameter type**

At line ~1166, change from `ctx: &ThreadLocalContext` to `session: &TeinSession`:

```rust
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
```

- [ ] **Step 2: Update `extract_hook_registrations` parameter type**

At line ~1191, change from `ctx: &ThreadLocalContext` to `session: &TeinSession`. Replace `ctx.evaluate(...)` with `session.evaluate(...)` throughout the function body.

- [ ] **Step 3: Update `extract_single_tool`**

Change parameter type at line ~786 from `ctx: ThreadLocalContext` to `session: TeinSession`. Replace all `ctx.evaluate(...)` with `session.evaluate(...)`, `extract_string(&ctx, ...)` with `extract_string(&session, ...)`, `extract_hook_registrations(&ctx)` with `extract_hook_registrations(&session)`, and `Arc::new(ctx)` with `Arc::new(session)`.

- [ ] **Step 4: Update `extract_multi_tools`**

Change parameter type at line ~846 from `ctx: ThreadLocalContext` to `session: TeinSession`. Same substitutions as step 3. The inner `context.evaluate(...)` on the `Arc` (line ~915-925 for binding exec handlers) works because `Arc<TeinSession>` auto-derefs to `TeinSession` which has `evaluate`.

- [ ] **Step 5: Update `load_tools_from_source_with_tier`**

At line ~747, rename destructuring from `(ctx, worker_thread_id)` to `(session, worker_thread_id)`. Replace `ctx.evaluate(...)` with `session.evaluate(...)`. Pass `session` instead of `ctx` to `extract_single_tool`/`extract_multi_tools`.

- [ ] **Step 6: Verify it compiles (expect downstream errors)**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -20`
Expected: compile errors from `registry.rs` and `hooks.rs` (they still reference `Arc<ThreadLocalContext>`) -- fixed in next tasks.

- [ ] **Step 7: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "refactor: tool extraction functions take TeinSession"
```

### Task 5: Update `execute_synthesised` to use `with_capture`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Update `execute_synthesised` signature and body**

Change parameter type from `context: &ThreadLocalContext` to `session: &TeinSession`. Use `session.evaluate()` for internal calls and `session.with_capture()` for the tool execution:

```rust
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
```

Also update the non-feature stub (line ~997) to change `_context: &()` parameter name to `_session: &()`.

- [ ] **Step 2: Verify it compiles (expect downstream errors)**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -20`
Expected: still compile errors from `registry.rs` and `hooks.rs`

- [ ] **Step 3: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat: execute_synthesised captures stdout/stderr via TeinSession"
```

### Task 6: Update `registry.rs` -- `ToolImpl::Synthesised` type

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

- [ ] **Step 1: Update `ToolImpl::Synthesised` field type**

At line ~80, change:
```rust
context: std::sync::Arc<tein::ThreadLocalContext>,
```
to:
```rust
context: std::sync::Arc<super::synthesised::TeinSession>,
```

- [ ] **Step 2: Verify `dispatch_impl` and `Clone` impl**

The destructuring at line ~287 and the `Clone` impl at line ~102 both use `context` -- with the type change, they should work without code changes (the field name is the same, `Arc::clone` and auto-deref work the same way). Verify by reading the compiler output.

- [ ] **Step 3: Verify it compiles (expect hooks.rs errors only)**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -20`
Expected: compile errors from `hooks.rs` only (or clean compile if hooks auto-deref works)

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/tools/registry.rs
git commit -m "refactor: ToolImpl::Synthesised stores Arc<TeinSession>"
```

### Task 7: Update `hooks.rs`

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs`

- [ ] **Step 1: Verify hook dispatch compiles**

At line ~195-201, the destructuring extracts `context` which is now `Arc<TeinSession>`. The existing code calls `context.evaluate(binding)` (line 232) and `context.call(&hook_fn, &[payload])` (line 244). These now resolve to `TeinSession::evaluate` and `TeinSession::call` -- the delegate methods. **No code change should be needed** -- the type change propagates correctly.

If the compiler complains, the issue is likely just the type annotation in the destructuring pattern. Fix any issues.

- [ ] **Step 2: Verify full crate compiles**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -5`
Expected: clean compile

- [ ] **Step 3: Commit (if any changes were needed)**

```bash
git add crates/chibi-core/src/tools/hooks.rs
git commit -m "refactor: hooks.rs uses TeinSession delegates"
```

## Chunk 2: eval.rs Changes and Tests

### Task 8: Update `eval.rs` types and `run_scheme`

**Files:**
- Modify: `crates/chibi-core/src/tools/eval.rs`

- [ ] **Step 1: Update imports**

Remove the `ThreadLocalContext` import (line 15). It's no longer needed -- `eval.rs` now works with `TeinSession`.

- [ ] **Step 2: Update `EvalContextMap` type alias and doc comment**

At line ~39-45, change:
```rust
/// Process-global store of persistent tein contexts, keyed by chibi context name.
/// Each entry is `(Arc<TeinSession>, worker_thread_id)`.
/// `TeinSession` wraps `ThreadLocalContext` with stdout/stderr capture.
///
/// Contexts are never evicted (process lifetime). Access serialised via Mutex.
#[cfg(feature = "synthesised-tools")]
type EvalContextMap = Mutex<HashMap<String, (Arc<super::synthesised::TeinSession>, std::thread::ThreadId)>>;
```

- [ ] **Step 3: Update `build_eval_context`**

At line ~80, change return type and body:
```rust
/// Build a sandboxed tein session for `scheme_eval`.
///
/// Delegates to `synthesised::build_sandboxed_harness_context` for the FFI
/// bridge setup, then evaluates `EVAL_PRELUDE` to pre-import standard modules.
/// Returns `(Arc<TeinSession>, worker_thread_id)`.
#[cfg(feature = "synthesised-tools")]
fn build_eval_context() -> io::Result<(Arc<super::synthesised::TeinSession>, std::thread::ThreadId)> {
    let (session, tid) = super::synthesised::build_sandboxed_harness_context()?;
    session.evaluate(EVAL_PRELUDE)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("eval prelude: {e}")))?;
    Ok((Arc::new(session), tid))
}
```

- [ ] **Step 4: Update `run_scheme`**

At line ~95, change parameter and body:
```rust
/// Run scheme code in the persistent tein session. Called on a blocking thread.
///
/// Injects `%context-name%`, evaluates user code with stdout/stderr capture.
/// Scheme errors are returned in the structured output -- they do not abort
/// the prompt cycle.
///
/// The caller must hold a `CallContextGuard` for the duration so that `call-tool`
/// bridge lookups resolve correctly from the tein worker thread.
#[cfg(feature = "synthesised-tools")]
fn run_scheme(
    session: &super::synthesised::TeinSession,
    context_name: &str,
    code: &str,
) -> io::Result<String> {
    let ctx_name_escaped = super::synthesised::scheme_escape_string(context_name);
    session
        .evaluate(&format!("(set! %context-name% \"{ctx_name_escaped}\")"))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("context-name: {e}")))?;

    // NOTE: similar capture pattern exists in execute_synthesised (synthesised.rs).
    // Changes here may need to be assessed for that callsite too.
    let captured = session.with_capture(|ctx| ctx.evaluate(code));
    Ok(captured.format_eval())
}
```

- [ ] **Step 5: Update `get_or_create_context`**

At line ~109, change return type to `Arc<super::synthesised::TeinSession>`:
```rust
fn get_or_create_context(
    context_name: &str,
) -> io::Result<(Arc<super::synthesised::TeinSession>, std::thread::ThreadId)> {
```

The body stays the same -- it works generically with `Arc`.

- [ ] **Step 6: Update `register_eval_tools`**

Rename `tein_ctx` to `session` in the handler closure for clarity. The `spawn_blocking` call:

```rust
Box::pin(async move {
    let (session, guard) = setup?;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        run_scheme(&session, &context_name, &code)
    })
    .await
    .map_err(|e| io::Error::other(format!("scheme_eval panicked: {e}")))?
})
```

- [ ] **Step 7: Update tool description**

At line ~54-63, update to mention stdout/stderr capture:

```rust
    description: "Evaluate Scheme (R7RS) expression(s) in a persistent sandboxed environment. \
                  State persists across calls -- define variables, build data structures, compose \
                  computations. Returns the result of the last expression along with any stdout \
                  and stderr output (e.g. from display, write). Pre-imported: \
                  (scheme base), (scheme write), (scheme read), (scheme char), (scheme case-lambda), \
                  (tein json) for json-parse/json-stringify, (tein safe-regexp) for regex, \
                  (tein docs) for module-docs/describe, \
                  (tein introspect) for available-modules/module-exports/binding-info/env-bindings, \
                  (srfi 1) for list operations, (srfi 130) for string cursors, \
                  (chibi match) for pattern matching, and (harness tools) for call-tool. \
                  Additional safe modules can be imported with (import ...).",
```

- [ ] **Step 8: Verify full build**

Run: `cargo build --features synthesised-tools -p chibi-core 2>&1 | tail -5`
Expected: clean compile (or warnings about unused in tests -- those get fixed next)

- [ ] **Step 9: Commit**

```bash
git add crates/chibi-core/src/tools/eval.rs crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat: scheme_eval uses TeinSession for stdout/stderr capture"
```

### Task 9: Update `eval.rs` tests

**Files:**
- Modify: `crates/chibi-core/src/tools/eval.rs`

The existing tests call `build_eval_context()` which now returns `Arc<TeinSession>`. Tests that use `ctx.evaluate(...)` now call `session.evaluate(...)` (TeinSession delegate). Tests that check `result.to_string()` still work since `evaluate` returns `tein::Value`.

- [ ] **Step 1: Update test variables**

All tests that destructure `(ctx, tid)` or `(ctx, _)` should rename to `(session, tid)` / `(session, _)`. Replace `ctx.evaluate(...)` with `session.evaluate(...)`.

- [ ] **Step 2: Add stdout capture test**

```rust
#[test]
fn test_stdout_capture() {
    let (session, _) = super::build_eval_context().expect("context should build");
    let result = super::run_scheme(&session, "test", "(display 42)").unwrap();
    assert!(result.contains("result: #<unspecified>"), "display returns unspecified: {result}");
    assert!(result.contains("stdout: 42"), "stdout should contain displayed value: {result}");
    assert!(result.contains("stderr: (empty)"), "stderr should be empty: {result}");
}
```

- [ ] **Step 3: Add stderr capture test**

```rust
#[test]
fn test_stderr_capture() {
    let (session, _) = super::build_eval_context().expect("context should build");
    let result = super::run_scheme(
        &session, "test",
        "(display \"oops\" (current-error-port))"
    ).unwrap();
    assert!(result.contains("stdout: (empty)"), "stdout should be empty: {result}");
    assert!(result.contains("stderr: oops"), "stderr should contain error output: {result}");
}
```

- [ ] **Step 4: Add combined value + stdout test**

```rust
#[test]
fn test_value_with_stdout() {
    let (session, _) = super::build_eval_context().expect("context should build");
    let result = super::run_scheme(
        &session, "test",
        "(begin (display \"hello\") (+ 1 2))"
    ).unwrap();
    assert!(result.contains("result: 3"), "value should be 3: {result}");
    assert!(result.contains("stdout: hello"), "stdout should contain display output: {result}");
}
```

- [ ] **Step 5: Add buffer isolation test (no bleed between calls)**

```rust
#[test]
fn test_no_stdout_bleed_between_calls() {
    let (session, _) = super::build_eval_context().expect("context should build");
    let r1 = super::run_scheme(&session, "test", "(display \"first\")").unwrap();
    assert!(r1.contains("stdout: first"), "first call: {r1}");
    let r2 = super::run_scheme(&session, "test", "(+ 1 2)").unwrap();
    assert!(r2.contains("stdout: (empty)"), "second call should have no stdout: {r2}");
}
```

- [ ] **Step 6: Add error reporting test**

```rust
#[test]
fn test_error_in_captured_output() {
    let (session, _) = super::build_eval_context().expect("context should build");
    let result = super::run_scheme(&session, "test", "undefined-var").unwrap();
    assert!(result.contains("result: error:"), "should contain error: {result}");
    assert!(result.contains("stdout: (empty)"), "stdout should be empty on error: {result}");
}
```

- [ ] **Step 7: Run all eval tests**

Run: `cargo test --features synthesised-tools -p chibi-core eval 2>&1 | tail -20`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add crates/chibi-core/src/tools/eval.rs
git commit -m "test: update eval tests for structured output, add stdout/stderr capture tests"
```

### Task 10: Update `synthesised.rs` tests

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

The structured output format breaks many integration tests. The cleanest fix: add
a helper that extracts the `result:` field, and update `run_io_tool` to use it.
This keeps all downstream `assert_eq!` assertions unchanged.

- [ ] **Step 1: Add `extract_result_field` test helper**

Add to the test module:

```rust
/// Extract the `result: ` field from structured output.
/// Structured format: "result: <value>\nstdout: ...\nstderr: ..."
fn extract_result_field(output: &str) -> &str {
    output
        .strip_prefix("result: ")
        .and_then(|s| s.split_once('\n'))
        .map(|(result, _)| result)
        .unwrap_or(output)
}
```

- [ ] **Step 2: Update `run_io_tool` to extract result field**

The `run_io_tool` helper returns the raw `execute_tool` result. Wrap its return to
extract just the result field:

```rust
async fn run_io_tool(
    chibi: &crate::Chibi,
    registry: &Arc<RwLock<ToolRegistry>>,
    source: &str,
) -> String {
    // ... existing tool registration logic ...
    let raw = chibi
        .execute_tool("default", "io_test", serde_json::json!({}))
        .await
        .unwrap();
    extract_result_field(&raw).to_string()
}
```

This preserves all 10 `assert_eq!` tests that go through `run_io_tool`:
- `test_harness_io_vfs_read_write_roundtrip` (line 2552)
- `test_harness_io_vfs_write_then_read` (line 2572)
- `test_harness_io_vfs_not_found_returns_false` (line 2592)
- `test_harness_io_vfs_append` (line 2613)
- `test_harness_io_vfs_exists` (line 2642)
- `test_harness_io_vfs_list` (line 2683)
- `test_harness_io_vfs_list_nonexistent_returns_empty` (line 2703)
- `test_harness_io_fs_write_read_roundtrip` (line 2733)
- `test_harness_io_fs_not_found_returns_false` (line 2753)
- `test_harness_io_vfs_delete` (line 2874)

- [ ] **Step 3: Verify `test_execute_synthesised_tool` still works**

At line ~1864, this test destructures `ToolImpl::Synthesised { ref context, ... }`.
`context` is now `Arc<TeinSession>`. The test calls `context.evaluate(exec_binding)`
and `context.call(...)` -- these resolve to `TeinSession` delegate methods.
The assertion `result.as_string() == Some("3")` works because the test bypasses
`execute_synthesised` and calls delegates directly (raw `Value`, no `with_capture`).

Verify it compiles and passes unchanged.

- [ ] **Step 4: Update `test_context_name_injection`**

At line ~2258, this test calls `chibi.execute_tool(...)` which goes through
`dispatch_impl` -> `execute_synthesised` -> `with_capture`. Update:

```rust
let result = chibi
    .execute_tool("default", "ctx_test", serde_json::json!({}))
    .await
    .unwrap();
let ctx_name = extract_result_field(&result);
assert_eq!(
    ctx_name, "default",
    "context name should be injected as %%context-name%%: {result}"
);
```

- [ ] **Step 5: Update `test_task_crud_integration` ID extraction**

At line ~2325, the task ID is extracted with:
```rust
let id = create_result.split_whitespace().nth(2)...
```

With structured output, the result field contains `"created task XXXX at ..."`.
Extract the result field first:

```rust
let create_body = extract_result_field(&create_result);
assert!(create_body.contains("created task"), "unexpected: {}", create_result);
let id = create_body
    .split_whitespace()
    .nth(2)
    .expect("expected id in create result")
    .to_string();
```

Apply the same `extract_result_field` pattern to all other `execute_tool` calls
in this test (`list_result`, `view_result`, `update_result`, `view2`,
`delete_result`, `list2`).

- [ ] **Step 6: Add synthesised tool error capture test**

Add a test verifying that scheme errors in synthesised tools are captured in the
structured output (intentional behavioral change from `Err(io::Error)` to
`Ok("result: error: ...")`):

```rust
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
```

- [ ] **Step 7: Run all synthesised tests**

Run: `cargo test --features synthesised-tools -p chibi-core synthesised 2>&1 | tail -30`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "test: update synthesised tool tests for structured output format"
```

### Task 11: Run full test suite and lint

- [ ] **Step 1: Run full test suite**

Run: `cargo test --features synthesised-tools -p chibi-core 2>&1 | tail -30`
Expected: all tests pass

- [ ] **Step 2: Run clippy / lint**

Run: `just lint 2>&1 | tail -30`
Expected: no new warnings

- [ ] **Step 3: Fix any issues**

Address any test failures or lint warnings.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "chore: lint fixes for tein stdout/stderr capture"
```

### Task 12: Update AGENTS.md quirks

- [ ] **Step 1: Add structured output format quirk**

Add to the quirks section of AGENTS.md:

```markdown
- `scheme_eval` and `execute_synthesised` return structured output: `result: <value>\nstdout: <output>\nstderr: <output>`. The `result` field contains the expression's return value (or `error: ...`). stdout/stderr show `(empty)` when no output was captured. `format_eval()` stringifies with `to_string()`; `format_tool()` unwraps scheme strings via `as_string()`.
```

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: add structured output format quirk for scheme_eval and synthesised tools"
```
