//! Hook system for plugin lifecycle events.
//!
//! Hooks allow plugins to be notified at specific points during chibi's execution,
//! such as before/after messages, tool calls, context switches, and compaction.

use super::Tool;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use strum::{AsRefStr, EnumString};

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
    PreFileWrite,     // Before file write/patch (can approve/deny/modify operation)
    PreShellExec,     // Before shell command execution (can approve/deny, fail-safe deny)
    PreSpawnAgent, // Before sub-agent call (can intercept/replace with {"response": "..."} or block)
    PostSpawnAgent, // After sub-agent call (observe only)
    PostIndexFile, // After a file is indexed (observe: path, lang, symbol_count, ref_count)
}

/// Execute a hook on all tools that registered for it
/// Returns a vector of (tool_name, result) for tools that returned non-empty output
///
/// Hook data is passed via stdin (JSON). The CHIBI_HOOK env var identifies which hook is firing.
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();
    let data_str = data.to_string();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        let mut child = Command::new(&tool.path)
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

        let output = child.wait_with_output().map_err(|e| {
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

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // All 26 hook points for testing
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
        ("pre_file_write", HookPoint::PreFileWrite),
        ("pre_shell_exec", HookPoint::PreShellExec),
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
    use std::path::PathBuf;

    /// Helper to create a test script and make it executable.
    #[cfg(unix)]
    fn create_test_script(dir: &std::path::Path, name: &str, content: &[u8]) -> PathBuf {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let script_path = dir.join(name);

        {
            let mut file = std::fs::File::create(&script_path).unwrap();
            file.write_all(content).unwrap();
            file.sync_all().unwrap();
        }

        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        script_path
    }

    /// Execute a hook with retry on ETXTBSY (text file busy).
    fn execute_hook_with_retry(
        tools: &[Tool],
        hook: HookPoint,
        data: &serde_json::Value,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        for attempt in 0..5 {
            match execute_hook(tools, hook, data) {
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
            path: script_path,
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
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
            path: script_path,
            hooks: vec![HookPoint::PreMessage],
            metadata: ToolMetadata::new(),
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
            path: script_path,
            hooks: vec![HookPoint::OnEnd],
            metadata: ToolMetadata::new(),
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
            path: script_path,
            hooks: vec![HookPoint::OnStart], // Registered for OnStart only
            metadata: ToolMetadata::new(),
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
            path: script_path,
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
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
                path: script1,
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
            },
            Tool {
                name: "second_hook".to_string(),
                description: "Second".to_string(),
                parameters: serde_json::json!({}),
                path: script2,
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
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
}
