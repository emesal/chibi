//! Core configuration types for chibi.
//!
//! This module contains the core configuration types needed for API calls,
//! tool management, and storage. Presentation-related config (images, markdown
//! rendering) lives in the CLI crate.

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
/// Either `effort` OR `max_tokens` should be set, not both (mutually exclusive).
/// If both are provided during deserialization, `max_tokens` wins and `effort` is cleared.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(into = "ReasoningConfigRaw")]
pub struct ReasoningConfig {
    /// Effort level (mutually exclusive with max_tokens)
    /// Supported by: OpenAI o1/o3/GPT-5 series, Grok models
    pub effort: Option<ReasoningEffort>,

    /// Maximum tokens for reasoning (mutually exclusive with effort)
    /// Supported by: Gemini thinking models, Anthropic, some Qwen models
    /// Anthropic: min 1024, max 128000
    pub max_tokens: Option<usize>,

    /// Exclude reasoning from response (model still reasons internally)
    /// Default: false
    pub exclude: Option<bool>,

    /// Explicitly enable reasoning (defaults to medium effort if true)
    pub enabled: Option<bool>,
}

/// Raw deserialization target for ReasoningConfig.
/// Mutual exclusivity is enforced during From conversion.
#[derive(Deserialize, Serialize)]
struct ReasoningConfigRaw {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
}

impl From<ReasoningConfigRaw> for ReasoningConfig {
    fn from(raw: ReasoningConfigRaw) -> Self {
        let mut config = ReasoningConfig {
            effort: raw.effort,
            max_tokens: raw.max_tokens,
            exclude: raw.exclude,
            enabled: raw.enabled,
        };
        if config.enforce_mutual_exclusivity() {
            eprintln!("[WARN] reasoning: both effort and max_tokens set, using max_tokens");
        }
        config
    }
}

impl From<ReasoningConfig> for ReasoningConfigRaw {
    fn from(config: ReasoningConfig) -> Self {
        ReasoningConfigRaw {
            effort: config.effort,
            max_tokens: config.max_tokens,
            exclude: config.exclude,
            enabled: config.enabled,
        }
    }
}

impl<'de> Deserialize<'de> for ReasoningConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = ReasoningConfigRaw::deserialize(deserializer)?;
        Ok(ReasoningConfig::from(raw))
    }
}

impl ReasoningConfig {
    /// Check if this config has any values set
    pub fn is_empty(&self) -> bool {
        self.effort.is_none()
            && self.max_tokens.is_none()
            && self.exclude.is_none()
            && self.enabled.is_none()
    }

    /// Enforce mutual exclusivity: effort and max_tokens cannot both be set.
    /// When both are present, max_tokens wins (it's the more explicit/specific setting).
    /// Returns true if a conflict was resolved.
    pub fn enforce_mutual_exclusivity(&mut self) -> bool {
        if self.effort.is_some() && self.max_tokens.is_some() {
            self.effort = None;
            true
        } else {
            false
        }
    }

    /// Merge with another ReasoningConfig, where `other` takes precedence
    pub fn merge_with(&self, other: &ReasoningConfig) -> ReasoningConfig {
        let effort = other.effort.or(self.effort);
        let max_tokens = other.max_tokens.or(self.max_tokens);

        let mut merged = ReasoningConfig {
            effort,
            max_tokens,
            exclude: other.exclude.or(self.exclude),
            enabled: other.enabled.or(self.enabled),
        };

        // If the override explicitly sets one side, the other must be cleared
        if other.effort.is_some() {
            merged.max_tokens = None;
        } else if other.max_tokens.is_some() {
            merged.effort = None;
        } else {
            // Neither side was explicitly overridden — if self somehow had both,
            // enforce the invariant
            merged.enforce_mutual_exclusivity();
        }

        merged
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

/// Tool filtering configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Allowlist - only these tools are available (if set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Blocklist - these tools are excluded
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

// ============================================================================
// Default Value Functions
// ============================================================================

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

// ============================================================================
// Configuration Structs
// ============================================================================

/// Global config from ~/.chibi/config.toml
/// Note: This is the core config. Presentation fields (image, markdown_style,
/// render_markdown) are handled by the CLI layer.
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
    /// Maximum characters for reflection tool output
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
/// Note: Core fields only. Presentation overrides are in CLI layer.
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
    /// Tool filtering configuration (include/exclude lists)
    #[serde(default)]
    pub tools: Option<ToolsConfig>,
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

/// Fully resolved configuration with all overrides applied.
/// Note: This is the core resolved config. CLI extends this with presentation fields.
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
    /// Tool filtering configuration (include/exclude lists)
    pub tools: ToolsConfig,
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
            "max_recursion_depth",
            "reflection_enabled",
            "tool_output_cache_threshold",
            "tool_cache_max_age_days",
            "auto_cleanup_cache",
            "tool_cache_preview_chars",
            "file_tools_allowed_paths",
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
            // Reasoning
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
    fn test_reasoning_effort_as_str() {
        assert_eq!(ReasoningEffort::XHigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::Minimal.as_str(), "minimal");
        assert_eq!(ReasoningEffort::None.as_str(), "none");
    }

    #[test]
    fn test_reasoning_config_mutual_exclusivity() {
        let mut config = ReasoningConfig {
            effort: Some(ReasoningEffort::High),
            max_tokens: Some(1000),
            exclude: None,
            enabled: None,
        };
        assert!(config.enforce_mutual_exclusivity());
        assert!(config.effort.is_none());
        assert_eq!(config.max_tokens, Some(1000));
    }

    #[test]
    fn test_reasoning_config_merge() {
        let base = ReasoningConfig {
            effort: Some(ReasoningEffort::Medium),
            max_tokens: None,
            exclude: Some(false),
            enabled: None,
        };
        let override_config = ReasoningConfig {
            effort: None,
            max_tokens: Some(2000),
            exclude: None,
            enabled: Some(true),
        };
        let merged = base.merge_with(&override_config);
        assert!(merged.effort.is_none()); // max_tokens wins
        assert_eq!(merged.max_tokens, Some(2000));
        assert_eq!(merged.exclude, Some(false)); // from base
        assert_eq!(merged.enabled, Some(true)); // from override
    }

    #[test]
    fn test_api_params_merge() {
        let base = ApiParams {
            temperature: Some(0.7),
            max_tokens: Some(1000),
            ..Default::default()
        };
        let override_params = ApiParams {
            temperature: Some(0.9),
            top_p: Some(0.95),
            ..Default::default()
        };
        let merged = base.merge_with(&override_params);
        assert_eq!(merged.temperature, Some(0.9));
        assert_eq!(merged.max_tokens, Some(1000));
        assert_eq!(merged.top_p, Some(0.95));
    }

    #[test]
    fn test_resolved_config_get_field() {
        let config = ResolvedConfig {
            api_key: "secret".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 4096,
            warn_threshold_percent: 80.0,
            base_url: DEFAULT_API_URL.to_string(),
            auto_compact: false,
            auto_compact_threshold: 80.0,
            max_recursion_depth: 30,
            username: "testuser".to_string(),
            reflection_enabled: true,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec!["/tmp".to_string()],
            api: ApiParams::defaults(),
            tools: ToolsConfig::default(),
        };

        assert_eq!(config.get_field("model"), Some("test-model".to_string()));
        assert_eq!(config.get_field("username"), Some("testuser".to_string()));
        assert_eq!(config.get_field("api_key"), None); // Excluded for security
        assert_eq!(
            config.get_field("file_tools_allowed_paths"),
            Some("/tmp".to_string())
        );
    }
}
