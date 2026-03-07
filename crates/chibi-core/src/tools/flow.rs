//!
//! Flow tools: control flow, spawning, coordination, and model introspection.
//! call_agent, call_user, send_message, model_info, spawn_agent, summarize_content.

use super::{BuiltinToolDef, Tool, ToolMetadata, ToolPropertyDef};
use super::{HookPoint, execute_hook};
use crate::config::ResolvedConfig;
use crate::gateway;
use crate::json_ext::JsonExt;
use serde_json::json;
use std::io::{self, ErrorKind};

// === Tool Name Constants ===

pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";
pub const CALL_AGENT_TOOL_NAME: &str = "call_agent";
pub const CALL_USER_TOOL_NAME: &str = "call_user";
pub const MODEL_INFO_TOOL_NAME: &str = "model_info";
pub const SPAWN_AGENT_TOOL_NAME: &str = "spawn_agent";
pub const SUMMARIZE_CONTENT_TOOL_NAME: &str = "summarize_content";

const SUMMARIZE_CONTENT_SYSTEM_PROMPT: &str = include_str!("../../prompts/summarize-content.md");

// === Tool Definition Registry ===

pub static FLOW_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: SEND_MESSAGE_TOOL_NAME,
        description: "Send a message to another context's inbox. The message will be delivered to the target context and shown to them before their next prompt.",
        properties: &[
            ToolPropertyDef {
                name: "to",
                prop_type: "string",
                description: "Target context name",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "Message content",
                default: None,
            },
            ToolPropertyDef {
                name: "from",
                prop_type: "string",
                description: "Optional sender name (defaults to current context)",
                default: None,
            },
        ],
        required: &["to", "content"],
        summary_params: &["to"],
    },
    // NOTE: call_agent is intentionally excluded from the tool registry.
    // It is not exposed to the LLM as a callable tool.
    // The CALL_AGENT_TOOL_NAME constant and HandoffTarget::Agent infrastructure
    // are retained for:
    //   1. The fallback tool mechanism (config.fallback_tool can be "call_agent")
    //   2. Hook overrides (hooks can set fallback to call_agent)
    //   3. Future inter-agent control transfer (call_agent will be repurposed
    //      to transfer control to another agent context)
    BuiltinToolDef {
        name: CALL_USER_TOOL_NAME,
        description: "End your turn immediately and return control to the user.",
        properties: &[ToolPropertyDef {
            name: "message",
            prop_type: "string",
            description: "Final message to show the user.",
            default: None,
        }],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: MODEL_INFO_TOOL_NAME,
        description: "Look up metadata for a model: context window, max output tokens, pricing, capabilities, and parameter ranges. Use this to check model specifications before making recommendations or decisions about model selection.",
        properties: &[ToolPropertyDef {
            name: "model",
            prop_type: "string",
            description: "Model identifier (e.g. 'anthropic/claude-sonnet-4')",
            default: None,
        }],
        required: &["model"],
        summary_params: &["model"],
    },
    BuiltinToolDef {
        name: SPAWN_AGENT_TOOL_NAME,
        description: "Spawn a sub-agent with a custom system prompt to process input. Returns the sub-agent's response. Use for analysis, summarization, translation, or any task benefiting from a focused system prompt.",
        properties: &[
            ToolPropertyDef {
                name: "system_prompt",
                prop_type: "string",
                description: "System prompt for the sub-agent",
                default: None,
            },
            ToolPropertyDef {
                name: "input",
                prop_type: "string",
                description: "Content for the sub-agent to process",
                default: None,
            },
            ToolPropertyDef {
                name: "model",
                prop_type: "string",
                description: "Model override (defaults to parent's model)",
                default: None,
            },
            ToolPropertyDef {
                name: "temperature",
                prop_type: "number",
                description: "Temperature override for the sub-agent",
                default: None,
            },
            ToolPropertyDef {
                name: "max_tokens",
                prop_type: "integer",
                description: "Max tokens override for the sub-agent",
                default: None,
            },
            ToolPropertyDef {
                name: "preset",
                prop_type: "string",
                description: "Named preset configuration for the sub-agent (dynamically populated)",
                default: None,
            },
        ],
        required: &["system_prompt", "input"],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: SUMMARIZE_CONTENT_TOOL_NAME,
        description: "Read a file or fetch a URL, then summarize or process the content through a sub-agent with your instructions. Use for summarizing documents, extracting information, or analyzing content.",
        properties: &[
            ToolPropertyDef {
                name: "source",
                prop_type: "string",
                description: "File path or URL to read",
                default: None,
            },
            ToolPropertyDef {
                name: "instructions",
                prop_type: "string",
                description: "How to process/summarize the content",
                default: None,
            },
            ToolPropertyDef {
                name: "model",
                prop_type: "string",
                description: "Model override (defaults to parent's model)",
                default: None,
            },
            ToolPropertyDef {
                name: "temperature",
                prop_type: "number",
                description: "Temperature override for the sub-agent",
                default: None,
            },
            ToolPropertyDef {
                name: "max_tokens",
                prop_type: "integer",
                description: "Max tokens override for the sub-agent",
                default: None,
            },
        ],
        required: &["source", "instructions"],
        summary_params: &["source"],
    },
];

// === Handoff Types ===

/// Target for control handoff after tool execution
#[derive(Debug, Clone)]
pub enum HandoffTarget {
    /// Continue with LLM processing
    Agent { prompt: String },
    /// Return control to user
    User { message: String },
}

impl Default for HandoffTarget {
    fn default() -> Self {
        Self::Agent {
            prompt: String::new(),
        }
    }
}

/// Tracks handoff decision during tool execution.
/// Last explicit call wins; falls back to configured default.
#[derive(Debug)]
pub struct Handoff {
    next: Option<HandoffTarget>,
    fallback: HandoffTarget,
}

impl Handoff {
    pub fn new(fallback: HandoffTarget) -> Self {
        Self {
            next: None,
            fallback,
        }
    }

    pub fn set_agent(&mut self, prompt: String) {
        self.next = Some(HandoffTarget::Agent { prompt });
    }

    pub fn set_user(&mut self, message: String) {
        self.next = Some(HandoffTarget::User { message });
    }

    /// Take the handoff decision, resetting to fallback for next use
    pub fn take(&mut self) -> HandoffTarget {
        self.next.take().unwrap_or_else(|| self.fallback.clone())
    }

    /// Override the fallback target (used by hooks)
    pub fn set_fallback(&mut self, target: HandoffTarget) {
        self.fallback = target;
    }

    /// Check if an explicit end-turn (call_user) has been requested
    pub fn ends_turn_requested(&self) -> bool {
        matches!(self.next, Some(HandoffTarget::User { .. }))
    }
}

// === Spawn Options ===

/// Options for sub-agent spawning.
/// Designed to grow as ratatoskr gains model metadata and presets.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// Model override (uses parent's model if None)
    pub model: Option<String>,
    /// Temperature override
    pub temperature: Option<f32>,
    /// Max tokens override
    pub max_tokens: Option<usize>,
    /// Preset capability name (e.g. "fast", "reasoning").
    /// Resolved against `config.subagent_cost_tier`. Explicit model/temperature/max_tokens win over preset defaults.
    pub preset: Option<String>,
}

impl SpawnOptions {
    /// Parse spawn options from tool arguments.
    pub fn from_args(args: &serde_json::Value) -> Self {
        Self {
            model: args.get_str("model").map(String::from),
            temperature: args
                .get("temperature")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32),
            max_tokens: args
                .get("max_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize),
            preset: args.get_str("preset").map(String::from),
        }
    }
}

/// Apply `PresetParameters` as defaults to `ApiParams`.
/// Fills `None` fields only — never overwrites `Some` values set by the caller.
fn apply_preset_defaults(params: &ratatoskr::PresetParameters, api: &mut crate::config::ApiParams) {
    macro_rules! fill {
        ($field:ident) => {
            if api.$field.is_none() {
                api.$field = params.$field.clone();
            }
        };
    }
    fill!(temperature);
    fill!(top_p);
    fill!(max_tokens);
    fill!(frequency_penalty);
    fill!(presence_penalty);
    fill!(seed);
    fill!(stop);
    fill!(parallel_tool_calls);
    // Note: top_k, reasoning, tool_choice, response_format, cache_prompt,
    // raw_provider_options are in PresetParameters but not in ApiParams — skip.
}

/// Apply spawn options to a cloned config, returning the effective config.
/// `gateway` is used for preset resolution when `opts.preset` is set.
fn apply_spawn_options(
    config: &ResolvedConfig,
    opts: &SpawnOptions,
    gateway: Option<&ratatoskr::EmbeddedGateway>,
) -> ResolvedConfig {
    let mut c = config.clone();

    // Resolve preset first (explicit opts override preset defaults)
    if let (Some(capability), Some(gw)) = (opts.preset.as_deref(), gateway) {
        use ratatoskr::ModelGateway;
        match gw.resolve_preset(&config.subagent_cost_tier, capability) {
            Some(resolution) => {
                c.model = resolution.model;
                if let Some(params) = resolution.parameters {
                    apply_preset_defaults(&params, &mut c.api);
                }
            }
            None => {
                eprintln!(
                    "[WARN] spawn_agent: no preset for tier '{}' / capability '{}' — using parent model",
                    config.subagent_cost_tier, capability
                );
            }
        }
    }

    // Explicit overrides win over preset defaults
    if let Some(ref model) = opts.model {
        c.model = model.clone();
    }
    if let Some(temp) = opts.temperature {
        c.api.temperature = Some(temp);
    }
    if let Some(max) = opts.max_tokens {
        c.api.max_tokens = Some(max);
    }
    c
}

// === Content Reading ===

/// Read a file, validated against `file_tools_allowed_paths`.
fn read_file(path: &str, config: &ResolvedConfig) -> io::Result<String> {
    let validated = super::security::validate_file_path(path, config)?;
    std::fs::read_to_string(&validated)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to read '{}': {}", path, e)))
}

/// Fetch content from a URL with a 30-second timeout and 1 MB limit.
async fn fetch_url(url: &str) -> io::Result<String> {
    super::fetch_url_with_limit(url, 1_048_576, 30).await
}

/// Check if a source string is a URL (http:// or https://).
pub fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

// === Core Async Functions ===

/// Reusable async primitive — both tool-facing and internal.
/// Fires pre/post_spawn_agent hooks.
pub async fn spawn_agent(
    config: &ResolvedConfig,
    system_prompt: &str,
    input: &str,
    options: &SpawnOptions,
    tools: &[Tool],
) -> io::Result<String> {
    let gateway = gateway::build_gateway(config).ok();
    let effective_config = apply_spawn_options(config, options, gateway.as_ref());

    let hook_data = json!({
        "system_prompt": system_prompt,
        "input": input,
        "model": effective_config.model,
        "temperature": effective_config.api.temperature,
        "max_tokens": effective_config.api.max_tokens,
    });
    let hook_results = execute_hook(tools, HookPoint::PreSpawnAgent, &hook_data)?;

    for (_hook_name, result) in &hook_results {
        if let Some(response) = result.get_str("response") {
            return Ok(response.to_string());
        }
        if result.get_bool_or("block", false) {
            let message = result
                .get_str_or("message", "Sub-agent call blocked by hook")
                .to_string();
            return Ok(message);
        }
    }

    let messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": input }),
    ];

    let response = gateway::chat(&effective_config, &messages).await?;

    let post_hook_data = json!({
        "system_prompt": system_prompt,
        "input": input,
        "model": effective_config.model,
        "response": response,
    });
    let _ = execute_hook(tools, HookPoint::PostSpawnAgent, &post_hook_data);

    Ok(response)
}

/// Reads file or fetches URL, then delegates to spawn_agent for processing.
pub async fn summarize_content(
    config: &ResolvedConfig,
    source: &str,
    instructions: &str,
    options: &SpawnOptions,
    tools: &[Tool],
) -> io::Result<String> {
    let content = if is_url(source) {
        fetch_url(source).await?
    } else {
        read_file(source, config)?
    };

    if content.is_empty() {
        return Ok(format!("Source '{}' is empty.", source));
    }

    let system_prompt = SUMMARIZE_CONTENT_SYSTEM_PROMPT.trim();
    let input = format!(
        "## Instructions\n{}\n\n## Content from: {}\n{}",
        instructions, source, content
    );

    spawn_agent(config, system_prompt, &input, options, tools).await
}

// === Predicates & Dispatcher ===

/// Check if a tool name is a flow tool.
pub fn is_flow_tool(name: &str) -> bool {
    FLOW_TOOL_DEFS.iter().any(|d| d.name == name)
}

/// Get metadata for flow tools.
pub fn flow_tool_metadata(name: &str) -> ToolMetadata {
    match name {
        // call_agent: disabled as LLM tool but metadata retained for fallback mechanism
        // and hook overrides. See FLOW_TOOL_DEFS comment for full rationale.
        CALL_AGENT_TOOL_NAME => ToolMetadata {
            parallel: false,
            flow_control: true,
            ends_turn: false,
        },
        CALL_USER_TOOL_NAME => ToolMetadata {
            parallel: false,
            flow_control: true,
            ends_turn: true,
        },
        SPAWN_AGENT_TOOL_NAME => ToolMetadata {
            parallel: false,
            flow_control: false,
            ends_turn: false,
        },
        _ => ToolMetadata::new(),
    }
}

/// Execute an async flow tool (spawn_agent, summarize_content).
///
/// Returns `Ok(Some(result))` if handled, `Ok(None)` if not a flow tool.
/// Note: call_agent, call_user, send_message, model_info are handled specially
/// by the dispatcher in `api/send.rs` due to their async/hook requirements.
pub async fn execute_flow_tool(
    config: &ResolvedConfig,
    tool_name: &str,
    args: &serde_json::Value,
    tools: &[Tool],
) -> io::Result<Option<String>> {
    let options = SpawnOptions::from_args(args);
    match tool_name {
        SPAWN_AGENT_TOOL_NAME => {
            let system_prompt = args.get_str("system_prompt").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'system_prompt' parameter")
            })?;
            let input = args.get_str("input").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'input' parameter")
            })?;
            spawn_agent(config, system_prompt, input, &options, tools)
                .await
                .map(Some)
        }
        SUMMARIZE_CONTENT_TOOL_NAME => {
            let source = args.get_str("source").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'source' parameter")
            })?;
            let instructions = args.get_str("instructions").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'instructions' parameter")
            })?;
            summarize_content(config, source, instructions, &options, tools)
                .await
                .map(Some)
        }
        _ => Ok(None),
    }
}

/// Register all flow tools into the registry with per-tool metadata overrides.
///
/// Flow tools that are intercepted by `send.rs` middleware (send_message,
/// call_user, model_info) are registered for metadata/lookup purposes but their
/// handler returns an error if directly dispatched — `send.rs` must intercept
/// them before calling `registry.dispatch_with_context`.
pub fn register_flow_tools(registry: &mut super::registry::ToolRegistry) {
    use super::Tool;
    use super::registry::{ToolCategory, ToolHandler};
    use std::sync::Arc;

    for def in FLOW_TOOL_DEFS {
        let name = def.name;
        // Per-tool metadata: flow tools have non-default ToolMetadata values.
        let metadata = flow_tool_metadata(name);

        let handler: ToolHandler = Arc::new(move |call| {
            let ctx = call.context;
            let config = ctx.config;
            let tool_name = call.name;
            let args = call.args;
            Box::pin(async move {
                execute_flow_tool(config, tool_name, args, &[])
                    .await
                    // io::Result<Option<String>> -> io::Result<String>
                    .and_then(|opt| {
                        opt.ok_or_else(|| {
                            io::Error::other(format!(
                                "flow tool '{tool_name}' is handled by send.rs middleware \
                                 and must not be dispatched through the registry directly"
                            ))
                        })
                    })
            })
        });

        // Construct with per-tool metadata override (from_builtin_def uses ToolMetadata::new()).
        let tool = Tool {
            name: def.name.to_string(),
            description: def.description.to_string(),
            parameters: def.to_json_schema(),
            path: std::path::PathBuf::new(),
            hooks: vec![],
            metadata,
            summary_params: def.summary_params.iter().map(|s| s.to_string()).collect(),
            r#impl: super::registry::ToolImpl::Builtin(handler),
            category: ToolCategory::Flow,
        };
        registry.register(tool);
    }
}

/// Convert all flow tools to API format.
///
/// `preset_capabilities`: capability names available via the configured cost tier.
/// If empty, the `preset` param description notes no presets are configured.
pub fn all_flow_tools_to_api_format(preset_capabilities: &[&str]) -> Vec<serde_json::Value> {
    let preset_desc = if preset_capabilities.is_empty() {
        "Preset capability name (no presets configured for this tier)".to_string()
    } else {
        format!(
            "Preset capability name — one of: {} (cost tier set by config). \
             Sets the model and default parameters for the sub-agent. \
             Explicit model/temperature/max_tokens override preset defaults.",
            preset_capabilities.join(", ")
        )
    };

    FLOW_TOOL_DEFS
        .iter()
        .map(|def| {
            let mut json = def.to_api_format();
            if def.name == SPAWN_AGENT_TOOL_NAME {
                json["function"]["parameters"]["properties"]["preset"]["description"] =
                    serde_json::Value::String(preset_desc.clone());
            }
            json
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_tool_api(name: &str) -> serde_json::Value {
        FLOW_TOOL_DEFS
            .iter()
            .find(|d| d.name == name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Registry ===

    #[test]
    fn test_flow_registry_contains_all_tools() {
        // call_agent excluded (disabled as LLM tool — see FLOW_TOOL_DEFS comment)
        assert_eq!(FLOW_TOOL_DEFS.len(), 5);
        let names: Vec<_> = FLOW_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&SEND_MESSAGE_TOOL_NAME));
        assert!(
            !names.contains(&CALL_AGENT_TOOL_NAME),
            "call_agent must not be in registry"
        );
        assert!(names.contains(&CALL_USER_TOOL_NAME));
        assert!(names.contains(&MODEL_INFO_TOOL_NAME));
        assert!(names.contains(&SPAWN_AGENT_TOOL_NAME));
        assert!(names.contains(&SUMMARIZE_CONTENT_TOOL_NAME));
    }

    #[test]
    fn test_flow_tool_constants() {
        assert_eq!(SEND_MESSAGE_TOOL_NAME, "send_message");
        assert_eq!(CALL_AGENT_TOOL_NAME, "call_agent");
        assert_eq!(CALL_USER_TOOL_NAME, "call_user");
        assert_eq!(MODEL_INFO_TOOL_NAME, "model_info");
        assert_eq!(SPAWN_AGENT_TOOL_NAME, "spawn_agent");
        assert_eq!(SUMMARIZE_CONTENT_TOOL_NAME, "summarize_content");
    }

    #[test]
    fn test_is_flow_tool() {
        assert!(is_flow_tool(SEND_MESSAGE_TOOL_NAME));
        // call_agent: not in FLOW_TOOL_DEFS (disabled as LLM tool)
        assert!(
            !is_flow_tool(CALL_AGENT_TOOL_NAME),
            "call_agent must not be in flow tool registry"
        );
        assert!(is_flow_tool(CALL_USER_TOOL_NAME));
        assert!(is_flow_tool(MODEL_INFO_TOOL_NAME));
        assert!(is_flow_tool(SPAWN_AGENT_TOOL_NAME));
        assert!(is_flow_tool(SUMMARIZE_CONTENT_TOOL_NAME));
        assert!(!is_flow_tool("file_head"));
        assert!(!is_flow_tool("update_reflection"));
    }

    // === Metadata ===

    #[test]
    fn test_flow_tool_metadata_call_agent() {
        let meta = flow_tool_metadata(CALL_AGENT_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(!meta.ends_turn);
    }

    #[test]
    fn test_flow_tool_metadata_call_user() {
        let meta = flow_tool_metadata(CALL_USER_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(meta.ends_turn);
    }

    #[test]
    fn test_flow_tool_metadata_spawn_agent() {
        let meta = flow_tool_metadata(SPAWN_AGENT_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(!meta.flow_control);
        assert!(!meta.ends_turn);
    }

    // === Handoff ===

    #[test]
    fn test_handoff_default() {
        let target = HandoffTarget::default();
        match target {
            HandoffTarget::Agent { prompt } => assert!(prompt.is_empty()),
            _ => panic!("Expected Agent variant"),
        }
    }

    #[test]
    fn test_handoff_explicit_takes_precedence() {
        let fallback = HandoffTarget::User {
            message: "fallback".to_string(),
        };
        let mut handoff = Handoff::new(fallback);
        handoff.set_agent("explicit prompt".to_string());

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "explicit prompt"),
            _ => panic!("Expected Agent variant"),
        }
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_last_wins() {
        let fallback = HandoffTarget::Agent {
            prompt: String::new(),
        };
        let mut handoff = Handoff::new(fallback);
        handoff.set_agent("first".to_string());
        handoff.set_user("second".to_string());
        handoff.set_agent("third".to_string());

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "third"),
            _ => panic!("Expected Agent variant"),
        }
    }

    #[test]
    fn test_handoff_ends_turn_requested_user() {
        let mut handoff = Handoff::new(HandoffTarget::Agent {
            prompt: String::new(),
        });
        handoff.set_user("bye".to_string());
        assert!(handoff.ends_turn_requested());
    }

    #[test]
    fn test_handoff_ends_turn_requested_agent() {
        let mut handoff = Handoff::new(HandoffTarget::User {
            message: String::new(),
        });
        handoff.set_agent("continue".to_string());
        assert!(!handoff.ends_turn_requested());
    }

    // === Tool API format ===

    #[test]
    fn test_send_message_tool_api_format() {
        let tool = get_tool_api(SEND_MESSAGE_TOOL_NAME);
        assert_eq!(tool["function"]["name"], SEND_MESSAGE_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&json!("to")));
        assert!(required.contains(&json!("content")));
    }

    #[test]
    fn test_call_agent_not_in_registry_but_metadata_works() {
        // call_agent is disabled as an LLM tool — must not appear in FLOW_TOOL_DEFS
        assert!(
            FLOW_TOOL_DEFS
                .iter()
                .all(|d| d.name != CALL_AGENT_TOOL_NAME),
            "call_agent must not be in tool registry"
        );
        // But its metadata must still work for the fallback mechanism
        let meta = flow_tool_metadata(CALL_AGENT_TOOL_NAME);
        assert!(
            meta.flow_control,
            "call_agent metadata must have flow_control=true"
        );
        assert!(!meta.ends_turn, "call_agent must not end the turn");
    }

    #[test]
    fn test_call_user_tool_api_format() {
        let tool = get_tool_api(CALL_USER_TOOL_NAME);
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_spawn_agent_tool_api_format() {
        let tool = get_tool_api(SPAWN_AGENT_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], SPAWN_AGENT_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&json!("system_prompt")));
        assert!(required.contains(&json!("input")));
        assert_eq!(required.len(), 2);
    }

    #[test]
    fn test_summarize_content_tool_api_format() {
        let tool = get_tool_api(SUMMARIZE_CONTENT_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&json!("source")));
        assert!(required.contains(&json!("instructions")));
        assert_eq!(required.len(), 2);
    }

    // === SpawnOptions ===

    #[test]
    fn test_spawn_options_from_args_all_fields() {
        let args = json!({
            "model": "gpt-4",
            "temperature": 0.7,
            "max_tokens": 1000,
        });
        let opts = SpawnOptions::from_args(&args);
        assert_eq!(opts.model.as_deref(), Some("gpt-4"));
        assert_eq!(opts.temperature, Some(0.7));
        assert_eq!(opts.max_tokens, Some(1000));
    }

    #[test]
    fn test_spawn_options_from_args_empty() {
        let opts = SpawnOptions::from_args(&json!({}));
        assert!(opts.model.is_none());
        assert!(opts.temperature.is_none());
        assert!(opts.max_tokens.is_none());
    }

    #[test]
    fn test_spawn_options_preset_from_args() {
        let args = json!({ "preset": "fast" });
        let opts = SpawnOptions::from_args(&args);
        assert_eq!(opts.preset, Some("fast".to_string()));
    }

    // === apply_spawn_options ===

    #[test]
    fn test_apply_spawn_options_model_override() {
        let config = make_test_config();
        let opts = SpawnOptions {
            model: Some("new-model".to_string()),
            ..Default::default()
        };
        let result = apply_spawn_options(&config, &opts, None);
        assert_eq!(result.model, "new-model");
    }

    #[test]
    fn test_apply_spawn_options_temperature_override() {
        let config = make_test_config();
        let opts = SpawnOptions {
            temperature: Some(0.9),
            ..Default::default()
        };
        let result = apply_spawn_options(&config, &opts, None);
        assert_eq!(result.api.temperature, Some(0.9));
    }

    #[test]
    fn test_apply_spawn_options_no_overrides() {
        let config = make_test_config();
        let opts = SpawnOptions::default();
        let result = apply_spawn_options(&config, &opts, None);
        assert_eq!(result.model, config.model);
    }

    // === apply_preset_defaults ===

    #[test]
    fn test_apply_preset_defaults_fills_none() {
        use ratatoskr::PresetParameters;
        let params = PresetParameters {
            temperature: Some(0.3),
            max_tokens: Some(2048),
            ..Default::default()
        };
        let mut api = crate::config::ApiParams::defaults();
        assert!(api.temperature.is_none());
        apply_preset_defaults(&params, &mut api);
        assert_eq!(api.temperature, Some(0.3));
        assert_eq!(api.max_tokens, Some(2048));
    }

    #[test]
    fn test_apply_preset_defaults_preserves_existing() {
        use ratatoskr::PresetParameters;
        let params = PresetParameters {
            temperature: Some(0.3),
            ..Default::default()
        };
        let mut api = crate::config::ApiParams::defaults();
        api.temperature = Some(0.9);
        apply_preset_defaults(&params, &mut api);
        assert_eq!(api.temperature, Some(0.9));
    }

    // === Dynamic preset description ===

    #[test]
    fn test_all_flow_tools_description_lists_capabilities() {
        let tools = all_flow_tools_to_api_format(&["fast", "reasoning"]);
        let spawn = tools
            .iter()
            .find(|t| t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME))
            .expect("spawn_agent in list");
        let desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(desc.contains("fast"));
        assert!(desc.contains("reasoning"));
    }

    #[test]
    fn test_all_flow_tools_description_no_capabilities() {
        let tools = all_flow_tools_to_api_format(&[]);
        let spawn = tools
            .iter()
            .find(|t| t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME))
            .expect("spawn_agent in list");
        let desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(desc.contains("no presets"));
    }

    // === Gateway preset wiring ===

    #[test]
    fn test_apply_spawn_options_preset_sets_model() {
        use ratatoskr::ModelGateway;
        let config = make_test_config();
        let gateway = crate::gateway::build_gateway(&config).expect("gateway should build");
        let presets = gateway.list_presets();
        if let Some((tier, caps)) = presets.iter().next()
            && let Some(capability) = caps.iter().next()
        {
            let mut effective_config = make_test_config();
            effective_config.subagent_cost_tier = tier.clone();
            let opts = SpawnOptions {
                preset: Some(capability.clone()),
                ..Default::default()
            };
            let result = apply_spawn_options(&effective_config, &opts, Some(&gateway));
            assert_ne!(result.model, effective_config.model);
        }
    }

    // === read_file ===

    #[test]
    fn test_read_file_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let result = read_file(path.to_str().unwrap(), &config).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_read_file_denies_outside_allowed_paths() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, "secret").unwrap();
        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![allowed.path().to_string_lossy().to_string()];
        assert!(read_file(file.to_str().unwrap(), &config).is_err());
    }

    // === is_url ===

    #[test]
    fn test_is_url() {
        assert!(is_url("http://example.com"));
        assert!(is_url("https://example.com/path"));
        assert!(!is_url("/home/user/file.txt"));
        assert!(!is_url("relative/path"));
    }

    // === Test helpers ===

    fn make_test_config() -> ResolvedConfig {
        use crate::config::{ApiParams, ToolsConfig};
        use std::collections::BTreeMap;
        ResolvedConfig {
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            context_window_limit: 128000,
            warn_threshold_percent: 80.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 0.9,
            fuel: 5,
            fuel_empty_response_cost: 15,
            username: "user".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 5000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams {
                temperature: Some(0.5),
                max_tokens: Some(4096),
                ..Default::default()
            },
            tools: ToolsConfig::default(),
            fallback_tool: "call_agent".to_string(),
            storage: crate::partition::StorageConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
            extra: BTreeMap::new(),
        }
    }
}
