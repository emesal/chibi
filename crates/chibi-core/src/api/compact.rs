//! Context compaction functions.
//!
//! This module provides different compaction strategies:
//! - Rolling compaction: strips messages and integrates them into the summary
//! - Full compaction: summarizes all messages and starts fresh
//! - By-name compaction: compact a specific context without LLM

use crate::config::ResolvedConfig;
use crate::context::{Context, now_timestamp};
use crate::gateway;
use crate::state::AppState;
use crate::tools;
use serde_json::json;
use std::io;

/// Rolling compaction: strips messages and integrates them into the summary
/// This is triggered automatically when context exceeds threshold
/// The LLM decides which messages to drop based on goals/todos, with fallback to percentage
pub async fn rolling_compact(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
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
    let target_drop_count =
        ((non_system_messages.len() as f32 * drop_percentage / 100.0).round() as usize).max(1);

    // Ask LLM which messages to drop
    let decision_prompt = format!(
        r#"You are deciding which conversation messages to archive during context compaction.

CURRENT MESSAGES (oldest first):
{}

{}{}
EXISTING SUMMARY:
{}

Your task: Select approximately {} messages to archive (move to summary).
Consider:
1. Keep messages directly relevant to current goals and todos
2. Keep recent messages (they provide immediate context)
3. Archive older messages that have been superseded or are less relevant
4. Preserve messages containing important decisions or key information
5. Tool call messages and their results should be archived together

Return ONLY a JSON array of message IDs to archive, e.g.: ["id1", "id2", "id3"]
No explanation, just the JSON array."#,
        serde_json::to_string_pretty(&messages_for_llm).unwrap_or_default(),
        if goals.is_empty() {
            String::new()
        } else {
            format!("CURRENT GOALS:\n{}\n\n", goals)
        },
        if todos.is_empty() {
            String::new()
        } else {
            format!("CURRENT TODOS:\n{}\n\n", todos)
        },
        if context.summary.is_empty() {
            "(No existing summary)"
        } else {
            &context.summary
        },
        target_drop_count,
    );

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
        eprintln!(
            "[Rolling compaction: LLM decision failed, falling back to dropping oldest {}%]",
            drop_percentage
        );
        non_system_messages
            .iter()
            .take(target_drop_count)
            .copied()
            .collect()
    } else {
        eprintln!(
            "[Rolling compaction: LLM selected {} messages to archive]",
            ids_to_drop.len()
        );
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
        eprintln!("[Rolling compaction: no messages to drop]");
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

    // Also collect tool_call_ids from dropped assistant messages so we can
    // atomically drop their tool results
    let mut drop_tool_call_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for m in &messages_to_drop {
        if let Some(tool_calls) = m["tool_calls"].as_array() {
            for tc in tool_calls {
                if let Some(id) = tc["id"].as_str() {
                    drop_tool_call_ids.insert(id.to_string());
                }
            }
        }
    }

    // Second LLM call: update summary with dropped content
    let update_prompt = format!(
        r#"You are updating a conversation summary. Your task is to integrate archived content into the existing summary.

EXISTING SUMMARY:
{}

CONTENT BEING ARCHIVED:
{}

{}{}
Create an updated summary that:
1. Preserves important information from the existing summary
2. Integrates key points from the archived content
3. Keeps information relevant to the goals and todos
4. Is concise but comprehensive
5. Maintains chronological awareness (what happened earlier vs later)

Output ONLY the updated summary, no preamble."#,
        if context.summary.is_empty() {
            "(No existing summary)"
        } else {
            &context.summary
        },
        stripped_text,
        if goals.is_empty() {
            String::new()
        } else {
            format!("\nCURRENT GOALS:\n{}\n", goals)
        },
        if todos.is_empty() {
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
        eprintln!("[WARN] Rolling compaction returned empty summary, keeping old state");
        return Ok(());
    }

    // Capture count before we drop the borrow
    let archived_count = messages_to_drop.len();

    // Drop the borrow by ending use of messages_to_drop
    drop(messages_to_drop);

    // Filter out dropped messages, keeping system messages and non-dropped messages.
    // Tool results whose tool_call_id matches a dropped assistant message are also dropped.
    let remaining_messages: Vec<serde_json::Value> = context
        .messages
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
        .collect();

    // Update context with new summary and remaining messages
    context.summary = new_summary.clone();
    context.messages = remaining_messages;
    context.updated_at = now_timestamp();

    // Finalize compaction: write anchor to transcript and mark dirty
    app.finalize_compaction(&context.name, &new_summary)?;

    // Save updated context (summary.md, context_meta.json, etc.)
    app.save_context(&context)?;

    eprintln!(
        "[Rolling compaction complete: {} messages remaining, {} archived]",
        context.messages.len(),
        archived_count
    );

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
) -> io::Result<()> {
    // Use rolling compaction for auto-triggered compaction
    rolling_compact(app, context_name, resolved_config).await
}

/// Full compaction: summarizes all messages and starts fresh (manual -c flag)
pub async fn compact_context_with_llm_manual(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
) -> io::Result<()> {
    compact_context_with_llm_internal(app, context_name, resolved_config, true).await
}

/// Compact a specific context by name (for -Z flag)
pub async fn compact_context_by_name(app: &AppState, context_name: &str) -> io::Result<()> {
    // Load the context
    let context = app.load_context(context_name)?;

    if context.messages.is_empty() {
        eprintln!("[Context '{}' is already empty]", context_name);
        return Ok(());
    }

    if context.messages.len() <= 2 {
        eprintln!(
            "[Context '{}' is already compact (2 or fewer messages)]",
            context_name
        );
        return Ok(());
    }

    // Finalize compaction: write anchor to transcript and mark dirty
    let simple_summary = format!(
        "Context compacted. {} messages archived to transcript.",
        context.messages.len()
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

    eprintln!(
        "[Context '{}' compacted: {} messages archived]",
        context_name,
        context.messages.len()
    );

    Ok(())
}

async fn compact_context_with_llm_internal(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    print_message: bool,
) -> io::Result<()> {
    let context = app.get_or_create_context(context_name)?;

    if context.messages.is_empty() {
        if print_message {
            println!("Context is already empty");
        }
        return Ok(());
    }

    if context.messages.len() <= 2 {
        if print_message {
            println!("Context is already compact (2 or fewer messages)");
        }
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

    if print_message {
        eprintln!(
            "[Compacting] Messages: {} -> requesting summary...",
            context.messages.len()
        );
    }

    // Load compaction prompt
    let compaction_prompt = app.load_prompt("compaction")?;
    let default_compaction_prompt = "Please summarize the following conversation into a concise summary. Capture the key points, decisions, and context.";
    let compaction_prompt = if compaction_prompt.is_empty() {
        eprintln!(
            "[WARN] No compaction prompt found at ~/.chibi/prompts/compaction.md. Using default."
        );
        default_compaction_prompt
    } else {
        &compaction_prompt
    };

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
                format!("{}... [truncated]", &content[..200])
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

    // Prepare continuation prompt
    let continuation_prompt = app.load_prompt("continuation")?;
    let continuation_prompt = if continuation_prompt.is_empty() {
        "Here is a summary of the previous conversation. Continue from this point."
    } else {
        &continuation_prompt
    };

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

    // Add assistant acknowledgment
    let ack_messages = vec![
        json!({
            "role": "system",
            "content": system_prompt,
        }),
        json!({
            "role": "user",
            "content": format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
        }),
    ];

    let acknowledgment = gateway::chat(resolved_config, &ack_messages).await?;

    new_context.messages.push(json!({
        "_id": uuid::Uuid::new_v4().to_string(),
        "role": "assistant",
        "content": acknowledgment,
    }));

    // Finalize compaction: write anchor to transcript and mark dirty
    app.finalize_compaction(&new_context.name, &summary)?;

    // Save the new context
    app.save_context(&new_context)?;

    if print_message {
        println!("Context compacted (history saved to transcript)");
    }

    // Execute post_compact hook
    let hook_data = serde_json::json!({
        "context_name": new_context.name,
        "message_count": new_context.messages.len(),
        "summary": new_context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PostCompact, &hook_data);

    Ok(())
}
