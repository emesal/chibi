use crate::config::ResolvedConfig;
use crate::context::{Context, InboxEntry, Message, now_timestamp};
use crate::state::AppState;
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use std::io::{self, ErrorKind};
use tokio::io::{AsyncWriteExt, stdout};
use uuid::Uuid;

/// Rolling compaction: strips messages and integrates them into the summary
/// This is triggered automatically when context exceeds threshold
/// The LLM decides which messages to drop based on goals/todos, with fallback to percentage
pub async fn rolling_compact(app: &AppState, verbose: bool) -> io::Result<()> {
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
    let decision_request = serde_json::json!({
        "model": app.config.model,
        "messages": [
            {
                "role": "user",
                "content": decision_prompt,
            }
        ],
        "stream": false,
    });

    let ids_to_drop: Vec<String> = match client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(decision_request.to_string())
        .send()
        .await
    {
        Ok(response) if response.status() == StatusCode::OK => {
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    let content = json["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("[]");
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

    let summary_request = serde_json::json!({
        "model": app.config.model,
        "messages": [
            {
                "role": "user",
                "content": update_prompt,
            }
        ],
        "stream": false,
    });

    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
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

    let new_summary = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

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
    context.summary = new_summary;
    context.messages = remaining_messages;
    context.updated_at = now_timestamp();

    // Save updated context
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
pub async fn compact_context_with_llm(app: &AppState, verbose: bool) -> io::Result<()> {
    // Use rolling compaction for auto-triggered compaction
    rolling_compact(app, verbose).await
}

/// Full compaction: summarizes all messages and starts fresh (manual -c flag)
pub async fn compact_context_with_llm_manual(app: &AppState, verbose: bool) -> io::Result<()> {
    compact_context_with_llm_internal(app, true, verbose).await
}

async fn compact_context_with_llm_internal(
    app: &AppState,
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

    // Append to transcript before compacting
    app.append_to_transcript(&context)?;

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
        serde_json::json!({
            "role": "system",
            "content": compaction_prompt,
        }),
        serde_json::json!({
            "role": "user",
            "content": format!("Please summarize this conversation:\n\n{}", conversation_text),
        }),
    ];

    let request_body = serde_json::json!({
        "model": app.config.model,
        "messages": compaction_messages,
        "stream": false,
    });

    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
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

    let summary = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["choices"][0]["content"].as_str())
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
    let messages = vec![
        serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }),
        serde_json::json!({
            "role": "user",
            "content": format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
        }),
    ];

    let request_body = serde_json::json!({
        "model": app.config.model,
        "messages": messages,
        "stream": false,
    });

    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
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

    let acknowledgment = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    new_context
        .messages
        .push(Message::new("assistant", acknowledgment));

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
    verbose: bool,
    use_reflection: bool,
    resolved_config: &ResolvedConfig,
) -> io::Result<()> {
    send_prompt_with_depth(
        app,
        prompt,
        tools,
        verbose,
        use_reflection,
        0,
        resolved_config,
    )
    .await
}

async fn send_prompt_with_depth(
    app: &AppState,
    prompt: String,
    tools: &[Tool],
    verbose: bool,
    use_reflection: bool,
    recursion_depth: usize,
    resolved_config: &ResolvedConfig,
) -> io::Result<()> {
    if prompt.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Prompt cannot be empty",
        ));
    }

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
        if verbose {
            eprintln!("[Inbox: {} message(s) injected]", inbox_messages.len());
        }
    }

    // Add user message
    app.add_message(&mut context, "user".to_string(), final_prompt.clone());

    // Log user message to JSONL transcript
    let user_entry = app.create_user_message_entry(&final_prompt, &resolved_config.username);
    let _ = app.append_to_jsonl_transcript(&user_entry);

    // Check if we need to warn about context window
    if app.should_warn(&context.messages) && verbose {
        let remaining = app.remaining_tokens(&context.messages);
        eprintln!("[Context window warning: {} tokens remaining]", remaining);
    }

    // Auto-compaction check
    if app.should_auto_compact(&context, resolved_config) {
        return compact_context_with_llm(app, verbose).await;
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

    let context_has_system = context.messages.iter().any(|m| m.role == "system");

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
            if verbose {
                eprintln!(
                    "[Hook pre_system_prompt: {} injected content]",
                    hook_tool_name
                );
            }
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
            if verbose {
                eprintln!(
                    "[Hook post_system_prompt: {} injected content]",
                    hook_tool_name
                );
            }
            full_system_prompt.push_str("\n\n");
            full_system_prompt.push_str(inject);
        }
    }

    let mut messages: Vec<serde_json::Value> =
        if !full_system_prompt.is_empty() && !context_has_system {
            vec![serde_json::json!({
                "role": "system",
                "content": full_system_prompt,
            })]
        } else {
            Vec::new()
        };

    // Add conversation messages
    for m in &context.messages {
        messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content,
        }));
    }

    // Build request with optional tools
    let mut request_body = serde_json::json!({
        "model": app.config.model,
        "messages": messages,
        "stream": true,
    });

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

    if !all_tools.is_empty() {
        request_body["tools"] = serde_json::json!(all_tools);
    }

    // Track if we should recurse (continue_processing was called)
    let mut should_recurse = false;
    let mut recurse_note = String::new();

    let client = Client::new();

    // Tool call loop - keep going until we get a final text response
    loop {
        let response = client
            .post(&app.config.base_url)
            .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
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

        let mut stream = response.bytes_stream();
        let mut stdout = stdout();
        let mut full_response = String::new();
        let mut is_first_content = true;

        // Tool call accumulation
        let mut tool_calls: Vec<ToolCallAccumulator> = Vec::new();
        let mut has_tool_calls = false;

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
                                        stdout.write_all(remaining.as_bytes()).await?;
                                        stdout.flush().await?;
                                    }
                                    continue;
                                }
                            }

                            full_response.push_str(content);
                            stdout.write_all(content.as_bytes()).await?;
                            stdout.flush().await?;
                        }

                        // Handle tool calls
                        if let Some(tc_array) = delta["tool_calls"].as_array() {
                            has_tool_calls = true;
                            for tc in tc_array {
                                let index = tc["index"].as_u64().unwrap_or(0) as usize;

                                while tool_calls.len() <= index {
                                    tool_calls.push(ToolCallAccumulator::default());
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
                if verbose {
                    eprintln!("[Tool: {}]", tc.name);
                }

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
                        if verbose {
                            eprintln!(
                                "[Hook pre_tool: {} blocked {} - {}]",
                                hook_tool_name, tc.name, block_message
                            );
                        }
                        break;
                    }

                    // Check for argument modification
                    if let Some(modified_args) = result.get("arguments") {
                        if verbose {
                            eprintln!(
                                "[Hook pre_tool: {} modified arguments for {}]",
                                hook_tool_name, tc.name
                            );
                        }
                        args = modified_args.clone();
                    }
                }

                // If blocked, skip execution and use block message as result
                let result = if blocked {
                    block_message
                } else if tc.name == tools::REFLECTION_TOOL_NAME && use_reflection {
                    // Handle built-in reflection tool
                    match tools::execute_reflection_tool(
                        &app.prompts_dir,
                        &args,
                        app.config.reflection_character_limit,
                    ) {
                        Ok(output) => output,
                        Err(e) => format!("Error: {}", e),
                    }
                } else if tc.name == tools::TODOS_TOOL_NAME {
                    // Handle built-in todos tool
                    let content = args["content"].as_str().unwrap_or("");
                    match app.save_current_todos(content) {
                        Ok(()) => format!("Todos updated ({} characters).", content.len()),
                        Err(e) => format!("Error saving todos: {}", e),
                    }
                } else if tc.name == tools::GOALS_TOOL_NAME {
                    // Handle built-in goals tool
                    let content = args["content"].as_str().unwrap_or("");
                    match app.save_current_goals(content) {
                        Ok(()) => format!("Goals updated ({} characters).", content.len()),
                        Err(e) => format!("Error saving goals: {}", e),
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
                        for (hook_tool_name, result) in &pre_hook_results {
                            if result
                                .get("delivered")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                let via = result
                                    .get("via")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(hook_tool_name);
                                delivered_via = Some(via.to_string());
                                if verbose {
                                    eprintln!(
                                        "[Hook pre_send_message: {} intercepted delivery]",
                                        hook_tool_name
                                    );
                                }
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
                        Ok(output) => output,
                        Err(e) => format!("Error: {}", e),
                    }
                } else {
                    format!("Error: Unknown tool '{}'", tc.name)
                };

                // Log tool call and result to JSONL transcript
                let tool_call_entry = app.create_tool_call_entry(&tc.name, &tc.arguments);
                let _ = app.append_to_jsonl_transcript(&tool_call_entry);
                let tool_result_entry = app.create_tool_result_entry(&tc.name, &result);
                let _ = app.append_to_jsonl_transcript(&tool_result_entry);

                // Execute post_tool hooks (observe only)
                let post_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "result": result,
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
                    "content": result,
                }));
            }

            request_body["messages"] = serde_json::json!(messages);
            continue;
        }

        // No tool calls - we have a final response
        app.add_message(&mut context, "assistant".to_string(), full_response.clone());
        app.save_current_context(&context)?;

        // Log assistant message to JSONL transcript
        let assistant_entry = app.create_assistant_message_entry(&full_response);
        let _ = app.append_to_jsonl_transcript(&assistant_entry);

        // Execute post_message hooks (observe only)
        let hook_data = serde_json::json!({
            "prompt": final_prompt,
            "response": full_response,
            "context_name": context.name,
        });
        let _ = tools::execute_hook(tools, tools::HookPoint::PostMessage, &hook_data, verbose);

        if app.should_warn(&context.messages) && verbose {
            let remaining = app.remaining_tokens(&context.messages);
            eprintln!("[Context window warning: {} tokens remaining]", remaining);
        }

        println!();

        // Check if we should recurse (continue_processing was called)
        if should_recurse {
            let new_depth = recursion_depth + 1;
            if new_depth >= app.config.max_recursion_depth {
                eprintln!(
                    "[Max recursion depth ({}) reached, stopping]",
                    app.config.max_recursion_depth
                );
                return Ok(());
            }
            if verbose {
                eprintln!(
                    "[Continuing processing ({}/{}): {}]",
                    new_depth, app.config.max_recursion_depth, recurse_note
                );
            }
            // Recursively call send_prompt with the note as the new prompt
            let continue_prompt = format!(
                "[Continuing from previous round]\n\nNote to self: {}",
                recurse_note
            );
            return Box::pin(send_prompt_with_depth(
                app,
                continue_prompt,
                tools,
                verbose,
                use_reflection,
                new_depth,
                resolved_config,
            ))
            .await;
        }

        return Ok(());
    }
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}
