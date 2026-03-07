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
//! Each synthesised tool gets its own sandboxed tein context, shared via
//! `Arc` for concurrent dispatch. All access goes through `ThreadLocalContext`,
//! which is `Send + Sync`.

#[cfg(feature = "synthesised-tools")]
use std::sync::Arc;

#[cfg(feature = "synthesised-tools")]
use tein::{Context, ThreadLocalContext, Value, sandbox::Modules};

use std::io;

use crate::tools::registry::{ToolCall, ToolRegistry};
use crate::tools::{Tool, ToolCategory, ToolImpl, ToolMetadata};
use crate::vfs::{Vfs, VfsCaller, VfsPath}; // Vfs+VfsCaller used in scan_and_register

/// Load a synthesised tool from scheme source.
///
/// Evaluates `source` in a sandboxed tein context and extracts the five
/// required bindings. Returns an error if any binding is missing or has the
/// wrong type.
#[cfg(feature = "synthesised-tools")]
pub fn load_tool_from_source(source: &str, vfs_path: &VfsPath) -> io::Result<Tool> {
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
            context,
        },
        category: ToolCategory::Synthesised,
    })
}

/// Execute a synthesised tool by calling its `tool-execute` procedure.
///
/// Converts JSON args to a scheme alist, resolves `tool-execute` in the
/// context, and calls it. The result is coerced to a string via
/// `Value::as_string()` (for scheme strings) or `Display` (for other values).
#[cfg(feature = "synthesised-tools")]
pub async fn execute_synthesised(
    context: &ThreadLocalContext,
    call: &ToolCall<'_>,
) -> io::Result<String> {
    let args_alist = json_args_to_scheme_alist(call.args)?;
    let exec_fn = context
        .evaluate("tool-execute")
        .map_err(|e| io::Error::other(format!("resolve tool-execute: {e}")))?;
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
pub async fn execute_synthesised(_context: &(), _call: &ToolCall<'_>) -> io::Result<String> {
    unreachable!("synthesised-tools feature not enabled")
}

// --- startup scan ------------------------------------------------------------

/// Scan writable VFS zones for `.scm` tool files and register them.
///
/// Called once at startup after the VFS and registry are fully constructed.
/// Silently skips zones that don't exist yet and logs warnings for files that
/// fail to load. Non-`.scm` entries are ignored.
///
/// **Zones scanned:** `/tools/shared` (globally shared tools).
/// Context-home and flock zones are deferred to a future scoping task.
#[cfg(feature = "synthesised-tools")]
pub async fn scan_and_register(vfs: &Vfs, registry: &mut ToolRegistry) -> io::Result<()> {
    let zones = ["/tools/shared"];
    for zone in &zones {
        let Ok(zone_path) = VfsPath::new(zone) else {
            continue;
        };
        if !vfs
            .exists(VfsCaller::System, &zone_path)
            .await
            .unwrap_or(false)
        {
            continue;
        }
        let entries = match vfs.list(VfsCaller::System, &zone_path).await {
            Ok(e) => e,
            Err(_) => continue, // zone unreadable — skip silently
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
            if let Ok(tool) = load_tool_from_source(&source_str, &file_path) {
                registry.register(tool);
                // invalid source — skip silently (caller can inspect via VFS)
            }
        }
    }
    Ok(())
}

// --- hot-reload callbacks ----------------------------------------------------

/// Reload (or register for the first time) a synthesised tool from source bytes.
///
/// Called synchronously from the `on_scm_change` callback after a successful
/// write. The `content` bytes are the data that was just written to the VFS,
/// so no re-read is needed.
#[cfg(feature = "synthesised-tools")]
pub fn reload_tool_from_content(
    registry: &std::sync::Arc<std::sync::RwLock<ToolRegistry>>,
    path: &VfsPath,
    content: &[u8],
) {
    let Ok(source_str) = std::str::from_utf8(content) else {
        return;
    };
    if let Ok(tool) = load_tool_from_source(source_str, path) {
        registry.write().unwrap().register(tool);
        // invalid source — leave previous version registered
    }
}

/// Unregister the synthesised tool whose VFS path matches `path`.
///
/// Called synchronously from the `on_scm_change` callback after a successful
/// delete.
#[cfg(feature = "synthesised-tools")]
pub fn unregister_tool_at_path(
    registry: &std::sync::Arc<std::sync::RwLock<ToolRegistry>>,
    path: &VfsPath,
) {
    let mut reg = registry.write().unwrap();
    if let Some(name) = reg.find_by_vfs_path(path).map(|t| t.name.clone()) {
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
        let mut registry = ToolRegistry::new();

        // Write a .scm tool to /tools/shared/
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();
        vfs.write(VfsCaller::System, &path, SCAN_TOOL.as_bytes())
            .await
            .unwrap();

        scan_and_register(&vfs, &mut registry).await.unwrap();

        assert!(
            registry.get("scan_hello").is_some(),
            "scan_hello should be registered after scan"
        );
    }

    #[tokio::test]
    async fn test_scan_and_register_ignores_non_scm() {
        let (_dir, vfs) = make_test_vfs();
        let mut registry = ToolRegistry::new();

        let path = VfsPath::new("/tools/shared/readme.txt").unwrap();
        vfs.write(VfsCaller::System, &path, b"not a tool")
            .await
            .unwrap();

        scan_and_register(&vfs, &mut registry).await.unwrap();
        assert_eq!(
            registry.all().count(),
            0,
            "non-.scm file should not register"
        );
    }

    #[tokio::test]
    async fn test_scan_and_register_skips_missing_zone() {
        let (_dir, vfs) = make_test_vfs();
        let mut registry = ToolRegistry::new();

        // /tools/shared does not exist — should not error
        let result = scan_and_register(&vfs, &mut registry).await;
        assert!(result.is_ok());
        assert_eq!(registry.all().count(), 0);
    }

    #[tokio::test]
    async fn test_scan_and_register_logs_bad_source() {
        let (_dir, vfs) = make_test_vfs();
        let mut registry = ToolRegistry::new();

        // Write an invalid .scm file
        let path = VfsPath::new("/tools/shared/bad_tool.scm").unwrap();
        vfs.write(VfsCaller::System, &path, b"(define tool-name \"bad\")")
            .await
            .unwrap();

        // Should complete without error (bad file is warned and skipped)
        let result = scan_and_register(&vfs, &mut registry).await;
        assert!(result.is_ok());
        assert_eq!(
            registry.all().count(),
            0,
            "invalid tool should not register"
        );
    }

    // --- hot-reload tests ---

    #[test]
    fn test_reload_tool_from_content_registers() {
        use std::sync::{Arc, RwLock};
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes());

        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "tool should be registered after reload"
        );
    }

    #[test]
    fn test_reload_tool_from_content_updates_existing() {
        use std::sync::{Arc, RwLock};
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register first version
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes());
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
        reload_tool_from_content(&registry, &path, updated.as_bytes());
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
        use std::sync::{Arc, RwLock};
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register first
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes());
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
        use std::sync::{Arc, RwLock};
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/ghost.scm").unwrap();
        // Should not panic or error
        unregister_tool_at_path(&registry, &path);
        assert_eq!(registry.read().unwrap().all().count(), 0);
    }

    #[test]
    fn test_reload_invalid_source_preserves_existing() {
        use std::sync::{Arc, RwLock};
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/scan_hello.scm").unwrap();

        // Register valid version first
        reload_tool_from_content(&registry, &path, SCAN_TOOL.as_bytes());
        assert!(registry.read().unwrap().get("scan_hello").is_some());

        // Try to reload with invalid source — should not remove the existing tool
        reload_tool_from_content(&registry, &path, b"(define tool-name \"bad\")");
        assert!(
            registry.read().unwrap().get("scan_hello").is_some(),
            "invalid reload should not remove existing valid tool"
        );
    }

    #[tokio::test]
    async fn test_vfs_write_triggers_hot_reload() {
        use crate::vfs::ScmChangeKind;
        use std::sync::{Arc, RwLock};

        let (_dir, mut vfs) = make_test_vfs();
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));

        let reg = Arc::clone(&registry);
        vfs.set_scm_change_callback(Arc::new(move |path, kind, content| match kind {
            ScmChangeKind::Write => {
                if let Some(bytes) = content {
                    reload_tool_from_content(&reg, path, bytes);
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
        use std::sync::Arc;

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
        let path = VfsPath::new("/tools/shared/word_count.scm").unwrap();
        let tool = load_tool_from_source(SIMPLE_TOOL, &path).unwrap();
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
        let path = VfsPath::new("/tools/shared/word_count.scm").unwrap();
        let tool = load_tool_from_source(SIMPLE_TOOL, &path).unwrap();
        let args = serde_json::json!({"text": "hello world foo"});

        if let ToolImpl::Synthesised { ref context, .. } = tool.r#impl {
            // Call json_args_to_scheme_alist + execute directly without a
            // ToolCallContext (this tool's tool-execute only uses args).
            let args_alist = json_args_to_scheme_alist(&args).unwrap();
            let exec_fn = context.evaluate("tool-execute").unwrap();
            let result = context.call(&exec_fn, &[args_alist]).unwrap();
            assert_eq!(result.as_string(), Some("3"));
        } else {
            panic!("expected Synthesised impl");
        }
    }

    #[test]
    fn test_load_tool_missing_bindings() {
        let path = VfsPath::new("/tools/shared/bad.scm").unwrap();
        let bad = "(define tool-name \"oops\")"; // missing other bindings
        let result = load_tool_from_source(bad, &path);
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
}
