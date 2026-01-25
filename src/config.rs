use crate::partition::StorageConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

// ============================================================================
// API Parameters Types
// ============================================================================

/// Reasoning effort level for models that support it (e.g., OpenAI o3, Grok)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    XHigh,
    High,
    #[default]
    Medium,
    Low,
    Minimal,
    None,
}

impl ReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningEffort::XHigh => "xhigh",
            ReasoningEffort::High => "high",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::None => "none",
        }
    }
}

/// Reasoning configuration for models that support extended thinking
/// Either `effort` OR `max_tokens` should be set, not both (mutually exclusive)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReasoningConfig {
    /// Effort level (mutually exclusive with max_tokens)
    /// Supported by: OpenAI o1/o3/GPT-5 series, Grok models
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,

    /// Maximum tokens for reasoning (mutually exclusive with effort)
    /// Supported by: Gemini thinking models, Anthropic, some Qwen models
    /// Anthropic: min 1024, max 128000
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// Exclude reasoning from response (model still reasons internally)
    /// Default: false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,

    /// Explicitly enable reasoning (defaults to medium effort if true)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl ReasoningConfig {
    /// Check if this config has any values set
    pub fn is_empty(&self) -> bool {
        self.effort.is_none()
            && self.max_tokens.is_none()
            && self.exclude.is_none()
            && self.enabled.is_none()
    }

    /// Merge with another ReasoningConfig, where `other` takes precedence
    pub fn merge_with(&self, other: &ReasoningConfig) -> ReasoningConfig {
        ReasoningConfig {
            effort: other.effort.or(self.effort),
            max_tokens: other.max_tokens.or(self.max_tokens),
            exclude: other.exclude.or(self.exclude),
            enabled: other.enabled.or(self.enabled),
        }
    }
}

/// Tool choice mode for the API
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceMode {
    Auto,
    None,
    Required,
}

/// Specific function to call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Tool choice - either a mode or a specific function
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(ToolChoiceMode),
    Function {
        #[serde(rename = "type")]
        type_: String,
        function: ToolChoiceFunction,
    },
}

/// Response format specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        #[serde(skip_serializing_if = "Option::is_none")]
        json_schema: Option<serde_json::Value>,
    },
}

/// API parameters that can be configured at various levels
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiParams {
    // OpenRouter-specific
    /// Enable prompt caching (default: true, mainly for Anthropic models)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<bool>,
    /// Reasoning configuration (effort, max_tokens, exclude, enabled)
    /// Use either `reasoning.effort` OR `reasoning.max_tokens`, not both
    #[serde(default, skip_serializing_if = "ReasoningConfig::is_empty")]
    pub reasoning: ReasoningConfig,

    // Generation control
    /// Sampling temperature (0.0 to 2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    /// Nucleus sampling parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Stop sequences
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    // Tool control
    /// Tool choice mode or specific function
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Allow parallel tool calls (default: true)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,

    // Sampling penalties
    /// Frequency penalty (-2.0 to 2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    /// Presence penalty (-2.0 to 2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// Random seed for deterministic sampling
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    // Output format
    /// Response format specification
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

impl ApiParams {
    /// Create ApiParams with sensible defaults
    pub fn defaults() -> Self {
        Self {
            prompt_caching: Some(true),
            reasoning: ReasoningConfig {
                effort: Some(ReasoningEffort::Medium),
                ..Default::default()
            },
            parallel_tool_calls: Some(true),
            temperature: None,
            max_tokens: None,
            top_p: None,
            stop: None,
            tool_choice: None,
            frequency_penalty: None,
            presence_penalty: None,
            seed: None,
            response_format: None,
        }
    }

    /// Merge with another ApiParams, where `other` takes precedence for set values
    pub fn merge_with(&self, other: &ApiParams) -> ApiParams {
        ApiParams {
            prompt_caching: other.prompt_caching.or(self.prompt_caching),
            reasoning: self.reasoning.merge_with(&other.reasoning),
            temperature: other.temperature.or(self.temperature),
            max_tokens: other.max_tokens.or(self.max_tokens),
            top_p: other.top_p.or(self.top_p),
            stop: other.stop.clone().or_else(|| self.stop.clone()),
            tool_choice: other
                .tool_choice
                .clone()
                .or_else(|| self.tool_choice.clone()),
            parallel_tool_calls: other.parallel_tool_calls.or(self.parallel_tool_calls),
            frequency_penalty: other.frequency_penalty.or(self.frequency_penalty),
            presence_penalty: other.presence_penalty.or(self.presence_penalty),
            seed: other.seed.or(self.seed),
            response_format: other
                .response_format
                .clone()
                .or_else(|| self.response_format.clone()),
        }
    }
}

fn default_auto_compact() -> bool {
    false
}

fn default_auto_compact_threshold() -> f32 {
    80.0
}

fn default_base_url() -> String {
    DEFAULT_API_URL.to_string()
}

fn default_reflection_enabled() -> bool {
    true
}

fn default_reflection_character_limit() -> usize {
    10000
}

fn default_max_recursion_depth() -> usize {
    30
}

fn default_lock_heartbeat_seconds() -> u64 {
    30
}

fn default_username() -> String {
    "user".to_string()
}

fn default_rolling_compact_drop_percentage() -> f32 {
    50.0
}

fn default_tool_output_cache_threshold() -> usize {
    4000
}

fn default_tool_cache_max_age_days() -> u64 {
    7
}

fn default_auto_cleanup_cache() -> bool {
    true
}

fn default_tool_cache_preview_chars() -> usize {
    500
}

/// Global config from ~/.chibi/config.toml
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub model: String,
    pub context_window_limit: usize,
    pub warn_threshold_percent: f32,
    #[serde(default = "default_auto_compact")]
    pub auto_compact: bool,
    #[serde(default = "default_auto_compact_threshold")]
    pub auto_compact_threshold: f32,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_reflection_enabled")]
    pub reflection_enabled: bool,
    #[serde(default = "default_reflection_character_limit")]
    pub reflection_character_limit: usize,
    #[serde(default = "default_max_recursion_depth")]
    pub max_recursion_depth: usize,
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default = "default_lock_heartbeat_seconds")]
    pub lock_heartbeat_seconds: u64,
    #[serde(default = "default_rolling_compact_drop_percentage")]
    pub rolling_compact_drop_percentage: f32,
    /// Threshold (in chars) above which tool output is cached
    #[serde(default = "default_tool_output_cache_threshold")]
    pub tool_output_cache_threshold: usize,
    /// Maximum age in days for cached tool outputs
    #[serde(default = "default_tool_cache_max_age_days")]
    pub tool_cache_max_age_days: u64,
    /// Automatically cleanup old cache entries on exit
    #[serde(default = "default_auto_cleanup_cache")]
    pub auto_cleanup_cache: bool,
    /// Number of preview characters to show in truncated message
    #[serde(default = "default_tool_cache_preview_chars")]
    pub tool_cache_preview_chars: usize,
    /// Paths allowed for file tools (empty = cache only)
    #[serde(default)]
    pub file_tools_allowed_paths: Vec<String>,
    /// API parameters (temperature, max_tokens, etc.)
    #[serde(default)]
    pub api: ApiParams,
    /// Storage configuration for partitioned context storage
    #[serde(default)]
    pub storage: StorageConfig,
}

/// Per-context config from ~/.chibi/contexts/<name>/local.toml
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct LocalConfig {
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub username: Option<String>,
    pub auto_compact: Option<bool>,
    pub auto_compact_threshold: Option<f32>,
    pub max_recursion_depth: Option<usize>,
    pub warn_threshold_percent: Option<f32>,
    pub context_window_limit: Option<usize>,
    pub reflection_enabled: Option<bool>,
    /// Threshold (in chars) above which tool output is cached
    pub tool_output_cache_threshold: Option<usize>,
    /// Maximum age in days for cached tool outputs
    pub tool_cache_max_age_days: Option<u64>,
    /// Automatically cleanup old cache entries on exit
    pub auto_cleanup_cache: Option<bool>,
    /// Number of preview characters to show in truncated message
    pub tool_cache_preview_chars: Option<usize>,
    /// Paths allowed for file tools (empty = cache only)
    pub file_tools_allowed_paths: Option<Vec<String>>,
    /// API parameters (temperature, max_tokens, etc.)
    #[serde(default)]
    pub api: Option<ApiParams>,
    /// Per-context storage configuration overrides
    #[serde(default)]
    pub storage: StorageConfig,
}

/// Model metadata from ~/.chibi/models.toml
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelMetadata {
    #[serde(default)]
    pub context_window: Option<usize>,
    /// API parameters for this specific model
    #[serde(default)]
    pub api: ApiParams,
}

/// Models config containing model aliases/metadata
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default)]
    pub models: HashMap<String, ModelMetadata>,
}

/// Fully resolved configuration with all overrides applied
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub api_key: String,
    pub model: String,
    pub context_window_limit: usize,
    pub warn_threshold_percent: f32,
    pub base_url: String,
    pub auto_compact: bool,
    pub auto_compact_threshold: f32,
    pub max_recursion_depth: usize,
    pub username: String,
    pub reflection_enabled: bool,
    /// Threshold (in chars) above which tool output is cached
    pub tool_output_cache_threshold: usize,
    /// Maximum age in days for cached tool outputs
    pub tool_cache_max_age_days: u64,
    /// Automatically cleanup old cache entries on exit
    pub auto_cleanup_cache: bool,
    /// Number of preview characters to show in truncated message
    pub tool_cache_preview_chars: usize,
    /// Paths allowed for file tools (empty = cache only)
    pub file_tools_allowed_paths: Vec<String>,
    /// Resolved API parameters (merged from all layers)
    pub api: ApiParams,
}

impl ResolvedConfig {
    /// Get a config field value by path (e.g., "model", "api.temperature", "api.reasoning.effort").
    /// Returns None if the field doesn't exist or has no value set.
    /// Note: api_key is intentionally excluded for security.
    pub fn get_field(&self, path: &str) -> Option<String> {
        match path {
            // Top-level fields (excluding api_key for security)
            "model" => Some(self.model.clone()),
            "username" => Some(self.username.clone()),
            "base_url" => Some(self.base_url.clone()),
            "context_window_limit" => Some(self.context_window_limit.to_string()),
            "warn_threshold_percent" => Some(format!("{}", self.warn_threshold_percent as i32)),
            "auto_compact" => Some(self.auto_compact.to_string()),
            "auto_compact_threshold" => Some(format!("{}", self.auto_compact_threshold as i32)),
            "max_recursion_depth" => Some(self.max_recursion_depth.to_string()),
            "reflection_enabled" => Some(self.reflection_enabled.to_string()),
            "tool_output_cache_threshold" => Some(self.tool_output_cache_threshold.to_string()),
            "tool_cache_max_age_days" => Some(self.tool_cache_max_age_days.to_string()),
            "auto_cleanup_cache" => Some(self.auto_cleanup_cache.to_string()),
            "tool_cache_preview_chars" => Some(self.tool_cache_preview_chars.to_string()),
            "file_tools_allowed_paths" => {
                if self.file_tools_allowed_paths.is_empty() {
                    Some("(empty)".to_string())
                } else {
                    Some(self.file_tools_allowed_paths.join(", "))
                }
            }

            // API params (api.*)
            "api.temperature" => self.api.temperature.map(|v| format!("{}", v)),
            "api.max_tokens" => self.api.max_tokens.map(|v| v.to_string()),
            "api.top_p" => self.api.top_p.map(|v| format!("{}", v)),
            "api.prompt_caching" => self.api.prompt_caching.map(|v| v.to_string()),
            "api.parallel_tool_calls" => self.api.parallel_tool_calls.map(|v| v.to_string()),
            "api.frequency_penalty" => self.api.frequency_penalty.map(|v| format!("{}", v)),
            "api.presence_penalty" => self.api.presence_penalty.map(|v| format!("{}", v)),
            "api.seed" => self.api.seed.map(|v| v.to_string()),
            "api.stop" => self.api.stop.as_ref().map(|v| v.join(", ")),

            // Reasoning config (api.reasoning.*)
            "api.reasoning.effort" => self.api.reasoning.effort.map(|v| v.as_str().to_string()),
            "api.reasoning.max_tokens" => self.api.reasoning.max_tokens.map(|v| v.to_string()),
            "api.reasoning.exclude" => self.api.reasoning.exclude.map(|v| v.to_string()),
            "api.reasoning.enabled" => self.api.reasoning.enabled.map(|v| v.to_string()),

            _ => None,
        }
    }

    /// List all inspectable config field paths.
    /// Note: api_key is intentionally excluded for security.
    pub fn list_fields() -> &'static [&'static str] {
        &[
            // Top-level fields
            "model",
            "username",
            "base_url",
            "context_window_limit",
            "warn_threshold_percent",
            "auto_compact",
            "auto_compact_threshold",
            "tool_output_cache_threshold",
            "tool_cache_max_age_days",
            "auto_cleanup_cache",
            "tool_cache_preview_chars",
            "file_tools_allowed_paths",
            "max_recursion_depth",
            "reflection_enabled",
            // API params
            "api.temperature",
            "api.max_tokens",
            "api.top_p",
            "api.prompt_caching",
            "api.parallel_tool_calls",
            "api.frequency_penalty",
            "api.presence_penalty",
            "api.seed",
            "api.stop",
            // Reasoning config
            "api.reasoning.effort",
            "api.reasoning.max_tokens",
            "api.reasoning.exclude",
            "api.reasoning.enabled",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_auto_compact() {
        assert!(!default_auto_compact());
    }

    #[test]
    fn test_default_auto_compact_threshold() {
        assert!((default_auto_compact_threshold() - 80.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_default_base_url() {
        assert_eq!(default_base_url(), DEFAULT_API_URL);
        assert!(default_base_url().starts_with("https://"));
    }

    #[test]
    fn test_default_reflection_enabled() {
        assert!(default_reflection_enabled());
    }

    #[test]
    fn test_default_reflection_character_limit() {
        assert_eq!(default_reflection_character_limit(), 10000);
    }

    #[test]
    fn test_default_max_recursion_depth() {
        assert_eq!(default_max_recursion_depth(), 30);
    }

    #[test]
    fn test_default_lock_heartbeat_seconds() {
        assert_eq!(default_lock_heartbeat_seconds(), 30);
    }

    #[test]
    fn test_default_username() {
        assert_eq!(default_username(), "user");
    }

    #[test]
    fn test_default_rolling_compact_drop_percentage() {
        assert_eq!(default_rolling_compact_drop_percentage(), 50.0);
    }

    #[test]
    fn test_config_deserialization_minimal() {
        let toml_str = r#"
            api_key = "test-key"
            model = "gpt-4"
            context_window_limit = 8000
            warn_threshold_percent = 75.0
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.context_window_limit, 8000);
        // Defaults should be applied
        assert!(!config.auto_compact);
        assert!((config.auto_compact_threshold - 80.0).abs() < f32::EPSILON);
        assert!(config.reflection_enabled);
        assert_eq!(config.max_recursion_depth, 30);
        assert_eq!(config.username, "user");
    }

    #[test]
    fn test_config_deserialization_full() {
        let toml_str = r#"
            api_key = "test-key"
            model = "gpt-4"
            context_window_limit = 8000
            warn_threshold_percent = 75.0
            auto_compact = true
            auto_compact_threshold = 90.0
            base_url = "https://custom.api/v1"
            reflection_enabled = false
            reflection_character_limit = 5000
            max_recursion_depth = 20
            username = "alice"
            lock_heartbeat_seconds = 60
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.auto_compact);
        assert!((config.auto_compact_threshold - 90.0).abs() < f32::EPSILON);
        assert_eq!(config.base_url, "https://custom.api/v1");
        assert!(!config.reflection_enabled);
        assert_eq!(config.reflection_character_limit, 5000);
        assert_eq!(config.max_recursion_depth, 20);
        assert_eq!(config.username, "alice");
        assert_eq!(config.lock_heartbeat_seconds, 60);
    }

    #[test]
    fn test_local_config_all_none() {
        let local = LocalConfig::default();
        assert!(local.model.is_none());
        assert!(local.api_key.is_none());
        assert!(local.base_url.is_none());
        assert!(local.username.is_none());
        assert!(local.auto_compact.is_none());
        assert!(local.max_recursion_depth.is_none());
    }

    #[test]
    fn test_local_config_deserialization() {
        let toml_str = r#"
            model = "claude-3"
            username = "bob"
            auto_compact = true
        "#;
        let local: LocalConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(local.model, Some("claude-3".to_string()));
        assert_eq!(local.username, Some("bob".to_string()));
        assert_eq!(local.auto_compact, Some(true));
        assert!(local.api_key.is_none());
        assert!(local.base_url.is_none());
    }

    #[test]
    fn test_models_config_empty() {
        let models = ModelsConfig::default();
        assert!(models.models.is_empty());
    }

    #[test]
    fn test_models_config_deserialization() {
        let toml_str = r#"
            [models.gpt-4]
            context_window = 128000

            [models.claude-3]
            context_window = 200000
        "#;
        let config: ModelsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.models.len(), 2);
        assert_eq!(
            config.models.get("gpt-4").unwrap().context_window,
            Some(128000)
        );
        assert_eq!(
            config.models.get("claude-3").unwrap().context_window,
            Some(200000)
        );
    }

    // ========== ApiParams tests ==========

    #[test]
    fn test_api_params_defaults() {
        let defaults = ApiParams::defaults();
        assert_eq!(defaults.prompt_caching, Some(true));
        assert_eq!(defaults.reasoning.effort, Some(ReasoningEffort::Medium));
        assert_eq!(defaults.parallel_tool_calls, Some(true));
        assert!(defaults.temperature.is_none());
        assert!(defaults.max_tokens.is_none());
    }

    #[test]
    fn test_api_params_merge_with() {
        let base = ApiParams {
            temperature: Some(0.5),
            max_tokens: Some(1000),
            prompt_caching: Some(true),
            ..Default::default()
        };
        let override_params = ApiParams {
            temperature: Some(0.8),
            top_p: Some(0.9),
            ..Default::default()
        };

        let merged = base.merge_with(&override_params);

        // Override takes precedence
        assert_eq!(merged.temperature, Some(0.8));
        // New value from override
        assert_eq!(merged.top_p, Some(0.9));
        // Base value preserved
        assert_eq!(merged.max_tokens, Some(1000));
        assert_eq!(merged.prompt_caching, Some(true));
    }

    #[test]
    fn test_api_params_deserialization() {
        let toml_str = r#"
            temperature = 0.7
            max_tokens = 2000
            prompt_caching = false

            [reasoning]
            effort = "high"
        "#;
        let params: ApiParams = toml::from_str(toml_str).unwrap();
        assert_eq!(params.temperature, Some(0.7));
        assert_eq!(params.max_tokens, Some(2000));
        assert_eq!(params.prompt_caching, Some(false));
        assert_eq!(params.reasoning.effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_config_with_api_section() {
        let toml_str = r#"
            api_key = "test-key"
            model = "gpt-4"
            context_window_limit = 8000
            warn_threshold_percent = 75.0

            [api]
            temperature = 0.5
            max_tokens = 4000
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api.temperature, Some(0.5));
        assert_eq!(config.api.max_tokens, Some(4000));
    }

    #[test]
    fn test_local_config_with_api() {
        let toml_str = r#"
            model = "claude-3"

            [api]
            temperature = 0.3

            [api.reasoning]
            effort = "high"
        "#;
        let local: LocalConfig = toml::from_str(toml_str).unwrap();
        assert!(local.api.is_some());
        let api = local.api.unwrap();
        assert_eq!(api.temperature, Some(0.3));
        assert_eq!(api.reasoning.effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_models_config_with_api() {
        let toml_str = r#"
            [models."openai/o3"]
            context_window = 200000

            [models."openai/o3".api]
            max_tokens = 8000

            [models."openai/o3".api.reasoning]
            effort = "high"
        "#;
        let config: ModelsConfig = toml::from_str(toml_str).unwrap();
        let o3 = config.models.get("openai/o3").unwrap();
        assert_eq!(o3.context_window, Some(200000));
        assert_eq!(o3.api.reasoning.effort, Some(ReasoningEffort::High));
        assert_eq!(o3.api.max_tokens, Some(8000));
    }

    #[test]
    fn test_reasoning_config_with_max_tokens() {
        let toml_str = r#"
            max_tokens = 4000
            exclude = true
        "#;
        let config: ReasoningConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_tokens, Some(4000));
        assert_eq!(config.exclude, Some(true));
        assert!(config.effort.is_none());
    }

    #[test]
    fn test_reasoning_config_merge() {
        let base = ReasoningConfig {
            effort: Some(ReasoningEffort::Medium),
            exclude: Some(false),
            ..Default::default()
        };
        let override_cfg = ReasoningConfig {
            effort: Some(ReasoningEffort::High),
            ..Default::default()
        };

        let merged = base.merge_with(&override_cfg);
        assert_eq!(merged.effort, Some(ReasoningEffort::High));
        assert_eq!(merged.exclude, Some(false)); // base preserved
    }

    #[test]
    fn test_reasoning_effort_serialization() {
        assert_eq!(ReasoningEffort::XHigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::Minimal.as_str(), "minimal");
        assert_eq!(ReasoningEffort::None.as_str(), "none");
    }

    #[test]
    fn test_tool_choice_mode_deserialization() {
        let json = r#""auto""#;
        let mode: ToolChoiceMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, ToolChoiceMode::Auto);

        let json = r#""required""#;
        let mode: ToolChoiceMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, ToolChoiceMode::Required);
    }

    #[test]
    fn test_response_format_deserialization() {
        let json = r#"{"type": "json_object"}"#;
        let format: ResponseFormat = serde_json::from_str(json).unwrap();
        assert!(matches!(format, ResponseFormat::JsonObject));
    }

    // ========== ResolvedConfig field inspection tests (TDD for issue #18) ==========

    fn create_test_resolved_config() -> ResolvedConfig {
        ResolvedConfig {
            api_key: "sk-test-key".to_string(),
            model: "anthropic/claude-3-opus".to_string(),
            context_window_limit: 200000,
            warn_threshold_percent: 75.0,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            auto_compact: true,
            auto_compact_threshold: 80.0,
            max_recursion_depth: 30,
            username: "alice".to_string(),
            reflection_enabled: true,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams {
                temperature: Some(0.7),
                max_tokens: Some(4096),
                prompt_caching: Some(true),
                reasoning: ReasoningConfig {
                    effort: Some(ReasoningEffort::High),
                    max_tokens: None,
                    exclude: Some(false),
                    enabled: Some(true),
                },
                parallel_tool_calls: Some(true),
                top_p: Some(0.9),
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_resolved_config_get_field_model() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("model"),
            Some("anthropic/claude-3-opus".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_username() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("username"), Some("alice".to_string()));
    }

    #[test]
    fn test_resolved_config_get_field_context_window_limit() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("context_window_limit"),
            Some("200000".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_auto_compact() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("auto_compact"), Some("true".to_string()));
    }

    #[test]
    fn test_resolved_config_get_field_warn_threshold_percent() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("warn_threshold_percent"),
            Some("75".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_reflection_enabled() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("reflection_enabled"),
            Some("true".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_max_recursion_depth() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("max_recursion_depth"),
            Some("30".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_base_url() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("base_url"),
            Some("https://openrouter.ai/api/v1".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_auto_compact_threshold() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("auto_compact_threshold"),
            Some("80".to_string())
        );
    }

    // Nested API params tests

    #[test]
    fn test_resolved_config_get_field_api_temperature() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("api.temperature"), Some("0.7".to_string()));
    }

    #[test]
    fn test_resolved_config_get_field_api_max_tokens() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("api.max_tokens"), Some("4096".to_string()));
    }

    #[test]
    fn test_resolved_config_get_field_api_prompt_caching() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("api.prompt_caching"),
            Some("true".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_api_top_p() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("api.top_p"), Some("0.9".to_string()));
    }

    #[test]
    fn test_resolved_config_get_field_api_parallel_tool_calls() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("api.parallel_tool_calls"),
            Some("true".to_string())
        );
    }

    // Deeply nested reasoning config tests

    #[test]
    fn test_resolved_config_get_field_api_reasoning_effort() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("api.reasoning.effort"),
            Some("high".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_api_reasoning_enabled() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("api.reasoning.enabled"),
            Some("true".to_string())
        );
    }

    #[test]
    fn test_resolved_config_get_field_api_reasoning_exclude() {
        let config = create_test_resolved_config();
        assert_eq!(
            config.get_field("api.reasoning.exclude"),
            Some("false".to_string())
        );
    }

    // None/unset values should return None

    #[test]
    fn test_resolved_config_get_field_unset_returns_none() {
        let config = create_test_resolved_config();
        // api.stop is not set
        assert_eq!(config.get_field("api.stop"), None);
        // api.seed is not set
        assert_eq!(config.get_field("api.seed"), None);
        // api.frequency_penalty is not set
        assert_eq!(config.get_field("api.frequency_penalty"), None);
    }

    #[test]
    fn test_resolved_config_get_field_invalid_returns_none() {
        let config = create_test_resolved_config();
        assert_eq!(config.get_field("nonexistent_field"), None);
        assert_eq!(config.get_field("api.nonexistent"), None);
        assert_eq!(config.get_field("api.reasoning.nonexistent"), None);
    }

    // List all inspectable fields

    #[test]
    fn test_resolved_config_list_fields() {
        let fields = ResolvedConfig::list_fields();
        // Should include top-level fields
        assert!(fields.contains(&"model"));
        assert!(fields.contains(&"username"));
        assert!(fields.contains(&"context_window_limit"));
        assert!(fields.contains(&"auto_compact"));
        assert!(fields.contains(&"reflection_enabled"));
        // Should include nested API params
        assert!(fields.contains(&"api.temperature"));
        assert!(fields.contains(&"api.max_tokens"));
        assert!(fields.contains(&"api.prompt_caching"));
        // Should include deeply nested reasoning config
        assert!(fields.contains(&"api.reasoning.effort"));
        assert!(fields.contains(&"api.reasoning.enabled"));
    }

    #[test]
    fn test_resolved_config_list_fields_excludes_api_key() {
        // api_key should NOT be inspectable for security reasons
        let fields = ResolvedConfig::list_fields();
        assert!(!fields.contains(&"api_key"));
    }
}
