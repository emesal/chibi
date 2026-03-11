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
use tein::ThreadLocalContext;

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
        (srfi 1)
        (srfi 130)
        (chibi match)
        (harness tools))
"#;

/// Process-global store of persistent tein contexts, keyed by chibi context name.
/// Each entry is `(Arc<ThreadLocalContext>, worker_thread_id)`.
/// `ThreadLocalContext` is not Clone — Arc provides cheap sharing.
///
/// Contexts are never evicted (process lifetime). Access serialised via Mutex.
#[cfg(feature = "synthesised-tools")]
type EvalContextMap = Mutex<HashMap<String, (Arc<ThreadLocalContext>, std::thread::ThreadId)>>;

#[cfg(feature = "synthesised-tools")]
static EVAL_CONTEXTS: LazyLock<EvalContextMap> = LazyLock::new(|| Mutex::new(HashMap::new()));

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

/// Build a sandboxed tein context for `scheme_eval`.
///
/// Delegates to `synthesised::build_sandboxed_harness_context` for the FFI
/// bridge setup, then evaluates `EVAL_PRELUDE` to pre-import standard modules.
/// Returns `(Arc<ThreadLocalContext>, worker_thread_id)`.
#[cfg(feature = "synthesised-tools")]
fn build_eval_context() -> io::Result<(Arc<ThreadLocalContext>, std::thread::ThreadId)> {
    let (ctx, tid) = super::synthesised::build_sandboxed_harness_context()?;
    ctx.evaluate(EVAL_PRELUDE)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("eval prelude: {e}")))?;
    Ok((Arc::new(ctx), tid))
}

/// Execute a `scheme_eval` tool call.
///
/// Retrieves or creates a persistent tein context for the calling chibi context,
/// sets `CallContextGuard` for `call-tool` support, injects `%context-name%`,
/// evaluates the code, and returns the display representation.
///
/// Errors from scheme evaluation are returned as `Ok("error: ...")` —
/// they do not abort the prompt cycle.
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
            contexts.insert(context_name.to_string(), (Arc::clone(&ctx), tid));
            (ctx, tid)
        }
    };

    // Set the call-tool bridge context for this evaluation.
    let _guard = CallContextGuard::set(call_ctx, registry, worker_tid);

    // Inject %context-name% so call-tool resolves VFS paths correctly.
    let ctx_name_escaped = super::synthesised::scheme_escape_string(context_name);
    tein_ctx
        .evaluate(&format!("(set! %context-name% \"{ctx_name_escaped}\")"))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("context-name: {e}")))?;

    // Evaluate the user's code. Scheme errors are returned as Ok("error: ...").
    match tein_ctx.evaluate(code) {
        Ok(val) => Ok(val.to_string()),
        Err(e) => Ok(format!("error: {e}")),
    }
}

/// Register the `scheme_eval` tool into the shared registry Arc.
///
/// Takes `&Arc<RwLock<ToolRegistry>>` (not `&mut ToolRegistry`) because the
/// handler closure needs to capture an `Arc` clone for `CallContextGuard`.
/// Must be called after the registry `Arc` is created in `chibi.rs`.
#[cfg(feature = "synthesised-tools")]
pub fn register_eval_tools(registry: &Arc<std::sync::RwLock<ToolRegistry>>) {
    let registry_for_handler = Arc::clone(registry);
    let handler: super::registry::ToolHandler = Arc::new(move |call| {
        let context_name = call.context.context_name.to_string();
        let code = call
            .args
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reg = Arc::clone(&registry_for_handler);
        Box::pin(async move { execute_scheme_eval(&context_name, &code, call.context, reg) })
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
        let result = ctx
            .evaluate("(fold + 0 '(1 2 3 4 5))")
            .expect("fold from srfi-1");
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
        // (tein json) exports json-parse and json-stringify
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate(r#"(json-parse "{\"a\":1}")"#)
            .expect("json-parse from tein json");
        assert!(result.to_string().contains("a"));
    }

    #[test]
    fn test_prelude_safe_regexp() {
        // (tein safe-regexp) exports regexp, regexp-search, regexp-matches?, etc.
        // regexp-search returns a vector of match vectors on success, #f on no match.
        let (ctx, _) = super::build_eval_context().expect("context should build");
        let result = ctx
            .evaluate(r#"(vector? (regexp-search "hello" "hello world"))"#)
            .expect("regexp-search should work");
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

    #[test]
    fn test_contexts_isolation() {
        // Two contexts should have independent state.
        let (ctx_a, _) = super::build_eval_context().expect("build a");
        let (ctx_b, _) = super::build_eval_context().expect("build b");
        ctx_a.evaluate("(define x 1)").unwrap();
        ctx_b.evaluate("(define x 2)").unwrap();
        assert_eq!(ctx_a.evaluate("x").unwrap().to_string(), "1");
        assert_eq!(ctx_b.evaluate("x").unwrap().to_string(), "2");
    }
}
