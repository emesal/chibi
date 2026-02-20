//! Agent tools for spawning sub-agents and retrieving content.
//!
//! Provides two LLM-facing tools:
//! - `spawn_agent` — general-purpose sub-agent spawning (also usable as internal Rust API)
//! - `summarize_content` — reads files/URLs and processes content through a sub-agent
//!
//! Sub-agent calls are non-streaming (results are tool outputs, not user-facing).
//! Hooks (`pre_spawn_agent` / `post_spawn_agent`) allow plugins to intercept or observe.

use super::builtin::{BuiltinToolDef, ToolPropertyDef};
use super::{HookPoint, Tool, execute_hook};
use crate::config::ResolvedConfig;
use crate::gateway;
use crate::json_ext::JsonExt;
use serde_json::json;
use std::io::{self, ErrorKind};

// === Tool Name Constants ===

pub const SPAWN_AGENT_TOOL_NAME: &str = "spawn_agent";
pub const SUMMARIZE_CONTENT_TOOL_NAME: &str = "summarize_content";

// === Tool Definition Registry ===

/// All agent tool definitions
pub static AGENT_TOOL_DEFS: &[BuiltinToolDef] = &[
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
                description: "PRESET_DESCRIPTION_PLACEHOLDER",
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

// === Types ===

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
fn apply_preset_defaults(
    params: &ratatoskr::PresetParameters,
    api: &mut crate::config::ApiParams,
) {
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
///
/// Delegates to `fetch_url_with_limit` for streaming size-limited fetching.
async fn fetch_url(url: &str) -> io::Result<String> {
    super::fetch_url_with_limit(url, 1_048_576, 30).await
}

/// Check if a source string is a URL (http:// or https://).
pub fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

// === Core Functions ===

/// Reusable async primitive — both tool-facing and internal.
/// Fires pre/post_spawn_agent hooks.
pub async fn spawn_agent(
    config: &ResolvedConfig,
    system_prompt: &str,
    input: &str,
    options: &SpawnOptions,
    tools: &[Tool],
) -> io::Result<String> {
    // Build gateway once for preset resolution; gateway::chat builds its own internally.
    let gateway = gateway::build_gateway(config).ok();
    let effective_config = apply_spawn_options(config, options, gateway.as_ref());

    // Fire pre_spawn_agent hook
    let hook_data = json!({
        "system_prompt": system_prompt,
        "input": input,
        "model": effective_config.model,
        "temperature": effective_config.api.temperature,
        "max_tokens": effective_config.api.max_tokens,
    });
    let hook_results = execute_hook(tools, HookPoint::PreSpawnAgent, &hook_data)?;

    for (_hook_name, result) in &hook_results {
        // Hook can provide a replacement response (skip LLM call)
        if let Some(response) = result.get_str("response") {
            return Ok(response.to_string());
        }
        // Hook can block the call entirely
        if result.get_bool_or("block", false) {
            let message = result
                .get_str_or("message", "Sub-agent call blocked by hook")
                .to_string();
            return Ok(message);
        }
    }

    // Build messages for the sub-agent call
    let messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": input }),
    ];

    let response = gateway::chat(&effective_config, &messages).await?;

    // Fire post_spawn_agent hook
    let post_hook_data = json!({
        "system_prompt": system_prompt,
        "input": input,
        "model": effective_config.model,
        "response": response,
    });
    let _ = execute_hook(tools, HookPoint::PostSpawnAgent, &post_hook_data);

    Ok(response)
}

/// Reads file or fetches URL, then delegates to spawn_agent for summarization.
pub async fn summarize_content(
    config: &ResolvedConfig,
    source: &str,
    instructions: &str,
    options: &SpawnOptions,
    tools: &[Tool],
) -> io::Result<String> {
    // Fetch content
    let content = if is_url(source) {
        fetch_url(source).await?
    } else {
        read_file(source, config)?
    };

    if content.is_empty() {
        return Ok(format!("Source '{}' is empty.", source));
    }

    // Build system prompt and input for the sub-agent
    let system_prompt = "You are a content processing assistant. Follow the user's instructions to process the provided content. Be concise and focused.";
    let input = format!(
        "## Instructions\n{}\n\n## Content from: {}\n{}",
        instructions, source, content
    );

    spawn_agent(config, system_prompt, &input, options, tools).await
}

// === Dispatcher & Utilities ===

/// Check if a tool name is an agent tool.
pub fn is_agent_tool(name: &str) -> bool {
    matches!(name, SPAWN_AGENT_TOOL_NAME | SUMMARIZE_CONTENT_TOOL_NAME)
}

/// Execute an agent tool by name.
pub async fn execute_agent_tool(
    config: &ResolvedConfig,
    tool_name: &str,
    args: &serde_json::Value,
    tools: &[Tool],
) -> io::Result<String> {
    let options = SpawnOptions::from_args(args);

    match tool_name {
        SPAWN_AGENT_TOOL_NAME => {
            let system_prompt = args.get_str("system_prompt").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'system_prompt' parameter")
            })?;
            let input = args.get_str("input").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'input' parameter")
            })?;
            spawn_agent(config, system_prompt, input, &options, tools).await
        }
        SUMMARIZE_CONTENT_TOOL_NAME => {
            let source = args.get_str("source").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'source' parameter")
            })?;
            let instructions = args.get_str("instructions").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'instructions' parameter")
            })?;
            summarize_content(config, source, instructions, &options, tools).await
        }
        _ => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Unknown agent tool: {}", tool_name),
        )),
    }
}

/// Convert all agent tools to API format.
///
/// `preset_capabilities`: capability names available via the configured cost tier.
/// If empty, the `preset` param description notes no presets are configured.
/// Explicit capability names are injected into the `spawn_agent` tool description
/// at runtime so the LLM knows which values are valid.
pub fn all_agent_tools_to_api_format(preset_capabilities: &[&str]) -> Vec<serde_json::Value> {
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

    AGENT_TOOL_DEFS
        .iter()
        .map(|def| {
            let mut json = def.to_api_format();
            // Inject dynamic preset description into spawn_agent only
            if def.name == SPAWN_AGENT_TOOL_NAME {
                json["function"]["parameters"]["properties"]["preset"]["description"] =
                    serde_json::Value::String(preset_desc.clone());
            }
            json
        })
        .collect()
}

/// Look up a specific agent tool definition by name.
pub fn get_agent_tool_def(name: &str) -> Option<&'static BuiltinToolDef> {
    AGENT_TOOL_DEFS.iter().find(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Helper ===
    fn get_tool_api(name: &str) -> serde_json::Value {
        get_agent_tool_def(name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Registry Tests ===

    #[test]
    fn test_agent_tool_registry_contains_all_tools() {
        assert_eq!(AGENT_TOOL_DEFS.len(), 2);
        let names: Vec<_> = AGENT_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&SPAWN_AGENT_TOOL_NAME));
        assert!(names.contains(&SUMMARIZE_CONTENT_TOOL_NAME));
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(SPAWN_AGENT_TOOL_NAME, "spawn_agent");
        assert_eq!(SUMMARIZE_CONTENT_TOOL_NAME, "summarize_content");
    }

    #[test]
    fn test_is_agent_tool() {
        assert!(is_agent_tool(SPAWN_AGENT_TOOL_NAME));
        assert!(is_agent_tool(SUMMARIZE_CONTENT_TOOL_NAME));
        assert!(!is_agent_tool("file_head"));
        assert!(!is_agent_tool("other_tool"));
    }

    #[test]
    fn test_get_agent_tool_def() {
        assert!(get_agent_tool_def(SPAWN_AGENT_TOOL_NAME).is_some());
        assert!(get_agent_tool_def(SUMMARIZE_CONTENT_TOOL_NAME).is_some());
        assert!(get_agent_tool_def("nonexistent").is_none());
    }

    #[test]
    fn test_all_agent_tools_to_api_format() {
        let tools = all_agent_tools_to_api_format(&[]);
        assert_eq!(tools.len(), 2);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].is_string());
        }
    }

    // === spawn_agent Tool API ===

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

    // === summarize_content Tool API ===

    #[test]
    fn test_summarize_content_tool_api_format() {
        let tool = get_tool_api(SUMMARIZE_CONTENT_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], SUMMARIZE_CONTENT_TOOL_NAME);
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
    fn test_spawn_options_from_args_partial() {
        let args = json!({ "model": "claude-3" });
        let opts = SpawnOptions::from_args(&args);
        assert_eq!(opts.model.as_deref(), Some("claude-3"));
        assert!(opts.temperature.is_none());
        assert!(opts.max_tokens.is_none());
    }

    #[test]
    fn test_spawn_options_from_args_empty() {
        let args = json!({});
        let opts = SpawnOptions::from_args(&args);
        assert!(opts.model.is_none());
        assert!(opts.temperature.is_none());
        assert!(opts.max_tokens.is_none());
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
        // Other fields unchanged
        assert_eq!(result.api.temperature, config.api.temperature);
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
        assert_eq!(result.model, config.model);
    }

    #[test]
    fn test_apply_spawn_options_no_overrides() {
        let config = make_test_config();
        let opts = SpawnOptions::default();
        let result = apply_spawn_options(&config, &opts, None);
        assert_eq!(result.model, config.model);
        assert_eq!(result.api.temperature, config.api.temperature);
        assert_eq!(result.api.max_tokens, config.api.max_tokens);
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
    fn test_read_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let result = read_file(
            &format!("{}/nonexistent.txt", dir.path().display()),
            &config,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();
        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let result = read_file(path.to_str().unwrap(), &config).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_file_respects_allowed_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("allowed.txt");
        std::fs::write(&file, "allowed content").unwrap();

        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
        let result = read_file(file.to_str().unwrap(), &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "allowed content");
    }

    #[test]
    fn test_read_file_denies_outside_allowed_paths() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, "secret").unwrap();

        let mut config = make_test_config();
        config.file_tools_allowed_paths = vec![allowed.path().to_string_lossy().to_string()];
        let result = read_file(file.to_str().unwrap(), &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_denies_empty_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();

        let config = make_test_config(); // empty allowed_paths
        let result = read_file(file.to_str().unwrap(), &config);
        assert!(result.is_err());
    }

    // === preset / apply_preset_defaults ===

    #[test]
    fn test_spawn_options_preset_from_args() {
        let args = json!({ "preset": "fast" });
        let opts = SpawnOptions::from_args(&args);
        assert_eq!(opts.preset, Some("fast".to_string()));
    }

    #[test]
    fn test_spawn_options_no_preset() {
        let args = json!({ "model": "some/model" });
        let opts = SpawnOptions::from_args(&args);
        assert!(opts.preset.is_none());
        assert_eq!(opts.model, Some("some/model".to_string()));
    }

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
        api.temperature = Some(0.9); // caller already set this
        apply_preset_defaults(&params, &mut api);
        assert_eq!(api.temperature, Some(0.9)); // caller wins
    }

    // === Dynamic tool description ===

    #[test]
    fn test_agent_tool_schema_has_preset_param() {
        let spawn = get_tool_api(SPAWN_AGENT_TOOL_NAME);
        let params = &spawn["function"]["parameters"]["properties"];
        assert!(params.get("preset").is_some(), "spawn_agent should have preset param");
    }

    #[test]
    fn test_agent_tools_description_lists_capabilities() {
        let tools = all_agent_tools_to_api_format(&["fast", "reasoning"]);
        let spawn = tools
            .iter()
            .find(|t| t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME))
            .expect("spawn_agent in list");
        let preset_desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(preset_desc.contains("fast"), "description should list 'fast'");
        assert!(
            preset_desc.contains("reasoning"),
            "description should list 'reasoning'"
        );
    }

    #[test]
    fn test_agent_tools_description_no_capabilities() {
        let tools = all_agent_tools_to_api_format(&[]);
        let spawn = tools
            .iter()
            .find(|t| t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME))
            .expect("spawn_agent in list");
        let preset_desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(
            preset_desc.contains("no presets"),
            "should mention no presets configured"
        );
    }

    // === Gateway preset wiring ===

    #[test]
    fn test_apply_spawn_options_preset_sets_model() {
        // Tests the plumbing: if we pass a real EmbeddedGateway and a preset
        // that exists, the model in the returned config should change.
        use ratatoskr::ModelGateway;
        let config = make_test_config();
        let gateway = crate::gateway::build_gateway(&config).expect("gateway should build");
        let presets = gateway.list_presets();
        // Only run the assertion if any preset exists
        if let Some((tier, caps)) = presets.iter().next() {
            if let Some(capability) = caps.iter().next() {
                let mut effective_config = make_test_config();
                effective_config.subagent_cost_tier = tier.clone();
                let opts = SpawnOptions {
                    preset: Some(capability.clone()),
                    ..Default::default()
                };
                let result = apply_spawn_options(&effective_config, &opts, Some(&gateway));
                // model should have changed from the default
                assert_ne!(
                    result.model, effective_config.model,
                    "preset should have changed the model"
                );
            }
        }
    }

    // === Source classification ===

    #[test]
    fn test_is_url() {
        assert!(is_url("http://example.com"));
        assert!(is_url("https://example.com/path"));
        assert!(!is_url("/home/user/file.txt"));
        assert!(!is_url("~/file.txt"));
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
