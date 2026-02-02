//! Gateway abstraction over ratatoskr.
//!
//! This module provides type conversions between chibi's internal types
//! and ratatoskr's ModelGateway types.

use crate::config::ResolvedConfig;
use crate::tools::Tool;
use ratatoskr::{
    ChatOptions, EmbeddedGateway, Message, ModelGateway, Ratatoskr, Role, ToolCall, ToolDefinition,
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

/// Convert ratatoskr Message back to chibi's JSON format.
///
/// Currently unused — will be needed when chibi's internals migrate from JSON to ratatoskr types.
#[allow(dead_code)]
pub fn from_ratatoskr_message(msg: &Message) -> serde_json::Value {
    use serde_json::json;

    let content = msg.content.as_text().unwrap_or("");

    match &msg.role {
        Role::System => json!({
            "role": "system",
            "content": content
        }),
        Role::User => json!({
            "role": "user",
            "content": content
        }),
        Role::Assistant => {
            let mut obj = json!({
                "role": "assistant",
                "content": content
            });
            if let Some(tool_calls) = &msg.tool_calls {
                let tc_json: Vec<serde_json::Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = json!(tc_json);
            }
            obj
        }
        Role::Tool { tool_call_id } => json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "content": content
        }),
    }
}

/// Convert chibi Tool to ratatoskr ToolDefinition.
///
/// Currently unused — will be needed when chibi's internals migrate from JSON to ratatoskr types.
#[allow(dead_code)]
pub fn to_tool_definition(tool: &Tool) -> ToolDefinition {
    ToolDefinition::new(&tool.name, &tool.description, tool.parameters.clone())
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

    opts
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
    fn test_from_ratatoskr_message_roundtrip() {
        let original = json!({
            "role": "user",
            "content": "Test message"
        });
        let msg = to_ratatoskr_message(&original).unwrap();
        let back = from_ratatoskr_message(&msg);
        assert_eq!(back["role"], "user");
        assert_eq!(back["content"], "Test message");
    }
}
