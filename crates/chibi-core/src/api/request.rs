//! API request building utilities.
//!
//! This module handles constructing the request body for LLM API calls,
//! applying all configuration parameters.

use crate::config::{ResolvedConfig, ToolChoice, ToolChoiceMode};
use crate::input::DebugKey;
use serde_json::json;

/// Options for controlling prompt execution behavior
#[derive(Debug, Clone)]
pub struct PromptOptions<'a> {
    pub verbose: bool,
    pub use_reflection: bool,
    pub debug: &'a [DebugKey],
    pub force_render: bool,
    /// Optional override for the fallback handoff target
    pub fallback_override: Option<crate::tools::HandoffTarget>,
}

impl<'a> PromptOptions<'a> {
    pub fn new(
        verbose: bool,
        use_reflection: bool,
        debug: &'a [DebugKey],
        force_render: bool,
    ) -> Self {
        Self {
            verbose,
            use_reflection,
            debug,
            force_render,
            fallback_override: None,
        }
    }

    /// Set the fallback handoff target override
    pub fn with_fallback(mut self, fallback: crate::tools::HandoffTarget) -> Self {
        self.fallback_override = Some(fallback);
        self
    }
}

/// Build the request body for the LLM API, applying all API parameters from ResolvedConfig
pub fn build_request_body(
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
