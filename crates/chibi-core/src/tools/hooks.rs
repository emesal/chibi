//! Hook system for plugin lifecycle events.
//!
//! Hooks allow plugins to be notified at specific points during chibi's execution,
//! such as before/after messages, tool calls, context switches, and compaction.

use super::Tool;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use strum::{AsRefStr, EnumString};

#[cfg(feature = "synthesised-tools")]
use std::cell::RefCell;
#[cfg(feature = "synthesised-tools")]
use std::collections::HashSet;
#[cfg(feature = "synthesised-tools")]
use std::sync::{Arc, RwLock};

// Tracks which hook points are currently being dispatched to tein callbacks.
// Prevents re-entrancy: if a tein hook callback triggers an action that fires
// the same hook point, tein callbacks are skipped on the recursive call.
// Subprocess hooks still fire normally regardless.
#[cfg(feature = "synthesised-tools")]
thread_local! {
    static TEIN_HOOK_GUARD: RefCell<HashSet<HookPoint>> = RefCell::new(HashSet::new());
}

/// Hook points where tools can register to be called
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum HookPoint {
    PreMessage,
    PostMessage,
    PreTool,
    PostTool,
    PreToolOutput,  // Before tool output is processed (can modify/block output)
    PostToolOutput, // After tool output is processed (observe only)
    PreClear,
    PostClear,
    PreCompact,
    PostCompact,
    PreRollingCompact,
    PostRollingCompact,
    OnStart,
    OnEnd,
    PreSystemPrompt,  // Can inject content before system prompt sections
    PostSystemPrompt, // Can inject content after all system prompt sections
    PreSendMessage,   // Can intercept delivery (return {"delivered": true, "via": "..."})
    PostSendMessage,  // Observe delivery (read-only)
    PreCacheOutput,   // Before caching large tool output (can provide custom summary)
    PostCacheOutput,  // After output is cached (notification only)
    PreApiTools,      // Before tools are sent to API (can filter tools)
    PreApiRequest,    // Before API request is sent (can modify full request body)
    PreAgenticLoop,   // Before entering the tool loop (can override fallback)
    PostToolBatch,    // After processing a batch of tool calls (can override fallback)
    PreFileRead, // Before reading a file outside allowed paths (can approve/deny, fail-safe deny)
    PreFileWrite, // Before file write/patch (can approve/deny/modify operation)
    PreShellExec, // Before shell command execution (can approve/deny, fail-safe deny)
    PreFetchUrl, // Before fetching a sensitive URL (can approve/deny, fail-safe deny)
    PreSpawnAgent, // Before sub-agent call (can intercept/replace with {"response": "..."} or block)
    PostSpawnAgent, // After sub-agent call (observe only)
    PostIndexFile, // After a file is indexed (observe: path, lang, symbol_count, ref_count)
}

/// Context needed to set up `BRIDGE_CALL_CTX` during tein hook dispatch,
/// enabling tein hook callbacks to use `call-tool` and `(harness io)`.
///
/// Pass `Some(...)` from async contexts that have the full app state.
/// Pass `None` from contexts without a tokio runtime (sync lifecycle hooks)
/// or tests — tein callbacks still dispatch but cannot use IO or `call-tool`.
///
/// When the `synthesised-tools` feature is disabled this is an empty struct;
/// the 4th `execute_hook` parameter is always present so call sites compile
/// uniformly with `, None` regardless of feature state.
pub struct TeinHookContext<'a> {
    #[cfg(feature = "synthesised-tools")]
    pub app: &'a crate::state::AppState,
    #[cfg(feature = "synthesised-tools")]
    pub context_name: &'a str,
    #[cfg(feature = "synthesised-tools")]
    pub config: &'a crate::config::ResolvedConfig,
    #[cfg(feature = "synthesised-tools")]
    pub project_root: &'a std::path::Path,
    #[cfg(feature = "synthesised-tools")]
    pub vfs: &'a crate::vfs::Vfs,
    #[cfg(feature = "synthesised-tools")]
    pub registry: Arc<RwLock<super::registry::ToolRegistry>>,
    /// Zero-sized phantom to keep the lifetime parameter valid when feature is off.
    #[cfg(not(feature = "synthesised-tools"))]
    _phantom: std::marker::PhantomData<&'a ()>,
}

/// Execute a hook on all tools that registered for it
/// Returns a vector of (tool_name, result) for tools that returned non-empty output
///
/// Hook data is passed via stdin (JSON). The CHIBI_HOOK env var identifies which hook is firing.
///
/// `tein_ctx` (synthesised-tools feature only): when `Some`, sets `BRIDGE_CALL_CTX` per tein tool
/// during dispatch, enabling `call-tool` and `(harness io)` from tein hook callbacks.
/// Pass `None` from sync contexts or tests that lack a tokio runtime.
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
    _tein_ctx: Option<&TeinHookContext<'_>>,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();
    let data_str = data.to_string();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        // Only plugin tools can register hooks; extract the executable path.
        let plugin_path = match &tool.r#impl {
            super::ToolImpl::Plugin(p) => p.clone(),
            _ => continue, // non-plugin tools cannot spawn hooks
        };
        let mut child = Command::new(&plugin_path)
            .env("CHIBI_HOOK", hook.as_ref())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                io::Error::other(format!(
                    "Failed to spawn hook {} on {}: {}",
                    hook.as_ref(),
                    tool.name,
                    e
                ))
            })?;

        // Write hook data to stdin (ignore BrokenPipe — child may exit before reading)
        if let Some(mut stdin) = child.stdin.take() {
            match stdin.write_all(data_str.as_bytes()) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                Err(e) => return Err(e),
            }
            // stdin is dropped here, closing the pipe and signaling EOF
        }

        let timeout = std::time::Duration::from_secs(super::PLUGIN_TIMEOUT_SECS);
        let context = format!("hook {} on {}", hook.as_ref(), tool.name);
        let output = super::wait_with_timeout(child, timeout, &context).map_err(|e| {
            io::Error::other(format!(
                "Failed to execute hook {} on {}: {}",
                hook.as_ref(),
                tool.name,
                e
            ))
        })?;

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Try to parse as JSON, otherwise wrap as string
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|_| serde_json::Value::String(trimmed.to_string()));

        results.push((tool.name.clone(), value));
    }

    // --- synthesised tein hooks ---
    #[cfg(feature = "synthesised-tools")]
    {
        let should_dispatch = TEIN_HOOK_GUARD.with(|guard| !guard.borrow().contains(&hook));

        if should_dispatch {
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

                let payload = match super::synthesised::json_args_to_scheme_alist(data) {
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
                        eprintln!("[WARN] tein hook {} on {}: {e}", hook.as_ref(), tool.name);
                        continue;
                    }
                };

                // empty list or nil → no-op, don't push a result
                if result.is_nil() {
                    continue;
                }
                if matches!(&result, tein::Value::List(items) if items.is_empty()) {
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

            TEIN_HOOK_GUARD.with(|guard| {
                guard.borrow_mut().remove(&hook);
            });
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // All 31 hook points for testing
    const ALL_HOOKS: &[(&str, HookPoint)] = &[
        ("pre_message", HookPoint::PreMessage),
        ("post_message", HookPoint::PostMessage),
        ("pre_tool", HookPoint::PreTool),
        ("post_tool", HookPoint::PostTool),
        ("pre_tool_output", HookPoint::PreToolOutput),
        ("post_tool_output", HookPoint::PostToolOutput),
        ("pre_clear", HookPoint::PreClear),
        ("post_clear", HookPoint::PostClear),
        ("pre_compact", HookPoint::PreCompact),
        ("post_compact", HookPoint::PostCompact),
        ("pre_rolling_compact", HookPoint::PreRollingCompact),
        ("post_rolling_compact", HookPoint::PostRollingCompact),
        ("on_start", HookPoint::OnStart),
        ("on_end", HookPoint::OnEnd),
        ("pre_system_prompt", HookPoint::PreSystemPrompt),
        ("post_system_prompt", HookPoint::PostSystemPrompt),
        ("pre_send_message", HookPoint::PreSendMessage),
        ("post_send_message", HookPoint::PostSendMessage),
        ("pre_cache_output", HookPoint::PreCacheOutput),
        ("post_cache_output", HookPoint::PostCacheOutput),
        ("pre_api_tools", HookPoint::PreApiTools),
        ("pre_api_request", HookPoint::PreApiRequest),
        ("pre_agentic_loop", HookPoint::PreAgenticLoop),
        ("post_tool_batch", HookPoint::PostToolBatch),
        ("pre_file_read", HookPoint::PreFileRead),
        ("pre_file_write", HookPoint::PreFileWrite),
        ("pre_shell_exec", HookPoint::PreShellExec),
        ("pre_fetch_url", HookPoint::PreFetchUrl),
        ("pre_spawn_agent", HookPoint::PreSpawnAgent),
        ("post_spawn_agent", HookPoint::PostSpawnAgent),
        ("post_index_file", HookPoint::PostIndexFile),
    ];

    #[test]
    fn test_hook_point_from_str_valid() {
        for (s, expected) in ALL_HOOKS {
            let result = s.parse::<HookPoint>();
            assert!(result.is_ok(), "parse failed for '{}'", s);
            assert_eq!(result.unwrap(), *expected);
        }
    }

    #[test]
    fn test_hook_point_from_str_invalid() {
        assert!("".parse::<HookPoint>().is_err());
        assert!("unknown".parse::<HookPoint>().is_err());
        assert!("PreMessage".parse::<HookPoint>().is_err()); // wrong case
        assert!("pre-message".parse::<HookPoint>().is_err()); // wrong separator
    }

    #[test]
    fn test_hook_point_as_str() {
        for (expected_str, hook) in ALL_HOOKS {
            assert_eq!(hook.as_ref(), *expected_str);
        }
    }

    #[test]
    fn test_hook_point_round_trip() {
        for (s, _) in ALL_HOOKS {
            let hook = s.parse::<HookPoint>().unwrap();
            assert_eq!(hook.as_ref(), *s);
        }
    }

    use super::super::ToolMetadata;
    use super::super::test_helpers::create_test_script;

    /// Execute a hook with retry on ETXTBSY (text file busy).
    fn execute_hook_with_retry(
        tools: &[Tool],
        hook: HookPoint,
        data: &serde_json::Value,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        for attempt in 0..5 {
            match execute_hook(tools, hook, data, None) {
                Ok(result) => return Ok(result),
                Err(e) if e.to_string().contains("Text file busy") && attempt < 4 => {
                    std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1) as u64));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    #[test]
    fn test_execute_hook_receives_stdin_data() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            b"#!/bin/bash\ncat\n", // Echo stdin to stdout
        );

        let tools = vec![Tool {
            name: "hook_tool".to_string(),
            description: "Hook tester".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let data = serde_json::json!({"event": "start", "context": "test"});
        let results = execute_hook_with_retry(&tools, HookPoint::OnStart, &data).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "hook_tool");
        assert_eq!(results[0].1["event"], "start");
        assert_eq!(results[0].1["context"], "test");
    }

    #[test]
    fn test_execute_hook_env_var() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook_env.sh",
            b"#!/bin/bash\ncat > /dev/null\necho \"hook=$CHIBI_HOOK\"\n",
        );

        let tools = vec![Tool {
            name: "env_hook".to_string(),
            description: "Env checker".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::PreMessage],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 1);
        // Result is string since it's not valid JSON
        let output = results[0].1.as_str().unwrap();
        assert!(output.contains("hook=pre_message"));
    }

    #[test]
    fn test_execute_hook_no_hook_data_env() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "verify_env.sh",
            br#"#!/bin/bash
cat > /dev/null
if [ -n "$CHIBI_HOOK_DATA" ]; then
  echo 'ERROR: CHIBI_HOOK_DATA should not be set'
  exit 1
fi
echo 'OK'
"#,
        );

        let tools = vec![Tool {
            name: "verify_hook".to_string(),
            description: "Env verifier".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnEnd],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnEnd, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.as_str().unwrap(), "OK");
    }

    #[test]
    fn test_execute_hook_skips_non_registered() {
        let dir = tempfile::tempdir().unwrap();
        let script_path =
            create_test_script(dir.path(), "skip.sh", b"#!/bin/bash\necho 'CALLED'\n");

        let tools = vec![Tool {
            name: "skip_tool".to_string(),
            description: "Should be skipped".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart], // Registered for OnStart only
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        // Call with OnEnd - should not execute the tool
        let results =
            execute_hook_with_retry(&tools, HookPoint::OnEnd, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_execute_hook_skips_failures() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(dir.path(), "fail.sh", b"#!/bin/bash\nexit 1\n");

        let tools = vec![Tool {
            name: "fail_hook".to_string(),
            description: "Always fails".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        // Failed hooks should be skipped (not error)
        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_execute_hook_multiple_tools() {
        let dir = tempfile::tempdir().unwrap();
        let script1 = create_test_script(
            dir.path(),
            "hook1.sh",
            b"#!/bin/bash\ncat > /dev/null\necho 'first'\n",
        );
        let script2 = create_test_script(
            dir.path(),
            "hook2.sh",
            b"#!/bin/bash\ncat > /dev/null\necho 'second'\n",
        );

        let tools = vec![
            Tool {
                name: "first_hook".to_string(),
                description: "First".to_string(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(script1),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "second_hook".to_string(),
                description: "Second".to_string(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(script2),
                category: crate::tools::ToolCategory::Plugin,
            },
        ];

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "first_hook");
        assert_eq!(results[0].1.as_str().unwrap(), "first");
        assert_eq!(results[1].0, "second_hook");
        assert_eq!(results[1].1.as_str().unwrap(), "second");
    }

    #[test]
    #[cfg(unix)]
    fn test_execute_hook_failure_cascade() {
        // Middle hook fails (exit 1) — first and third should still produce results
        let dir = tempfile::tempdir().unwrap();

        let ok1 = create_test_script(dir.path(), "ok1.sh", b"#!/bin/bash\necho '{\"order\": 1}'");
        let fail = create_test_script(dir.path(), "fail.sh", b"#!/bin/bash\nexit 1");
        let ok2 = create_test_script(dir.path(), "ok2.sh", b"#!/bin/bash\necho '{\"order\": 3}'");

        let tools = vec![
            Tool {
                name: "ok1".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(ok1),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "fail".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(fail),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "ok2".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(ok2),
                category: crate::tools::ToolCategory::Plugin,
            },
        ];

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 2, "failed hook should be skipped silently");
        assert_eq!(results[0].0, "ok1");
        assert_eq!(results[0].1["order"], 1);
        assert_eq!(results[1].0, "ok2");
        assert_eq!(results[1].1["order"], 3);
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_execute_hook_dispatches_to_synthesised() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        let data = serde_json::json!({"event": "start"});
        let results = execute_hook(&tools, HookPoint::OnStart, &data, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "tein-hook-test");
        assert_eq!(results[0].1["saw_event"], "start");
    }

    #[test]
    #[cfg(unix)]
    fn test_execute_hook_ordering() {
        // Results must arrive in tool registration order
        let dir = tempfile::tempdir().unwrap();

        let scripts: Vec<_> = (1..=3)
            .map(|i| {
                create_test_script(
                    dir.path(),
                    &format!("hook{i}.sh"),
                    format!("#!/bin/bash\necho '{{\"order\": {i}}}'").as_bytes(),
                )
            })
            .collect();

        let tools: Vec<_> = scripts
            .into_iter()
            .enumerate()
            .map(|(i, path)| Tool {
                name: format!("hook{}", i + 1),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(path),
                category: crate::tools::ToolCategory::Plugin,
            })
            .collect();

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 3);
        for (i, (name, value)) in results.iter().enumerate() {
            assert_eq!(*name, format!("hook{}", i + 1));
            assert_eq!(value["order"], (i + 1) as u64);
        }
    }

    // --- tein (synthesised) hook dispatch tests ---

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_empty_list_return_is_noop() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        let results = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(
            results.len(),
            0,
            "empty list return should be treated as no-op"
        );
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_json_object_return() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        let results = execute_hook(
            &tools,
            HookPoint::PreMessage,
            &serde_json::json!({"prompt": "hello"}),
            None,
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1["prompt"], "modified prompt");
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_skips_unregistered_hook_point() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        // fire on_end — tool is registered for on_start only
        let results = execute_hook(&tools, HookPoint::OnEnd, &serde_json::json!({}), None).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_error_in_callback_skipped() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        // should not error — failed hooks are skipped silently
        let results = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    #[cfg(all(feature = "synthesised-tools", unix))]
    fn test_mixed_plugin_and_tein_hooks() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        // subprocess plugin hook
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

        // tein synthesised hook
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
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();
        tools.insert(0, plugin_tool);

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        // plugin first (subprocess loop), then tein (synthesised loop)
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1["from"], "plugin");
        assert_eq!(results[1].1["from"], "tein");
    }

    // --- re-entrancy guard tests ---

    /// Verify that when a hook point is already in TEIN_HOOK_GUARD (simulating
    /// a recursive call from within a tein hook callback), tein callbacks are
    /// skipped entirely while the guard is held.
    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_reentrancy_guard_skips_tein_callbacks() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "reentrancy-guard-test")
(define tool-description "Should be skipped under guard")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/reentrancy.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        // Simulate re-entrancy: mark on_start as already-in-progress
        TEIN_HOOK_GUARD.with(|guard| {
            guard.borrow_mut().insert(HookPoint::OnStart);
        });

        let results = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();

        // Clean up guard state so other tests in this thread aren't affected
        TEIN_HOOK_GUARD.with(|guard| {
            guard.borrow_mut().remove(&HookPoint::OnStart);
        });

        assert_eq!(
            results.len(),
            0,
            "tein callbacks must be skipped when guard is held (re-entrancy)"
        );
    }

    /// Verify that the guard is cleared after execute_hook completes, so
    /// a subsequent call on the same thread dispatches normally.
    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_reentrancy_guard_cleared_after_dispatch() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source_with_tier;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "guard-cleanup-test")
(define tool-description "Checks guard is cleared post-dispatch")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/guard-cleanup.scm").unwrap();
        let tools = load_tools_from_source_with_tier(
            source,
            &path,
            &registry,
            crate::config::SandboxTier::Sandboxed,
        )
        .unwrap();

        // First call — fires normally
        let r1 = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(r1.len(), 1, "first call should fire normally");

        // Second call on the same thread — guard must be cleared; fires again
        let r2 = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(r2.len(), 1, "guard must be cleared; second call must fire");
    }
}
