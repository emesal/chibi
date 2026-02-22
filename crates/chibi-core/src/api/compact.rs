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
use crate::state::AppState;
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
/// The LLM decides which messages to drop based on goals/todos, with fallback to percentage
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

    // Execute pre_rolling_compact hook
    let tools = tools::load_tools(&app.plugins_dir)?;
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "non_system_count": non_system_messages.len(),
        "summary": context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PreRollingCompact, &hook_data);

    // Load goals and todos to guide compaction decisions
    let goals = app.load_goals(context_name)?;
    let todos = app.load_todos(context_name)?;

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
                    format!("{}... [truncated]", &content[..500])
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
    // {GOALS} and {TODOS} expand to a labelled section or empty string — the
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
                format!("CURRENT GOALS:\n{}\n\n", goals)
            },
        )
        .replace(
            "{TODOS}",
            &if todos.is_empty() {
                String::new()
            } else {
                format!("CURRENT TODOS:\n{}\n\n", todos)
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
    // {GOALS} and {TODOS} expand to a labelled section or empty string — the
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
                format!("\nCURRENT GOALS:\n{}\n", goals)
            },
        )
        .replace(
            "{TODOS}",
            &if todos.is_empty() {
                String::new()
            } else {
                format!("\nCURRENT TODOS:\n{}\n", todos)
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

    // Execute post_rolling_compact hook
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "messages_archived": archived_count,
        "summary": context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PostRollingCompact, &hook_data);

    Ok(())
}

/// Full compaction: summarizes all messages and starts fresh (auto-triggered)
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

    if message_count == 0 || message_count <= 2 {
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

    // Execute pre_compact hook
    let tools = tools::load_tools(&app.plugins_dir)?;
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "summary": context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PreCompact, &hook_data);

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

    // Execute post_compact hook
    let hook_data = serde_json::json!({
        "context_name": new_context.name,
        "message_count": new_context.messages.len(),
        "summary": new_context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PostCompact, &hook_data);

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
