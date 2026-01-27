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

use crate::cache;
use crate::config::{ResolvedConfig, ToolsConfig};
use crate::context::{InboxEntry, now_timestamp};
use crate::llm;
use crate::markdown::MarkdownStream;
use crate::output::OutputHandler;
use crate::state::AppState;
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
use serde_json::json;
use std::io::{self, ErrorKind};
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
fn filter_tools_from_hook_results(
    tools: Vec<serde_json::Value>,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    output: &OutputHandler,
) -> Vec<serde_json::Value> {
    if hook_results.is_empty() {
        return tools;
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

            output.diagnostic(
                &format!(
                    "[Hook pre_api_tools: {} include filter: {:?}]",
                    hook_name, include_names
                ),
                verbose,
            );

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

            output.diagnostic(
                &format!(
                    "[Hook pre_api_tools: {} exclude filter: {:?}]",
                    hook_name, exclude_names
                ),
                verbose,
            );

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

    result
}

/// Apply request modifications from pre_api_request hook results
/// Hook returns are merged (not replaced) into the request body
fn apply_request_modifications(
    mut request_body: serde_json::Value,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    output: &OutputHandler,
) -> serde_json::Value {
    for (hook_name, hook_result) in hook_results {
        if let Some(modifications) = hook_result.get("request_body")
            && let Some(mods_obj) = modifications.as_object()
        {
            output.diagnostic(
                &format!(
                    "[Hook pre_api_request: {} modifying request (keys: {:?})]",
                    hook_name,
                    mods_obj.keys().collect::<Vec<_>>()
                ),
                verbose,
            );

            // Merge modifications into request body
            if let Some(body_obj) = request_body.as_object_mut() {
                for (key, value) in mods_obj {
                    body_obj.insert(key.clone(), value.clone());
                }
            }
        }
    }

    request_body
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
    // Note: recurse tool is now external (loaded from tools directory)
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

    // Apply config-based tool filtering (from local.toml [tools] section)
    all_tools = filter_tools_by_config(all_tools, &resolved_config.tools);

    // Execute pre_api_tools hook - allows plugins to filter tools dynamically
    let tool_info = build_tool_info_list(&all_tools, tools);
    let hook_data = json!({
        "context_name": context.name,
        "tools": tool_info,
        "recursion_depth": recursion_depth,
    });
    let hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreApiTools, &hook_data, verbose)?;
    all_tools = filter_tools_from_hook_results(all_tools, &hook_results, verbose, output);

    // Build request with tools and API params from resolved config
    let mut request_body = build_request_body(resolved_config, &messages, Some(&all_tools), true);

    // Execute pre_api_request hook - allows plugins to modify full request body
    let hook_data = json!({
        "context_name": context.name,
        "request_body": request_body,
        "recursion_depth": recursion_depth,
    });
    let hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreApiRequest, &hook_data, verbose)?;
    request_body = apply_request_modifications(request_body, &hook_results, verbose, output);

    // Track if we should recurse (continue_processing was called)
    let mut should_recurse = false;
    let mut recurse_note = String::new();

    // Tool call loop - keep going until we get a final text response
    loop {
        // Log request if debug logging is enabled
        log_request_if_enabled(app, debug, &request_body);

        let response = llm::send_streaming_request(resolved_config, request_body.clone()).await?;

        let mut stream = response.bytes_stream();
        let mut md = MarkdownStream::new(crate::markdown::MarkdownConfig {
            render_markdown: resolved_config.render_markdown,
            render_images: resolved_config.render_images,
            image_max_download_bytes: resolved_config.image_max_download_bytes,
            image_fetch_timeout_seconds: resolved_config.image_fetch_timeout_seconds,
            image_allow_http: resolved_config.image_allow_http,
            image_max_height_lines: resolved_config.image_max_height_lines,
            image_max_width_percent: resolved_config.image_max_width_percent,
            image_alignment: resolved_config.image_alignment.clone(),
            image_render_mode: resolved_config.image_render_mode.clone(),
            image_enable_truecolor: resolved_config.image_enable_truecolor,
            image_enable_ansi: resolved_config.image_enable_ansi,
            image_enable_ascii: resolved_config.image_enable_ascii,
            image_cache_dir: if resolved_config.image_cache_enabled {
                Some(app.chibi_dir.join("image_cache"))
            } else {
                None
            },
            image_cache_max_bytes: resolved_config.image_cache_max_bytes,
            image_cache_max_age_days: resolved_config.image_cache_max_age_days,
        });
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
                                            md.write_chunk(remaining)?;
                                        }
                                    }
                                    continue;
                                }
                            }

                            full_response.push_str(content);
                            // Only stream in normal mode
                            if !json_mode {
                                md.write_chunk(content)?;
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

        // Flush any remaining markdown buffer
        if !json_mode {
            md.finish()?;
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
                } else if tools::is_file_tool(&tc.name) {
                    // Handle file/cache access tools
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

                // Check if output should be cached (for non-error results exceeding threshold)
                // All tools are subject to caching - no exceptions
                let (final_result, was_cached) = if !tool_result.starts_with("Error:")
                    && cache::should_cache(
                        &tool_result,
                        resolved_config.tool_output_cache_threshold,
                    ) {
                    // Cache the large output
                    let cache_dir = app.tool_cache_dir(&context.name);
                    match cache::cache_output(&cache_dir, &tc.name, &tool_result, &args) {
                        Ok(entry) => {
                            match cache::generate_truncated_message(
                                &entry,
                                resolved_config.tool_cache_preview_chars,
                            ) {
                                Ok(truncated) => {
                                    output.diagnostic(
                                        &format!(
                                            "[Cached {} chars from {} as {}]",
                                            tool_result.len(),
                                            tc.name,
                                            entry.metadata.id
                                        ),
                                        verbose,
                                    );
                                    (truncated, true)
                                }
                                Err(_) => (tool_result.clone(), false),
                            }
                        }
                        Err(e) => {
                            output.diagnostic(&format!("[Failed to cache output: {}]", e), verbose);
                            (tool_result.clone(), false)
                        }
                    }
                } else {
                    (tool_result.clone(), false)
                };

                // Log tool call and result to both transcript.jsonl and context.jsonl
                // Note: We log the original tool_result to transcript, but use final_result for API
                let tool_call_entry = app.create_tool_call_entry(&tc.name, &tc.arguments);
                app.append_to_current_transcript_and_context(&tool_call_entry)?;
                output.emit(&tool_call_entry)?;

                // Log original or truncated result based on caching
                let logged_result = if was_cached {
                    &final_result
                } else {
                    &tool_result
                };
                let tool_result_entry = app.create_tool_result_entry(&tc.name, logged_result);
                app.append_to_current_transcript_and_context(&tool_result_entry)?;
                output.emit(&tool_result_entry)?;

                // Execute post_tool hooks (observe only)
                let post_hook_data = serde_json::json!({
                    "tool_name": tc.name,
                    "arguments": args,
                    "result": tool_result,  // Pass original result to hooks
                    "cached": was_cached,
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
                    "content": final_result,  // Use truncated message if cached
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
        assert_eq!(
            classify_tool_type("update_reflection", &plugin_names),
            ToolType::Builtin
        );
        assert_eq!(
            classify_tool_type("send_message", &plugin_names),
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
            classify_tool_type("file_tail", &plugin_names),
            ToolType::File
        );
        assert_eq!(
            classify_tool_type("file_lines", &plugin_names),
            ToolType::File
        );
        assert_eq!(
            classify_tool_type("file_grep", &plugin_names),
            ToolType::File
        );
        assert_eq!(
            classify_tool_type("cache_list", &plugin_names),
            ToolType::File
        );
    }

    #[test]
    fn test_classify_tool_type_plugin() {
        let plugin_names: Vec<&str> = vec!["my_plugin", "other_plugin"];
        assert_eq!(
            classify_tool_type("my_plugin", &plugin_names),
            ToolType::Plugin
        );
        assert_eq!(
            classify_tool_type("other_plugin", &plugin_names),
            ToolType::Plugin
        );
    }

    #[test]
    fn test_classify_tool_type_unknown_defaults_to_plugin() {
        let plugin_names: Vec<&str> = vec!["known_plugin"];
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
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
        ];
        let config = ToolsConfig::default();
        let result = filter_tools_by_config(tools.clone(), &config);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tools_by_config_include() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let config = ToolsConfig {
            include: Some(vec!["tool_a".to_string(), "tool_c".to_string()]),
            exclude: None,
        };
        let result = filter_tools_by_config(tools, &config);
        assert_eq!(result.len(), 2);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_c"));
        assert!(!names.contains(&"tool_b"));
    }

    #[test]
    fn test_filter_tools_by_config_exclude() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let config = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool_b".to_string()]),
        };
        let result = filter_tools_by_config(tools, &config);
        assert_eq!(result.len(), 2);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_c"));
        assert!(!names.contains(&"tool_b"));
    }

    #[test]
    fn test_filter_tools_by_config_include_and_exclude() {
        // Exclude takes effect after include
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let config = ToolsConfig {
            include: Some(vec![
                "tool_a".to_string(),
                "tool_b".to_string(),
                "tool_c".to_string(),
            ]),
            exclude: Some(vec!["tool_b".to_string()]),
        };
        let result = filter_tools_by_config(tools, &config);
        assert_eq!(result.len(), 2);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_c"));
    }

    #[test]
    fn test_filter_tools_from_hook_results_empty() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
        ];
        let hook_results: Vec<(String, serde_json::Value)> = vec![];
        let output = OutputHandler::new(false);
        let result = filter_tools_from_hook_results(tools.clone(), &hook_results, false, &output);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tools_from_hook_results_exclude() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let hook_results = vec![("test_hook".to_string(), json!({"exclude": ["tool_b"]}))];
        let output = OutputHandler::new(false);
        let result = filter_tools_from_hook_results(tools, &hook_results, false, &output);
        assert_eq!(result.len(), 2);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_c"));
    }

    #[test]
    fn test_filter_tools_from_hook_results_include() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let hook_results = vec![(
            "test_hook".to_string(),
            json!({"include": ["tool_a", "tool_c"]}),
        )];
        let output = OutputHandler::new(false);
        let result = filter_tools_from_hook_results(tools, &hook_results, false, &output);
        assert_eq!(result.len(), 2);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_c"));
    }

    #[test]
    fn test_filter_tools_multiple_hooks_excludes_union() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let hook_results = vec![
            ("hook1".to_string(), json!({"exclude": ["tool_a"]})),
            ("hook2".to_string(), json!({"exclude": ["tool_b"]})),
        ];
        let output = OutputHandler::new(false);
        let result = filter_tools_from_hook_results(tools, &hook_results, false, &output);
        assert_eq!(result.len(), 1);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_c"));
    }

    #[test]
    fn test_filter_tools_multiple_hooks_includes_intersect() {
        let tools = vec![
            json!({"function": {"name": "tool_a"}}),
            json!({"function": {"name": "tool_b"}}),
            json!({"function": {"name": "tool_c"}}),
        ];
        let hook_results = vec![
            (
                "hook1".to_string(),
                json!({"include": ["tool_a", "tool_b"]}),
            ),
            (
                "hook2".to_string(),
                json!({"include": ["tool_a", "tool_c"]}),
            ),
        ];
        let output = OutputHandler::new(false);
        let result = filter_tools_from_hook_results(tools, &hook_results, false, &output);
        assert_eq!(result.len(), 1);

        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        assert!(names.contains(&"tool_a"));
    }

    #[test]
    fn test_apply_request_modifications_empty() {
        let request_body = json!({
            "model": "test-model",
            "temperature": 0.5,
        });
        let hook_results: Vec<(String, serde_json::Value)> = vec![];
        let output = OutputHandler::new(false);
        let result =
            apply_request_modifications(request_body.clone(), &hook_results, false, &output);
        assert_eq!(result, request_body);
    }

    #[test]
    fn test_apply_request_modifications_merge() {
        let request_body = json!({
            "model": "test-model",
            "temperature": 0.5,
        });
        let hook_results = vec![(
            "test_hook".to_string(),
            json!({"request_body": {"temperature": 0.8, "max_tokens": 1000}}),
        )];
        let output = OutputHandler::new(false);
        let result = apply_request_modifications(request_body, &hook_results, false, &output);

        assert_eq!(result["model"], "test-model");
        assert_eq!(result["temperature"], 0.8);
        assert_eq!(result["max_tokens"], 1000);
    }

    #[test]
    fn test_apply_request_modifications_multiple_hooks() {
        let request_body = json!({
            "model": "test-model",
            "temperature": 0.5,
        });
        let hook_results = vec![
            (
                "hook1".to_string(),
                json!({"request_body": {"temperature": 0.7}}),
            ),
            (
                "hook2".to_string(),
                json!({"request_body": {"max_tokens": 500}}),
            ),
        ];
        let output = OutputHandler::new(false);
        let result = apply_request_modifications(request_body, &hook_results, false, &output);

        assert_eq!(result["model"], "test-model");
        assert_eq!(result["temperature"], 0.7);
        assert_eq!(result["max_tokens"], 500);
    }

    #[test]
    fn test_build_tool_info_list() {
        use crate::tools::Tool;
        use std::path::PathBuf;

        let all_tools = vec![
            json!({"function": {"name": "update_todos"}}),
            json!({"function": {"name": "file_head"}}),
            json!({"function": {"name": "my_plugin"}}),
        ];
        let plugin_tools = vec![Tool {
            name: "my_plugin".to_string(),
            description: "test".to_string(),
            parameters: json!({}),
            path: PathBuf::from("/test"),
            hooks: vec![],
        }];

        let result = build_tool_info_list(&all_tools, &plugin_tools);
        assert_eq!(result.len(), 3);

        // Find each tool and verify its type
        let todos = result.iter().find(|t| t["name"] == "update_todos").unwrap();
        assert_eq!(todos["type"], "builtin");

        let file_head = result.iter().find(|t| t["name"] == "file_head").unwrap();
        assert_eq!(file_head["type"], "file");

        let plugin = result.iter().find(|t| t["name"] == "my_plugin").unwrap();
        assert_eq!(plugin["type"], "plugin");
    }
}
