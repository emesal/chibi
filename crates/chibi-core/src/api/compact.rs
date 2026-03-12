//! Context compaction functions.
//!
//! This module provides different compaction strategies:
//! - Rolling compaction: strips messages and integrates them into the summary
//! - Full compaction: summarizes all messages and starts fresh
//! - By-name compaction: compact a specific context without LLM

use crate::config::ResolvedConfig;
use crate::context::{Context, now_timestamp};
use crate::gateway;
use crate::output::{CommandEvent, OutputSink};
use crate::state::{AppState, format_flock_sections, load_flock_contexts};
use crate::tools;
use serde_json::json;
use std::io;

const ROLLING_COMPACT_DECISION_TEMPLATE: &str =
    include_str!("../../prompts/rolling-compact-decision.md");
const ROLLING_COMPACT_UPDATE_TEMPLATE: &str =
    include_str!("../../prompts/rolling-compact-update.md");

/// Calculate how many messages to drop during rolling compaction.
///
/// Formula: `(count * percentage / 100).round().max(1)`
pub(crate) fn drop_count(message_count: usize, drop_percentage: f32) -> usize {
    ((message_count as f32 * drop_percentage / 100.0).round() as usize).max(1)
}

/// Collect `tool_call_id`s from messages that have `tool_calls` arrays.
///
/// Used to atomically drop tool results when their assistant message is dropped.
pub(crate) fn collect_tool_call_ids(
    messages: &[serde_json::Value],
) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for m in messages {
        if let Some(tool_calls) = m["tool_calls"].as_array() {
            for tc in tool_calls {
                if let Some(id) = tc["id"].as_str() {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    ids
}

/// Filter messages for compaction: keep system messages unconditionally,
/// drop messages whose `_id` is in `drop_ids`, and drop tool results whose
/// `tool_call_id` matches a dropped assistant message's tool call.
pub(crate) fn filter_messages(
    messages: &[serde_json::Value],
    drop_ids: &std::collections::HashSet<String>,
    drop_tool_call_ids: &std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|m| {
            let role = m["role"].as_str().unwrap_or("");
            if role == "system" {
                return true;
            }
            let id = m["_id"].as_str().unwrap_or("");
            if drop_ids.contains(id) {
                return false;
            }
            // Atomic tool exchange: drop tool results whose call was dropped
            if role == "tool"
                && let Some(tc_id) = m["tool_call_id"].as_str()
                && drop_tool_call_ids.contains(tc_id)
            {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// Rolling compaction: strips messages and integrates them into the summary
/// This is triggered automatically when context exceeds threshold
/// The LLM decides which messages to drop based on goals/tasks, with fallback to percentage
pub async fn rolling_compact(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn OutputSink,
) -> io::Result<()> {
    let mut context = app.get_or_create_context(context_name)?;

    // Skip system messages when counting
    let non_system_messages: Vec<&serde_json::Value> = context
        .messages
        .iter()
        .filter(|m| m["role"].as_str() != Some("system"))
        .collect();

    if non_system_messages.len() <= 4 {
        // Not enough messages to strip - too few to meaningfully compact
        return Ok(());
    }

    // Execute pre_rolling_compact hook.
    // TeinHookContext is None: compact runs from a background task that does not hold a
    // full ToolCallContext (no per-request VFS caller or registry snapshot). Tein hook
    // callbacks fire but cannot use call-tool or (harness io). Known limitation — promote
    // to Some if a full async context is threaded through compact in the future.
    let tools = tools::load_tools(&app.plugins_dir)?;
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "non_system_count": non_system_messages.len(),
        "summary": context.summary,
    });
    let _ = tools::execute_hook(
        &tools,
        tools::HookPoint::PreRollingCompact,
        &hook_data,
        None,
    );

    // Load tasks and flock goals to guide compaction decisions.
    let task_metas = crate::state::tasks::collect_tasks(&app.vfs, context_name).await;
    let task_table = crate::state::tasks::build_summary_table(&task_metas);
    let flock_contexts = load_flock_contexts(&app.vfs, context_name).unwrap_or_default();
    let goals = format_flock_sections(&flock_contexts);

    // Build message list in transcript format for LLM to analyze.
    // For tool messages, include a summary representation.
    let messages_for_llm: Vec<serde_json::Value> = non_system_messages
        .iter()
        .map(|m| {
            let id = m["_id"].as_str().unwrap_or("");
            let role = m["role"].as_str().unwrap_or("");
            let content_repr = if let Some(tool_calls) = m["tool_calls"].as_array() {
                // Assistant message with tool calls
                let names: Vec<&str> = tool_calls
                    .iter()
                    .filter_map(|tc| tc["function"]["name"].as_str())
                    .collect();
                format!("[tool calls: {}]", names.join(", "))
            } else {
                let content = m["content"].as_str().unwrap_or("");
                if content.len() > 500 {
                    let end = content.floor_char_boundary(500);
                    format!("{}... [truncated]", &content[..end])
                } else {
                    content.to_string()
                }
            };
            serde_json::json!({
                "id": id,
                "role": role,
                "content": content_repr,
            })
        })
        .collect();

    // Calculate target drop count based on config percentage
    let drop_percentage = resolved_config.rolling_compact_drop_percentage;
    let target_drop_count = drop_count(non_system_messages.len(), drop_percentage);

    // Ask LLM which messages to drop.
    // {GOALS} and {TASKS} expand to a labelled section or empty string — the
    // adjacent placeholders in the template collapse cleanly when both are absent.
    let decision_prompt = ROLLING_COMPACT_DECISION_TEMPLATE
        .replace(
            "{MESSAGES}",
            &serde_json::to_string_pretty(&messages_for_llm).unwrap_or_default(),
        )
        .replace(
            "{GOALS}",
            &if goals.is_empty() {
                String::new()
            } else {
                format!("{}\n\n", goals)
            },
        )
        .replace(
            "{TASKS}",
            &if task_table.is_empty() {
                String::new()
            } else {
                format!("CURRENT TASKS:\n{}\n\n", task_table)
            },
        )
        .replace(
            "{SUMMARY}",
            if context.summary.is_empty() {
                "(No existing summary)"
            } else {
                &context.summary
            },
        )
        .replace("{TARGET_COUNT}", &target_drop_count.to_string());

    // First LLM call: decide what to drop
    let decision_messages = vec![json!({
        "role": "user",
        "content": decision_prompt,
    })];

    let ids_to_drop: Vec<String> = match gateway::chat(resolved_config, &decision_messages).await {
        Ok(content) => {
            // Try to parse as JSON array
            serde_json::from_str(&content).unwrap_or_else(|_| {
                // Try to extract JSON array from response if wrapped in other text
                if let Some(start) = content.find('[') {
                    if let Some(end) = content.rfind(']') {
                        serde_json::from_str(&content[start..=end]).unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            })
        }
        Err(_) => Vec::new(),
    };

    // Determine which messages to actually drop.
    // Tool exchanges (assistant with tool_calls + subsequent tool results) are atomic:
    // if the assistant message is dropped, drop associated tool results too.
    let messages_to_drop: Vec<&serde_json::Value> = if ids_to_drop.is_empty() {
        // Fallback: drop oldest N messages based on percentage
        sink.emit_event(CommandEvent::RollingCompactionFallback {
            drop_percentage: drop_percentage as f64,
        });
        non_system_messages
            .iter()
            .take(target_drop_count)
            .copied()
            .collect()
    } else {
        sink.emit_event(CommandEvent::RollingCompactionDecision {
            archived: ids_to_drop.len(),
        });
        non_system_messages
            .iter()
            .filter(|m| {
                let id = m["_id"].as_str().unwrap_or("");
                ids_to_drop.iter().any(|drop_id| drop_id == id)
            })
            .copied()
            .collect()
    };

    if messages_to_drop.is_empty() {
        return Ok(());
    }

    // Build text of messages to summarize
    let mut stripped_text = String::new();
    for m in &messages_to_drop {
        let role = m["role"].as_str().unwrap_or("unknown").to_uppercase();
        if let Some(tool_calls) = m["tool_calls"].as_array() {
            let names: Vec<&str> = tool_calls
                .iter()
                .filter_map(|tc| tc["function"]["name"].as_str())
                .collect();
            stripped_text.push_str(&format!(
                "[{}]: [called tools: {}]\n\n",
                role,
                names.join(", ")
            ));
        } else {
            let content = m["content"].as_str().unwrap_or("");
            stripped_text.push_str(&format!("[{}]: {}\n\n", role, content));
        }
    }

    // Collect IDs of messages to drop for filtering
    let drop_ids: std::collections::HashSet<String> = messages_to_drop
        .iter()
        .filter_map(|m| m["_id"].as_str().map(|s| s.to_string()))
        .collect();

    // Atomically drop tool results whose assistant message was dropped
    let dropped_msgs: Vec<serde_json::Value> =
        messages_to_drop.iter().map(|m| (*m).clone()).collect();
    let drop_tool_call_ids = collect_tool_call_ids(&dropped_msgs);

    // Second LLM call: update summary with dropped content.
    // {GOALS} and {TASKS} expand to a labelled section or empty string — the
    // adjacent placeholders in the template collapse cleanly when both are absent.
    let update_prompt = ROLLING_COMPACT_UPDATE_TEMPLATE
        .replace(
            "{SUMMARY}",
            if context.summary.is_empty() {
                "(No existing summary)"
            } else {
                &context.summary
            },
        )
        .replace("{ARCHIVED}", &stripped_text)
        .replace(
            "{GOALS}",
            &if goals.is_empty() {
                String::new()
            } else {
                format!("\n{}\n", goals)
            },
        )
        .replace(
            "{TASKS}",
            &if task_table.is_empty() {
                String::new()
            } else {
                format!("\nCURRENT TASKS:\n{}\n", task_table)
            },
        );

    let summary_messages = vec![json!({
        "role": "user",
        "content": update_prompt,
    })];

    let new_summary = gateway::chat(resolved_config, &summary_messages).await?;

    if new_summary.is_empty() {
        return Ok(());
    }

    // Capture count before we drop the borrow
    let archived_count = messages_to_drop.len();

    // Drop the borrow by ending use of messages_to_drop
    drop(messages_to_drop);

    // Filter out dropped messages (system messages always preserved)
    let remaining_messages = filter_messages(&context.messages, &drop_ids, &drop_tool_call_ids);

    // Update context with new summary and remaining messages
    context.summary = new_summary.clone();
    context.messages = remaining_messages;
    context.updated_at = now_timestamp();

    // Finalize compaction: write anchor to transcript and mark dirty
    app.finalize_compaction(&context.name, &new_summary)?;

    // Save updated context (summary.md, context_meta.json, etc.)
    app.save_context(&context)?;

    sink.emit_event(CommandEvent::RollingCompactionComplete {
        archived: archived_count,
        remaining: context.messages.len(),
    });

    // Execute post_rolling_compact hook. TeinHookContext: None — see PreRollingCompact above.
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "messages_archived": archived_count,
        "summary": context.summary,
    });
    let _ = tools::execute_hook(
        &tools,
        tools::HookPoint::PostRollingCompact,
        &hook_data,
        None,
    );

    Ok(())
}

/// Auto-triggered compaction: delegates to rolling compaction.
///
/// Rolling compaction summarises older messages while preserving recent ones,
/// keeping the context window within budget without discarding everything.
/// For full compaction (manual `-c` flag), see `compact_context_with_llm_manual`.
pub async fn compact_context_with_llm(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn OutputSink,
) -> io::Result<()> {
    // Use rolling compaction for auto-triggered compaction
    rolling_compact(app, context_name, resolved_config, sink).await
}

/// Full compaction: summarizes all messages and starts fresh (manual -c flag)
pub async fn compact_context_with_llm_manual(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn OutputSink,
) -> io::Result<()> {
    compact_context_with_llm_internal(app, context_name, resolved_config, sink).await
}

/// Compact a specific context by name (for -Z flag)
pub async fn compact_context_by_name(
    app: &AppState,
    context_name: &str,
    sink: &dyn OutputSink,
) -> io::Result<()> {
    // Load the context
    let context = app.load_context(context_name)?;
    let message_count = context.messages.len();

    if message_count <= 2 {
        return Ok(());
    }

    // Finalize compaction: write anchor to transcript and mark dirty
    let simple_summary = format!(
        "Context compacted. {} messages archived to transcript.",
        message_count
    );
    app.finalize_compaction(context_name, &simple_summary)?;

    // Create fresh context preserving summary
    let new_context = Context {
        name: context_name.to_string(),
        messages: Vec::new(),
        created_at: context.created_at,
        updated_at: now_timestamp(),
        summary: context.summary,
    };

    app.save_context(&new_context)?;

    sink.emit_event(CommandEvent::CompactionComplete {
        context: context_name.to_string(),
        archived: message_count,
        remaining: 0,
    });

    Ok(())
}

async fn compact_context_with_llm_internal(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn OutputSink,
) -> io::Result<()> {
    let context = app.get_or_create_context(context_name)?;

    if context.messages.is_empty() || context.messages.len() <= 2 {
        return Ok(());
    }

    // Execute pre_compact hook. TeinHookContext: None — compact_context_with_llm_internal
    // does not carry a full ToolCallContext; tein callbacks fire but cannot use call-tool
    // or (harness io). Known limitation — see PreRollingCompact in rolling_compact.
    let tools = tools::load_tools(&app.plugins_dir)?;
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "summary": context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PreCompact, &hook_data, None);

    sink.emit_event(CommandEvent::CompactionStarted {
        context: context_name.to_string(),
        message_count: context.messages.len(),
    });

    // Load compaction prompt (falls back to compiled-in default)
    let compaction_prompt = app.load_prompt("compaction")?;

    // Build conversation text for summarization, including tool interactions
    let mut conversation_text = String::new();
    for m in &context.messages {
        let role = m["role"].as_str().unwrap_or("unknown");
        if role == "system" {
            continue;
        }
        if let Some(tool_calls) = m["tool_calls"].as_array() {
            let names: Vec<&str> = tool_calls
                .iter()
                .filter_map(|tc| tc["function"]["name"].as_str())
                .collect();
            conversation_text.push_str(&format!(
                "[ASSISTANT]: [called tools: {}]\n\n",
                names.join(", ")
            ));
        } else if role == "tool" {
            let content = m["content"].as_str().unwrap_or("");
            let preview = if content.len() > 200 {
                let end = content
                    .char_indices()
                    .nth(200)
                    .map(|(i, _)| i)
                    .unwrap_or(content.len());
                format!("{}... [truncated]", &content[..end])
            } else {
                content.to_string()
            };
            conversation_text.push_str(&format!("[TOOL RESULT]: {}\n\n", preview));
        } else {
            let content = m["content"].as_str().unwrap_or("");
            conversation_text.push_str(&format!("[{}]: {}\n\n", role.to_uppercase(), content));
        }
    }

    // Prepare messages for compaction request
    let compaction_messages = vec![
        json!({
            "role": "system",
            "content": compaction_prompt,
        }),
        json!({
            "role": "user",
            "content": format!("Please summarize this conversation:\n\n{}", conversation_text),
        }),
    ];

    let summary = gateway::chat(resolved_config, &compaction_messages).await?;

    if summary.is_empty() {
        return Err(io::Error::other(
            "Empty summary received from LLM. This can happen with free-tier models. Try again or use a different model.",
        ));
    }

    // Prepare continuation prompt (falls back to compiled-in default)
    let continuation_prompt = app.load_prompt("continuation")?;

    // Load system prompt
    let system_prompt = app.load_system_prompt_for(context_name)?;

    // Create new context with system prompt, continuation instructions, and summary
    let mut new_context = Context {
        name: context.name.clone(),
        messages: Vec::new(),
        created_at: context.created_at,
        updated_at: now_timestamp(),
        summary: summary.clone(),
    };

    // Add system prompt as first message
    if !system_prompt.is_empty() {
        new_context.messages.push(json!({
            "_id": uuid::Uuid::new_v4().to_string(),
            "role": "system",
            "content": system_prompt,
        }));
    }

    // Add continuation prompt + summary as user message
    new_context.messages.push(json!({
        "_id": uuid::Uuid::new_v4().to_string(),
        "role": "user",
        "content": format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
    }));

    // Finalize compaction: write anchor to transcript and mark dirty
    app.finalize_compaction(&new_context.name, &summary)?;

    // Save the new context
    app.save_context(&new_context)?;

    sink.emit_event(CommandEvent::CompactionComplete {
        context: context_name.to_string(),
        archived: context.messages.len(),
        remaining: new_context.messages.len(),
    });

    // Execute post_compact hook. TeinHookContext: None — see PreCompact above.
    let hook_data = serde_json::json!({
        "context_name": new_context.name,
        "message_count": new_context.messages.len(),
        "summary": new_context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PostCompact, &hook_data, None);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiParams, Config, ToolsConfig, VfsConfig};
    use crate::output::NoopSink;
    use crate::partition::StorageConfig;
    use serde_json::json;
    use std::collections::HashSet;
    use tempfile::TempDir;

    // === Helper constructors ===

    /// Build a minimal message with role and _id.
    fn msg(role: &str, id: &str) -> serde_json::Value {
        json!({ "role": role, "_id": id, "content": format!("msg-{id}") })
    }

    /// Build a system message (no _id needed for filtering logic).
    fn system_msg(id: &str) -> serde_json::Value {
        json!({ "role": "system", "_id": id, "content": "system prompt" })
    }

    /// Build an assistant message with tool_calls.
    fn assistant_with_tools(id: &str, tool_call_ids: &[&str]) -> serde_json::Value {
        let calls: Vec<serde_json::Value> = tool_call_ids
            .iter()
            .map(|tc_id| {
                json!({
                    "id": tc_id,
                    "function": { "name": format!("tool_{tc_id}") }
                })
            })
            .collect();
        json!({ "role": "assistant", "_id": id, "tool_calls": calls })
    }

    /// Build a tool result message.
    fn tool_result(id: &str, tool_call_id: &str) -> serde_json::Value {
        json!({
            "role": "tool",
            "_id": id,
            "tool_call_id": tool_call_id,
            "content": format!("result for {tool_call_id}")
        })
    }

    /// Create a test AppState with a temporary directory.
    fn create_test_app() -> (AppState, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: Some("test-key".to_string()),
            model: Some("test-model".to_string()),
            context_window_limit: Some(8000),
            warn_threshold_percent: 75.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            reflection_enabled: false,
            reflection_character_limit: 10000,
            fuel: 0,
            fuel_empty_response_cost: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
            fallback_tool: "call_user".to_string(),
            tools: ToolsConfig::default(),
            vfs: VfsConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
            models: Default::default(),
            site: None,
        };
        let app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();
        (app, temp_dir)
    }

    // === drop_count tests ===

    #[test]
    fn drop_count_basic_percentage() {
        assert_eq!(drop_count(10, 50.0), 5);
    }

    #[test]
    fn drop_count_rounds_correctly() {
        // 7 * 30% = 2.1, rounds to 2
        assert_eq!(drop_count(7, 30.0), 2);
    }

    #[test]
    fn drop_count_floor_is_one() {
        // 1 * 10% = 0.1, rounds to 0, .max(1) → 1
        assert_eq!(drop_count(1, 10.0), 1);
    }

    #[test]
    fn drop_count_hundred_percent() {
        assert_eq!(drop_count(10, 100.0), 10);
    }

    #[test]
    fn drop_count_small_percentage_large_set() {
        // 100 * 3% = 3.0
        assert_eq!(drop_count(100, 3.0), 3);
    }

    #[test]
    fn drop_count_zero_count_gives_one() {
        // 0 * 50% = 0, .max(1) → 1
        assert_eq!(drop_count(0, 50.0), 1);
    }

    // === collect_tool_call_ids tests ===

    #[test]
    fn collect_tool_call_ids_extracts_ids() {
        let messages = vec![
            assistant_with_tools("a1", &["tc-1", "tc-2"]),
            msg("user", "u1"),
            assistant_with_tools("a2", &["tc-3"]),
        ];
        let ids = collect_tool_call_ids(&messages);
        assert_eq!(ids.len(), 3);
        assert!(ids.contains("tc-1"));
        assert!(ids.contains("tc-2"));
        assert!(ids.contains("tc-3"));
    }

    #[test]
    fn collect_tool_call_ids_ignores_no_tool_calls() {
        let messages = vec![msg("user", "u1"), msg("assistant", "a1")];
        let ids = collect_tool_call_ids(&messages);
        assert!(ids.is_empty());
    }

    // === filter_messages tests ===

    #[test]
    fn filter_preserves_system_messages() {
        let messages = vec![system_msg("s1"), msg("user", "u1"), msg("assistant", "a1")];
        let drop_ids: HashSet<String> = ["s1", "u1", "a1"].iter().map(|s| s.to_string()).collect();
        let result = filter_messages(&messages, &drop_ids, &HashSet::new());
        // system message preserved despite its ID being in drop_ids
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "system");
    }

    #[test]
    fn filter_drops_matching_ids() {
        let messages = vec![msg("user", "u1"), msg("assistant", "a1"), msg("user", "u2")];
        let drop_ids: HashSet<String> = ["u1", "a1"].iter().map(|s| s.to_string()).collect();
        let result = filter_messages(&messages, &drop_ids, &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["_id"], "u2");
    }

    #[test]
    fn filter_retains_non_matching() {
        let messages = vec![msg("user", "u1"), msg("user", "u2")];
        let drop_ids: HashSet<String> = ["u1"].iter().map(|s| s.to_string()).collect();
        let result = filter_messages(&messages, &drop_ids, &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["_id"], "u2");
    }

    #[test]
    fn filter_drops_tool_results_for_dropped_calls() {
        let messages = vec![
            assistant_with_tools("a1", &["tc-1"]),
            tool_result("t1", "tc-1"),
            msg("user", "u1"),
        ];
        // Drop the assistant message
        let drop_ids: HashSet<String> = ["a1"].iter().map(|s| s.to_string()).collect();
        // Its tool call IDs should cause the tool result to be dropped too
        let drop_tc_ids: HashSet<String> = ["tc-1"].iter().map(|s| s.to_string()).collect();
        let result = filter_messages(&messages, &drop_ids, &drop_tc_ids);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["_id"], "u1");
    }

    #[test]
    fn filter_retains_tool_results_for_kept_calls() {
        let messages = vec![
            assistant_with_tools("a1", &["tc-1"]),
            tool_result("t1", "tc-1"),
            msg("user", "u1"),
        ];
        // Don't drop the assistant — tool result should also stay
        let result = filter_messages(&messages, &HashSet::new(), &HashSet::new());
        assert_eq!(result.len(), 3);
    }

    // === rolling_compact early return ===

    #[tokio::test]
    async fn rolling_compact_early_return_few_messages() {
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-compact";

        // Create context with ≤4 non-system messages (should early-return)
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("s1"),
            msg("user", "u1"),
            msg("assistant", "a1"),
            msg("user", "u2"),
            msg("assistant", "a2"),
        ];
        app.save_context(&context).unwrap();

        // Build a dummy ResolvedConfig — we never reach the LLM call
        let resolved = dummy_resolved_config();

        // Should return Ok without calling LLM (4 non-system messages, threshold is ≤4)
        let result = rolling_compact(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok());

        // Context should be unchanged (system messages are not persisted via save_context)
        let after = app.get_or_create_context(ctx_name).unwrap();
        assert_eq!(after.messages.len(), 4);
    }

    // === compact_context_by_name tests ===

    #[test]
    fn compact_by_name_noop_for_few_messages() {
        let (app, _tmp) = create_test_app();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let ctx_name = "test-compact-name";
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![msg("user", "u1"), msg("assistant", "a1")];
        app.save_context(&context).unwrap();

        rt.block_on(async {
            let result = compact_context_by_name(&app, ctx_name, &NoopSink).await;
            assert!(result.is_ok());
        });

        // ≤2 messages → context unchanged
        let after = app.load_context(ctx_name).unwrap();
        assert_eq!(after.messages.len(), 2);
    }

    #[test]
    fn compact_by_name_clears_messages_preserves_summary() {
        let (app, _tmp) = create_test_app();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let ctx_name = "test-compact-clear";
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.summary = "existing summary".to_string();
        context.messages = vec![msg("user", "u1"), msg("assistant", "a1"), msg("user", "u2")];
        app.save_context(&context).unwrap();

        rt.block_on(async {
            let result = compact_context_by_name(&app, ctx_name, &NoopSink).await;
            assert!(result.is_ok());
        });

        let after = app.load_context(ctx_name).unwrap();
        assert!(after.messages.is_empty());
        assert_eq!(after.summary, "existing summary");
    }

    // === rolling_compact LLM path tests ===

    /// Spin up a minimal OpenAI-compatible stub server that always returns the
    /// given `response_body` string as the assistant message content.
    ///
    /// Returns the base URL (e.g. `"http://127.0.0.1:PORT"`) and a join handle.
    /// The server shuts down automatically when the handle is dropped.
    async fn spawn_stub_server(response_body: String) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{}", port);

        let body = serde_json::json!({
            "id": "stub-1",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "stub",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": response_body },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string();

        let handle = tokio::spawn(async move {
            for _ in 0..10 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    // Drain the full HTTP request (read until \r\n\r\n)
                    let mut buf = vec![0u8; 16384];
                    let mut total = 0;
                    loop {
                        match stream.read(&mut buf[total..]).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                total += n;
                                if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    // Connection: close forces reqwest to open a fresh connection per request
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        (url, handle)
    }

    /// ResolvedConfig that routes LLM calls to the given stub base URL.
    fn stub_resolved_config(stub_base_url: &str) -> ResolvedConfig {
        let mut cfg = dummy_resolved_config();
        cfg.extra
            .insert("stub_base_url".to_string(), stub_base_url.to_string());
        cfg
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rolling_compact_with_stub_llm_reduces_message_count() {
        // Note: save_context regenerates _ids via messages_to_entries, so the
        // stub's returned ids won't match the stored context — the fallback path
        // activates, dropping the oldest N (50% of 6 = 3) messages. What we're
        // testing here is that the full rolling_compact pipeline (LLM call →
        // save → reload) completes successfully and actually reduces messages.
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-llm-path";

        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("s1"),
            msg("user", "u1"),
            msg("assistant", "a1"),
            msg("user", "u2"),
            msg("assistant", "a2"),
            msg("user", "u3"),
            msg("assistant", "a3"),
        ];
        app.save_context(&context).unwrap();

        // Stub returns a non-empty string for the summary update call so save proceeds.
        let (url, _handle) = spawn_stub_server("updated summary".to_string()).await;
        let resolved = stub_resolved_config(&url);

        let result = rolling_compact(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok(), "rolling_compact failed: {:?}", result);

        let after = app.get_or_create_context(ctx_name).unwrap();
        let non_system_count = after
            .messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .count();
        assert!(
            non_system_count < 6,
            "expected messages to be reduced, got {} (unchanged)",
            non_system_count
        );
        assert_eq!(
            after.summary, "updated summary",
            "summary should be updated"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rolling_compact_fallback_drops_oldest_n_on_empty_llm_response() {
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-fallback-path";

        // 6 non-system messages
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("s1"),
            msg("user", "u1"),
            msg("assistant", "a1"),
            msg("user", "u2"),
            msg("assistant", "a2"),
            msg("user", "u3"),
            msg("assistant", "a3"),
        ];
        app.save_context(&context).unwrap();

        // Stub returns empty string → ids_to_drop parses as empty → fallback activates.
        // The summary update call also gets empty → early return before save.
        let (url, _handle) = spawn_stub_server(String::new()).await;
        let resolved = stub_resolved_config(&url);

        let result = rolling_compact(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok(), "rolling_compact failed: {:?}", result);

        // Empty summary → early return before save, so messages are unchanged
        let after = app.get_or_create_context(ctx_name).unwrap();
        assert_eq!(
            after.messages.len(),
            6,
            "expected 6 messages (unchanged due to empty summary), got {}",
            after.messages.len()
        );
    }

    /// Integration test: full rolling_compact pipeline with a real LLM.
    ///
    /// Gated on CHIBI_API_KEY. Uses ratatoskr:free/summariser (free OpenRouter preset).
    /// Either the LLM decision path or the fallback path may activate depending on
    /// whether the model returns valid IDs — both are correct behaviours.
    ///
    /// Skip with: unset CHIBI_API_KEY
    /// Run with:  CHIBI_API_KEY=<key> cargo test rolling_compact_real_llm -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn rolling_compact_real_llm_reduces_message_count_and_writes_anchor() {
        let Ok(api_key) = std::env::var("CHIBI_API_KEY") else {
            eprintln!("CHIBI_API_KEY not set — skipping rolling compaction integration test");
            return;
        };

        let (app, _tmp) = create_test_app();
        let ctx_name = "integration-rolling";

        // Build a 12-message context: system + user/assistant pairs + one tool exchange.
        // _ids are set so the LLM decision path can identify messages to drop.
        // If the LLM returns unrecognised IDs, the fallback (drop oldest N%) activates —
        // both paths are valid and the invariants below hold for both.
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("You are a helpful assistant."),
            msg("user", "Tell me about Rust."),
            msg("assistant", "Rust is a systems language focused on safety."),
            msg("user", "What about async?"),
            msg("assistant", "Async in Rust uses futures and tokio."),
            msg("user", "How do I write a test?"),
            msg("assistant", "Use #[test] for unit tests."),
            msg("user", "What is compaction?"),
            msg(
                "assistant",
                "Compaction archives old messages into a summary.",
            ),
            assistant_with_tools("m10", &["tc1"]),
            tool_result("m11", "tc1"),
            msg("user", "Thanks, that was helpful!"),
        ];
        app.save_context(&context).unwrap();

        // Real ResolvedConfig pointing at free/summariser via OpenRouter.
        // free/agentic is a valid fallback if summariser is unavailable.
        let mut resolved = dummy_resolved_config();
        resolved.api_key = Some(api_key);
        resolved.model = "ratatoskr:free/summariser".to_string();
        resolved.rolling_compact_drop_percentage = 50.0;

        let result = rolling_compact(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok(), "rolling_compact failed: {:?}", result);

        let after = app.get_or_create_context(ctx_name).unwrap();

        // 1. Message count decreased
        let after_non_system: Vec<_> = after
            .messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .collect();
        assert!(
            after_non_system.len() < 11,
            "expected fewer than 11 non-system messages after compaction, got {}",
            after_non_system.len()
        );

        // 2. Summary is non-empty
        assert!(
            !after.summary.is_empty(),
            "summary should be non-empty after compaction"
        );

        // 3. System messages preserved (none removed)
        let system_count = after
            .messages
            .iter()
            .filter(|m| m["role"].as_str() == Some("system"))
            .count();
        let original_system_count = 1;
        assert_eq!(
            system_count, original_system_count,
            "system messages should be preserved"
        );

        // 4. No orphaned tool results: every tool message must have a corresponding
        //    assistant message with a matching tool_call id still present.
        let remaining_tool_call_ids: std::collections::HashSet<String> = after
            .messages
            .iter()
            .filter_map(|m| m["tool_calls"].as_array())
            .flatten()
            .filter_map(|tc| tc["id"].as_str().map(|s| s.to_string()))
            .collect();
        let orphaned_tool_results: Vec<_> = after
            .messages
            .iter()
            .filter(|m| m["role"].as_str() == Some("tool"))
            .filter(|m| {
                let tc_id = m["tool_call_id"].as_str().unwrap_or("");
                !remaining_tool_call_ids.contains(tc_id)
            })
            .collect();
        assert!(
            orphaned_tool_results.is_empty(),
            "found orphaned tool results: {:?}",
            orphaned_tool_results
        );

        // 5. Transcript anchor was written (finalize_compaction side effect)
        let entries = app.read_transcript_entries(ctx_name).unwrap();
        let has_compaction_anchor = entries
            .iter()
            .any(|e| e.entry_type == crate::context::ENTRY_TYPE_COMPACTION);
        assert!(
            has_compaction_anchor,
            "expected a compaction anchor in the transcript"
        );
    }

    // === manual compaction tests ===

    #[tokio::test]
    async fn manual_compact_early_return_few_messages() {
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-manual-early-return";

        // 2 messages → guard triggers, returns Ok immediately
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![msg("user", "hello"), msg("assistant", "hi")];
        app.save_context(&context).unwrap();

        let resolved = dummy_resolved_config();
        let result = compact_context_with_llm_manual(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(
            result.is_ok(),
            "expected Ok for ≤2 messages, got {:?}",
            result
        );

        // Context must be unchanged
        let after = app.get_or_create_context(ctx_name).unwrap();
        assert_eq!(after.messages.len(), 2, "context should be unchanged");
        assert!(after.summary.is_empty(), "summary should still be empty");
    }

    #[tokio::test]
    async fn manual_compact_empty_summary_returns_error() {
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-manual-empty-summary";

        // 3 messages so the guard doesn't trigger
        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![msg("user", "a"), msg("assistant", "b"), msg("user", "c")];
        app.save_context(&context).unwrap();

        // Stub returns empty string
        let (stub_url, _handle) = spawn_stub_server("".to_string()).await;
        let resolved = stub_resolved_config(&stub_url);
        let result = compact_context_with_llm_manual(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_err(), "expected Err for empty summary");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Empty summary"),
            "expected 'Empty summary' in error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn manual_compact_stub_llm_produces_bootstrap_context() {
        let (app, _tmp) = create_test_app();
        let ctx_name = "test-manual-stub-bootstrap";

        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("You are a helpful assistant."),
            msg("user", "Tell me about Rust."),
            msg("assistant", "Rust is a systems language."),
            msg("user", "What about ownership?"),
            msg("assistant", "Ownership is Rust's memory model."),
        ];
        app.save_context(&context).unwrap();

        let stub_summary = "Rust discussion: safety, ownership, memory model.";
        let (stub_url, _handle) = spawn_stub_server(stub_summary.to_string()).await;
        let resolved = stub_resolved_config(&stub_url);
        let result = compact_context_with_llm_manual(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok(), "compact failed: {:?}", result);

        let after = app.get_or_create_context(ctx_name).unwrap();

        // 1. Summary populated
        assert!(
            !after.summary.is_empty(),
            "summary should be non-empty after compaction"
        );

        // 2. Original messages replaced — system prompts are stored separately in
        //    context_meta, not persisted in the transcript; bootstrap has exactly 1
        //    message: the user message containing the summary block.
        assert_eq!(
            after.messages.len(),
            1,
            "expected exactly 1 bootstrap message (user), got {}",
            after.messages.len()
        );

        // 3. User message contains the summary block
        let user_msg = after
            .messages
            .iter()
            .find(|m| m["role"].as_str() == Some("user"))
            .expect("expected a user message in bootstrap");
        let content = user_msg["content"].as_str().unwrap_or("");
        assert!(
            content.contains("--- SUMMARY ---"),
            "user message should contain '--- SUMMARY ---', got: {content}"
        );

        // 4. Transcript anchor written
        let entries = app.read_transcript_entries(ctx_name).unwrap();
        let has_anchor = entries
            .iter()
            .any(|e| e.entry_type == crate::context::ENTRY_TYPE_COMPACTION);
        assert!(has_anchor, "expected a compaction anchor in the transcript");
    }

    /// Integration test: full manual compact pipeline with a real LLM.
    ///
    /// Gated on CHIBI_API_KEY. Uses ratatoskr:free/text-generation (free OpenRouter preset).
    ///
    /// Skip with: unset CHIBI_API_KEY
    /// Run with:  CHIBI_API_KEY=<key> cargo test manual_compact_real_llm -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn manual_compact_real_llm_produces_bootstrap_context() {
        let Ok(api_key) = std::env::var("CHIBI_API_KEY") else {
            eprintln!("CHIBI_API_KEY not set — skipping manual compaction integration test");
            return;
        };

        let (app, _tmp) = create_test_app();
        let ctx_name = "integration-manual";

        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = vec![
            system_msg("You are a helpful assistant."),
            msg("user", "Tell me about Rust."),
            msg("assistant", "Rust is a systems language focused on safety."),
            msg("user", "What about ownership?"),
            msg("assistant", "Ownership is Rust's memory management model."),
        ];
        app.save_context(&context).unwrap();

        let mut resolved = dummy_resolved_config();
        resolved.api_key = Some(api_key);
        resolved.model = "ratatoskr:free/text-generation".to_string();

        let result = compact_context_with_llm_manual(&app, ctx_name, &resolved, &NoopSink).await;
        assert!(result.is_ok(), "manual compact failed: {:?}", result);

        let after = app.get_or_create_context(ctx_name).unwrap();

        // 1. Summary non-empty
        assert!(
            !after.summary.is_empty(),
            "summary should be non-empty after compaction"
        );

        // 2. Bootstrap has exactly 1 message — the user message with the summary block.
        //    System prompts are stored separately in context_meta and not persisted in
        //    the transcript, so they don't appear when re-loading the context.
        assert_eq!(
            after.messages.len(),
            1,
            "expected exactly 1 bootstrap message (user), got {}",
            after.messages.len()
        );

        // 3. User message contains summary block
        let user_msg = after
            .messages
            .iter()
            .find(|m| m["role"].as_str() == Some("user"))
            .expect("expected a user message in bootstrap");
        let content = user_msg["content"].as_str().unwrap_or("");
        assert!(
            content.contains("--- SUMMARY ---"),
            "user message should contain '--- SUMMARY ---', got: {content}"
        );

        // 4. Transcript anchor written
        let entries = app.read_transcript_entries(ctx_name).unwrap();
        let has_anchor = entries
            .iter()
            .any(|e| e.entry_type == crate::context::ENTRY_TYPE_COMPACTION);
        assert!(has_anchor, "expected a compaction anchor in the transcript");
    }

    // === stress / correctness test ===

    /// Integration stress test: large transcript with repeated rolling compaction.
    ///
    /// Builds a synthetic 90-message transcript (deterministic, seeded from a fixed
    /// message table), then drives `rolling_compact` in a loop until the context
    /// stabilises at ≤4 non-system messages.  Per-round invariants:
    ///
    /// - no orphaned tool results
    /// - system messages preserved throughout
    /// - summary is non-empty after each round
    /// - non-system message count strictly decreases (or has hit the ≤4 floor)
    /// - a compaction anchor is written to the transcript each round
    ///
    /// After full compaction the final context is re-loaded and verified to be
    /// coherent (parseable, no corrupt JSON, loadable via `get_or_create_context`).
    ///
    /// Gated on `CHIBI_API_KEY`.  Free models may be slow; the test allows up to
    /// 20 compaction rounds as a safety cap.
    ///
    /// Skip with: `unset CHIBI_API_KEY`
    /// Run with:  `CHIBI_API_KEY=<key> cargo test large_transcript_stress -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn large_transcript_stress_repeated_rolling_compaction() {
        let Ok(api_key) = std::env::var("CHIBI_API_KEY") else {
            eprintln!("CHIBI_API_KEY not set — skipping large transcript stress test");
            return;
        };

        let (app, _tmp) = create_test_app();
        let ctx_name = "stress-rolling";

        // --- Build a deterministic 90-message synthetic transcript ---
        //
        // Pattern (repeating slice):
        //   user, assistant, user, assistant+tool_call, tool_result, user, assistant
        // This gives a realistic mix of plain exchanges and tool interactions.
        // IDs are stable strings so the LLM decision path can identify messages.
        // If the LLM returns unrecognised IDs the fallback (oldest-N%) activates —
        // both paths are valid and all invariants must hold for both.
        let topics = [
            (
                "What is Rust?",
                "Rust is a systems language focused on safety and performance.",
            ),
            (
                "Explain ownership.",
                "Ownership ensures memory safety without a garbage collector.",
            ),
            (
                "How does borrowing work?",
                "Borrowing allows references without transferring ownership.",
            ),
            (
                "What are lifetimes?",
                "Lifetimes annotate how long references are valid.",
            ),
            (
                "Describe traits.",
                "Traits define shared behaviour across types.",
            ),
            (
                "What is async/await?",
                "Async/await enables non-blocking I/O with a cooperative model.",
            ),
            (
                "Explain enums.",
                "Enums represent a value that can be one of several variants.",
            ),
            (
                "What are closures?",
                "Closures capture their environment and can be stored or passed.",
            ),
            (
                "How do iterators work?",
                "Iterators lazily produce a sequence of values.",
            ),
            (
                "What is pattern matching?",
                "Pattern matching deconstructs values into their components.",
            ),
        ];

        let mut messages: Vec<serde_json::Value> = vec![
            json!({ "role": "system", "_id": "sys-1", "content": "You are a knowledgeable Rust tutor." }),
        ];

        for (i, (question, answer)) in topics.iter().cycle().take(30).enumerate() {
            let base = i * 3;
            let uid = format!("u{}", base);
            let aid = format!("a{}", base);
            messages.push(json!({ "role": "user",      "_id": uid, "content": question }));
            messages.push(json!({ "role": "assistant",  "_id": aid, "content": answer  }));

            // Every third exchange: inject a tool call + result pair
            if i % 3 == 2 {
                let tc_id = format!("tc{}", base);
                let tcm_id = format!("atc{}", base);
                let tr_id = format!("tr{}", base);
                messages.push(json!({
                    "role": "assistant",
                    "_id": tcm_id,
                    "tool_calls": [{
                        "id": tc_id,
                        "function": { "name": "lookup_docs", "arguments": format!("{{\"topic\":\"{}\"}}", i) }
                    }]
                }));
                messages.push(json!({
                    "role": "tool",
                    "_id": tr_id,
                    "tool_call_id": tc_id,
                    "content": format!("Documentation for topic {} retrieved successfully.", i)
                }));
            }
        }

        // Confirm we built a sufficiently large transcript (should be ~100 messages)
        let non_system_initial: usize = messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .count();
        assert!(
            non_system_initial >= 80,
            "expected ≥80 non-system messages to start, got {non_system_initial}"
        );
        eprintln!("stress test: built transcript with {non_system_initial} non-system messages");

        let mut context = app.get_or_create_context(ctx_name).unwrap();
        context.messages = messages;
        app.save_context(&context).unwrap();

        let mut resolved = dummy_resolved_config();
        resolved.api_key = Some(api_key);
        resolved.model = "ratatoskr:free/agentic".to_string();
        resolved.rolling_compact_drop_percentage = 50.0;

        let mut prev_non_system_count = non_system_initial;
        let max_rounds = 20;

        for round in 1..=max_rounds {
            let before = app.get_or_create_context(ctx_name).unwrap();
            let non_system_before: usize = before
                .messages
                .iter()
                .filter(|m| m["role"].as_str() != Some("system"))
                .count();

            if non_system_before <= 4 {
                eprintln!(
                    "stress test: stable at round {round} ({non_system_before} non-system msgs) — done"
                );
                break;
            }

            eprintln!("stress test: round {round} — {non_system_before} non-system messages");

            let result = rolling_compact(&app, ctx_name, &resolved, &NoopSink).await;
            assert!(
                result.is_ok(),
                "round {round}: rolling_compact failed: {:?}",
                result
            );

            let after = app.get_or_create_context(ctx_name).unwrap();

            // --- Per-round invariants ---

            // 1. System messages are fully preserved.
            let system_after = after
                .messages
                .iter()
                .filter(|m| m["role"].as_str() == Some("system"))
                .count();
            assert!(
                system_after >= 1,
                "round {round}: system messages must be preserved (found {system_after})"
            );

            // 2. No orphaned tool results.
            let remaining_tc_ids: HashSet<String> = after
                .messages
                .iter()
                .filter_map(|m| m["tool_calls"].as_array())
                .flatten()
                .filter_map(|tc| tc["id"].as_str().map(|s| s.to_string()))
                .collect();
            let orphaned: Vec<_> = after
                .messages
                .iter()
                .filter(|m| m["role"].as_str() == Some("tool"))
                .filter(|m| {
                    let tc_id = m["tool_call_id"].as_str().unwrap_or("");
                    !remaining_tc_ids.contains(tc_id)
                })
                .collect();
            assert!(
                orphaned.is_empty(),
                "round {round}: found orphaned tool results: {:?}",
                orphaned
            );

            // 3. Summary grows or stays non-empty.
            assert!(
                !after.summary.is_empty(),
                "round {round}: summary must be non-empty after compaction"
            );

            // 4. Non-system message count strictly decreases (or already at floor).
            let non_system_after: usize = after
                .messages
                .iter()
                .filter(|m| m["role"].as_str() != Some("system"))
                .count();
            if non_system_before > 4 {
                assert!(
                    non_system_after < prev_non_system_count,
                    "round {round}: expected message count to decrease from {prev_non_system_count}, got {non_system_after}"
                );
            }
            prev_non_system_count = non_system_after;

            // 5. Compaction anchor written to transcript.
            let entries = app.read_transcript_entries(ctx_name).unwrap();
            let anchor_count = entries
                .iter()
                .filter(|e| e.entry_type == crate::context::ENTRY_TYPE_COMPACTION)
                .count();
            assert!(
                anchor_count >= round,
                "round {round}: expected at least {round} compaction anchor(s) in transcript, found {anchor_count}"
            );

            if non_system_after <= 4 {
                eprintln!(
                    "stress test: floor reached after round {round} ({non_system_after} msgs)"
                );
                break;
            }
        }

        // --- Final coherence check ---
        // Context must be loadable and parseable with no corrupt JSON.
        let final_ctx = app.get_or_create_context(ctx_name).unwrap();
        assert!(
            !final_ctx.summary.is_empty(),
            "final context: summary must be non-empty"
        );
        // Re-serialise and re-parse every message to confirm no corruption.
        for (idx, msg) in final_ctx.messages.iter().enumerate() {
            let serialised = serde_json::to_string(msg).unwrap_or_else(|e| {
                panic!("final context: message {idx} failed to serialise: {e}")
            });
            serde_json::from_str::<serde_json::Value>(&serialised)
                .unwrap_or_else(|e| panic!("final context: message {idx} failed to re-parse: {e}"));
        }
        eprintln!(
            "stress test: final context has {} messages, summary len={}",
            final_ctx.messages.len(),
            final_ctx.summary.len()
        );
    }

    // === helpers ===

    /// Minimal ResolvedConfig for tests that never reach the LLM.
    fn dummy_resolved_config() -> ResolvedConfig {
        ResolvedConfig {
            api_key: None,
            model: "test-model".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            fuel: 0,
            fuel_empty_response_cost: 15,
            username: "testuser".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            tools: ToolsConfig::default(),
            fallback_tool: "call_user".to_string(),
            storage: StorageConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }
}
