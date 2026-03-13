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
        (tein docs)
        (tein introspect)
        (srfi 1)
        (srfi 130)
        (chibi match)
        (harness tools))
"#;

/// Process-global store of persistent tein sessions, keyed by chibi context name.
/// Each entry is `(Arc<TeinSession>, worker_thread_id)`.
/// `TeinSession` wraps `ThreadLocalContext` with stdout/stderr capture.
///
/// Contexts are never evicted (process lifetime). Access serialised via Mutex.
#[cfg(feature = "synthesised-tools")]
type EvalContextMap =
    Mutex<HashMap<String, (Arc<super::synthesised::TeinSession>, std::thread::ThreadId)>>;

#[cfg(feature = "synthesised-tools")]
static EVAL_CONTEXTS: LazyLock<EvalContextMap> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub const SCHEME_EVAL_TOOL_NAME: &str = "scheme_eval";

pub static EVAL_TOOL_DEFS: &[BuiltinToolDef] = &[BuiltinToolDef {
    name: SCHEME_EVAL_TOOL_NAME,
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
/// Delegates to `synthesised::build_sandboxed_harness_context` for the FFI
/// bridge setup, then evaluates `EVAL_PRELUDE` to pre-import standard modules.
/// Returns `(Arc<TeinSession>, worker_thread_id)`.
#[cfg(feature = "synthesised-tools")]
fn build_eval_context() -> io::Result<(Arc<super::synthesised::TeinSession>, std::thread::ThreadId)>
{
    let (session, tid) = super::synthesised::build_sandboxed_harness_context()?;
    session
        .evaluate(EVAL_PRELUDE)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("eval prelude: {e}")))?;
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
}
