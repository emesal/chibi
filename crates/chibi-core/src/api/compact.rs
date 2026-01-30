//! Context compaction functions.
//!
//! This module provides different compaction strategies:
//! - Rolling compaction: strips messages and integrates them into the summary
//! - Full compaction: summarizes all messages and starts fresh
//! - By-name compaction: compact a specific context without LLM

use super::request::{build_request_body, extract_choice_content};
use crate::config::ResolvedConfig;
use crate::context::{Context, Message, now_timestamp};
use crate::state::AppState;
use crate::tools;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::json;
use std::io;

/// Rolling compaction: strips messages and integrates them into the summary
/// This is triggered automatically when context exceeds threshold
/// The LLM decides which messages to drop based on goals/todos, with fallback to percentage
pub async fn rolling_compact(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    let mut context = app.get_or_create_context(context_name)?;

    // Skip system messages when counting
    let non_system_messages: Vec<_> = context
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .collect();

    if non_system_messages.len() <= 4 {
        // Not enough messages to strip - too few to meaningfully compact
        return Ok(());
    }

    // Execute pre_rolling_compact hook
    let tools = tools::load_tools(&app.plugins_dir, verbose)?;
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
        verbose,
    );

    // Load goals and todos to guide compaction decisions
    let goals = app.load_goals(context_name)?;
    let todos = app.load_todos(context_name)?;

    // Build message list in transcript format for LLM to analyze
    let messages_for_llm: Vec<serde_json::Value> = non_system_messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "role": m.role,
                "content": if m.content.len() > 500 {
                    format!("{}... [truncated]", &m.content[..500])
                } else {
                    m.content.clone()
                }
            })
        })
        .collect();

    // Calculate target drop count based on config percentage
    let drop_percentage = app.config.rolling_compact_drop_percentage;
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

    let client = Client::new();

    // First LLM call: decide what to drop
    let decision_messages = vec![json!({
        "role": "user",
        "content": decision_prompt,
    })];
    let decision_request = build_request_body(resolved_config, &decision_messages, None, false);

    let ids_to_drop: Vec<String> = match client
        .post(&resolved_config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", resolved_config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(decision_request.to_string())
        .send()
        .await
    {
        Ok(response) if response.status() == StatusCode::OK => {
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    let content = extract_choice_content(&json).unwrap_or("[]");
                    // Try to parse as JSON array
                    serde_json::from_str(content).unwrap_or_else(|_| {
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
            }
        }
        _ => Vec::new(),
    };

    // Determine which messages to actually drop
    let messages_to_drop: Vec<&Message> = if ids_to_drop.is_empty() {
        // Fallback: drop oldest N messages based on percentage
        if verbose {
            eprintln!(
                "[Rolling compaction: LLM decision failed, falling back to dropping oldest {}%]",
                drop_percentage
            );
        }
        non_system_messages
            .iter()
            .take(target_drop_count)
            .copied()
            .collect()
    } else {
        if verbose {
            eprintln!(
                "[Rolling compaction: LLM selected {} messages to archive]",
                ids_to_drop.len()
            );
        }
        non_system_messages
            .iter()
            .filter(|m| ids_to_drop.contains(&m.id))
            .copied()
            .collect()
    };

    if messages_to_drop.is_empty() {
        if verbose {
            eprintln!("[Rolling compaction: no messages to drop]");
        }
        return Ok(());
    }

    // Build text of messages to summarize
    let mut stripped_text = String::new();
    for m in &messages_to_drop {
        stripped_text.push_str(&format!("[{}]: {}\n\n", m.role.to_uppercase(), m.content));
    }

    // Collect IDs of messages to drop for filtering
    let drop_ids: std::collections::HashSet<_> = messages_to_drop.iter().map(|m| &m.id).collect();

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
    let summary_request = build_request_body(resolved_config, &summary_messages, None, false);

    let response = client
        .post(&resolved_config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", resolved_config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(summary_request.to_string())
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Rolling compact request failed: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::other(format!(
            "Rolling compact API error ({}): {}",
            status, body
        )));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| {
        io::Error::other(format!("Failed to parse rolling compact response: {}", e))
    })?;

    let new_summary = extract_choice_content(&json).unwrap_or("").to_string();

    if new_summary.is_empty() {
        if verbose {
            eprintln!("[WARN] Rolling compaction returned empty summary, keeping old state");
        }
        return Ok(());
    }

    // Capture count before we drop the borrow
    let archived_count = messages_to_drop.len();

    // Drop the borrow by ending use of messages_to_drop
    drop(messages_to_drop);

    // Filter out dropped messages, keeping system messages and non-dropped messages
    let remaining_messages: Vec<Message> = context
        .messages
        .iter()
        .filter(|m| m.role == "system" || !drop_ids.contains(&m.id))
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

    if verbose {
        eprintln!(
            "[Rolling compaction complete: {} messages remaining, {} archived]",
            context.messages.len(),
            archived_count
        );
    }

    // Execute post_rolling_compact hook
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
        verbose,
    );

    Ok(())
}

/// Full compaction: summarizes all messages and starts fresh (auto-triggered)
pub async fn compact_context_with_llm(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    // Use rolling compaction for auto-triggered compaction
    rolling_compact(app, context_name, resolved_config, verbose).await
}

/// Full compaction: summarizes all messages and starts fresh (manual -c flag)
pub async fn compact_context_with_llm_manual(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    compact_context_with_llm_internal(app, context_name, resolved_config, true, verbose).await
}

/// Compact a specific context by name (for -Z flag)
pub async fn compact_context_by_name(
    app: &AppState,
    context_name: &str,
    verbose: bool,
) -> io::Result<()> {
    // Load the context
    let context = app.load_context(context_name)?;

    if context.messages.is_empty() {
        if verbose {
            eprintln!("[Context '{}' is already empty]", context_name);
        }
        return Ok(());
    }

    if context.messages.len() <= 2 {
        if verbose {
            eprintln!(
                "[Context '{}' is already compact (2 or fewer messages)]",
                context_name
            );
        }
        return Ok(());
    }

    // Archive to transcript.md for human readability
    app.append_to_transcript_md(&context)?;

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

    if verbose {
        eprintln!(
            "[Context '{}' compacted: {} messages archived]",
            context_name,
            context.messages.len()
        );
    }

    Ok(())
}

async fn compact_context_with_llm_internal(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    print_message: bool,
    verbose: bool,
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
    let tools = tools::load_tools(&app.plugins_dir, verbose)?;
    let hook_data = serde_json::json!({
        "context_name": context.name,
        "message_count": context.messages.len(),
        "summary": context.summary,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::PreCompact, &hook_data, verbose);

    // Append to transcript.md before compacting (for archival)
    app.append_to_transcript_md(&context)?;

    if print_message && verbose {
        eprintln!(
            "[Compacting] Messages: {} -> requesting summary...",
            context.messages.len()
        );
    }

    let client = Client::new();

    // Load compaction prompt
    let compaction_prompt = app.load_prompt("compaction")?;
    let default_compaction_prompt = "Please summarize the following conversation into a concise summary. Capture the key points, decisions, and context.";
    let compaction_prompt = if compaction_prompt.is_empty() {
        if verbose {
            eprintln!(
                "[WARN] No compaction prompt found at ~/.chibi/prompts/compaction.md. Using default."
            );
        }
        default_compaction_prompt
    } else {
        &compaction_prompt
    };

    // Build conversation text for summarization
    let mut conversation_text = String::new();
    for m in &context.messages {
        if m.role == "system" {
            continue;
        }
        conversation_text.push_str(&format!("[{}]: {}\n\n", m.role.to_uppercase(), m.content));
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

    let request_body = build_request_body(resolved_config, &compaction_messages, None, false);

    let response = client
        .post(&resolved_config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", resolved_config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Failed to send request: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::other(format!(
            "API error ({}): {}",
            status, body
        )));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| io::Error::other(format!("Failed to parse response: {}", e)))?;

    let summary = extract_choice_content(&json)
        .or_else(|| {
            // Fallback: try alternative response structure
            json.get("choices")?.get(0)?.get("content")?.as_str()
        })
        .unwrap_or_else(|| {
            if verbose {
                eprintln!("[DEBUG] Response structure: {}", json);
            }
            ""
        })
        .to_string();

    if summary.is_empty() {
        if verbose {
            eprintln!("[DEBUG] Full response: {}", json);
        }
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
    let system_prompt = app.load_system_prompt()?;

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
        new_context
            .messages
            .push(Message::new("system", system_prompt.clone()));
    }

    // Add continuation prompt + summary as user message
    new_context.messages.push(Message::new(
        "user",
        format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
    ));

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

    let request_body = build_request_body(resolved_config, &ack_messages, None, false);

    let response = client
        .post(&resolved_config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", resolved_config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Failed to send request: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::other(format!(
            "API error ({}): {}",
            status, body
        )));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| io::Error::other(format!("Failed to parse response: {}", e)))?;

    let acknowledgment = extract_choice_content(&json).unwrap_or("").to_string();

    new_context
        .messages
        .push(Message::new("assistant", acknowledgment));

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
    let _ = tools::execute_hook(&tools, tools::HookPoint::PostCompact, &hook_data, verbose);

    Ok(())
}
