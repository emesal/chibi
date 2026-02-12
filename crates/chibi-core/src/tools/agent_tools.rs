//! Agent tools for spawning sub-agents and retrieving content.
//!
//! Provides two LLM-facing tools:
//! - `spawn_agent` — general-purpose sub-agent spawning (also usable as internal Rust API)
//! - `retrieve_content` — reads files/URLs and processes content through a sub-agent
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
pub const RETRIEVE_CONTENT_TOOL_NAME: &str = "retrieve_content";

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
        ],
        required: &["system_prompt", "input"],
    },
    BuiltinToolDef {
        name: RETRIEVE_CONTENT_TOOL_NAME,
        description: "Read a file or fetch a URL, then process the content through a sub-agent with your instructions. Use for summarizing documents, extracting information, or analyzing content.",
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
        }
    }
}

/// Apply spawn options to a cloned config, returning the effective config.
fn apply_spawn_options(config: &ResolvedConfig, opts: &SpawnOptions) -> ResolvedConfig {
    let mut c = config.clone();
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

/// Read a file, with tilde expansion.
fn read_file(path: &str) -> io::Result<String> {
    let resolved = if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?;
        home.join(rest)
    } else if path == "~" {
        dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?
    } else {
        std::path::PathBuf::from(path)
    };
    std::fs::read_to_string(&resolved)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to read '{}': {}", path, e)))
}

/// Fetch content from a URL with a 30-second timeout.
async fn fetch_url(url: &str) -> io::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| io::Error::other(format!("Failed to build HTTP client: {}", e)))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Failed to fetch '{}': {}", url, e)))?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "HTTP {} fetching '{}'",
            response.status(),
            url
        )));
    }

    response
        .text()
        .await
        .map_err(|e| io::Error::other(format!("Failed to read response from '{}': {}", url, e)))
}

/// Check if a source string is a URL (http:// or https://).
fn is_url(source: &str) -> bool {
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
    let effective_config = apply_spawn_options(config, options);

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

/// Reads file or fetches URL, then delegates to spawn_agent.
pub async fn retrieve_content(
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
        read_file(source)?
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
    matches!(name, SPAWN_AGENT_TOOL_NAME | RETRIEVE_CONTENT_TOOL_NAME)
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
        RETRIEVE_CONTENT_TOOL_NAME => {
            let source = args.get_str("source").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'source' parameter")
            })?;
            let instructions = args.get_str("instructions").ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "Missing 'instructions' parameter")
            })?;
            retrieve_content(config, source, instructions, &options, tools).await
        }
        _ => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Unknown agent tool: {}", tool_name),
        )),
    }
}

/// Convert all agent tools to API format.
pub fn all_agent_tools_to_api_format() -> Vec<serde_json::Value> {
    AGENT_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
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
        assert!(names.contains(&RETRIEVE_CONTENT_TOOL_NAME));
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(SPAWN_AGENT_TOOL_NAME, "spawn_agent");
        assert_eq!(RETRIEVE_CONTENT_TOOL_NAME, "retrieve_content");
    }

    #[test]
    fn test_is_agent_tool() {
        assert!(is_agent_tool(SPAWN_AGENT_TOOL_NAME));
        assert!(is_agent_tool(RETRIEVE_CONTENT_TOOL_NAME));
        assert!(!is_agent_tool("file_head"));
        assert!(!is_agent_tool("other_tool"));
    }

    #[test]
    fn test_get_agent_tool_def() {
        assert!(get_agent_tool_def(SPAWN_AGENT_TOOL_NAME).is_some());
        assert!(get_agent_tool_def(RETRIEVE_CONTENT_TOOL_NAME).is_some());
        assert!(get_agent_tool_def("nonexistent").is_none());
    }

    #[test]
    fn test_all_agent_tools_to_api_format() {
        let tools = all_agent_tools_to_api_format();
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

    // === retrieve_content Tool API ===

    #[test]
    fn test_retrieve_content_tool_api_format() {
        let tool = get_tool_api(RETRIEVE_CONTENT_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], RETRIEVE_CONTENT_TOOL_NAME);
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
        let result = apply_spawn_options(&config, &opts);
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
        let result = apply_spawn_options(&config, &opts);
        assert_eq!(result.api.temperature, Some(0.9));
        assert_eq!(result.model, config.model);
    }

    #[test]
    fn test_apply_spawn_options_no_overrides() {
        let config = make_test_config();
        let opts = SpawnOptions::default();
        let result = apply_spawn_options(&config, &opts);
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
        let result = read_file(path.to_str().unwrap()).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_read_file_not_found() {
        let result = read_file("/nonexistent/path/to/file.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();
        let result = read_file(path.to_str().unwrap()).unwrap();
        assert_eq!(result, "");
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
        ResolvedConfig {
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 128000,
            warn_threshold_percent: 0.8,
            verbose: false,
            hide_tool_calls: false,
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
        }
    }
}
