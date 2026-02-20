//! Gateway abstraction over ratatoskr.
//!
//! This module provides type conversions between chibi's internal types
//! and ratatoskr's ModelGateway types.

use crate::config::{self, ResolvedConfig};
use ratatoskr::{
    ChatOptions, EmbeddedGateway, Message, ModelGateway, Ratatoskr,
    ReasoningConfig as RatatoskrReasoningConfig, ReasoningEffort as RatatoskrReasoningEffort,
    ResponseFormat as RatatoskrResponseFormat, ToolCall, ToolChoice as RatatoskrToolChoice,
    ToolDefinition,
};
use std::io;

/// Convert chibi's JSON message format to ratatoskr Message.
pub fn to_ratatoskr_message(json: &serde_json::Value) -> io::Result<Message> {
    let role_str = json["role"]
        .as_str()
        .ok_or_else(|| io::Error::other("missing role"))?;

    let content = json["content"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_default();

    match role_str {
        "system" => Ok(Message::system(content)),
        "user" => Ok(Message::user(content)),
        "assistant" => {
            // Check for tool calls
            if let Some(tool_calls_json) = json.get("tool_calls").and_then(|v| v.as_array()) {
                let tool_calls: Vec<ToolCall> = tool_calls_json
                    .iter()
                    .filter_map(|tc| {
                        let id = tc["id"].as_str()?;
                        let name = tc["function"]["name"].as_str()?;
                        let arguments = tc["function"]["arguments"].as_str().unwrap_or("{}");
                        Some(ToolCall::new(id, name, arguments))
                    })
                    .collect();

                let content_opt = if content.is_empty() {
                    None
                } else {
                    Some(content)
                };
                Ok(Message::assistant_with_tool_calls(content_opt, tool_calls))
            } else {
                Ok(Message::assistant(content))
            }
        }
        "tool" => {
            let tool_call_id = json["tool_call_id"]
                .as_str()
                .ok_or_else(|| io::Error::other("tool message missing tool_call_id"))?;
            Ok(Message::tool_result(tool_call_id, content))
        }
        other => Err(io::Error::other(format!("unknown role: {}", other))),
    }
}

/// Convert OpenAI-format tool JSON to ratatoskr ToolDefinition.
///
/// Expects the format: `{"type": "function", "function": {"name": ..., "description": ..., "parameters": ...}}`
pub fn json_tool_to_definition(json: &serde_json::Value) -> io::Result<ToolDefinition> {
    ToolDefinition::try_from(json)
        .map_err(|e| io::Error::other(format!("Invalid tool definition: {}", e)))
}

/// Convert ResolvedConfig to ChatOptions.
pub fn to_chat_options(config: &ResolvedConfig) -> ChatOptions {
    let mut opts = ChatOptions::new(&config.model);

    let api = &config.api;

    if let Some(temp) = api.temperature {
        opts = opts.temperature(temp);
    }
    if let Some(max_tokens) = api.max_tokens {
        opts = opts.max_tokens(max_tokens);
    }
    if let Some(top_p) = api.top_p {
        opts = opts.top_p(top_p);
    }
    if let Some(ref stop) = api.stop {
        opts = opts.stop(stop.clone());
    }
    if let Some(seed) = api.seed {
        opts = opts.seed(seed);
    }
    if let Some(penalty) = api.frequency_penalty {
        opts = opts.frequency_penalty(penalty);
    }
    if let Some(penalty) = api.presence_penalty {
        opts = opts.presence_penalty(penalty);
    }
    if let Some(parallel) = api.parallel_tool_calls {
        opts = opts.parallel_tool_calls(parallel);
    }
    if let Some(ref tool_choice) = api.tool_choice {
        opts = opts.tool_choice(to_ratatoskr_tool_choice(tool_choice));
    }
    if let Some(ref format) = api.response_format {
        opts = opts.response_format(to_ratatoskr_response_format(format));
    }
    if let Some(cache) = api.prompt_caching {
        opts = opts.cache_prompt(cache);
    }

    if !api.reasoning.is_empty() {
        opts = opts.reasoning(to_ratatoskr_reasoning(&api.reasoning));
    }

    opts
}

/// Convert chibi's ToolChoice to ratatoskr's ToolChoice.
fn to_ratatoskr_tool_choice(choice: &config::ToolChoice) -> RatatoskrToolChoice {
    match choice {
        config::ToolChoice::Mode(mode) => match mode {
            config::ToolChoiceMode::Auto => RatatoskrToolChoice::Auto,
            config::ToolChoiceMode::None => RatatoskrToolChoice::None,
            config::ToolChoiceMode::Required => RatatoskrToolChoice::Required,
        },
        config::ToolChoice::Function { function, .. } => RatatoskrToolChoice::Function {
            name: function.name.clone(),
        },
    }
}

/// Convert chibi's ResponseFormat to ratatoskr's ResponseFormat.
fn to_ratatoskr_response_format(format: &config::ResponseFormat) -> RatatoskrResponseFormat {
    match format {
        config::ResponseFormat::Text => RatatoskrResponseFormat::Text,
        config::ResponseFormat::JsonObject => RatatoskrResponseFormat::JsonObject,
        config::ResponseFormat::JsonSchema { json_schema } => RatatoskrResponseFormat::JsonSchema {
            schema: json_schema
                .clone()
                .unwrap_or(serde_json::Value::Object(Default::default())),
        },
    }
}

/// Convert chibi's ReasoningConfig to ratatoskr's ReasoningConfig.
///
/// When `enabled = true` without explicit effort or max_tokens, defaults to medium effort.
fn to_ratatoskr_reasoning(reasoning: &config::ReasoningConfig) -> RatatoskrReasoningConfig {
    let effort = match (reasoning.effort, reasoning.enabled) {
        // Explicit effort always wins
        (Some(e), _) => Some(match e {
            config::ReasoningEffort::XHigh => RatatoskrReasoningEffort::XHigh,
            config::ReasoningEffort::High => RatatoskrReasoningEffort::High,
            config::ReasoningEffort::Medium => RatatoskrReasoningEffort::Medium,
            config::ReasoningEffort::Low => RatatoskrReasoningEffort::Low,
            config::ReasoningEffort::Minimal => RatatoskrReasoningEffort::Minimal,
            config::ReasoningEffort::None => RatatoskrReasoningEffort::None,
        }),
        // enabled=true without effort or max_tokens → default to medium
        (None, Some(true)) if reasoning.max_tokens.is_none() => {
            Some(RatatoskrReasoningEffort::Medium)
        }
        _ => None,
    };

    RatatoskrReasoningConfig {
        effort,
        max_tokens: reasoning.max_tokens,
        exclude_from_output: reasoning.exclude,
    }
}

/// Build a gateway from ResolvedConfig.
///
/// Passes `api_key` as `Option<&str>` to ratatoskr — `None` enables keyless
/// free-tier access via openrouter.
pub fn build_gateway(config: &ResolvedConfig) -> io::Result<EmbeddedGateway> {
    Ratatoskr::builder()
        .openrouter(config.api_key.as_deref())
        .build()
        .map_err(|e| io::Error::other(format!("Failed to build gateway: {}", e)))
}

/// Resolve `context_window_limit` from ratatoskr's model registry.
///
/// When `context_window_limit` is 0 (the "unknown" sentinel), performs a
/// synchronous registry lookup — no network I/O — to fill it in from
/// ratatoskr's built-in model metadata.  If the model isn't in the registry,
/// the limit stays at 0 and the existing guards (skip compaction/warnings)
/// remain in effect.
pub fn resolve_context_window(config: &mut ResolvedConfig, gateway: &EmbeddedGateway) {
    if config.context_window_limit == 0
        && let Some(meta) = gateway.model_metadata(&config.model)
        && let Some(ctx) = meta.info.context_window
    {
        config.context_window_limit = ctx;
    }
}

/// Ensure `context_window_limit` is populated in the config.
///
/// Convenience wrapper: builds a gateway and calls `resolve_context_window`.
/// Safe to call even when the limit is already set (no-op) or the model
/// isn't in the registry (limit stays 0, compaction/warnings remain guarded).
pub fn ensure_context_window(config: &mut ResolvedConfig) {
    if config.context_window_limit == 0
        && let Ok(gateway) = build_gateway(config)
    {
        resolve_context_window(config, &gateway);
    }
}

/// Simple non-streaming chat completion.
///
/// Converts JSON messages to ratatoskr format, sends request, returns content string.
pub async fn chat(config: &ResolvedConfig, messages: &[serde_json::Value]) -> io::Result<String> {
    let gateway = build_gateway(config)?;
    let options = to_chat_options(config);

    let ratatoskr_messages: Vec<Message> = messages
        .iter()
        .map(to_ratatoskr_message)
        .collect::<io::Result<Vec<_>>>()?;

    let response = gateway
        .chat(&ratatoskr_messages, None, &options)
        .await
        .map_err(|e| io::Error::other(format!("Chat request failed: {}", e)))?;

    Ok(response.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedConfig;
    use ratatoskr::Role;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn test_to_ratatoskr_message_user() {
        let json = json!({
            "role": "user",
            "content": "Hello, world!"
        });
        let msg = to_ratatoskr_message(&json).unwrap();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.as_text(), Some("Hello, world!"));
    }

    #[test]
    fn test_to_ratatoskr_message_assistant() {
        let json = json!({
            "role": "assistant",
            "content": "Hi there!"
        });
        let msg = to_ratatoskr_message(&json).unwrap();
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content.as_text(), Some("Hi there!"));
    }

    #[test]
    fn test_to_ratatoskr_message_system() {
        let json = json!({
            "role": "system",
            "content": "You are helpful."
        });
        let msg = to_ratatoskr_message(&json).unwrap();
        assert_eq!(msg.role, Role::System);
    }

    #[test]
    fn test_to_ratatoskr_message_tool_result() {
        let json = json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "Result here"
        });
        let msg = to_ratatoskr_message(&json).unwrap();
        assert!(matches!(msg.role, Role::Tool { tool_call_id } if tool_call_id == "call_123"));
    }

    #[test]
    fn test_to_chat_options_includes_parallel_tool_calls() {
        let config = test_config(|api| {
            api.parallel_tool_calls = Some(true);
        });
        let opts = to_chat_options(&config);
        assert_eq!(opts.parallel_tool_calls, Some(true));
    }

    #[test]
    fn test_to_chat_options_omits_parallel_tool_calls_when_none() {
        let config = test_config(|_| {});
        let opts = to_chat_options(&config);
        assert_eq!(opts.parallel_tool_calls, None);
    }

    #[test]
    fn test_to_chat_options_includes_tool_choice() {
        let config = test_config(|api| {
            api.tool_choice = Some(config::ToolChoice::Mode(config::ToolChoiceMode::Required));
        });
        let opts = to_chat_options(&config);
        assert!(matches!(
            opts.tool_choice,
            Some(RatatoskrToolChoice::Required)
        ));
    }

    #[test]
    fn test_tool_choice_conversion_auto() {
        let choice = config::ToolChoice::Mode(config::ToolChoiceMode::Auto);
        assert!(matches!(
            to_ratatoskr_tool_choice(&choice),
            RatatoskrToolChoice::Auto
        ));
    }

    #[test]
    fn test_tool_choice_conversion_none() {
        let choice = config::ToolChoice::Mode(config::ToolChoiceMode::None);
        assert!(matches!(
            to_ratatoskr_tool_choice(&choice),
            RatatoskrToolChoice::None
        ));
    }

    #[test]
    fn test_tool_choice_conversion_required() {
        let choice = config::ToolChoice::Mode(config::ToolChoiceMode::Required);
        assert!(matches!(
            to_ratatoskr_tool_choice(&choice),
            RatatoskrToolChoice::Required
        ));
    }

    #[test]
    fn test_tool_choice_conversion_function() {
        let choice = config::ToolChoice::Function {
            type_: "function".to_string(),
            function: config::ToolChoiceFunction {
                name: "my_tool".to_string(),
            },
        };
        match to_ratatoskr_tool_choice(&choice) {
            RatatoskrToolChoice::Function { name } => assert_eq!(name, "my_tool"),
            other => panic!("expected Function, got {:?}", other),
        }
    }

    /// Helper to build a minimal ResolvedConfig for gateway tests.
    fn test_config(api_modifier: impl FnOnce(&mut config::ApiParams)) -> ResolvedConfig {
        let mut api = config::ApiParams::default();
        api_modifier(&mut api);
        ResolvedConfig {
            api_key: None,
            model: "test-model".to_string(),
            context_window_limit: 4096,
            warn_threshold_percent: 80.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 0.9,
            fuel: 10,
            fuel_empty_response_cost: 15,
            username: "test".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 10000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api,
            tools: config::ToolsConfig::default(),
            fallback_tool: "call_user".to_string(),
            storage: crate::partition::StorageConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn test_to_ratatoskr_message_assistant_with_tool_calls() {
        let json = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_abc",
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "arguments": "{\"city\":\"Paris\"}"
                }
            }]
        });
        let msg = to_ratatoskr_message(&json).unwrap();
        assert_eq!(msg.role, Role::Assistant);
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "get_weather");
    }

    #[test]
    fn resolve_context_window_skips_when_already_set() {
        let gateway = build_gateway(&test_config(|_| {})).unwrap();
        let mut config = test_config(|_| {});
        config.context_window_limit = 8192;
        resolve_context_window(&mut config, &gateway);
        assert_eq!(config.context_window_limit, 8192);
    }

    #[test]
    fn resolve_context_window_stays_zero_for_unknown_model() {
        let gateway = build_gateway(&test_config(|_| {})).unwrap();
        let mut config = test_config(|_| {});
        config.context_window_limit = 0;
        config.model = "nonexistent/model-that-does-not-exist".to_string();
        resolve_context_window(&mut config, &gateway);
        assert_eq!(config.context_window_limit, 0);
    }
}
