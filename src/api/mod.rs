//! LLM API communication module.
//!
//! This module handles all communication with the LLM API:
//! - Building and sending requests
//! - Streaming responses
//! - Tool execution loop
//! - Context compaction

mod compact;
mod logging;
mod request;

use crate::config::ResolvedConfig;
use crate::context::{InboxEntry, now_timestamp};
use crate::llm;
use crate::output::OutputHandler;
use crate::state::AppState;
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use serde_json::json;
use std::io::{self, ErrorKind};
use tokio::io::{AsyncWriteExt, stdout};
use uuid::Uuid;

// Re-export submodule items
pub use compact::{
    compact_context_by_name, compact_context_with_llm, compact_context_with_llm_manual,
};
// rolling_compact is used internally by compact_context_with_llm
pub use request::PromptOptions;

// Internal use from submodules
use logging::{log_request_if_enabled, log_response_meta_if_enabled};
use request::build_request_body;

/// Maximum number of simultaneous tool calls allowed (prevents memory exhaustion from malicious responses)
const MAX_TOOL_CALLS: usize = 100;

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
    let mut request_body = build_request_body(resolved_config, &messages, Some(&all_tools), true);

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
                } else if let Some(builtin_result) =
                    tools::execute_builtin_tool(app, &tc.name, &args)
                {
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
