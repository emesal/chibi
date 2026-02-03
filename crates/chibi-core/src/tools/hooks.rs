//! Hook system for plugin lifecycle events.
//!
//! Hooks allow plugins to be notified at specific points during chibi's execution,
//! such as before/after messages, tool calls, context switches, and compaction.

use super::Tool;
use std::io;
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
}

/// Execute a hook on all tools that registered for it
/// Returns a vector of (tool_name, result) for tools that returned non-empty output
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        let output = Command::new(&tool.path)
            .env("CHIBI_HOOK", hook.as_ref())
            .env("CHIBI_HOOK_DATA", data.to_string())
            .env_remove("CHIBI_TOOL_ARGS") // Clear tool args to avoid confusion
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
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

    // All 24 hook points for testing
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
}
