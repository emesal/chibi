//! Main prompt sending functionality.
//!
//! This module handles sending prompts to the LLM API with streaming support,
//! tool execution, and hook integration. It uses the ResponseSink trait to
//! decouple from presentation concerns.

use super::compact::compact_context_with_llm;
use super::logging::{log_request_if_enabled, log_response_meta_if_enabled};
use super::request::{PromptOptions, build_request_body};
use super::sink::{ResponseEvent, ResponseSink};
use crate::cache;
use crate::config::{ResolvedConfig, ToolsConfig};
use crate::context::{InboxEntry, now_timestamp};
use crate::llm;
use crate::state::{
    AppState, StatePaths, create_assistant_message_entry, create_tool_call_entry,
    create_tool_result_entry, create_user_message_entry,
};
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use serde_json::json;
use std::io::{self, ErrorKind};
use uuid::Uuid;

/// Maximum number of simultaneous tool calls allowed (prevents memory exhaustion from malicious responses)
const MAX_TOOL_CALLS: usize = 100;

/// Tool type classification for pre_api_tools hook
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolType {
    Builtin,
    File,
    Plugin,
}

impl ToolType {
    fn as_str(&self) -> &'static str {
        match self {
            ToolType::Builtin => "builtin",
            ToolType::File => "file",
            ToolType::Plugin => "plugin",
        }
    }
}

/// Built-in tool names (todos, goals, reflection, send_message)
const BUILTIN_TOOL_NAMES: &[&str] = &[
    "update_todos",
    "update_goals",
    "update_reflection",
    "send_message",
];

/// File tool names (file_head, file_tail, file_lines, file_grep, cache_list)
const FILE_TOOL_NAMES: &[&str] = &[
    "file_head",
    "file_tail",
    "file_lines",
    "file_grep",
    "cache_list",
];

/// Classify a tool's type based on its name
fn classify_tool_type(name: &str, plugin_names: &[&str]) -> ToolType {
    if BUILTIN_TOOL_NAMES.contains(&name) {
        ToolType::Builtin
    } else if FILE_TOOL_NAMES.contains(&name) {
        ToolType::File
    } else if plugin_names.contains(&name) {
        ToolType::Plugin
    } else {
        // Unknown tools default to plugin type
        ToolType::Plugin
    }
}

/// Build tool info list for pre_api_tools hook data
fn build_tool_info_list(
    all_tools: &[serde_json::Value],
    plugin_tools: &[Tool],
) -> Vec<serde_json::Value> {
    let plugin_names: Vec<&str> = plugin_tools.iter().map(|t| t.name.as_str()).collect();

    all_tools
        .iter()
        .filter_map(|tool| {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())?;
            let tool_type = classify_tool_type(name, &plugin_names);
            Some(json!({
                "name": name,
                "type": tool_type.as_str(),
            }))
        })
        .collect()
}

/// Filter tools based on config include/exclude lists
fn filter_tools_by_config(
    tools: Vec<serde_json::Value>,
    config: &ToolsConfig,
) -> Vec<serde_json::Value> {
    let mut result = tools;

    // Apply include filter first (if set, only these tools are considered)
    if let Some(ref include) = config.include {
        result.retain(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|name| include.contains(&name.to_string()))
        });
    }

    // Apply exclude filter (remove these tools from remaining)
    if let Some(ref exclude) = config.exclude {
        result.retain(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|name| !exclude.contains(&name.to_string()))
        });
    }

    result
}

/// Filter tools based on hook results
/// Multiple hooks: includes are intersected, excludes are unioned
fn filter_tools_from_hook_results<S: ResponseSink>(
    tools: Vec<serde_json::Value>,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    sink: &mut S,
) -> io::Result<Vec<serde_json::Value>> {
    if hook_results.is_empty() {
        return Ok(tools);
    }

    let mut result = tools;

    // Collect all includes and excludes from hook results
    let mut all_includes: Option<Vec<String>> = None;
    let mut all_excludes: Vec<String> = Vec::new();

    for (hook_name, hook_result) in hook_results {
        // Handle include lists (intersection)
        if let Some(include) = hook_result.get("include").and_then(|v| v.as_array()) {
            let include_names: Vec<String> = include
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();

            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!(
                        "[Hook pre_api_tools: {} include filter: {:?}]",
                        hook_name, include_names
                    ),
                    verbose_only: true,
                })?;
            }

            all_includes = Some(match all_includes {
                Some(existing) => existing
                    .into_iter()
                    .filter(|name| include_names.contains(name))
                    .collect(),
                None => include_names,
            });
        }

        // Handle exclude lists (union)
        if let Some(exclude) = hook_result.get("exclude").and_then(|v| v.as_array()) {
            let exclude_names: Vec<String> = exclude
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();

            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!(
                        "[Hook pre_api_tools: {} exclude filter: {:?}]",
                        hook_name, exclude_names
                    ),
                    verbose_only: true,
                })?;
            }

            for name in exclude_names {
                if !all_excludes.contains(&name) {
                    all_excludes.push(name);
                }
            }
        }
    }

    // Apply collected includes (intersection)
    if let Some(includes) = all_includes {
        result.retain(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|name| includes.contains(&name.to_string()))
        });
    }

    // Apply collected excludes (union)
    if !all_excludes.is_empty() {
        result.retain(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|name| !all_excludes.contains(&name.to_string()))
        });
    }

    Ok(result)
}

/// Apply request modifications from pre_api_request hook results
/// Hook returns are merged (not replaced) into the request body
fn apply_request_modifications<S: ResponseSink>(
    mut request_body: serde_json::Value,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    sink: &mut S,
) -> io::Result<serde_json::Value> {
    for (hook_name, hook_result) in hook_results {
        if let Some(modifications) = hook_result.get("request_body")
            && let Some(mods_obj) = modifications.as_object()
        {
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!(
                        "[Hook pre_api_request: {} modifying request (keys: {:?})]",
                        hook_name,
                        mods_obj.keys().collect::<Vec<_>>()
                    ),
                    verbose_only: true,
                })?;
            }

            // Merge modifications into request body
            if let Some(body_obj) = request_body.as_object_mut() {
                for (key, value) in mods_obj {
                    body_obj.insert(key.clone(), value.clone());
                }
            }
        }
    }

    Ok(request_body)
}

/// Apply fallback override from hook results.
/// Hooks can return `{"fallback": "call_agent"}` or `{"fallback": "call_user"}` to override.
fn apply_fallback_override<S: ResponseSink>(
    handoff: &mut tools::Handoff,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    sink: &mut S,
) -> io::Result<()> {
    for (hook_name, hook_result) in hook_results {
        if let Some(fallback_str) = hook_result.get("fallback").and_then(|v| v.as_str()) {
            let new_fallback = match fallback_str {
                "call_user" => tools::HandoffTarget::User {
                    message: String::new(),
                },
                _ => tools::HandoffTarget::Agent {
                    prompt: String::new(),
                },
            };
            handoff.set_fallback(new_fallback);
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Hook {} set fallback to {}]", hook_name, fallback_str),
                    verbose_only: true,
                })?;
            }
        }
    }
    Ok(())
}

/// Send a prompt to the LLM with streaming response via ResponseSink.
///
/// This is the main entry point for sending prompts. It handles:
/// - Hook execution (pre_message, post_message, etc.)
/// - Inbox message injection
/// - Tool execution loop
/// - Context management
/// - Auto-compaction
///
/// # Arguments
///
/// * `context_name` - The name of the context to use for this prompt
pub async fn send_prompt<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    prompt: String,
    tools: &[Tool],
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
    sink: &mut S,
) -> io::Result<()> {
    send_prompt_with_depth(
        app,
        context_name,
        prompt,
        tools,
        0,
        resolved_config,
        options,
        sink,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn send_prompt_with_depth<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    prompt: String,
    tools: &[Tool],
    recursion_depth: usize,
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
    sink: &mut S,
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

    let mut context = app.get_or_create_context(context_name)?;

    // Execute pre_message hooks (can modify prompt)
    let mut final_prompt = prompt.clone();
    let hook_data = serde_json::json!({
        "prompt": prompt,
        "context_name": context.name,
        "summary": context.summary,
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PreMessage, &hook_data)?;
    for (tool_name, result) in hook_results {
        if let Some(modified) = result.get("prompt").and_then(|v| v.as_str()) {
            if verbose {
                eprintln!("[Hook pre_message: {} modified prompt]", tool_name);
            }
            final_prompt = modified.to_string();
        }
    }

    // Check inbox and inject messages before the user prompt
    let inbox_messages = app.load_and_clear_inbox(context_name)?;
    if !inbox_messages.is_empty() {
        let mut inbox_content = String::from("--- INBOX MESSAGES ---\n");
        for msg in &inbox_messages {
            inbox_content.push_str(&format!("[From: {}] {}\n", msg.from, msg.content));
        }
        inbox_content.push_str("--- END INBOX ---\n\n");
        final_prompt = format!("{}{}", inbox_content, final_prompt);
        if verbose {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[Inbox: {} message(s) injected]", inbox_messages.len()),
                verbose_only: true,
            })?;
        }
    }

    // Add user message to in-memory context
    app.add_message(&mut context, "user".to_string(), final_prompt.clone());

    // Append user message to both transcript.jsonl and context.jsonl (tandem write)
    let user_entry =
        create_user_message_entry(context_name, &final_prompt, &resolved_config.username);
    app.append_to_transcript_and_context(context_name, &user_entry)?;
    sink.handle(ResponseEvent::TranscriptEntry(user_entry))?;

    // Check if we need to warn about context window
    if app.should_warn(&context.messages) {
        let remaining = app.remaining_tokens(&context.messages);
        if verbose {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[Context window warning: {} tokens remaining]", remaining),
                verbose_only: true,
            })?;
        }
    }

    // Auto-compaction check
    if app.should_auto_compact(&context, resolved_config) {
        return compact_context_with_llm(app, context_name, resolved_config, verbose).await;
    }

    // Prepare messages for API
    let system_prompt = app.load_system_prompt_for(context_name)?;
    let reflection_prompt = if use_reflection {
        app.load_reflection_prompt()?
    } else {
        String::new()
    };

    // Load context-specific state: todos, goals, and summary
    let todos = app.load_todos(context_name)?;
    let goals = app.load_goals(context_name)?;
    let summary = &context.summary;

    // Execute pre_system_prompt hook - can inject content before system prompt sections
    let pre_sys_hook_data = serde_json::json!({
        "context_name": context.name,
        "summary": summary,
        "todos": todos,
        "goals": goals,
    });
    let pre_sys_hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreSystemPrompt, &pre_sys_hook_data)?;

    // Build full system prompt with all components
    let mut full_system_prompt = system_prompt.clone();

    // Prepend any content from pre_system_prompt hooks
    for (hook_tool_name, result) in &pre_sys_hook_results {
        if let Some(inject) = result.get("inject").and_then(|v| v.as_str())
            && !inject.is_empty()
        {
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!(
                        "[Hook pre_system_prompt: {} injected content]",
                        hook_tool_name
                    ),
                    verbose_only: true,
                })?;
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
    )?;

    // Append any content from post_system_prompt hooks
    for (hook_tool_name, result) in &post_sys_hook_results {
        if let Some(inject) = result.get("inject").and_then(|v| v.as_str())
            && !inject.is_empty()
        {
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!(
                        "[Hook post_system_prompt: {} injected content]",
                        hook_tool_name
                    ),
                    verbose_only: true,
                })?;
            }
            full_system_prompt.push_str("\n\n");
            full_system_prompt.push_str(inject);
        }
    }

    // Store combined prompt in context_meta for API request reconstruction
    if !full_system_prompt.is_empty() {
        app.save_combined_system_prompt(&context.name, &full_system_prompt)?;
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
    all_tools.push(tools::todos_tool_to_api_format());
    all_tools.push(tools::goals_tool_to_api_format());
    all_tools.push(tools::send_message_tool_to_api_format());

    // Add reflection tool if enabled
    if use_reflection {
        all_tools.push(tools::reflection_tool_to_api_format());
    }

    // Add file/cache access tools
    all_tools.push(tools::file_head_tool_to_api_format());
    all_tools.push(tools::file_tail_tool_to_api_format());
    all_tools.push(tools::file_lines_tool_to_api_format());
    all_tools.push(tools::file_grep_tool_to_api_format());
    all_tools.push(tools::cache_list_tool_to_api_format());

    // Add control flow tools (call_agent/call_user)
    all_tools.push(tools::call_agent_tool_to_api_format());
    all_tools.push(tools::call_user_tool_to_api_format());

    // Apply config-based tool filtering (from local.toml [tools] section)
    all_tools = filter_tools_by_config(all_tools, &resolved_config.tools);

    // Execute pre_api_tools hook - allows plugins to filter tools dynamically
    let tool_info = build_tool_info_list(&all_tools, tools);
    let hook_data = json!({
        "context_name": context.name,
        "tools": tool_info,
        "recursion_depth": recursion_depth,
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PreApiTools, &hook_data)?;
    all_tools = filter_tools_from_hook_results(all_tools, &hook_results, verbose, sink)?;

    // Build request with tools and API params from resolved config
    let mut request_body = build_request_body(resolved_config, &messages, Some(&all_tools), true);

    // Execute pre_api_request hook - allows plugins to modify full request body
    let hook_data = json!({
        "context_name": context.name,
        "request_body": request_body,
        "recursion_depth": recursion_depth,
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PreApiRequest, &hook_data)?;
    request_body = apply_request_modifications(request_body, &hook_results, verbose, sink)?;

    // Determine fallback from config or override
    let fallback = options.fallback_override.clone().unwrap_or_else(|| {
        match resolved_config.fallback_tool.as_str() {
            "call_user" => tools::HandoffTarget::User {
                message: String::new(),
            },
            _ => tools::HandoffTarget::Agent {
                prompt: String::new(),
            },
        }
    });
    let mut handoff = tools::Handoff::new(fallback);

    // Execute pre_agentic_loop hook - allows plugins to override fallback before the loop
    let hook_data = json!({
        "context_name": context.name,
        "recursion_depth": recursion_depth,
        "current_fallback": resolved_config.fallback_tool,
        "message": final_prompt,
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PreAgenticLoop, &hook_data)?;
    apply_fallback_override(&mut handoff, &hook_results, verbose, sink)?;

    // Tool call loop - keep going until we get a final text response
    loop {
        // Log request if debug logging is enabled
        log_request_if_enabled(app, context_name, debug, &request_body);

        let response = llm::send_streaming_request(resolved_config, request_body.clone()).await?;

        let mut stream = response.bytes_stream();
        let mut full_response = String::new();
        let mut is_first_content = true;
        let json_mode = sink.is_json_mode();

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
                                            sink.handle(ResponseEvent::TextChunk(remaining))?;
                                        }
                                    }
                                    continue;
                                }
                            }

                            full_response.push_str(content);
                            // Only stream in normal mode
                            if !json_mode {
                                sink.handle(ResponseEvent::TextChunk(content))?;
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
            log_response_meta_if_enabled(app, context_name, debug, meta);
        }

        // Signal that streaming is finished
        if !json_mode {
            sink.handle(ResponseEvent::Finished)?;
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
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!("[Tool: {}]", tc.name),
                        verbose_only: true,
                    })?;
                }

                sink.handle(ResponseEvent::ToolStart {
                    name: tc.name.clone(),
                })?;

                let mut args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));

                // Check for control flow tools first
                let is_handoff_tool = if tc.name == tools::CALL_AGENT_TOOL_NAME {
                    let prompt = args["prompt"].as_str().unwrap_or("").to_string();
                    handoff.set_agent(prompt);
                    true
                } else if tc.name == tools::CALL_USER_TOOL_NAME {
                    let message = args["message"].as_str().unwrap_or("").to_string();
                    handoff.set_user(message);
                    true
                } else if let Some(note) = tools::check_recurse_signal(&tc.name, &args) {
                    // Backwards compat: treat external recurse plugin as call_agent
                    handoff.set_agent(note);
                    true
                } else {
                    false
                };

                // Execute pre_tool hooks (can modify arguments OR block execution)
                let pre_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                });
                let pre_hook_results =
                    tools::execute_hook(tools, tools::HookPoint::PreTool, &pre_hook_data)?;

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
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[Hook pre_tool: {} blocked {} - {}]",
                                    hook_tool_name, tc.name, block_message
                                ),
                                verbose_only: true,
                            })?;
                        }
                        break;
                    }

                    // Check for argument modification
                    if let Some(modified_args) = result.get("arguments") {
                        if verbose {
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[Hook pre_tool: {} modified arguments for {}]",
                                    hook_tool_name, tc.name
                                ),
                                verbose_only: true,
                            })?;
                        }
                        args = modified_args.clone();
                    }
                }

                // If blocked, skip execution and use block message as result
                let tool_result = if blocked {
                    block_message
                } else if is_handoff_tool {
                    // Handoff tools don't execute - they just set the handoff target
                    if tc.name == tools::CALL_AGENT_TOOL_NAME {
                        let prompt = args["prompt"].as_str().unwrap_or("");
                        if prompt.is_empty() {
                            "Continuing processing".to_string()
                        } else {
                            format!("Continuing with: {}", prompt)
                        }
                    } else if tc.name == tools::CALL_USER_TOOL_NAME {
                        let message = args["message"].as_str().unwrap_or("");
                        if message.is_empty() {
                            "Returning to user".to_string()
                        } else {
                            message.to_string()
                        }
                    } else {
                        // recurse tool backwards compat
                        "Continuing...".to_string()
                    }
                } else if tc.name == tools::REFLECTION_TOOL_NAME && !use_reflection {
                    "Error: Reflection tool is not enabled".to_string()
                } else if let Some(builtin_result) =
                    tools::execute_builtin_tool(app, context_name, &tc.name, &args)
                {
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
                        // Execute pre_send_message hooks
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
                                if verbose {
                                    sink.handle(ResponseEvent::Diagnostic {
                                        message: format!(
                                            "[Hook pre_send_message: {} intercepted delivery]",
                                            hook_tool_name
                                        ),
                                        verbose_only: true,
                                    })?;
                                }
                                break;
                            }
                        }

                        let delivery_result = if let Some(via) = delivered_via {
                            format!("Message delivered to '{}' via {}", to, via)
                        } else {
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

                        // Execute post_send_message hooks
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
                        );

                        delivery_result
                    }
                } else if tools::is_file_tool(&tc.name) {
                    match tools::execute_file_tool(
                        app,
                        &context.name,
                        &tc.name,
                        &args,
                        resolved_config,
                    ) {
                        Some(Ok(r)) => r,
                        Some(Err(e)) => format!("Error: {}", e),
                        None => format!("Error: Unknown file tool '{}'", tc.name),
                    }
                } else if let Some(tool) = tools::find_tool(tools, &tc.name) {
                    match tools::execute_tool(tool, &args, verbose) {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                } else {
                    format!("Error: Unknown tool '{}'", tc.name)
                };

                // Execute pre_tool_output hooks (can modify or replace output)
                let mut tool_result = tool_result;
                let pre_output_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "output": tool_result,
                });
                let pre_output_hook_results = tools::execute_hook(
                    tools,
                    tools::HookPoint::PreToolOutput,
                    &pre_output_hook_data,
                )?;

                for (hook_tool_name, result) in pre_output_hook_results {
                    if result
                        .get("block")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        let replacement = result
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Output blocked by hook")
                            .to_string();
                        if verbose {
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[Hook pre_tool_output: {} blocked output from {}]",
                                    hook_tool_name, tc.name
                                ),
                                verbose_only: true,
                            })?;
                        }
                        tool_result = replacement;
                        break;
                    }

                    if let Some(modified_output) = result.get("output").and_then(|v| v.as_str()) {
                        if verbose {
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[Hook pre_tool_output: {} modified output from {}]",
                                    hook_tool_name, tc.name
                                ),
                                verbose_only: true,
                            })?;
                        }
                        tool_result = modified_output.to_string();
                    }
                }

                // Check if output should be cached
                let (final_result, was_cached) = if !tool_result.starts_with("Error:")
                    && cache::should_cache(
                        &tool_result,
                        resolved_config.tool_output_cache_threshold,
                    ) {
                    let cache_dir = app.tool_cache_dir(&context.name);
                    match cache::cache_output(&cache_dir, &tc.name, &tool_result, &args) {
                        Ok(entry) => {
                            match cache::generate_truncated_message(
                                &entry,
                                resolved_config.tool_cache_preview_chars,
                            ) {
                                Ok(truncated) => {
                                    if verbose {
                                        sink.handle(ResponseEvent::Diagnostic {
                                            message: format!(
                                                "[Cached {} chars from {} as {}]",
                                                tool_result.len(),
                                                tc.name,
                                                entry.metadata.id
                                            ),
                                            verbose_only: true,
                                        })?;
                                    }
                                    (truncated, true)
                                }
                                Err(_) => (tool_result.clone(), false),
                            }
                        }
                        Err(e) => {
                            if verbose {
                                sink.handle(ResponseEvent::Diagnostic {
                                    message: format!("[Failed to cache output: {}]", e),
                                    verbose_only: true,
                                })?;
                            }
                            (tool_result.clone(), false)
                        }
                    }
                } else {
                    (tool_result.clone(), false)
                };

                // Execute post_tool_output hooks
                let post_output_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "output": tool_result,
                    "final_output": final_result,
                    "cached": was_cached,
                });
                let _ = tools::execute_hook(
                    tools,
                    tools::HookPoint::PostToolOutput,
                    &post_output_hook_data,
                );

                // Log tool call and result
                let tool_call_entry = create_tool_call_entry(context_name, &tc.name, &tc.arguments);
                app.append_to_transcript_and_context(context_name, &tool_call_entry)?;
                sink.handle(ResponseEvent::TranscriptEntry(tool_call_entry))?;

                let logged_result = if was_cached {
                    &final_result
                } else {
                    &tool_result
                };
                let tool_result_entry =
                    create_tool_result_entry(context_name, &tc.name, logged_result);
                app.append_to_transcript_and_context(context_name, &tool_result_entry)?;
                sink.handle(ResponseEvent::TranscriptEntry(tool_result_entry))?;

                sink.handle(ResponseEvent::ToolResult {
                    name: tc.name.clone(),
                    result: final_result.clone(),
                    cached: was_cached,
                })?;

                // Execute post_tool hooks
                let post_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "result": tool_result,
                    "cached": was_cached,
                });
                let _ = tools::execute_hook(tools, tools::HookPoint::PostTool, &post_hook_data);

                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tc.id,
                    "content": final_result,
                }));
            }

            // Execute post_tool_batch hook - allows plugins to override fallback after seeing tool results
            let tool_batch_info: Vec<serde_json::Value> = tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "name": tc.name,
                        "arguments": serde_json::from_str::<serde_json::Value>(&tc.arguments).unwrap_or(json!({})),
                    })
                })
                .collect();
            let hook_data = json!({
                "context_name": context.name,
                "recursion_depth": recursion_depth,
                "current_fallback": resolved_config.fallback_tool,
                "tool_calls": tool_batch_info,
            });
            let hook_results =
                tools::execute_hook(tools, tools::HookPoint::PostToolBatch, &hook_data)?;
            apply_fallback_override(&mut handoff, &hook_results, verbose, sink)?;

            request_body["messages"] = serde_json::json!(messages);
            continue;
        }

        // No tool calls - we have a final response
        app.add_message(&mut context, "assistant".to_string(), full_response.clone());

        let assistant_entry = create_assistant_message_entry(context_name, &full_response);
        app.append_to_transcript_and_context(context_name, &assistant_entry)?;
        sink.handle(ResponseEvent::TranscriptEntry(assistant_entry))?;

        // Execute post_message hooks
        let hook_data = serde_json::json!({
            "prompt": final_prompt,
            "response": full_response,
            "context_name": context.name,
        });
        let _ = tools::execute_hook(tools, tools::HookPoint::PostMessage, &hook_data);

        if app.should_warn(&context.messages) {
            let remaining = app.remaining_tokens(&context.messages);
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Context window warning: {} tokens remaining]", remaining),
                    verbose_only: true,
                })?;
            }
        }

        sink.handle(ResponseEvent::Newline)?;

        // Determine next action based on handoff
        match handoff.take() {
            tools::HandoffTarget::User { message } => {
                if !message.is_empty() {
                    // Output the message as final text
                    sink.handle(ResponseEvent::TextChunk(&message))?;
                    sink.handle(ResponseEvent::Newline)?;
                }
                return Ok(());
            }
            tools::HandoffTarget::Agent { prompt } => {
                let new_depth = recursion_depth + 1;
                if new_depth >= app.config.max_recursion_depth {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[Max recursion depth ({}) reached, stopping]",
                            app.config.max_recursion_depth
                        ),
                        verbose_only: false,
                    })?;
                    return Ok(());
                }
                if verbose {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[Continuing processing ({}/{}): {}]",
                            new_depth,
                            app.config.max_recursion_depth,
                            if prompt.is_empty() {
                                "(no prompt)"
                            } else {
                                &prompt
                            }
                        ),
                        verbose_only: true,
                    })?;
                }
                let continue_prompt = if prompt.is_empty() {
                    "[Continuing from previous round]".to_string()
                } else {
                    format!("[Continuing from previous round]\n\n{}", prompt)
                };
                return Box::pin(send_prompt_with_depth(
                    app,
                    context_name,
                    continue_prompt,
                    tools,
                    new_depth,
                    resolved_config,
                    options,
                    sink,
                ))
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_tool_type_builtin() {
        let plugin_names: Vec<&str> = vec!["my_plugin"];
        assert_eq!(
            classify_tool_type("update_todos", &plugin_names),
            ToolType::Builtin
        );
        assert_eq!(
            classify_tool_type("update_goals", &plugin_names),
            ToolType::Builtin
        );
    }

    #[test]
    fn test_classify_tool_type_file() {
        let plugin_names: Vec<&str> = vec!["my_plugin"];
        assert_eq!(
            classify_tool_type("file_head", &plugin_names),
            ToolType::File
        );
        assert_eq!(
            classify_tool_type("cache_list", &plugin_names),
            ToolType::File
        );
    }

    #[test]
    fn test_classify_tool_type_plugin() {
        let plugin_names: Vec<&str> = vec!["my_plugin"];
        assert_eq!(
            classify_tool_type("my_plugin", &plugin_names),
            ToolType::Plugin
        );
    }

    #[test]
    fn test_classify_tool_type_unknown_defaults_to_plugin() {
        let plugin_names: Vec<&str> = vec!["my_plugin"];
        assert_eq!(
            classify_tool_type("unknown_tool", &plugin_names),
            ToolType::Plugin
        );
    }

    #[test]
    fn test_tool_type_as_str() {
        assert_eq!(ToolType::Builtin.as_str(), "builtin");
        assert_eq!(ToolType::File.as_str(), "file");
        assert_eq!(ToolType::Plugin.as_str(), "plugin");
    }

    #[test]
    fn test_filter_tools_by_config_no_filters() {
        let tools = vec![
            json!({"function": {"name": "tool1"}}),
            json!({"function": {"name": "tool2"}}),
        ];
        let config = ToolsConfig::default();
        let result = filter_tools_by_config(tools.clone(), &config);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tools_by_config_include() {
        let tools = vec![
            json!({"function": {"name": "tool1"}}),
            json!({"function": {"name": "tool2"}}),
            json!({"function": {"name": "tool3"}}),
        ];
        let config = ToolsConfig {
            include: Some(vec!["tool1".to_string(), "tool3".to_string()]),
            exclude: None,
        };
        let result = filter_tools_by_config(tools, &config);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tools_by_config_exclude() {
        let tools = vec![
            json!({"function": {"name": "tool1"}}),
            json!({"function": {"name": "tool2"}}),
            json!({"function": {"name": "tool3"}}),
        ];
        let config = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool2".to_string()]),
        };
        let result = filter_tools_by_config(tools, &config);
        assert_eq!(result.len(), 2);
    }
}
