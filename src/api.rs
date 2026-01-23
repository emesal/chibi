use crate::config::{ResolvedConfig, ToolChoice, ToolChoiceMode};
use crate::context::{Context, InboxEntry, Message, now_timestamp};
use crate::input::DebugKey;
use crate::llm;
use crate::output::OutputHandler;
use crate::state::AppState;
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::json;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Write};
use tokio::io::{AsyncWriteExt, stdout};
use uuid::Uuid;

/// Maximum number of simultaneous tool calls allowed (prevents memory exhaustion from malicious responses)
const MAX_TOOL_CALLS: usize = 100;

/// Options for controlling prompt execution behavior
#[derive(Debug, Clone)]
pub struct PromptOptions<'a> {
    pub verbose: bool,
    pub use_reflection: bool,
    pub json_output: bool,
    pub debug: Option<&'a DebugKey>,
}

impl<'a> PromptOptions<'a> {
    pub fn new(
        verbose: bool,
        use_reflection: bool,
        json_output: bool,
        debug: Option<&'a DebugKey>,
    ) -> Self {
        Self {
            verbose,
            use_reflection,
            json_output,
            debug,
        }
    }
}

/// Safely extract content from an API response's first choice.
/// Returns None if the response is malformed or empty.
fn extract_choice_content(json: &serde_json::Value) -> Option<&str> {
    json.get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()
}

/// Log an API request to requests.jsonl if debug logging is enabled
fn log_request_if_enabled(
    app: &AppState,
    debug: Option<&DebugKey>,
    request_body: &serde_json::Value,
) {
    let should_log = matches!(debug, Some(DebugKey::RequestLog) | Some(DebugKey::All));
    if !should_log {
        return;
    }

    let log_entry = json!({
        "timestamp": now_timestamp(),
        "request": request_body,
    });

    let log_path = app.context_dir(&app.state.current_context).join("requests.jsonl");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path)
        && let Ok(json) = serde_json::to_string(&log_entry)
    {
        let _ = writeln!(file, "{}", json);
    }
}

/// Log response metadata to response_meta.jsonl if debug logging is enabled
fn log_response_meta_if_enabled(
    app: &AppState,
    debug: Option<&DebugKey>,
    response_meta: &serde_json::Value,
) {
    let should_log = matches!(debug, Some(DebugKey::ResponseMeta) | Some(DebugKey::All));
    if !should_log {
        return;
    }

    let log_entry = json!({
        "timestamp": now_timestamp(),
        "response": response_meta,
    });

    let log_path = app
        .context_dir(&app.state.current_context)
        .join("response_meta.jsonl");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path)
        && let Ok(json) = serde_json::to_string(&log_entry)
    {
        let _ = writeln!(file, "{}", json);
    }
}

/// Build the request body for the LLM API, applying all API parameters from ResolvedConfig
fn build_request_body(
    config: &ResolvedConfig,
    messages: &[serde_json::Value],
    tools: Option<&[serde_json::Value]>,
    stream: bool,
) -> serde_json::Value {
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "stream": stream,
    });

    // Add tools if provided
    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = json!(tools);
    }

    // Apply API parameters from resolved config
    let api = &config.api;

    // Temperature
    if let Some(temp) = api.temperature {
        body["temperature"] = json!(temp);
    }

    // Max tokens
    if let Some(max_tokens) = api.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    // Top P
    if let Some(top_p) = api.top_p {
        body["top_p"] = json!(top_p);
    }

    // Stop sequences
    if let Some(ref stop) = api.stop {
        body["stop"] = json!(stop);
    }

    // Tool choice
    if let Some(ref tool_choice) = api.tool_choice {
        match tool_choice {
            ToolChoice::Mode(mode) => {
                let mode_str = match mode {
                    ToolChoiceMode::Auto => "auto",
                    ToolChoiceMode::None => "none",
                    ToolChoiceMode::Required => "required",
                };
                body["tool_choice"] = json!(mode_str);
            }
            ToolChoice::Function { type_, function } => {
                body["tool_choice"] = json!({
                    "type": type_,
                    "function": { "name": function.name }
                });
            }
        }
    }

    // Parallel tool calls
    if let Some(parallel) = api.parallel_tool_calls {
        body["parallel_tool_calls"] = json!(parallel);
    }

    // Frequency penalty
    if let Some(freq) = api.frequency_penalty {
        body["frequency_penalty"] = json!(freq);
    }

    // Presence penalty
    if let Some(pres) = api.presence_penalty {
        body["presence_penalty"] = json!(pres);
    }

    // Seed
    if let Some(seed) = api.seed {
        body["seed"] = json!(seed);
    }

    // Response format
    if let Some(ref format) = api.response_format {
        body["response_format"] = serde_json::to_value(format).unwrap_or(json!(null));
    }

    // Reasoning configuration (OpenRouter-specific)
    // Either effort OR max_tokens can be set, plus optional exclude/enabled
    if !api.reasoning.is_empty() {
        let mut reasoning_obj = json!({});
        if let Some(ref effort) = api.reasoning.effort {
            reasoning_obj["effort"] = json!(effort.as_str());
        }
        if let Some(max_tokens) = api.reasoning.max_tokens {
            reasoning_obj["max_tokens"] = json!(max_tokens);
        }
        if let Some(exclude) = api.reasoning.exclude {
            reasoning_obj["exclude"] = json!(exclude);
        }
        if let Some(enabled) = api.reasoning.enabled {
            reasoning_obj["enabled"] = json!(enabled);
        }
        body["reasoning"] = reasoning_obj;
    }

    body
}

/// Rolling compaction: strips messages and integrates them into the summary
/// This is triggered automatically when context exceeds threshold
/// The LLM decides which messages to drop based on goals/todos, with fallback to percentage
pub async fn rolling_compact(
    app: &AppState,
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    let mut context = app.get_current_context()?;

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
    let goals = app.load_current_goals()?;
    let todos = app.load_current_todos()?;

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

    // Write compaction anchor to transcript (authoritative log)
    let compaction_anchor = app.create_compaction_anchor(&context.name, &new_summary);
    app.append_to_transcript(&context.name, &compaction_anchor)?;

    // Mark context dirty so it rebuilds with new anchor on next load
    app.mark_context_dirty(&context.name)?;

    // Save updated context (summary.md, context_meta.json, etc.)
    app.save_current_context(&context)?;

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
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    // Use rolling compaction for auto-triggered compaction
    rolling_compact(app, resolved_config, verbose).await
}

/// Full compaction: summarizes all messages and starts fresh (manual -c flag)
pub async fn compact_context_with_llm_manual(
    app: &AppState,
    resolved_config: &ResolvedConfig,
    verbose: bool,
) -> io::Result<()> {
    compact_context_with_llm_internal(app, resolved_config, true, verbose).await
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

    // Write compaction anchor to transcript.jsonl
    // For non-current contexts without LLM, we use a simple "archived" summary
    let simple_summary = format!(
        "Context compacted. {} messages archived to transcript.",
        context.messages.len()
    );
    let compaction_anchor = app.create_compaction_anchor(context_name, &simple_summary);
    app.append_to_transcript(context_name, &compaction_anchor)?;

    // Mark context dirty so it rebuilds with new anchor on next load
    app.mark_context_dirty(context_name)?;

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
    resolved_config: &ResolvedConfig,
    print_message: bool,
    verbose: bool,
) -> io::Result<()> {
    let context = app.get_current_context()?;

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

    // Write compaction anchor to transcript (authoritative log)
    let compaction_anchor = app.create_compaction_anchor(&new_context.name, &summary);
    app.append_to_transcript(&new_context.name, &compaction_anchor)?;

    // Mark context dirty so it rebuilds with new anchor on next load
    app.mark_context_dirty(&new_context.name)?;

    // Save the new context
    app.save_current_context(&new_context)?;

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

pub async fn send_prompt(
    app: &AppState,
    prompt: String,
    tools: &[Tool],
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
) -> io::Result<()> {
    let output = OutputHandler::new(options.json_output);
    send_prompt_with_depth(app, prompt, tools, 0, resolved_config, &output, options).await
}

async fn send_prompt_with_depth(
    app: &AppState,
    prompt: String,
    tools: &[Tool],
    recursion_depth: usize,
    resolved_config: &ResolvedConfig,
    output: &OutputHandler,
    options: &PromptOptions<'_>,
) -> io::Result<()> {
    if prompt.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Prompt cannot be empty",
        ));
    }

    let verbose = options.verbose;
    let use_reflection = options.use_reflection;
    let debug = options.debug;

    let mut context = app.get_current_context()?;

    // Execute pre_message hooks (can modify prompt)
    let mut final_prompt = prompt.clone();
    let hook_data = serde_json::json!({
        "prompt": prompt,
        "context_name": context.name,
        "summary": context.summary,
    });
    let hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreMessage, &hook_data, verbose)?;
    for (tool_name, result) in hook_results {
        if let Some(modified) = result.get("prompt").and_then(|v| v.as_str()) {
            if verbose {
                eprintln!("[Hook pre_message: {} modified prompt]", tool_name);
            }
            final_prompt = modified.to_string();
        }
    }

    // Check inbox and inject messages before the user prompt
    let inbox_messages = app.load_and_clear_current_inbox()?;
    if !inbox_messages.is_empty() {
        let mut inbox_content = String::from("--- INBOX MESSAGES ---\n");
        for msg in &inbox_messages {
            inbox_content.push_str(&format!("[From: {}] {}\n", msg.from, msg.content));
        }
        inbox_content.push_str("--- END INBOX ---\n\n");
        final_prompt = format!("{}{}", inbox_content, final_prompt);
        output.diagnostic(
            &format!("[Inbox: {} message(s) injected]", inbox_messages.len()),
            verbose,
        );
    }

    // Add user message to in-memory context
    app.add_message(&mut context, "user".to_string(), final_prompt.clone());

    // Append user message to both transcript.jsonl and context.jsonl (tandem write)
    let user_entry = app.create_user_message_entry(&final_prompt, &resolved_config.username);
    app.append_to_current_transcript_and_context(&user_entry)?;
    output.emit(&user_entry)?;

    // Check if we need to warn about context window
    if app.should_warn(&context.messages) {
        let remaining = app.remaining_tokens(&context.messages);
        output.diagnostic(
            &format!("[Context window warning: {} tokens remaining]", remaining),
            verbose,
        );
    }

    // Auto-compaction check
    if app.should_auto_compact(&context, resolved_config) {
        return compact_context_with_llm(app, resolved_config, verbose).await;
    }

    // Prepare messages for API
    let system_prompt = app.load_system_prompt()?;
    let reflection_prompt = if use_reflection {
        app.load_reflection_prompt()?
    } else {
        String::new()
    };

    // Load context-specific state: todos, goals, and summary
    let todos = app.load_current_todos()?;
    let goals = app.load_current_goals()?;
    let summary = &context.summary;

    // Execute pre_system_prompt hook - can inject content before system prompt sections
    let pre_sys_hook_data = serde_json::json!({
        "context_name": context.name,
        "summary": summary,
        "todos": todos,
        "goals": goals,
    });
    let pre_sys_hook_results = tools::execute_hook(
        tools,
        tools::HookPoint::PreSystemPrompt,
        &pre_sys_hook_data,
        verbose,
    )?;

    // Build full system prompt with all components
    let mut full_system_prompt = system_prompt.clone();

    // Prepend any content from pre_system_prompt hooks
    for (hook_tool_name, result) in &pre_sys_hook_results {
        if let Some(inject) = result.get("inject").and_then(|v| v.as_str())
            && !inject.is_empty()
        {
            output.diagnostic(
                &format!(
                    "[Hook pre_system_prompt: {} injected content]",
                    hook_tool_name
                ),
                verbose,
            );
            full_system_prompt = format!("{}\n\n{}", inject, full_system_prompt);
        }
    }

    // Add username info at the start if not "user"
    if resolved_config.username != "user" {
        full_system_prompt.push_str(&format!(
            "\n\nThe user speaking to you is called: {}",
            resolved_config.username
        ));
    }

    // Add summary if present
    if !summary.is_empty() {
        full_system_prompt.push_str("\n\n--- CONVERSATION SUMMARY ---\n");
        full_system_prompt.push_str(summary);
    }

    // Add goals if present
    if !goals.is_empty() {
        full_system_prompt.push_str("\n\n--- CURRENT GOALS ---\n");
        full_system_prompt.push_str(&goals);
    }

    // Add todos if present
    if !todos.is_empty() {
        full_system_prompt.push_str("\n\n--- CURRENT TODOS ---\n");
        full_system_prompt.push_str(&todos);
    }

    // Add reflection prompt last (personality layer)
    if !reflection_prompt.is_empty() {
        full_system_prompt.push_str("\n\n");
        full_system_prompt.push_str(&reflection_prompt);
    }

    // Execute post_system_prompt hook - can inject content after all system prompt sections
    let post_sys_hook_data = serde_json::json!({
        "context_name": context.name,
        "summary": summary,
        "todos": todos,
        "goals": goals,
    });
    let post_sys_hook_results = tools::execute_hook(
        tools,
        tools::HookPoint::PostSystemPrompt,
        &post_sys_hook_data,
        verbose,
    )?;

    // Append any content from post_system_prompt hooks
    for (hook_tool_name, result) in &post_sys_hook_results {
        if let Some(inject) = result.get("inject").and_then(|v| v.as_str())
            && !inject.is_empty()
        {
            output.diagnostic(
                &format!(
                    "[Hook post_system_prompt: {} injected content]",
                    hook_tool_name
                ),
                verbose,
            );
            full_system_prompt.push_str("\n\n");
            full_system_prompt.push_str(inject);
        }
    }

    let mut messages: Vec<serde_json::Value> = if !full_system_prompt.is_empty() {
        vec![serde_json::json!({
            "role": "system",
            "content": full_system_prompt,
        })]
    } else {
        Vec::new()
    };

    // Add conversation messages (skip system messages as they're already included via full_system_prompt)
    for m in &context.messages {
        if m.role == "system" {
            continue;
        }
        messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content,
        }));
    }

    // Collect all tools (user-defined + built-in tools)
    let mut all_tools = tools::tools_to_api_format(tools);

    // Always add agentic tools (todos, goals, send_message)
    // Note: recurse tool is now external (loaded from tools directory)
    all_tools.push(tools::todos_tool_to_api_format());
    all_tools.push(tools::goals_tool_to_api_format());
    all_tools.push(tools::send_message_tool_to_api_format());

    // Add reflection tool if enabled
    if use_reflection {
        all_tools.push(tools::reflection_tool_to_api_format());
    }

    // Build request with tools and API params from resolved config
    let mut request_body =
        build_request_body(resolved_config, &messages, Some(&all_tools), true);

    // Track if we should recurse (continue_processing was called)
    let mut should_recurse = false;
    let mut recurse_note = String::new();

    // Tool call loop - keep going until we get a final text response
    loop {
        // Log request if debug logging is enabled
        log_request_if_enabled(app, debug, &request_body);

        let response = llm::send_streaming_request(resolved_config, request_body.clone()).await?;

        let mut stream = response.bytes_stream();
        let mut stdout = stdout();
        let mut full_response = String::new();
        let mut is_first_content = true;
        let json_mode = output.is_json_mode();

        // Tool call accumulation
        let mut tool_calls: Vec<llm::ToolCallAccumulator> = Vec::new();
        let mut has_tool_calls = false;

        // Response metadata accumulation (usage stats, model info)
        let mut response_meta: Option<serde_json::Value> = None;

        while let Some(chunk_result) = stream.next().await {
            let chunk =
                chunk_result.map_err(|e| io::Error::other(format!("Stream error: {}", e)))?;
            let chunk_str = std::str::from_utf8(&chunk)
                .map_err(|e| io::Error::other(format!("UTF-8 error: {}", e)))?;

            // Parse Server-Sent Events format
            for line in chunk_str.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }

                    let json: serde_json::Value = serde_json::from_str(data)
                        .map_err(|e| io::Error::other(format!("JSON parse error: {}", e)))?;

                    // Capture response metadata (usage stats, model, id)
                    // These typically appear in all chunks or the final chunk
                    if json.get("usage").is_some()
                        || json.get("model").is_some()
                        || json.get("id").is_some()
                    {
                        let mut meta = response_meta.take().unwrap_or(json!({}));
                        if let Some(usage) = json.get("usage") {
                            meta["usage"] = usage.clone();
                        }
                        if let Some(model) = json.get("model") {
                            meta["model"] = model.clone();
                        }
                        if let Some(id) = json.get("id") {
                            meta["id"] = id.clone();
                        }
                        response_meta = Some(meta);
                    }

                    if let Some(choices) = json["choices"].as_array()
                        && let Some(choice) = choices.first()
                        && let Some(delta) = choice.get("delta")
                    {
                        // Handle regular content
                        if let Some(content) = delta["content"].as_str() {
                            if is_first_content {
                                is_first_content = false;
                                if let Some(remaining) = content.strip_prefix('\n') {
                                    if !remaining.is_empty() {
                                        full_response.push_str(remaining);
                                        // Only stream in normal mode
                                        if !json_mode {
                                            stdout.write_all(remaining.as_bytes()).await?;
                                            stdout.flush().await?;
                                        }
                                    }
                                    continue;
                                }
                            }

                            full_response.push_str(content);
                            // Only stream in normal mode
                            if !json_mode {
                                stdout.write_all(content.as_bytes()).await?;
                                stdout.flush().await?;
                            }
                        }

                        // Handle tool calls
                        if let Some(tc_array) = delta["tool_calls"].as_array() {
                            has_tool_calls = true;
                            for tc in tc_array {
                                let index = tc["index"].as_u64().unwrap_or(0) as usize;

                                // Prevent memory exhaustion from malicious API responses
                                if index >= MAX_TOOL_CALLS {
                                    if verbose {
                                        eprintln!(
                                            "[WARN] Tool call index {} exceeds limit {}, skipping",
                                            index, MAX_TOOL_CALLS
                                        );
                                    }
                                    continue;
                                }

                                while tool_calls.len() <= index {
                                    tool_calls.push(llm::ToolCallAccumulator::default());
                                }

                                if let Some(id) = tc["id"].as_str() {
                                    tool_calls[index].id = id.to_string();
                                }
                                if let Some(func) = tc.get("function") {
                                    if let Some(name) = func["name"].as_str() {
                                        tool_calls[index].name = name.to_string();
                                    }
                                    if let Some(args) = func["arguments"].as_str() {
                                        tool_calls[index].arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Log response metadata if debug logging is enabled
        if let Some(ref meta) = response_meta {
            log_response_meta_if_enabled(app, debug, meta);
        }

        // If we have tool calls, execute them and continue the loop
        if has_tool_calls && !tool_calls.is_empty() {
            let tool_calls_json: Vec<serde_json::Value> = tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }
                    })
                })
                .collect();

            messages.push(serde_json::json!({
                "role": "assistant",
                "tool_calls": tool_calls_json,
            }));

            // Execute each tool and add results
            for tc in &tool_calls {
                output.diagnostic(&format!("[Tool: {}]", tc.name), verbose);

                let mut args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));

                // Check for recurse tool first (special handling - triggers recursion after this turn)
                if let Some(note) = tools::check_recurse_signal(&tc.name, &args) {
                    should_recurse = true;
                    recurse_note = note;
                    // Still execute the tool normally (it's a noop that just returns a message)
                    // The tool result will be added below after normal tool execution
                }

                // Execute pre_tool hooks (can modify arguments OR block execution)
                let pre_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                });
                let pre_hook_results =
                    tools::execute_hook(tools, tools::HookPoint::PreTool, &pre_hook_data, verbose)?;

                let mut blocked = false;
                let mut block_message = String::new();

                for (hook_tool_name, result) in pre_hook_results {
                    // Check for block signal first
                    if result
                        .get("block")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        blocked = true;
                        block_message = result
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Tool call blocked by hook")
                            .to_string();
                        output.diagnostic(
                            &format!(
                                "[Hook pre_tool: {} blocked {} - {}]",
                                hook_tool_name, tc.name, block_message
                            ),
                            verbose,
                        );
                        break;
                    }

                    // Check for argument modification
                    if let Some(modified_args) = result.get("arguments") {
                        output.diagnostic(
                            &format!(
                                "[Hook pre_tool: {} modified arguments for {}]",
                                hook_tool_name, tc.name
                            ),
                            verbose,
                        );
                        args = modified_args.clone();
                    }
                }

                // If blocked, skip execution and use block message as result
                let tool_result = if blocked {
                    block_message
                } else if tc.name == tools::REFLECTION_TOOL_NAME && !use_reflection {
                    // Reflection tool called but reflection is disabled
                    "Error: Reflection tool is not enabled".to_string()
                } else if let Some(builtin_result) = tools::execute_builtin_tool(app, &tc.name, &args) {
                    // Handle built-in tools (todos, goals, reflection)
                    match builtin_result {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                } else if tc.name == tools::SEND_MESSAGE_TOOL_NAME {
                    // Handle built-in send_message tool
                    let to = args["to"].as_str().unwrap_or("");
                    let content = args["content"].as_str().unwrap_or("");
                    let from = args["from"].as_str().unwrap_or(&context.name);

                    if to.is_empty() {
                        "Error: 'to' field is required".to_string()
                    } else if content.is_empty() {
                        "Error: 'content' field is required".to_string()
                    } else {
                        // Execute pre_send_message hooks - can intercept delivery
                        let pre_hook_data = serde_json::json!({
                            "from": from,
                            "to": to,
                            "content": content,
                            "context_name": context.name,
                        });
                        let pre_hook_results = tools::execute_hook(
                            tools,
                            tools::HookPoint::PreSendMessage,
                            &pre_hook_data,
                            verbose,
                        )?;

                        // Check if any hook claimed delivery
                        let mut delivered_via: Option<String> = None;
                        for (hook_tool_name, hook_result) in &pre_hook_results {
                            if hook_result
                                .get("delivered")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                let via = hook_result
                                    .get("via")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(hook_tool_name);
                                delivered_via = Some(via.to_string());
                                output.diagnostic(
                                    &format!(
                                        "[Hook pre_send_message: {} intercepted delivery]",
                                        hook_tool_name
                                    ),
                                    verbose,
                                );
                                break;
                            }
                        }

                        let delivery_result = if let Some(via) = delivered_via {
                            // Hook claimed delivery, skip local inbox
                            format!("Message delivered to '{}' via {}", to, via)
                        } else {
                            // No hook claimed delivery, write to local inbox
                            let entry = InboxEntry {
                                id: Uuid::new_v4().to_string(),
                                timestamp: now_timestamp(),
                                from: from.to_string(),
                                to: to.to_string(),
                                content: content.to_string(),
                            };
                            match app.append_to_inbox(to, &entry) {
                                Ok(()) => format!("Message delivered to '{}' via local inbox", to),
                                Err(e) => format!("Error delivering message: {}", e),
                            }
                        };

                        // Execute post_send_message hooks (observe only)
                        let post_hook_data = serde_json::json!({
                            "from": from,
                            "to": to,
                            "content": content,
                            "context_name": context.name,
                            "delivery_result": delivery_result,
                        });
                        let _ = tools::execute_hook(
                            tools,
                            tools::HookPoint::PostSendMessage,
                            &post_hook_data,
                            verbose,
                        );

                        delivery_result
                    }
                } else if let Some(tool) = tools::find_tool(tools, &tc.name) {
                    match tools::execute_tool(tool, &args, verbose) {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                } else {
                    format!("Error: Unknown tool '{}'", tc.name)
                };

                // Log tool call and result to both transcript.jsonl and context.jsonl
                let tool_call_entry = app.create_tool_call_entry(&tc.name, &tc.arguments);
                app.append_to_current_transcript_and_context(&tool_call_entry)?;
                output.emit(&tool_call_entry)?;

                let tool_result_entry = app.create_tool_result_entry(&tc.name, &tool_result);
                app.append_to_current_transcript_and_context(&tool_result_entry)?;
                output.emit(&tool_result_entry)?;

                // Execute post_tool hooks (observe only)
                let post_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "result": tool_result,
                });
                let _ = tools::execute_hook(
                    tools,
                    tools::HookPoint::PostTool,
                    &post_hook_data,
                    verbose,
                );

                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tc.id,
                    "content": tool_result,
                }));
            }

            request_body["messages"] = serde_json::json!(messages);
            continue;
        }

        // No tool calls - we have a final response
        // Update in-memory context for this session
        app.add_message(&mut context, "assistant".to_string(), full_response.clone());

        // Append assistant message to both transcript.jsonl and context.jsonl (tandem write)
        let assistant_entry = app.create_assistant_message_entry(&full_response);
        app.append_to_current_transcript_and_context(&assistant_entry)?;
        output.emit(&assistant_entry)?;

        // Execute post_message hooks (observe only)
        let hook_data = serde_json::json!({
            "prompt": final_prompt,
            "response": full_response,
            "context_name": context.name,
        });
        let _ = tools::execute_hook(tools, tools::HookPoint::PostMessage, &hook_data, verbose);

        if app.should_warn(&context.messages) {
            let remaining = app.remaining_tokens(&context.messages);
            output.diagnostic(
                &format!("[Context window warning: {} tokens remaining]", remaining),
                verbose,
            );
        }

        output.newline();

        // Check if we should recurse (continue_processing was called)
        if should_recurse {
            let new_depth = recursion_depth + 1;
            if new_depth >= app.config.max_recursion_depth {
                output.diagnostic_always(&format!(
                    "[Max recursion depth ({}) reached, stopping]",
                    app.config.max_recursion_depth
                ));
                return Ok(());
            }
            output.diagnostic(
                &format!(
                    "[Continuing processing ({}/{}): {}]",
                    new_depth, app.config.max_recursion_depth, recurse_note
                ),
                verbose,
            );
            // Recursively call send_prompt with the note as the new prompt
            let continue_prompt = format!(
                "[Continuing from previous round]\n\nNote to self: {}",
                recurse_note
            );
            return Box::pin(send_prompt_with_depth(
                app,
                continue_prompt,
                tools,
                new_depth,
                resolved_config,
                output,
                options,
            ))
            .await;
        }

        return Ok(());
    }
}
