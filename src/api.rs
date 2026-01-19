use crate::context::{Context, Message, now_timestamp};
use crate::state::AppState;
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use std::io::{self, ErrorKind};
use tokio::io::{stdout, AsyncWriteExt};

pub async fn compact_context_with_llm(app: &AppState, verbose: bool) -> io::Result<()> {
    compact_context_with_llm_internal(app, false, verbose).await
}

pub async fn compact_context_with_llm_manual(app: &AppState, verbose: bool) -> io::Result<()> {
    compact_context_with_llm_internal(app, true, verbose).await
}

async fn compact_context_with_llm_internal(app: &AppState, print_message: bool, verbose: bool) -> io::Result<()> {
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

    // Append to transcript before compacting
    app.append_to_transcript(&context)?;

    if print_message && verbose {
        eprintln!("[Compacting] Messages: {} -> requesting summary...", context.messages.len());
    }

    let client = Client::new();

    // Load compaction prompt
    let compaction_prompt = app.load_prompt("compaction")?;
    let default_compaction_prompt = "Please summarize the following conversation into a concise summary. Capture the key points, decisions, and context.";
    let compaction_prompt = if compaction_prompt.is_empty() {
        if verbose {
            eprintln!("[WARN] No compaction prompt found at ~/.chibi/prompts/compaction.md. Using default.");
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
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("API error ({}): {}", status, body),
        ));
    }

    let json: serde_json::Value = response.json().await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to parse response: {}", e)))?;

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
        return Err(io::Error::new(
            ErrorKind::Other,
            "Empty summary received from LLM. This can happen with free-tier models. Try again or use a different model."
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
    };

    // Add system prompt as first message
    if !system_prompt.is_empty() {
        new_context.messages.push(Message {
            role: "system".to_string(),
            content: system_prompt.clone(),
        });
    }

    // Add continuation prompt + summary as user message
    new_context.messages.push(Message {
        role: "user".to_string(),
        content: format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
    });

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
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("API error ({}): {}", status, body),
        ));
    }

    let json: serde_json::Value = response.json().await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to parse response: {}", e)))?;

    let acknowledgment = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    new_context.messages.push(Message {
        role: "assistant".to_string(),
        content: acknowledgment,
    });

    // Save the new context
    app.save_current_context(&new_context)?;

    if print_message {
        println!("Context compacted (history saved to transcript)");
    }
    Ok(())
}

pub async fn send_prompt(app: &AppState, prompt: String, tools: &[Tool], verbose: bool) -> io::Result<()> {
    if prompt.trim().is_empty() {
        return Err(io::Error::new(ErrorKind::InvalidInput, "Prompt cannot be empty"));
    }

    let mut context = app.get_current_context()?;

    // Add user message
    app.add_message(&mut context, "user".to_string(), prompt.clone());

    // Check if we need to warn about context window
    if app.should_warn(&context.messages) && verbose {
        let remaining = app.remaining_tokens(&context.messages);
        eprintln!("[Context window warning: {} tokens remaining]", remaining);
    }

    // Auto-compaction check
    if app.should_auto_compact(&context) {
        return compact_context_with_llm(app, verbose).await;
    }

    // Prepare messages for API
    let system_prompt = app.load_system_prompt()?;
    let context_has_system = context.messages.iter().any(|m| m.role == "system");

    let mut messages: Vec<serde_json::Value> = if !system_prompt.is_empty() && !context_has_system {
        vec![serde_json::json!({
            "role": "system",
            "content": system_prompt,
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

    if !tools.is_empty() {
        request_body["tools"] = serde_json::json!(tools::tools_to_api_format(tools));
    }

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
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;

        if response.status() != StatusCode::OK {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(io::Error::new(
                ErrorKind::Other,
                format!("API error ({}): {}", status, body),
            ));
        }

        let mut stream = response.bytes_stream();
        let mut stdout = stdout();
        let mut full_response = String::new();
        let mut is_first_content = true;

        // Tool call accumulation
        let mut tool_calls: Vec<ToolCallAccumulator> = Vec::new();
        let mut has_tool_calls = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| io::Error::new(ErrorKind::Other, format!("Stream error: {}", e)))?;
            let chunk_str = std::str::from_utf8(&chunk)
                .map_err(|e| io::Error::new(ErrorKind::Other, format!("UTF-8 error: {}", e)))?;

            // Parse Server-Sent Events format
            for line in chunk_str.lines() {
                if line.starts_with("data: ") {
                    let data = &line[6..];
                    if data == "[DONE]" {
                        continue;
                    }

                    let json: serde_json::Value = serde_json::from_str(data)
                        .map_err(|e| io::Error::new(ErrorKind::Other, format!("JSON parse error: {}", e)))?;

                    if let Some(choices) = json["choices"].as_array() {
                        if let Some(choice) = choices.get(0) {
                            if let Some(delta) = choice.get("delta") {
                                // Handle regular content
                                if let Some(content) = delta["content"].as_str() {
                                    if is_first_content {
                                        is_first_content = false;
                                        if content.starts_with('\n') {
                                            let remaining = &content[1..];
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

                let result = if let Some(tool) = tools::find_tool(tools, &tc.name) {
                    let args: serde_json::Value = serde_json::from_str(&tc.arguments)
                        .unwrap_or(serde_json::json!({}));

                    match tools::execute_tool(tool, &args) {
                        Ok(output) => output,
                        Err(e) => format!("Error: {}", e),
                    }
                } else {
                    format!("Error: Unknown tool '{}'", tc.name)
                };

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
        app.add_message(&mut context, "assistant".to_string(), full_response);
        app.save_current_context(&context)?;

        if app.should_warn(&context.messages) && verbose {
            let remaining = app.remaining_tokens(&context.messages);
            eprintln!("[Context window warning: {} tokens remaining]", remaining);
        }

        println!();
        return Ok(());
    }
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}
