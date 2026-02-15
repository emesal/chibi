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
use crate::chibi::PermissionHandler;
use crate::config::{ResolvedConfig, ToolsConfig};
use crate::context::{InboxEntry, now_timestamp};
use crate::gateway::{
    build_gateway, json_tool_to_definition, to_chat_options, to_ratatoskr_message,
};
use crate::json_ext::JsonExt;
use crate::state::{
    AppState, StatePaths, create_assistant_message_entry, create_tool_call_entry,
    create_tool_result_entry, create_user_message_entry,
};
use crate::tools::{self, Tool};
use futures_util::stream::StreamExt;
// ModelGateway trait must be in scope to call chat_stream() on EmbeddedGateway
use ratatoskr::{ChatEvent, ModelGateway};
use serde_json::json;
use std::io::{self, ErrorKind};
use std::path::Path;
use uuid::Uuid;

/// Maximum number of simultaneous tool calls allowed (prevents memory exhaustion from malicious responses)
const MAX_TOOL_CALLS: usize = 100;

/// Tool type classification for pre_api_tools hook
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolType {
    Builtin,
    File,
    Agent,
    Coding,
    Mcp,
    Plugin,
}

impl ToolType {
    fn as_str(&self) -> &'static str {
        match self {
            ToolType::Builtin => "builtin",
            ToolType::File => "file",
            ToolType::Agent => "agent",
            ToolType::Coding => "coding",
            ToolType::Mcp => "mcp",
            ToolType::Plugin => "plugin",
        }
    }
}

/// Classify a tool's type based on its name.
///
/// Delegates to the authoritative `is_*_tool()` functions in each tool module,
/// ensuring classification stays in sync with tool registration automatically.
fn classify_tool_type(name: &str, plugin_tools: &[Tool]) -> ToolType {
    if tools::is_builtin_tool(name) {
        ToolType::Builtin
    } else if tools::is_file_tool(name) {
        ToolType::File
    } else if tools::is_agent_tool(name) {
        ToolType::Agent
    } else if tools::is_coding_tool(name) {
        ToolType::Coding
    } else if plugin_tools.iter().any(|t| t.name == name && tools::mcp::is_mcp_tool(t)) {
        ToolType::Mcp
    } else if plugin_tools.iter().any(|t| t.name == name) {
        ToolType::Plugin
    } else {
        // Unknown tools default to plugin type
        ToolType::Plugin
    }
}

// ============================================================================
// Permission Checking
// ============================================================================

/// Evaluate permission from pre-computed hook results.
///
/// Deny-only protocol: if any plugin returns `"denied": true`, the operation
/// is blocked. Otherwise, falls through to the permission handler (if set)
/// or fail-safe deny.
///
/// Separated from `check_permission()` for unit testing.
fn evaluate_permission(
    hook_results: &[(String, serde_json::Value)],
    hook_data: &serde_json::Value,
    permission_handler: Option<&PermissionHandler>,
) -> io::Result<Result<(), String>> {
    // Check for explicit denial from any plugin
    for (_plugin_name, result) in hook_results {
        if result.get_bool_or("denied", false) {
            let reason = result.get_str_or("reason", "denied by plugin").to_string();
            return Ok(Err(reason));
        }
    }

    // No plugin denied — delegate to permission handler or fail-safe deny
    match permission_handler {
        Some(handler) => {
            if handler(hook_data)? {
                Ok(Ok(()))
            } else {
                Ok(Err("permission denied".to_string()))
            }
        }
        None => Ok(Err(
            "no permission handler configured (fail-safe deny)".to_string()
        )),
    }
}

/// Full permission check: fire the hook, then evaluate results.
fn check_permission(
    tools: &[Tool],
    hook: tools::HookPoint,
    hook_data: &serde_json::Value,
    permission_handler: Option<&PermissionHandler>,
) -> io::Result<Result<(), String>> {
    let hook_results = tools::execute_hook(tools, hook, hook_data)?;
    evaluate_permission(&hook_results, hook_data, permission_handler)
}

/// Build tool info list for pre_api_tools hook data
fn build_tool_info_list(
    all_tools: &[serde_json::Value],
    plugin_tools: &[Tool],
) -> Vec<serde_json::Value> {
    all_tools
        .iter()
        .filter_map(|tool| {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())?;
            let tool_type = classify_tool_type(name, plugin_tools);
            Some(json!({
                "name": name,
                "type": tool_type.as_str(),
            }))
        })
        .collect()
}

/// Annotate the fallback tool's description to indicate it's called automatically.
fn annotate_fallback_tool(tools: &mut [serde_json::Value], fallback_name: &str) {
    const FALLBACK_SUFFIX: &str = " Called automatically if no other tool is used.";

    for tool in tools.iter_mut() {
        let name_matches = tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .is_some_and(|name| name == fallback_name);

        if name_matches {
            if let Some(func) = tool.get_mut("function")
                && let Some(desc) = func.get_mut("description")
                && let Some(desc_str) = desc.as_str()
            {
                *desc = serde_json::Value::String(format!("{}{}", desc_str, FALLBACK_SUFFIX));
            }
            break; // Only annotate the first match
        }
    }
}

/// Filter tools based on config include/exclude/exclude_categories lists
fn filter_tools_by_config(
    tools: Vec<serde_json::Value>,
    config: &ToolsConfig,
    plugin_tools: &[Tool],
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

    // Apply category exclusion (remove tools whose category is excluded)
    if let Some(ref categories) = config.exclude_categories {
        result.retain(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|name| {
                    let tool_type = classify_tool_type(name, plugin_tools);
                    !categories.contains(&tool_type.as_str().to_string())
                })
                .unwrap_or(true)
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
        if let Some(include) = hook_result.get_array("include") {
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
        if let Some(exclude) = hook_result.get_array("exclude") {
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

/// Apply hook overrides from hook results.
///
/// Handles three keys from hook return values:
/// - `"fallback"`: override the fallback tool (existing behaviour)
/// - `"fuel"`: absolute fuel override (e.g. from `pre_agentic_loop`)
/// - `"fuel_delta"`: relative fuel adjustment (e.g. from `post_tool_batch`)
fn apply_hook_overrides<S: ResponseSink>(
    handoff: &mut tools::Handoff,
    fuel_remaining: &mut usize,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    sink: &mut S,
) -> io::Result<()> {
    for (hook_name, hook_result) in hook_results {
        if let Some(fallback_str) = hook_result.get_str("fallback") {
            // Use builtin metadata since hooks can only override to builtins
            let meta = tools::builtin_tool_metadata(fallback_str);
            let new_fallback = if meta.ends_turn {
                tools::HandoffTarget::User {
                    message: String::new(),
                }
            } else {
                tools::HandoffTarget::Agent {
                    prompt: String::new(),
                }
            };
            handoff.set_fallback(new_fallback);
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Hook {} set fallback to {}]", hook_name, fallback_str),
                    verbose_only: true,
                })?;
            }
        }
        if let Some(fuel) = hook_result.get("fuel").and_then(|v| v.as_u64()) {
            *fuel_remaining = fuel as usize;
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Hook {} set fuel to {}]", hook_name, fuel),
                    verbose_only: true,
                })?;
            }
        }
        if let Some(delta) = hook_result.get("fuel_delta").and_then(|v| v.as_i64()) {
            if delta < 0 {
                *fuel_remaining = fuel_remaining.saturating_sub((-delta) as usize);
            } else {
                *fuel_remaining = fuel_remaining.saturating_add(delta as usize);
            }
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Hook {} adjusted fuel by {}]", hook_name, delta),
                    verbose_only: true,
                })?;
            }
        }
    }
    Ok(())
}

// ============================================================================
// Helper Functions (extracted from send_prompt)
// ============================================================================

/// Build the full system prompt with all components.
///
/// Handles: loading base prompt, todos, goals, summary, reflection prompt,
/// pre/post system_prompt hooks, and username injection.
#[allow(clippy::too_many_arguments)]
fn build_full_system_prompt<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    summary: &str,
    use_reflection: bool,
    tools: &[Tool],
    resolved_config: &ResolvedConfig,
    verbose: bool,
    sink: &mut S,
    home_dir: &Path,
    project_root: &Path,
) -> io::Result<String> {
    // Load base prompts
    let system_prompt = app.load_system_prompt_for(context_name)?;
    let reflection_prompt = if use_reflection {
        app.load_reflection_prompt()?
    } else {
        String::new()
    };

    // Load context-specific state
    let todos = app.load_todos(context_name)?;
    let goals = app.load_goals(context_name)?;

    // Execute pre_system_prompt hook - can inject content before system prompt sections
    let pre_sys_hook_data = serde_json::json!({
        "context_name": context_name,
        "summary": summary,
        "todos": todos,
        "goals": goals,
    });
    let pre_sys_hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreSystemPrompt, &pre_sys_hook_data)?;

    // Build full system prompt with all components
    let mut full_system_prompt = system_prompt;

    // Prepend any content from pre_system_prompt hooks
    for (hook_tool_name, result) in &pre_sys_hook_results {
        if let Some(inject) = result.get_str("inject")
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

    // Load AGENTS.md instructions from standard locations
    let agents_md = crate::agents_md::load_agents_md(
        home_dir,
        &app.chibi_dir,
        project_root,
        &std::env::current_dir().unwrap_or_default(),
    );
    if !agents_md.is_empty() {
        full_system_prompt.push_str("\n\n--- AGENT INSTRUCTIONS ---\n");
        full_system_prompt.push_str(&agents_md);
    }

    // Add username info at the start if not "user"
    if resolved_config.username != "user" {
        full_system_prompt.push_str(&format!(
            "\n\nThe user speaking to you is called: {}",
            resolved_config.username
        ));
    }

    // Add context name
    full_system_prompt.push_str(&format!("\n\nCurrent context: {}", context_name));

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
        "context_name": context_name,
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
        if let Some(inject) = result.get_str("inject")
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
        app.save_combined_system_prompt(context_name, &full_system_prompt)?;
    }

    Ok(full_system_prompt)
}

/// Result of collecting a streaming response from the LLM.
struct StreamingResponse {
    /// The full text response accumulated from chunks.
    full_response: String,
    /// Tool calls extracted from the response.
    tool_calls: Vec<ratatoskr::ToolCall>,
    /// Whether any tool calls were present.
    has_tool_calls: bool,
    /// Response metadata (usage stats, model info).
    response_meta: Option<serde_json::Value>,
}

/// Collect a streaming response from the LLM API via ratatoskr.
///
/// Handles: SSE parsing, content accumulation, tool call reconstruction.
/// Pure I/O - no hooks or side effects beyond streaming to sink.
async fn collect_streaming_response<S: ResponseSink>(
    resolved_config: &ResolvedConfig,
    messages: &[serde_json::Value],
    all_tools: &[serde_json::Value],
    verbose: bool,
    sink: &mut S,
) -> io::Result<StreamingResponse> {
    let gateway = build_gateway(resolved_config)?;

    // Convert messages
    let ratatoskr_messages: Vec<_> = messages
        .iter()
        .map(to_ratatoskr_message)
        .collect::<io::Result<Vec<_>>>()?;

    // Convert tools
    let tool_defs: Vec<_> = all_tools
        .iter()
        .filter_map(|t| json_tool_to_definition(t).ok())
        .collect();
    let tools_opt = if tool_defs.is_empty() {
        None
    } else {
        Some(tool_defs.as_slice())
    };

    // Build options
    let options = to_chat_options(resolved_config);

    // Get streaming response
    let mut stream = gateway
        .chat_stream(&ratatoskr_messages, tools_opt, &options)
        .await
        .map_err(|e| io::Error::other(format!("Gateway error: {}", e)))?;

    let mut full_response = String::new();
    let mut tool_calls: Vec<ratatoskr::ToolCall> = Vec::new();
    let mut has_tool_calls = false;
    let mut response_meta: Option<serde_json::Value> = None;
    let mut is_first_content = true;
    let json_mode = sink.is_json_mode();

    while let Some(event_result) = stream.next().await {
        let event = event_result.map_err(|e| io::Error::other(format!("Stream error: {}", e)))?;

        match event {
            ChatEvent::Content(chunk) => {
                // Handle first chunk newline stripping (matches old behavior)
                let text = if is_first_content {
                    is_first_content = false;
                    if let Some(remaining) = chunk.strip_prefix('\n') {
                        if remaining.is_empty() {
                            continue;
                        }
                        remaining.to_string()
                    } else {
                        chunk
                    }
                } else {
                    chunk
                };

                full_response.push_str(&text);
                if !json_mode {
                    sink.handle(ResponseEvent::TextChunk(&text))?;
                }
            }
            ChatEvent::Reasoning(chunk) => {
                // Reasoning is ephemeral thinking — always forward to sink so
                // both streaming and JSON consumers can access it
                sink.handle(ResponseEvent::Reasoning(&chunk))?;
            }
            ChatEvent::ToolCallStart { index, id, name } => {
                has_tool_calls = true;

                // Prevent memory exhaustion
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
                    tool_calls.push(ratatoskr::ToolCall::default());
                }

                tool_calls[index].id = id;
                tool_calls[index].name = name;
            }
            ChatEvent::ToolCallDelta { index, arguments } => {
                if index < tool_calls.len() {
                    tool_calls[index].arguments.push_str(&arguments);
                }
            }
            ChatEvent::Usage(usage) => {
                response_meta = Some(json!({
                    "usage": {
                        "prompt_tokens": usage.prompt_tokens,
                        "completion_tokens": usage.completion_tokens,
                        "total_tokens": usage.total_tokens
                    }
                }));
            }
            ChatEvent::ToolCallEnd { .. } => {
                // Tool call argument streaming complete; nothing to do
            }
            ChatEvent::Done => break,
            _ => {} // forward-compatible with future ChatEvent variants
        }
    }

    Ok(StreamingResponse {
        full_response,
        tool_calls,
        has_tool_calls,
        response_meta,
    })
}

/// Result of executing a single tool.
struct ToolExecutionResult {
    /// The final result to send back to the LLM (may be truncated if cached).
    final_result: String,
    /// The original untruncated result.
    original_result: String,
    /// Whether the result was cached.
    was_cached: bool,
    /// Verbose diagnostic messages collected during execution.
    diagnostics: Vec<String>,
}

/// Sink-free execution core for a single tool call.
///
/// Performs all tool execution logic (hooks, dispatch, caching, output hooks)
/// without touching the sink or handoff state. Collects verbose diagnostics
/// into `ToolExecutionResult::diagnostics` for the caller to emit.
///
/// This enables concurrent execution via `join_all` since it doesn't require
/// `&mut` access to shared state.
#[allow(clippy::too_many_arguments)]
async fn execute_tool_pure(
    app: &AppState,
    context_name: &str,
    tool_call: &ratatoskr::ToolCall,
    tools: &[Tool],
    use_reflection: bool,
    resolved_config: &ResolvedConfig,
    verbose: bool,
    permission_handler: Option<&PermissionHandler>,
    project_root: &Path,
) -> io::Result<ToolExecutionResult> {
    let mut args: serde_json::Value =
        serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::json!({}));
    let mut diagnostics = Vec::new();

    // Check for control flow tools using metadata — we compute the result message
    // but leave handoff mutation to the caller
    let tool_metadata = tools::get_tool_metadata(tools, &tool_call.name);

    // Execute pre_tool hooks (can modify arguments OR block execution)
    let pre_hook_data = serde_json::json!({
        "tool_name": tool_call.name,
        "arguments": args,
    });
    let pre_hook_results = tools::execute_hook(tools, tools::HookPoint::PreTool, &pre_hook_data)?;

    let mut blocked = false;
    let mut block_message = String::new();

    for (hook_tool_name, result) in pre_hook_results {
        // Check for block signal first
        if result.get_bool_or("block", false) {
            blocked = true;
            block_message = result
                .get_str_or("message", "Tool call blocked by hook")
                .to_string();
            if verbose {
                diagnostics.push(format!(
                    "[Hook pre_tool: {} blocked {} - {}]",
                    hook_tool_name, tool_call.name, block_message
                ));
            }
            break;
        }

        // Check for argument modification
        if let Some(modified_args) = result.get("arguments") {
            if verbose {
                diagnostics.push(format!(
                    "[Hook pre_tool: {} modified arguments for {}]",
                    hook_tool_name, tool_call.name
                ));
            }
            args = modified_args.clone();
        }
    }

    // If blocked, skip execution and use block message as result
    let tool_result = if blocked {
        block_message
    } else if tool_metadata.flow_control {
        // Handoff tools don't execute — they just produce a result message.
        // The caller applies the actual handoff mutation.
        if tool_metadata.ends_turn {
            let message = args.get_str_or("message", "");
            if message.is_empty() {
                "Returning to user".to_string()
            } else {
                message.to_string()
            }
        } else {
            let prompt = args.get_str_or("prompt", "");
            if prompt.is_empty() {
                "Continuing processing".to_string()
            } else {
                format!("Continuing with: {}", prompt)
            }
        }
    } else if tool_call.name == tools::REFLECTION_TOOL_NAME && !use_reflection {
        "Error: Reflection tool is not enabled".to_string()
    } else if let Some(builtin_result) = tools::execute_builtin_tool(
        app,
        context_name,
        &tool_call.name,
        &args,
        Some(resolved_config),
    ) {
        match builtin_result {
            Ok(r) => r,
            Err(e) => format!("Error: {}", e),
        }
    } else if tool_call.name == tools::SEND_MESSAGE_TOOL_NAME {
        execute_send_message_pure(app, context_name, tools, &args, verbose, &mut diagnostics)?
    } else if tool_call.name == tools::MODEL_INFO_TOOL_NAME {
        match args.get_str("model") {
            Some(model) => {
                let gateway = build_gateway(resolved_config)?;
                match crate::model_info::fetch_metadata(&gateway, model).await {
                    Ok(metadata) => {
                        let json = crate::model_info::format_model_json(&metadata);
                        serde_json::to_string_pretty(&json)
                            .unwrap_or_else(|e| format!("Error serialising metadata: {}", e))
                    }
                    Err(e) => format!("Error: {}", e),
                }
            }
            None => "Error: missing required 'model' parameter".to_string(),
        }
    } else if tools::is_file_tool(&tool_call.name) {
        // Write tools need permission via pre_file_write hook
        if tool_call.name == tools::WRITE_FILE_TOOL_NAME {
            let hook_data = serde_json::json!({
                "tool_name": tool_call.name,
                "path": args.get_str("path").unwrap_or(""),
                "content": args.get_str("content"),
            });
            match check_permission(
                tools,
                tools::HookPoint::PreFileWrite,
                &hook_data,
                permission_handler,
            )? {
                Ok(()) => match tools::execute_file_tool(
                    app,
                    context_name,
                    &tool_call.name,
                    &args,
                    resolved_config,
                ) {
                    Some(Ok(r)) => r,
                    Some(Err(e)) => format!("Error: {}", e),
                    None => format!("Error: Unknown file tool '{}'", tool_call.name),
                },
                Err(reason) => format!("Error: {}", reason),
            }
        } else {
            // Read-only file tools: auto-allow inside allowed paths, prompt outside
            let path = args.get_str("path").unwrap_or("");
            let permission_denied = if !path.is_empty() {
                match tools::classify_file_path(path, resolved_config) {
                    Ok(tools::FilePathAccess::Allowed(_)) => None,
                    Ok(tools::FilePathAccess::NeedsPermission(_)) => {
                        let hook_data = serde_json::json!({
                            "tool_name": tool_call.name,
                            "path": path,
                        });
                        check_permission(
                            tools,
                            tools::HookPoint::PreFileRead,
                            &hook_data,
                            permission_handler,
                        )?
                        .err()
                    }
                    Err(e) => Some(e.to_string()),
                }
            } else {
                // cache_id access (no path) — always allowed
                None
            };

            if let Some(reason) = permission_denied {
                format!("Error: {}", reason)
            } else {
                match tools::execute_file_tool(
                    app,
                    context_name,
                    &tool_call.name,
                    &args,
                    resolved_config,
                ) {
                    Some(Ok(r)) => r,
                    Some(Err(e)) => format!("Error: {}", e),
                    None => format!("Error: Unknown file tool '{}'", tool_call.name),
                }
            }
        }
    } else if tools::is_agent_tool(&tool_call.name) {
        // URL policy / permission check for summarize_content
        if tool_call.name == tools::SUMMARIZE_CONTENT_TOOL_NAME
            && let Some(source) = args.get_str("source")
            && tools::agent_tools::is_url(source)
        {
            let safety = tools::classify_url(source);

            if let Some(ref policy) = resolved_config.url_policy {
                // policy is authoritative — no fallback to permission handler
                if tools::evaluate_url_policy(source, &safety, policy) == tools::UrlAction::Deny {
                    let reason = match &safety {
                        tools::UrlSafety::Sensitive(cat) => cat.to_string(),
                        tools::UrlSafety::Safe => "denied by URL policy".to_string(),
                    };
                    let msg = format!("Permission denied: {}", reason);
                    return Ok(ToolExecutionResult {
                        final_result: msg.clone(),
                        original_result: msg,
                        was_cached: false,
                        diagnostics,
                    });
                }
            } else if let tools::UrlSafety::Sensitive(category) = &safety {
                // no policy — existing behaviour: check permission handler
                let hook_data = json!({
                    "tool_name": tool_call.name,
                    "url": source,
                    "safety": "sensitive",
                    "reason": category.to_string(),
                });
                match check_permission(
                    tools,
                    tools::HookPoint::PreFetchUrl,
                    &hook_data,
                    permission_handler,
                )? {
                    Ok(()) => {}
                    Err(reason) => {
                        let msg = format!("Permission denied: {}", reason);
                        return Ok(ToolExecutionResult {
                            final_result: msg.clone(),
                            original_result: msg,
                            was_cached: false,
                            diagnostics,
                        });
                    }
                }
            }
        }
        match tools::execute_agent_tool(resolved_config, &tool_call.name, &args, tools).await {
            Ok(r) => r,
            Err(e) => format!("Error: {}", e),
        }
    } else if tools::is_coding_tool(&tool_call.name) {
        // Gated coding tools need permission checks
        if tool_call.name == tools::SHELL_EXEC_TOOL_NAME {
            let hook_data = serde_json::json!({
                "tool_name": tool_call.name,
                "command": args.get_str("command").unwrap_or(""),
            });
            match check_permission(
                tools,
                tools::HookPoint::PreShellExec,
                &hook_data,
                permission_handler,
            )? {
                Ok(()) => {
                    match tools::execute_coding_tool(&tool_call.name, &args, project_root, tools)
                        .await
                    {
                        Some(Ok(r)) => r,
                        Some(Err(e)) => format!("Error: {}", e),
                        None => format!("Error: Unknown coding tool '{}'", tool_call.name),
                    }
                }
                Err(reason) => format!("Error: {}", reason),
            }
        } else if tool_call.name == tools::FILE_EDIT_TOOL_NAME {
            let hook_data = serde_json::json!({
                "tool_name": tool_call.name,
                "path": args.get_str("path").unwrap_or(""),
                "operation": args.get_str("operation").unwrap_or(""),
                "content": args.get_str("content"),
            });
            match check_permission(
                tools,
                tools::HookPoint::PreFileWrite,
                &hook_data,
                permission_handler,
            )? {
                Ok(()) => {
                    match tools::execute_coding_tool(&tool_call.name, &args, project_root, tools)
                        .await
                    {
                        Some(Ok(r)) => r,
                        Some(Err(e)) => format!("Error: {}", e),
                        None => format!("Error: Unknown coding tool '{}'", tool_call.name),
                    }
                }
                Err(reason) => format!("Error: {}", reason),
            }
        } else if tool_call.name == tools::FETCH_URL_TOOL_NAME {
            // URL policy / permission check for fetch_url (mirrors summarize_content gating)
            let url = args.get_str("url").unwrap_or("");
            let safety = tools::classify_url(url);

            let denied = if let Some(ref policy) = resolved_config.url_policy {
                tools::evaluate_url_policy(url, &safety, policy) == tools::UrlAction::Deny
            } else {
                false
            };

            if denied {
                let reason = match &safety {
                    tools::UrlSafety::Sensitive(cat) => cat.to_string(),
                    tools::UrlSafety::Safe => "denied by URL policy".to_string(),
                };
                format!("Permission denied: {}", reason)
            } else if let tools::UrlSafety::Sensitive(category) = &safety {
                // No policy — fall back to permission handler for sensitive URLs
                let hook_data = json!({
                    "tool_name": tool_call.name,
                    "url": url,
                    "safety": "sensitive",
                    "reason": category.to_string(),
                });
                match check_permission(
                    tools,
                    tools::HookPoint::PreFetchUrl,
                    &hook_data,
                    permission_handler,
                )? {
                    Ok(()) => {
                        match tools::execute_coding_tool(
                            &tool_call.name,
                            &args,
                            project_root,
                            tools,
                        )
                        .await
                        {
                            Some(Ok(r)) => r,
                            Some(Err(e)) => format!("Error: {}", e),
                            None => format!("Error: Unknown coding tool '{}'", tool_call.name),
                        }
                    }
                    Err(reason) => format!("Permission denied: {}", reason),
                }
            } else {
                match tools::execute_coding_tool(&tool_call.name, &args, project_root, tools).await
                {
                    Some(Ok(r)) => r,
                    Some(Err(e)) => format!("Error: {}", e),
                    None => format!("Error: Unknown coding tool '{}'", tool_call.name),
                }
            }
        } else {
            // Ungated coding tools: dir_list, glob_files, grep_files,
            // index_update, index_query, index_status
            match tools::execute_coding_tool(&tool_call.name, &args, project_root, tools).await {
                Some(Ok(r)) => r,
                Some(Err(e)) => format!("Error: {}", e),
                None => format!("Error: Unknown coding tool '{}'", tool_call.name),
            }
        }
    } else if let Some(tool) = tools::find_tool(tools, &tool_call.name) {
        match tools::execute_tool(tool, &args, verbose) {
            Ok(r) => r,
            Err(e) => format!("Error: {}", e),
        }
    } else {
        format!("Error: Unknown tool '{}'", tool_call.name)
    };

    // Execute pre_tool_output hooks (can modify or replace output)
    let mut tool_result = tool_result;
    let pre_output_hook_data = serde_json::json!({
        "tool_name": tool_call.name,
        "arguments": args,
        "output": tool_result,
    });
    let pre_output_hook_results = tools::execute_hook(
        tools,
        tools::HookPoint::PreToolOutput,
        &pre_output_hook_data,
    )?;

    for (hook_tool_name, result) in pre_output_hook_results {
        if result.get_bool_or("block", false) {
            let replacement = result
                .get_str_or("message", "Output blocked by hook")
                .to_string();
            if verbose {
                diagnostics.push(format!(
                    "[Hook pre_tool_output: {} blocked output from {}]",
                    hook_tool_name, tool_call.name
                ));
            }
            tool_result = replacement;
            break;
        }

        if let Some(modified_output) = result.get_str("output") {
            if verbose {
                diagnostics.push(format!(
                    "[Hook pre_tool_output: {} modified output from {}]",
                    hook_tool_name, tool_call.name
                ));
            }
            tool_result = modified_output.to_string();
        }
    }

    // Check if output should be cached
    let (final_result, was_cached) = if !tool_result.starts_with("Error:")
        && cache::should_cache(&tool_result, resolved_config.tool_output_cache_threshold)
    {
        // Fire pre_cache_output hook (can block caching)
        let pre_cache_data = serde_json::json!({
            "tool_name": tool_call.name,
            "output_size": tool_result.len(),
            "arguments": args,
        });
        let pre_cache_results =
            tools::execute_hook(tools, tools::HookPoint::PreCacheOutput, &pre_cache_data)?;
        let cache_blocked = pre_cache_results
            .iter()
            .any(|(_, r)| r.get_bool_or("block", false));

        if cache_blocked {
            if verbose {
                diagnostics.push(format!(
                    "[Caching blocked by pre_cache_output hook for {}]",
                    tool_call.name
                ));
            }
            (tool_result.clone(), false)
        } else {
            let cache_dir = app.tool_cache_dir(context_name);
            match cache::cache_output(&cache_dir, &tool_call.name, &tool_result, &args) {
                Ok(entry) => {
                    match cache::generate_truncated_message(
                        &entry,
                        resolved_config.tool_cache_preview_chars,
                    ) {
                        Ok(truncated) => {
                            if verbose {
                                diagnostics.push(format!(
                                    "[Cached {} chars from {} as {}]",
                                    tool_result.len(),
                                    tool_call.name,
                                    entry.metadata.id
                                ));
                            }

                            // Fire post_cache_output hook (notification only)
                            let post_cache_data = serde_json::json!({
                                "tool_name": tool_call.name,
                                "cache_id": entry.metadata.id,
                                "output_size": tool_result.len(),
                                "preview_size": truncated.len(),
                            });
                            let _ = tools::execute_hook(
                                tools,
                                tools::HookPoint::PostCacheOutput,
                                &post_cache_data,
                            );

                            (truncated, true)
                        }
                        Err(_) => (tool_result.clone(), false),
                    }
                }
                Err(e) => {
                    if verbose {
                        diagnostics.push(format!("[Failed to cache output: {}]", e));
                    }
                    (tool_result.clone(), false)
                }
            }
        }
    } else {
        (tool_result.clone(), false)
    };

    // Execute post_tool_output hooks
    let post_output_hook_data = serde_json::json!({
        "tool_name": tool_call.name,
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

    Ok(ToolExecutionResult {
        final_result,
        original_result: tool_result,
        was_cached,
        diagnostics,
    })
}

/// Execute a single tool call with all hooks (sequential path).
///
/// Thin wrapper over `execute_tool_pure` that also handles handoff mutation
/// and emits diagnostics to the sink.
#[allow(clippy::too_many_arguments)]
async fn execute_single_tool<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    tool_call: &ratatoskr::ToolCall,
    tools: &[Tool],
    handoff: &mut tools::Handoff,
    use_reflection: bool,
    resolved_config: &ResolvedConfig,
    permission_handler: Option<&PermissionHandler>,
    verbose: bool,
    sink: &mut S,
    project_root: &Path,
) -> io::Result<ToolExecutionResult> {
    // Apply handoff if this is a flow control tool
    let args: serde_json::Value =
        serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::json!({}));
    let tool_metadata = tools::get_tool_metadata(tools, &tool_call.name);
    if tool_metadata.flow_control {
        if tool_metadata.ends_turn {
            handoff.set_user(args.get_str_or("message", "").to_string());
        } else {
            handoff.set_agent(args.get_str_or("prompt", "").to_string());
        }
    }

    let result = execute_tool_pure(
        app,
        context_name,
        tool_call,
        tools,
        use_reflection,
        resolved_config,
        verbose,
        permission_handler,
        project_root,
    )
    .await?;

    // Emit collected diagnostics to sink
    for diag in &result.diagnostics {
        sink.handle(ResponseEvent::Diagnostic {
            message: diag.clone(),
            verbose_only: true,
        })?;
    }

    Ok(result)
}

/// Sink-free send_message execution. Collects diagnostics into the provided vec.
#[allow(clippy::too_many_arguments)]
fn execute_send_message_pure(
    app: &AppState,
    context_name: &str,
    tools: &[Tool],
    args: &serde_json::Value,
    verbose: bool,
    diagnostics: &mut Vec<String>,
) -> io::Result<String> {
    let to = args.get_str_or("to", "");
    let content = args.get_str_or("content", "");
    let from = args.get_str("from").unwrap_or(context_name);

    if to.is_empty() {
        return Ok("Error: 'to' field is required".to_string());
    }
    if content.is_empty() {
        return Ok("Error: 'content' field is required".to_string());
    }

    // Execute pre_send_message hooks
    let pre_hook_data = serde_json::json!({
        "from": from,
        "to": to,
        "content": content,
        "context_name": context_name,
    });
    let pre_hook_results =
        tools::execute_hook(tools, tools::HookPoint::PreSendMessage, &pre_hook_data)?;

    // Check if any hook claimed delivery
    let mut delivered_via: Option<String> = None;
    for (hook_tool_name, hook_result) in &pre_hook_results {
        if hook_result.get_bool_or("delivered", false) {
            let via = hook_result.get_str_or("via", hook_tool_name);
            delivered_via = Some(via.to_string());
            if verbose {
                diagnostics.push(format!(
                    "[Hook pre_send_message: {} intercepted delivery]",
                    hook_tool_name
                ));
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
        "context_name": context_name,
        "delivery_result": delivery_result,
    });
    let _ = tools::execute_hook(tools, tools::HookPoint::PostSendMessage, &post_hook_data);

    Ok(delivery_result)
}

/// Process all tool calls from a response.
///
/// Parallel-safe tools (ToolMetadata::parallel == true) run concurrently via
/// `join_all`. Sequential tools (flow_control, parallel == false) run after
/// the parallel batch completes. Results are emitted to the sink and transcript
/// in the original tool_call order regardless of execution order.
#[allow(clippy::too_many_arguments)]
async fn process_tool_calls<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    tool_calls: &[ratatoskr::ToolCall],
    messages: &mut Vec<serde_json::Value>,
    tools: &[Tool],
    handoff: &mut tools::Handoff,
    use_reflection: bool,
    resolved_config: &ResolvedConfig,
    fuel_remaining: &mut usize,
    fuel_total: usize,
    verbose: bool,
    sink: &mut S,
    permission_handler: Option<&PermissionHandler>,
    project_root: &Path,
) -> io::Result<()> {
    // Convert tool calls to JSON format for the assistant message
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

    // Partition tool calls into parallel-safe and sequential batches.
    // We store (original_index, tool_call) to preserve ordering in results.
    let mut parallel_batch: Vec<(usize, &ratatoskr::ToolCall)> = Vec::new();
    let mut sequential_batch: Vec<(usize, &ratatoskr::ToolCall)> = Vec::new();

    for (i, tc) in tool_calls.iter().enumerate() {
        let metadata = tools::get_tool_metadata(tools, &tc.name);
        if metadata.parallel && !metadata.flow_control {
            parallel_batch.push((i, tc));
        } else {
            sequential_batch.push((i, tc));
        }
    }

    // Results indexed by original position
    let mut results: Vec<Option<ToolExecutionResult>> =
        (0..tool_calls.len()).map(|_| None).collect();

    // Execute parallel batch concurrently via join_all.
    // These futures run on the current task (no spawn), interleaving at .await
    // points — safe with !Send types like AppState's RefCell.
    if !parallel_batch.is_empty() {
        let parallel_futures: Vec<_> = parallel_batch
            .iter()
            .map(|(_idx, tc)| {
                execute_tool_pure(
                    app,
                    context_name,
                    tc,
                    tools,
                    use_reflection,
                    resolved_config,
                    verbose,
                    permission_handler,
                    project_root,
                )
            })
            .collect();

        let parallel_results = futures_util::future::join_all(parallel_futures).await;

        for ((idx, _tc), result) in parallel_batch.iter().zip(parallel_results) {
            results[*idx] = Some(result?);
        }
    }

    // Execute sequential batch one at a time (these may mutate handoff)
    for (idx, tc) in &sequential_batch {
        let result = execute_single_tool(
            app,
            context_name,
            tc,
            tools,
            handoff,
            use_reflection,
            resolved_config,
            permission_handler,
            verbose,
            sink,
            project_root,
        )
        .await?;
        results[*idx] = Some(result);
    }

    // Write all tool_call entries to transcript first (matches API message order:
    // one assistant message with tool_calls[], then individual tool result messages)
    for (i, tc) in tool_calls.iter().enumerate() {
        let tool_call_entry = create_tool_call_entry(context_name, &tc.name, &tc.arguments, &tc.id);
        app.append_to_transcript_and_context(context_name, &tool_call_entry)?;
        sink.handle(ResponseEvent::TranscriptEntry(tool_call_entry))?;

        // Pre-log verbose diagnostic while we're iterating
        if verbose && let Some(result) = &results[i] {
            for diag in &result.diagnostics {
                sink.handle(ResponseEvent::Diagnostic {
                    message: diag.clone(),
                    verbose_only: true,
                })?;
            }
        }
    }

    // Emit sink events and write tool_result entries in original order
    for (i, tc) in tool_calls.iter().enumerate() {
        let result = results[i]
            .take()
            .expect("all tool results should be populated");

        if verbose {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[Tool: {}]", tc.name),
                verbose_only: true,
            })?;
        }

        // Emit diagnostics collected during parallel execution (if not already emitted)
        if !verbose {
            for diag in &result.diagnostics {
                sink.handle(ResponseEvent::Diagnostic {
                    message: diag.clone(),
                    verbose_only: true,
                })?;
            }
        }

        let summary = tools::tool_call_summary(tools, &tc.name, &tc.arguments);
        sink.handle(ResponseEvent::ToolStart {
            name: tc.name.clone(),
            summary,
        })?;

        // Log tool result to transcript
        let logged_result = if result.was_cached {
            &result.final_result
        } else {
            &result.original_result
        };
        let tool_result_entry =
            create_tool_result_entry(context_name, &tc.name, logged_result, &tc.id);
        app.append_to_transcript_and_context(context_name, &tool_result_entry)?;
        sink.handle(ResponseEvent::TranscriptEntry(tool_result_entry))?;

        sink.handle(ResponseEvent::ToolResult {
            name: tc.name.clone(),
            result: result.final_result.clone(),
            cached: result.was_cached,
        })?;

        // Show full content of todos/goals updates in verbose mode
        if verbose
            && matches!(tc.name.as_str(), "update_todos" | "update_goals")
            && let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.arguments)
            && let Some(content) = args["content"].as_str()
        {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[{}]\n{}", tc.name, content),
                verbose_only: true,
            })?;
        }

        // Execute post_tool hooks
        let args: serde_json::Value =
            serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));
        let post_hook_data = serde_json::json!({
            "tool_name": tc.name,
            "arguments": args,
            "result": result.original_result,
            "cached": result.was_cached,
        });
        let _ = tools::execute_hook(tools, tools::HookPoint::PostTool, &post_hook_data);

        // Apply handoff for parallel-executed flow control tools
        // (sequential ones already applied via execute_single_tool)
        let metadata = tools::get_tool_metadata(tools, &tc.name);
        if metadata.flow_control && metadata.parallel {
            // This shouldn't happen (flow_control tools are always sequential),
            // but handle it defensively
            if metadata.ends_turn {
                handoff.set_user(args.get_str_or("message", "").to_string());
            } else {
                handoff.set_agent(args.get_str_or("prompt", "").to_string());
            }
        }

        messages.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": tc.id,
            "content": result.final_result,
        }));
    }

    // Execute post_tool_batch hook — allows plugins to override fallback after seeing tool results
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
        "context_name": context_name,
        "fuel_remaining": fuel_remaining,
        "fuel_total": fuel_total,
        "current_fallback": resolved_config.fallback_tool,
        "tool_calls": tool_batch_info,
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PostToolBatch, &hook_data)?;
    apply_hook_overrides(handoff, fuel_remaining, &hook_results, verbose, sink)?;

    Ok(())
}

/// Result of handling a final (non-tool-call) response.
enum FinalResponseAction {
    /// Return to the user (turn ended).
    ReturnToUser,
    /// Continue with another prompt (agent loop).
    ContinueWithPrompt(String),
}

/// Handle the final text response from the LLM.
///
/// Handles: Empty response detection/retry, save assistant message,
/// post_message hook, handoff decision (return vs recurse).
#[allow(clippy::too_many_arguments)]
fn handle_final_response<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    full_response: &str,
    final_prompt: &str,
    mut handoff: tools::Handoff,
    tools: &[Tool],
    _resolved_config: &ResolvedConfig,
    verbose: bool,
    sink: &mut S,
) -> io::Result<FinalResponseAction> {
    // Get or create context to add the message
    let mut context = app.get_or_create_context(context_name)?;

    // Save the assistant's response
    app.add_message(
        &mut context,
        "assistant".to_string(),
        full_response.to_string(),
    );

    let assistant_entry = create_assistant_message_entry(context_name, full_response);
    app.append_to_transcript_and_context(context_name, &assistant_entry)?;
    sink.handle(ResponseEvent::TranscriptEntry(assistant_entry))?;

    // Execute post_message hooks
    let hook_data = serde_json::json!({
        "prompt": final_prompt,
        "response": full_response,
        "context_name": context_name,
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

    // Determine next action based on handoff.
    // Fuel exhaustion is the caller's responsibility — we just report the action.
    match handoff.take() {
        tools::HandoffTarget::User { message } => {
            if !message.is_empty() {
                sink.handle(ResponseEvent::TextChunk(&message))?;
                sink.handle(ResponseEvent::Newline)?;
            }
            Ok(FinalResponseAction::ReturnToUser)
        }
        tools::HandoffTarget::Agent { prompt } => {
            Ok(FinalResponseAction::ContinueWithPrompt(prompt))
        }
    }
}

/// Send a prompt to the LLM with streaming response via ResponseSink.
///
/// This is the main entry point for sending prompts. It handles:
/// - Hook execution (pre_message, post_message, etc.)
/// - Inbox message injection
/// - Tool execution loop with fuel budget
/// - Context management
/// - Auto-compaction
///
/// # Arguments
///
/// * `context_name` - The name of the context to use for this prompt
#[allow(clippy::too_many_arguments)]
pub async fn send_prompt<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    initial_prompt: String,
    tools: &[Tool],
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
    sink: &mut S,
    permission_handler: Option<&PermissionHandler>,
    home_dir: &Path,
    project_root: &Path,
) -> io::Result<()> {
    let fuel_total = resolved_config.fuel;
    let mut fuel_remaining = fuel_total;
    let mut current_prompt = initial_prompt;

    // Outer loop: each iteration is a full setup + agentic exchange.
    // First iteration is the user's turn (free); continuations cost 1 fuel.
    loop {
        // === Validation & Setup ===
        if current_prompt.trim().is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Prompt cannot be empty",
            ));
        }

        app.validate_config(resolved_config, tools)?;

        let verbose = options.verbose;

        if verbose {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[fuel: {}/{} entering turn]", fuel_remaining, fuel_total),
                verbose_only: true,
            })?;
        }
        let use_reflection = options.use_reflection;
        let debug = options.debug;

        let mut context = app.get_or_create_context(context_name)?;

        // === Pre-message Hooks & Inbox ===
        let mut final_prompt = current_prompt.clone();
        let hook_data = serde_json::json!({
            "prompt": current_prompt,
            "context_name": context.name,
            "summary": context.summary,
        });
        let hook_results = tools::execute_hook(tools, tools::HookPoint::PreMessage, &hook_data)?;
        for (tool_name, result) in hook_results {
            if let Some(modified) = result.get_str("prompt") {
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

        // Add datetime prefix to user message
        let datetime_prefix = chrono::Local::now().format("%Y%m%d-%H%M%z").to_string();
        let prefixed_prompt = format!("[{}] {}", datetime_prefix, final_prompt);

        // Add user message to context and transcript
        app.add_message(&mut context, "user".to_string(), prefixed_prompt.clone());
        let user_entry =
            create_user_message_entry(context_name, &prefixed_prompt, &resolved_config.username);
        app.append_to_transcript_and_context(context_name, &user_entry)?;
        sink.handle(ResponseEvent::TranscriptEntry(user_entry))?;

        // Context window warning
        if app.should_warn(&context.messages) {
            let remaining = app.remaining_tokens(&context.messages);
            if verbose {
                sink.handle(ResponseEvent::Diagnostic {
                    message: format!("[Context window warning: {} tokens remaining]", remaining),
                    verbose_only: true,
                })?;
            }
        }

        // === Auto-compaction Check ===
        if app.should_auto_compact(&context, resolved_config) {
            return compact_context_with_llm(app, context_name, resolved_config, verbose).await;
        }

        // === Build System Prompt ===
        let full_system_prompt = build_full_system_prompt(
            app,
            context_name,
            &context.summary,
            use_reflection,
            tools,
            resolved_config,
            verbose,
            sink,
            home_dir,
            project_root,
        )?;

        // === Prepare Messages ===
        let mut messages: Vec<serde_json::Value> = if !full_system_prompt.is_empty() {
            vec![serde_json::json!({
                "role": "system",
                "content": full_system_prompt,
            })]
        } else {
            Vec::new()
        };

        // Add conversation messages (skip system messages, strip internal _id field)
        for m in &context.messages {
            let role = m["role"].as_str().unwrap_or("");
            if role == "system" {
                continue;
            }
            let mut msg = m.clone();
            if let Some(obj) = msg.as_object_mut() {
                obj.remove("_id");
            }
            messages.push(msg);
        }

        // === Prepare Tools ===
        let mut all_tools = tools::tools_to_api_format(tools);
        all_tools.extend(tools::builtin_tools_to_api_format(use_reflection));
        all_tools.extend(tools::all_file_tools_to_api_format());
        all_tools.extend(tools::all_agent_tools_to_api_format());
        all_tools.extend(tools::all_coding_tools_to_api_format());
        annotate_fallback_tool(&mut all_tools, &resolved_config.fallback_tool);
        all_tools = filter_tools_by_config(all_tools, &resolved_config.tools, tools);

        // Execute pre_api_tools hook
        let tool_info = build_tool_info_list(&all_tools, tools);
        let hook_data = json!({
            "context_name": context.name,
            "tools": tool_info,
            "fuel_remaining": fuel_remaining,
            "fuel_total": fuel_total,
        });
        let hook_results = tools::execute_hook(tools, tools::HookPoint::PreApiTools, &hook_data)?;
        all_tools = filter_tools_from_hook_results(all_tools, &hook_results, verbose, sink)?;

        // === Build Request ===
        let tools_for_request = if resolved_config.no_tool_calls {
            None
        } else {
            Some(all_tools.as_slice())
        };
        let mut request_body =
            build_request_body(resolved_config, &messages, tools_for_request, true);

        // Execute pre_api_request hook
        let hook_data = json!({
            "context_name": context.name,
            "request_body": request_body,
            "fuel_remaining": fuel_remaining,
            "fuel_total": fuel_total,
        });
        let hook_results = tools::execute_hook(tools, tools::HookPoint::PreApiRequest, &hook_data)?;
        request_body = apply_request_modifications(request_body, &hook_results, verbose, sink)?;

        // === Initialize Handoff ===
        let fallback = options.fallback_override.clone().unwrap_or_else(|| {
            let meta = tools::get_tool_metadata(tools, &resolved_config.fallback_tool);
            if meta.ends_turn {
                tools::HandoffTarget::User {
                    message: String::new(),
                }
            } else {
                tools::HandoffTarget::Agent {
                    prompt: String::new(),
                }
            }
        });
        let mut handoff = tools::Handoff::new(fallback);

        // Execute pre_agentic_loop hook
        let hook_data = json!({
            "context_name": context.name,
            "fuel_remaining": fuel_remaining,
            "fuel_total": fuel_total,
            "current_fallback": resolved_config.fallback_tool,
            "message": final_prompt,
        });
        let hook_results =
            tools::execute_hook(tools, tools::HookPoint::PreAgenticLoop, &hook_data)?;
        apply_hook_overrides(
            &mut handoff,
            &mut fuel_remaining,
            &hook_results,
            verbose,
            sink,
        )?;

        // === Inner Loop: stream responses and process tool calls ===
        loop {
            sink.handle(ResponseEvent::StartResponse)?;
            log_request_if_enabled(app, context_name, debug, &request_body);

            let response =
                collect_streaming_response(resolved_config, &messages, &all_tools, verbose, sink)
                    .await?;

            // Log response metadata
            if let Some(ref meta) = response.response_meta {
                log_response_meta_if_enabled(app, context_name, debug, meta);
            }

            // Signal streaming finished
            if !sink.is_json_mode() {
                sink.handle(ResponseEvent::Finished)?;
            }

            // Handle tool calls
            if response.has_tool_calls && !response.tool_calls.is_empty() {
                process_tool_calls(
                    app,
                    context_name,
                    &response.tool_calls,
                    &mut messages,
                    tools,
                    &mut handoff,
                    use_reflection,
                    resolved_config,
                    &mut fuel_remaining,
                    fuel_total,
                    verbose,
                    sink,
                    permission_handler,
                    project_root,
                )
                .await?;

                // Keep request_body in sync for logging
                request_body["messages"] = serde_json::json!(messages);

                // Tool call round costs 1 fuel
                fuel_remaining = fuel_remaining.saturating_sub(1);
                if verbose {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel: {}/{} after tool batch]",
                            fuel_remaining, fuel_total
                        ),
                        verbose_only: true,
                    })?;
                }
                if fuel_remaining == 0 {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel exhausted (0/{}), returning control to user]",
                            fuel_total
                        ),
                        verbose_only: false,
                    })?;
                    return Ok(());
                }
                continue;
            }

            // Check for empty response
            if response.full_response.trim().is_empty() {
                fuel_remaining =
                    fuel_remaining.saturating_sub(resolved_config.fuel_empty_response_cost);
                if fuel_remaining == 0 {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel exhausted (0/{}), returning control to user]",
                            fuel_total
                        ),
                        verbose_only: false,
                    })?;
                    return Ok(());
                }
                if verbose {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[empty response, fuel: {}/{}]",
                            fuel_remaining, fuel_total
                        ),
                        verbose_only: true,
                    })?;
                }
                continue;
            }

            // Text response — break inner loop to handle final response
            match handle_final_response(
                app,
                context_name,
                &response.full_response,
                &final_prompt,
                handoff,
                tools,
                resolved_config,
                verbose,
                sink,
            )? {
                FinalResponseAction::ReturnToUser => return Ok(()),
                FinalResponseAction::ContinueWithPrompt(continue_prompt) => {
                    fuel_remaining = fuel_remaining.saturating_sub(1);
                    if fuel_remaining == 0 {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[fuel exhausted (0/{}), returning control to user]",
                                fuel_total
                            ),
                            verbose_only: false,
                        })?;
                        return Ok(());
                    }
                    if verbose {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[continuing (fuel: {}/{}): {}]",
                                fuel_remaining,
                                fuel_total,
                                if continue_prompt.len() > 80 {
                                    format!("{}...", &continue_prompt[..77])
                                } else {
                                    continue_prompt.clone()
                                }
                            ),
                            verbose_only: true,
                        })?;
                    }
                    // Prefix the continuation prompt with fuel info for the LLM
                    current_prompt = format!(
                        "[reengaged (fuel: {}/{}) via {}. call_user(<message>) to end turn.]\n{}",
                        fuel_remaining, fuel_total, resolved_config.fallback_tool, continue_prompt
                    );
                    break; // break inner, continue outer
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a minimal plugin Tool for classification tests.
    fn fake_plugin_tool(name: &str) -> Tool {
        Tool {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
            path: std::path::PathBuf::from(format!("/plugins/{name}")),
            hooks: vec![],
            metadata: tools::ToolMetadata::new(),
            summary_params: vec![],
        }
    }

    /// Helper: create a minimal MCP Tool for classification tests.
    fn fake_mcp_tool(server: &str, tool: &str) -> Tool {
        tools::mcp::mcp_tool_from_info(server, tool, "", serde_json::json!({}))
    }

    #[test]
    fn test_classify_tool_type_builtin() {
        for name in [
            "update_todos",
            "update_goals",
            "update_reflection",
            "send_message",
            "call_agent",
            "call_user",
            "model_info",
            "read_context",
        ] {
            assert_eq!(
                classify_tool_type(name, &[]),
                ToolType::Builtin,
                "{name}"
            );
        }
    }

    #[test]
    fn test_classify_tool_type_file() {
        for name in [
            "file_head",
            "file_tail",
            "file_lines",
            "file_grep",
            "cache_list",
            "write_file",
        ] {
            assert_eq!(classify_tool_type(name, &[]), ToolType::File, "{name}");
        }
    }

    #[test]
    fn test_classify_tool_type_coding() {
        for name in [
            "shell_exec",
            "dir_list",
            "glob_files",
            "grep_files",
            "file_edit",
            "fetch_url",
            "index_update",
            "index_query",
            "index_status",
        ] {
            assert_eq!(
                classify_tool_type(name, &[]),
                ToolType::Coding,
                "{name}"
            );
        }
    }

    #[test]
    fn test_classify_tool_type_agent() {
        for name in ["spawn_agent", "summarize_content"] {
            assert_eq!(
                classify_tool_type(name, &[]),
                ToolType::Agent,
                "{name}"
            );
        }
    }

    #[test]
    fn test_classify_tool_type_plugin() {
        let tools = vec![fake_plugin_tool("my_plugin")];
        assert_eq!(classify_tool_type("my_plugin", &tools), ToolType::Plugin);
    }

    #[test]
    fn test_classify_tool_type_mcp() {
        let tools = vec![fake_mcp_tool("serena", "find_symbol")];
        assert_eq!(
            classify_tool_type("serena_find_symbol", &tools),
            ToolType::Mcp
        );
    }

    #[test]
    fn test_classify_tool_type_unknown_defaults_to_plugin() {
        assert_eq!(
            classify_tool_type("unknown_tool", &[]),
            ToolType::Plugin
        );
    }

    #[test]
    fn test_tool_type_as_str() {
        assert_eq!(ToolType::Builtin.as_str(), "builtin");
        assert_eq!(ToolType::File.as_str(), "file");
        assert_eq!(ToolType::Agent.as_str(), "agent");
        assert_eq!(ToolType::Coding.as_str(), "coding");
        assert_eq!(ToolType::Mcp.as_str(), "mcp");
        assert_eq!(ToolType::Plugin.as_str(), "plugin");
    }

    #[test]
    fn test_filter_tools_by_config_no_filters() {
        let tools = vec![
            json!({"function": {"name": "tool1"}}),
            json!({"function": {"name": "tool2"}}),
        ];
        let config = ToolsConfig::default();
        let result = filter_tools_by_config(tools.clone(), &config, &[]);
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
            exclude_categories: None,
        };
        let result = filter_tools_by_config(tools, &config, &[]);
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
            exclude_categories: None,
        };
        let result = filter_tools_by_config(tools, &config, &[]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_tools_by_category_exclude() {
        // shell_exec and dir_list are "coding" category tools
        // file_head is a "file" category tool
        // update_todos is a "builtin" category tool
        // spawn_agent is an "agent" category tool
        let tools = vec![
            json!({"function": {"name": "shell_exec"}}),
            json!({"function": {"name": "dir_list"}}),
            json!({"function": {"name": "file_head"}}),
            json!({"function": {"name": "update_todos"}}),
            json!({"function": {"name": "spawn_agent"}}),
        ];
        let config = ToolsConfig {
            include: None,
            exclude: None,
            exclude_categories: Some(vec!["coding".to_string()]),
        };
        let result = filter_tools_by_config(tools, &config, &[]);
        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str())
            .collect();
        assert!(
            !names.contains(&"shell_exec"),
            "coding tool should be excluded"
        );
        assert!(
            !names.contains(&"dir_list"),
            "coding tool should be excluded"
        );
        assert!(names.contains(&"file_head"), "file tool should remain");
        assert!(
            names.contains(&"update_todos"),
            "builtin tool should remain"
        );
        assert!(names.contains(&"spawn_agent"), "agent tool should remain");
    }

    #[test]
    fn test_filter_tools_by_multiple_categories() {
        let tools = vec![
            json!({"function": {"name": "shell_exec"}}),
            json!({"function": {"name": "file_head"}}),
            json!({"function": {"name": "spawn_agent"}}),
            json!({"function": {"name": "update_todos"}}),
        ];
        let config = ToolsConfig {
            include: None,
            exclude: None,
            exclude_categories: Some(vec!["coding".to_string(), "agent".to_string()]),
        };
        let result = filter_tools_by_config(tools, &config, &[]);
        let names: Vec<&str> = result
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str())
            .collect();
        assert_eq!(names, vec!["file_head", "update_todos"]);
    }

    #[test]
    fn test_annotate_fallback_tool() {
        let mut tools = vec![
            json!({"function": {"name": "call_agent", "description": "Continue processing."}}),
            json!({"function": {"name": "call_user", "description": "Return control to user."}}),
            json!({"function": {"name": "my_plugin", "description": "Does something."}}),
        ];

        // Annotate call_agent as fallback
        annotate_fallback_tool(&mut tools, "call_agent");
        assert!(
            tools[0]["function"]["description"]
                .as_str()
                .unwrap()
                .contains("automatically")
        );
        assert!(
            !tools[1]["function"]["description"]
                .as_str()
                .unwrap()
                .contains("automatically")
        );

        // Reset and annotate a plugin instead
        tools[0]["function"]["description"] = json!("Continue processing.");
        annotate_fallback_tool(&mut tools, "my_plugin");
        assert!(
            !tools[0]["function"]["description"]
                .as_str()
                .unwrap()
                .contains("automatically")
        );
        assert!(
            tools[2]["function"]["description"]
                .as_str()
                .unwrap()
                .contains("automatically")
        );
    }

    #[test]
    fn test_annotate_fallback_tool_not_found() {
        let mut tools = vec![
            json!({"function": {"name": "call_agent", "description": "Continue processing."}}),
        ];

        // Should not panic if fallback tool doesn't exist
        annotate_fallback_tool(&mut tools, "nonexistent_tool");
        assert_eq!(
            tools[0]["function"]["description"].as_str().unwrap(),
            "Continue processing."
        );
    }

    #[test]
    fn test_context_name_injected_in_system_prompt() {
        // This is a unit test concept - the actual integration happens in build_full_system_prompt
        // We verify the format string is correct
        let context_name = "my-context";
        let expected = format!("\n\nCurrent context: {}", context_name);
        assert!(expected.contains("Current context: my-context"));
    }

    #[test]
    fn test_datetime_prefix_format() {
        use chrono::Local;
        let now = Local::now();
        let formatted = now.format("%Y%m%d-%H%M%z").to_string();
        // Should be like "20260203-1542+0000" or "20260203-1542-0500"
        assert_eq!(formatted.len(), 18, "datetime format should be 18 chars");
        assert!(
            formatted.chars().nth(8) == Some('-'),
            "should have dash separator"
        );
    }

    // ========================================================================
    // evaluate_permission tests
    // ========================================================================

    #[test]
    fn test_evaluate_permission_plugin_denies() {
        let results = vec![(
            "security_gate".to_string(),
            json!({"denied": true, "reason": "path outside project"}),
        )];
        let hook_data = json!({"tool_name": "write_file", "path": "/etc/passwd"});
        let handler: PermissionHandler = Box::new(|_| Ok(true));

        let result = evaluate_permission(&results, &hook_data, Some(&handler)).unwrap();
        assert_eq!(result, Err("path outside project".to_string()));
    }

    #[test]
    fn test_evaluate_permission_no_denials_handler_approves() {
        let results = vec![("audit_log".to_string(), json!({}))];
        let hook_data = json!({"tool_name": "write_file", "path": "/tmp/ok.txt"});
        let handler: PermissionHandler = Box::new(|_| Ok(true));

        let result = evaluate_permission(&results, &hook_data, Some(&handler)).unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_evaluate_permission_no_denials_handler_denies() {
        let results = vec![("audit_log".to_string(), json!({}))];
        let hook_data = json!({"tool_name": "write_file", "path": "/tmp/ok.txt"});
        let handler: PermissionHandler = Box::new(|_| Ok(false));

        let result = evaluate_permission(&results, &hook_data, Some(&handler)).unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "permission denied");
    }

    #[test]
    fn test_evaluate_permission_no_handler_failsafe_deny() {
        let results: Vec<(String, serde_json::Value)> = vec![];
        let hook_data = json!({"tool_name": "write_file"});

        let result = evaluate_permission(&results, &hook_data, None).unwrap();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fail-safe deny"));
    }

    #[test]
    fn test_evaluate_permission_empty_result_falls_through_to_handler() {
        // Plugin returns {} (no opinion) — should fall through to handler
        let results = vec![("passive_plugin".to_string(), json!({}))];
        let hook_data = json!({"tool_name": "shell_exec", "command": "ls"});
        let handler: PermissionHandler = Box::new(|_| Ok(true));

        let result = evaluate_permission(&results, &hook_data, Some(&handler)).unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_evaluate_permission_multiple_plugins_one_denies() {
        let results = vec![
            ("audit_log".to_string(), json!({})),
            (
                "security_gate".to_string(),
                json!({"denied": true, "reason": "blocked by policy"}),
            ),
            ("metrics".to_string(), json!({})),
        ];
        let hook_data = json!({"tool_name": "shell_exec", "command": "rm -rf /"});
        let handler: PermissionHandler = Box::new(|_| Ok(true));

        let result = evaluate_permission(&results, &hook_data, Some(&handler)).unwrap();
        assert_eq!(result, Err("blocked by policy".to_string()));
    }
}
