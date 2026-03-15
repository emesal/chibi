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

use super::registry::{ToolCategory, ToolRegistry};
use super::{BuiltinToolDef, Tool, ToolPropertyDef};

/// Process-global store of persistent tein sessions, keyed by chibi context name.
/// Each entry is `(Arc<TeinSession>, worker_thread_id)`.
/// `TeinSession` wraps `ThreadLocalContext` with stdout/stderr capture.
///
/// Entries are evicted on context clear/destroy/rename via [`evict_eval_context`]
/// so the next `scheme_eval` call gets a fresh session with the current prelude.
/// Without eviction, stale sessions can accumulate corrupted C-level state in
/// chibi-scheme's heap, leading to segfaults.
#[cfg(feature = "synthesised-tools")]
type EvalContextMap =
    Mutex<HashMap<String, (Arc<super::synthesised::TeinSession>, std::thread::ThreadId)>>;

#[cfg(feature = "synthesised-tools")]
static EVAL_CONTEXTS: LazyLock<EvalContextMap> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Remove the cached tein session for a context name.
///
/// Called from context lifecycle operations (clear, destroy, rename) so the
/// next `scheme_eval` call creates a fresh session. This prevents:
/// - stale interpreter state carrying over after context archive/clear
/// - C-level heap corruption from accumulated bad state in long-lived sessions
///
/// No-op if the context has no cached session (e.g. `scheme_eval` was never
/// called for this context, or the feature is disabled).
pub fn evict_eval_context(name: &str) {
    #[cfg(feature = "synthesised-tools")]
    {
        EVAL_CONTEXTS.lock().unwrap().remove(name);
    }
    let _ = name; // suppress unused warning when feature disabled
}

pub const SCHEME_EVAL_TOOL_NAME: &str = "scheme_eval";

pub static EVAL_TOOL_DEFS: &[BuiltinToolDef] = &[BuiltinToolDef {
    name: SCHEME_EVAL_TOOL_NAME,
    description: "Evaluate Scheme (R7RS) expression(s) in a persistent sandboxed environment. \
                  State persists across calls -- define variables, build data structures, compose \
                  computations. Returns the result of the last expression along with any stdout \
                  and stderr output (e.g. from display, write). Pre-imported: \
                  (scheme base), (scheme write), (scheme read), (scheme char), (scheme case-lambda), \
                  (scheme inexact) for sin/cos/atan/sqrt/exp/log/finite?/nan?, \
                  (scheme complex) for make-polar/magnitude/angle/real-part/imag-part, \
                  (tein json) for json-parse/json-stringify, (tein safe-regexp) for regex, \
                  (tein docs) for module-docs/describe, \
                  (tein introspect) for available-modules/module-exports/binding-info/env-bindings, \
                  (srfi 1) for list operations, (srfi 27) for random-integer/random-real, \
                  (srfi 69) for basic hash tables, (srfi 95) for sort/merge, \
                  (srfi 125) for comprehensive hash tables, (srfi 128) for comparators, \
                  (srfi 130) for string cursors, \
                  (srfi 132) for comprehensive sorting, (srfi 133) for vector operations, \
                  (chibi match) for pattern matching, and (harness tools) for call-tool. \
                  Additional safe modules can be imported with (import ...).",
    properties: &[ToolPropertyDef {
        name: "code",
        prop_type: "string",
        description: "Scheme expression(s) to evaluate",
        default: None,
    }],
    required: &["code"],
    summary_params: &["code"],
}];

/// Build a sandboxed tein session for `scheme_eval`.
///
/// Delegates to `synthesised::build_sandboxed_harness_context`, which now
/// includes `EVAL_PRELUDE` in `build_tein_context` — no separate prelude step.
/// Returns `(Arc<TeinSession>, worker_thread_id)`.
#[cfg(feature = "synthesised-tools")]
fn build_eval_context() -> io::Result<(Arc<super::synthesised::TeinSession>, std::thread::ThreadId)>
{
    let (session, tid) = super::synthesised::build_sandboxed_harness_context()?;
    Ok((Arc::new(session), tid))
}

/// Run scheme code in the persistent tein context. Called on a blocking thread.
///
/// Injects `%context-name%`, evaluates user code. Scheme errors are returned as
/// `Ok("error: ...")` — they do not abort the prompt cycle.
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

    let captured = session.with_capture(|ctx| ctx.evaluate(code));
    Ok(captured.format_eval())
}

/// Get or create the persistent tein context for a chibi context name.
#[cfg(feature = "synthesised-tools")]
fn get_or_create_context(
    context_name: &str,
) -> io::Result<(Arc<super::synthesised::TeinSession>, std::thread::ThreadId)> {
    let mut contexts = EVAL_CONTEXTS.lock().unwrap();
    if let Some(entry) = contexts.get(context_name) {
        Ok((Arc::clone(&entry.0), entry.1))
    } else {
        let (session, tid) = build_eval_context()?;
        contexts.insert(context_name.to_string(), (Arc::clone(&session), tid));
        Ok((session, tid))
    }
}

/// Register the `scheme_eval` tool into the shared registry Arc.
///
/// Takes `&Arc<RwLock<ToolRegistry>>` (not `&mut ToolRegistry`) because the
/// handler closure needs to capture an `Arc` clone for `CallContextGuard`.
/// Must be called after the registry `Arc` is created in `chibi.rs`.
///
/// Two-phase execution avoids blocking a tokio worker thread:
///
/// 1. **Setup** (sync, tokio thread): extract owned args, get/create tein
///    context, `CallContextGuard::set` snapshots `&ToolCallContext` into
///    `BRIDGE_CALL_CTX`. This is the only step needing the borrowed lifetime.
/// 2. **Eval** (`spawn_blocking`): guard + context + owned args move to a
///    blocking thread. Scheme runs there; guard drops and cleans up on return.
#[cfg(feature = "synthesised-tools")]
pub fn register_eval_tools(registry: &Arc<std::sync::RwLock<ToolRegistry>>) {
    use super::synthesised::CallContextGuard;

    let registry_for_handler = Arc::clone(registry);
    let handler: super::registry::ToolHandler = Arc::new(move |call| {
        let context_name = call.context.context_name.to_string();
        let code = call
            .args
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if code.is_empty() {
            return Box::pin(async { Ok(String::new()) });
        }

        // Phase 1 (tokio thread): setup while borrowed ToolCallContext is valid.
        let reg = Arc::clone(&registry_for_handler);
        let setup = (|| -> io::Result<_> {
            let (session, worker_tid) = get_or_create_context(&context_name)?;
            let guard = CallContextGuard::set(call.context, reg, worker_tid);
            Ok((session, guard))
        })();

        // Phase 2 (blocking thread): scheme code runs off the tokio pool.
        Box::pin(async move {
            let (session, guard) = setup?;
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                run_scheme(&session, &context_name, &code)
            })
            .await
            .map_err(|e| io::Error::other(format!("scheme_eval panicked: {e}")))?
        })
    });

    let mut tool = Tool::from_builtin_def(&EVAL_TOOL_DEFS[0], handler, ToolCategory::Eval);
    // Concurrent calls for the same context would collide on BRIDGE_CALL_CTX.
    tool.metadata.parallel = false;
    registry.write().unwrap().register(tool);
}

/// Stub when synthesised-tools feature is disabled.
#[cfg(not(feature = "synthesised-tools"))]
pub fn register_eval_tools(_registry: &Arc<std::sync::RwLock<ToolRegistry>>) {}

#[cfg(all(test, feature = "synthesised-tools"))]
mod tests {
    #[test]
    fn test_tool_def() {
        assert_eq!(super::EVAL_TOOL_DEFS[0].name, "scheme_eval");
        assert_eq!(super::EVAL_TOOL_DEFS[0].required, &["code"]);
    }

    #[test]
    fn test_build_context_basic() {
        let (session, tid) = super::build_eval_context().expect("context should build");
        let result = session.evaluate("(+ 1 2)").expect("eval should succeed");
        assert_eq!(result.to_string(), "3");
        // Worker thread should differ from test thread
        assert_ne!(tid, std::thread::current().id());
    }

    #[test]
    fn test_context_persistence() {
        let (session, _) = super::build_eval_context().expect("context should build");
        session
            .evaluate("(define x 42)")
            .expect("define should work");
        let result = session.evaluate("x").expect("x should be defined");
        assert_eq!(result.to_string(), "42");
    }

    #[test]
    fn test_prelude_srfi_1() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = session
            .evaluate("(fold + 0 '(1 2 3 4 5))")
            .expect("fold from srfi-1");
        assert_eq!(result.to_string(), "15");
    }

    #[test]
    fn test_prelude_srfi_130() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = session
            .evaluate(r#"(string-contains "hello world" "world")"#)
            .expect("string-contains from srfi-130");
        // Returns cursor index, not #f
        assert_ne!(result.to_string(), "#f");
    }

    #[test]
    fn test_prelude_tein_json() {
        // (tein json) exports json-parse and json-stringify
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = session
            .evaluate(r#"(json-parse "{\"a\":1}")"#)
            .expect("json-parse from tein json");
        assert!(result.to_string().contains("a"));
    }

    #[test]
    fn test_prelude_safe_regexp() {
        // (tein safe-regexp) exports regexp, regexp-search, regexp-matches?, etc.
        // regexp-search returns a vector of match vectors on success, #f on no match.
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = session
            .evaluate(r#"(vector? (regexp-search "hello" "hello world"))"#)
            .expect("regexp-search should work");
        assert_eq!(result.to_string(), "#t");
    }

    #[test]
    fn test_prelude_chibi_match() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = session
            .evaluate("(match '(1 2 3) ((a b c) (+ a b c)))")
            .expect("chibi match");
        assert_eq!(result.to_string(), "6");
    }

    #[test]
    fn test_prelude_tein_docs() {
        let (session, _) = super::build_eval_context().expect("context should build");
        // module-docs returns doc pairs from an alist — just verify the binding exists
        let result = session
            .evaluate("(procedure? module-docs)")
            .expect("module-docs from tein docs");
        assert_eq!(result.to_string(), "#t");
    }

    #[test]
    fn test_prelude_tein_introspect() {
        let (session, _) = super::build_eval_context().expect("context should build");
        // available-modules returns a list of importable modules
        let result = session
            .evaluate("(list? (available-modules))")
            .expect("available-modules from tein introspect");
        assert_eq!(result.to_string(), "#t");
    }

    #[test]
    fn test_error_reporting() {
        let (session, _) = super::build_eval_context().expect("context should build");
        // errors are captured in structured output, not propagated as Err
        let result = super::run_scheme(&session, "test", "undefined-var").unwrap();
        assert!(
            result.contains("result: error:"),
            "should contain error: {result}"
        );
    }

    #[test]
    fn test_contexts_isolation() {
        // Two contexts should have independent state.
        let (session_a, _) = super::build_eval_context().expect("build a");
        let (session_b, _) = super::build_eval_context().expect("build b");
        session_a.evaluate("(define x 1)").unwrap();
        session_b.evaluate("(define x 2)").unwrap();
        assert_eq!(session_a.evaluate("x").unwrap().to_string(), "1");
        assert_eq!(session_b.evaluate("x").unwrap().to_string(), "2");
    }

    #[test]
    fn test_stdout_capture() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = super::run_scheme(&session, "test", "(display 42)").unwrap();
        assert!(
            result.contains("result: #<unspecified>"),
            "display returns unspecified: {result}"
        );
        assert!(
            result.contains("stdout: 42"),
            "stdout should contain displayed value: {result}"
        );
        assert!(
            result.contains("stderr: (empty)"),
            "stderr should be empty: {result}"
        );
    }

    #[test]
    fn test_stderr_capture() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result =
            super::run_scheme(&session, "test", r#"(display "oops" (current-error-port))"#)
                .unwrap();
        assert!(
            result.contains("stdout: (empty)"),
            "stdout should be empty: {result}"
        );
        assert!(
            result.contains("stderr: oops"),
            "stderr should contain error output: {result}"
        );
    }

    #[test]
    fn test_value_with_stdout() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result =
            super::run_scheme(&session, "test", r#"(begin (display "hello") (+ 1 2))"#).unwrap();
        assert!(result.contains("result: 3"), "value should be 3: {result}");
        assert!(
            result.contains("stdout: hello"),
            "stdout should contain display output: {result}"
        );
    }

    #[test]
    fn test_no_stdout_bleed_between_calls() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r1 = super::run_scheme(&session, "test", r#"(display "first")"#).unwrap();
        assert!(r1.contains("stdout: first"), "first call: {r1}");
        let r2 = super::run_scheme(&session, "test", "(+ 1 2)").unwrap();
        assert!(
            r2.contains("stdout: (empty)"),
            "second call should have no stdout: {r2}"
        );
    }

    #[test]
    fn test_error_in_captured_output() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let result = super::run_scheme(&session, "test", "undefined-var").unwrap();
        assert!(
            result.contains("result: error:"),
            "should contain error: {result}"
        );
        assert!(
            result.contains("stdout: (empty)"),
            "stdout should be empty on error: {result}"
        );
    }

    #[test]
    fn test_prelude_scheme_inexact() {
        let (session, _) = super::build_eval_context().expect("context should build");
        // atan, sin, cos, sqrt, exp, log, finite?, nan? all from (scheme inexact)
        let r = session.evaluate("(atan 1.0)").expect("atan");
        assert!(r.to_string().starts_with("0.785"), "atan(1) ≈ π/4: {r}");
        let r = session.evaluate("(finite? 1.0)").expect("finite?");
        assert_eq!(r.to_string(), "#t");
        let r = session.evaluate("(nan? +nan.0)").expect("nan?");
        assert_eq!(r.to_string(), "#t");
    }

    #[test]
    fn test_prelude_scheme_complex() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session
            .evaluate("(magnitude (make-rectangular 3.0 4.0))")
            .expect("magnitude");
        // magnitude of 3+4i = 5; chibi may return exact 5 or inexact 5.0
        assert!(
            r.to_string() == "5" || r.to_string() == "5.0",
            "magnitude of 3+4i = 5: {r}"
        );
    }

    #[test]
    fn test_prelude_srfi_27() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session
            .evaluate("(number? (random-integer 100))")
            .expect("random-integer");
        assert_eq!(r.to_string(), "#t");
    }

    #[test]
    fn test_prelude_srfi_69() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session
            .evaluate("(let ((ht (make-hash-table equal?))) (hash-table-set! ht 'a 1) (hash-table-ref ht 'a #f))")
            .expect("srfi-69 hash-table");
        assert_eq!(r.to_string(), "1");
    }

    #[test]
    fn test_prelude_srfi_95() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session.evaluate("(sort '(3 1 2) <)").expect("srfi-95 sort");
        assert_eq!(r.to_string(), "(1 2 3)");
    }

    #[test]
    fn test_prelude_srfi_132() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session
            .evaluate("(list-sort < '(3 1 2))")
            .expect("srfi-132 list-sort");
        assert_eq!(r.to_string(), "(1 2 3)");
    }

    #[test]
    fn test_prelude_srfi_133() {
        let (session, _) = super::build_eval_context().expect("context should build");
        let r = session
            .evaluate("(vector->list (vector-map + #(1 2 3) #(10 20 30)))")
            .expect("srfi-133 vector-map");
        assert_eq!(r.to_string(), "(11 22 33)");
    }

    #[test]
    fn test_evict_eval_context() {
        // Insert a context, evict it, verify it's gone and a fresh one is created.
        let name = "evict-test";
        let (session, _) = super::get_or_create_context(name).expect("create");
        session.evaluate("(define evict-marker 99)").unwrap();
        assert_eq!(session.evaluate("evict-marker").unwrap().to_string(), "99");

        super::evict_eval_context(name);

        let (session2, _) = super::get_or_create_context(name).expect("recreate");
        let r = session2.evaluate("(+ 1 1)").expect("basic eval");
        assert_eq!(r.to_string(), "2");
        // The old binding should be gone in the new session.
        let r = super::run_scheme(&session2, name, "evict-marker").unwrap();
        assert!(
            r.contains("error:"),
            "old binding should not survive eviction: {r}"
        );
    }
}
