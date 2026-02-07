//! Gateway abstraction over ratatoskr.
//!
//! This module provides type conversions between chibi's internal types
//! and ratatoskr's ModelGateway types.

use crate::config::{self, ResolvedConfig};
use ratatoskr::{
    ChatOptions, EmbeddedGateway, Message, ModelGateway, Ratatoskr, ToolCall,
    ToolChoice as RatatoskrToolChoice, ToolDefinition,
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
    let mut opts = ChatOptions::default().model(&config.model);

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
    if let Some(parallel) = api.parallel_tool_calls {
        opts = opts.parallel_tool_calls(parallel);
    }
    if let Some(ref tool_choice) = api.tool_choice {
        opts = opts.tool_choice(to_ratatoskr_tool_choice(tool_choice));
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

/// Build a gateway from ResolvedConfig.
pub fn build_gateway(config: &ResolvedConfig) -> io::Result<EmbeddedGateway> {
    // For now, always use OpenRouter (chibi's current default)
    Ratatoskr::builder()
        .openrouter(&config.api_key)
        .build()
        .map_err(|e| io::Error::other(format!("Failed to build gateway: {}", e)))
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
            api_key: String::new(),
            model: "test-model".to_string(),
            context_window_limit: 4096,
            warn_threshold_percent: 80.0,
            verbose: false,
            hide_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 0.9,
            max_recursion_depth: 10,
            max_empty_responses: 3,
            username: "test".to_string(),
            reflection_enabled: false,
            tool_output_cache_threshold: 10000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api,
            tools: config::ToolsConfig::default(),
            fallback_tool: "call_user".to_string(),
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
}
