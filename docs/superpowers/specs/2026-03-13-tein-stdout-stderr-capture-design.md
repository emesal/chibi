# Tein Stdout/Stderr Capture for scheme_eval and Synthesised Tools

**Date:** 2026-03-13
**Status:** Approved
**Scope:** chibi-core (no tein changes)

## Problem

`scheme_eval` and `execute_synthesised` only capture the return value of scheme evaluation. Side-effect output via `display`, `write`, or `(current-error-port)` is silently discarded. When an LLM uses `(display pi)`, the tool returns `#<unspecified>` (the return value of `display`) instead of the printed output.

## Solution

Introduce `TeinSession` — a wrapper struct that owns a `ThreadLocalContext` alongside shared stdout/stderr buffers. A `with_capture` method encapsulates the drain-eval-flush-read cycle. Both `scheme_eval` and `execute_synthesised` use this struct, producing a structured `result + stdout + stderr` response for every call.

## Design

### `TeinSession` struct

Lives in `synthesised.rs`. Owns the three pieces that always travel together:

```rust
pub(crate) struct TeinSession {
    ctx: ThreadLocalContext,
    stdout_buf: Arc<Mutex<Vec<u8>>>,
    stderr_buf: Arc<Mutex<Vec<u8>>>,
}
```

Constructed by `build_tein_context`. The init closure creates the buffers, wires them via `Context::open_output_port` + `set_current_output_port` / `set_current_error_port`. These ports survive across evaluations (confirmed by tein's own test suite: `test_set_current_output_port_survives_multiple_evals`).

`build_tein_context` return type changes from `(ThreadLocalContext, ThreadId)` to `(TeinSession, ThreadId)`.

### `with_capture` method

The core drain-eval-flush-read cycle. Takes a closure that receives `&ThreadLocalContext` and returns `Result<Value>`:

```rust
impl TeinSession {
    fn with_capture<F>(&self, f: F) -> CapturedOutput
    where
        F: FnOnce(&ThreadLocalContext) -> tein::Result<Value>,
    {
        self.stdout_buf.lock().unwrap().clear();
        self.stderr_buf.lock().unwrap().clear();

        let value = f(&self.ctx);

        let _ = self.ctx.evaluate("(flush-output (current-output-port))");
        let _ = self.ctx.evaluate("(flush-output (current-error-port))");

        let stdout = String::from_utf8_lossy(&self.stdout_buf.lock().unwrap()).to_string();
        let stderr = String::from_utf8_lossy(&self.stderr_buf.lock().unwrap()).to_string();

        CapturedOutput { value, stdout, stderr }
    }

    /// Delegate to inner context for internal calls (e.g. setting %context-name%,
    /// resolving bindings) that don't need output capture.
    pub(crate) fn evaluate(&self, code: &str) -> tein::Result<Value> {
        self.ctx.evaluate(code)
    }

    /// Delegate to inner context for calling procedures without capture.
    /// Used by hook dispatch (hooks.rs) which doesn't need stdout/stderr capture.
    pub(crate) fn call(&self, proc: &Value, args: &[Value]) -> tein::Result<Value> {
        self.ctx.call(proc, args)
    }
}
```

Thread safety: `ThreadLocalContext` serialises all calls via its internal channel mutex, so no interleaving is possible between drain and read.

### `CapturedOutput` struct

Holds the raw `Result<Value>` so callers can stringify as needed:

```rust
pub(crate) struct CapturedOutput {
    pub value: tein::Result<Value>,
    pub stdout: String,
    pub stderr: String,
}
```

Two formatting methods for the two callsites:

- `format_eval()` — stringifies value with `to_string()` (for `scheme_eval`)
- `format_tool()` — stringifies value with `as_string()` unwrap, falling back to `to_string()` (for synthesised tools, which conventionally return scheme strings)

Both produce:
```
result: <value>
stdout: <stdout or (empty)>
stderr: <stderr or (empty)>
```

Errors from user code become `"error: <message>"` in the value field (same as `scheme_eval` today — the evaluation doesn't abort).

**Behavioral change for synthesised tools:** currently `execute_synthesised` propagates scheme errors as `io::Error` (tool failure). With `with_capture`, scheme errors are captured in `CapturedOutput.value` and formatted as `"error: <message>"` in the result — returning `Ok(String)` rather than `Err(io::Error)`. This is an intentional alignment: both `scheme_eval` and synthesised tools now treat scheme errors as informational output rather than tool failures, giving the LLM the error message alongside any stdout/stderr produced before the error.

### Callsite: `eval.rs`

`run_scheme` changes to use `TeinSession`:

```rust
fn run_scheme(session: &TeinSession, context_name: &str, code: &str) -> io::Result<String> {
    // inject context name (internal, no capture needed)
    session.evaluate(&format!(...))?;

    // NOTE: similar capture pattern exists in execute_synthesised (synthesised.rs).
    // Changes here may need to be assessed for that callsite too.
    let captured = session.with_capture(|ctx| ctx.evaluate(code));
    Ok(captured.format_eval())
}
```

`EVAL_CONTEXTS` type changes: values become `(Arc<TeinSession>, ThreadId)` instead of `(Arc<ThreadLocalContext>, ThreadId)`.

### Callsite: `execute_synthesised`

```rust
pub async fn execute_synthesised(
    session: &TeinSession,  // was: context: &ThreadLocalContext
    exec_binding: &str,
    call: &ToolCall<'_>,
    registry: Arc<RwLock<ToolRegistry>>,
    worker_thread_id: std::thread::ThreadId,
) -> io::Result<String> {
    // ... inject context name via session.evaluate() ...

    let args_alist = json_args_to_scheme_alist(call.args)?;
    let exec_fn = session.evaluate(exec_binding)
        .map_err(|e| io::Error::other(format!("resolve {exec_binding}: {e}")))?;

    // NOTE: similar capture pattern exists in run_scheme (eval.rs).
    // Changes here may need to be assessed for that callsite too.
    let captured = session.with_capture(|ctx| ctx.call(&exec_fn, &[args_alist]));
    Ok(captured.format_tool())
}
```

### `ToolImpl::Synthesised` storage

Changes from `Arc<ThreadLocalContext>` to `Arc<TeinSession>`. Propagates through:
- `ToolImpl::Synthesised` field type in `registry.rs` (and its `Clone` impl)
- `dispatch_impl` in `registry.rs` (passes session to `execute_synthesised`)
- `scan_and_register` in `synthesised.rs`
- `reload_tool_from_content` in `synthesised.rs`
- Tool handler closures in `load_tools_from_source`
- `execute_hook` in `hooks.rs` (destructures `ToolImpl::Synthesised` to access `context` — uses `session.evaluate()` and `session.call()` delegates, no capture needed for hooks)
- `build_sandboxed_harness_context` return type (used by `eval.rs`)
- `build_eval_context` in `eval.rs` (wraps result in `Arc<TeinSession>`)

### Tool description update

`EVAL_TOOL_DEFS` description updated to mention that the tool returns the expression value, stdout, and stderr — so the LLM knows `display` works and output is captured.

### `SharedWriter`

The `SharedWriter(Arc<Mutex<Vec<u8>>>)` pattern is used in tein's test suite but isn't exported. We define our own in `synthesised.rs`:

```rust
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

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

### Init closure changes in `build_tein_context`

The init closure receives `&Context`, creates two `SharedWriter`-backed output ports, and sets them as current-output-port and current-error-port:

```rust
let stdout_buf = Arc::new(Mutex::new(Vec::new()));
let stderr_buf = Arc::new(Mutex::new(Vec::new()));
let stdout_clone = Arc::clone(&stdout_buf);
let stderr_clone = Arc::clone(&stderr_buf);

let init = move |ctx: &Context| -> tein::Result<()> {
    // ... existing init logic ...

    // wire stdout/stderr capture
    let out_port = ctx.open_output_port(SharedWriter(stdout_clone.clone()))?;
    ctx.set_current_output_port(&out_port)?;
    let err_port = ctx.open_output_port(SharedWriter(stderr_clone.clone()))?;
    ctx.set_current_error_port(&err_port)?;

    Ok(())
};
```

The `stdout_buf` / `stderr_buf` arcs are then bundled into `TeinSession` alongside the `ThreadLocalContext`.

## Output format examples

`(display (* 4 (atan 1.0)))`:
```
result: #<unspecified>
stdout: 3.141592653589793
stderr: (empty)
```

`(+ 1 2)`:
```
result: 3
stdout: (empty)
stderr: (empty)
```

`(begin (display "hello") (+ 1 2))`:
```
result: 3
stdout: hello
stderr: (empty)
```

Error case:
```
result: error: undefined variable: (foo)
stdout: (empty)
stderr: (empty)
```

## Scope

- **In scope:** `TeinSession`, `CapturedOutput`, `with_capture`, callsite updates in `eval.rs` and `synthesised.rs`, `ToolImpl::Synthesised` storage change, tool description update, cross-reference comments.
- **Out of scope:** tein library changes, `ThreadLocalContext` API additions.

**Breaking change:** output format changes from bare value to structured `result/stdout/stderr`. Project philosophy: "backwards compatibility not a priority" (pre-alpha).

## Testing

- Unit test: `TeinSession::with_capture` captures `display` output in stdout.
- Unit test: stderr via `(display "x" (current-error-port))` captured.
- Unit test: expression value returned alongside output.
- Unit test: buffers drained between calls (no bleed).
- Existing tests updated to assert on the new format.
