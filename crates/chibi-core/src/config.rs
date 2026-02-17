//! Core configuration types for chibi.
//!
//! This module contains the core configuration types needed for API calls,
//! tool management, and storage. Presentation-related config (images, markdown
//! rendering) lives in the CLI crate.

use crate::partition::StorageConfig;
use crate::tools::security::UrlPolicy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ============================================================================
// Config Macros
// ============================================================================

/// Apply `Option`-field overrides from a source struct to a target struct.
///
/// For each field name, if `$src.field` is `Some(v)`, sets `$dst.field = v`
/// (cloning as needed). This eliminates the repetitive `if let Some` blocks
/// in config resolution.
macro_rules! apply_option_overrides {
    ($src:expr, $dst:expr, $($field:ident),+ $(,)?) => {
        $(
            if let Some(ref v) = $src.$field {
                $dst.$field = v.clone();
            }
        )+
    };
}

/// Early-return from `get_field` for simple config fields.
///
/// Call at the top of `get_field` — handles standard fields and returns,
/// falling through for special cases handled by the caller's match block.
///
/// Display modes:
/// - `display`: uses `to_string()` (bool, usize, u64)
/// - `clone`: returns `.clone()` (String fields)
/// - `int`: casts f32 to i32 before display (no decimals)
/// - `fmt`: uses `format!("{}", v)` (f32 with decimals)
macro_rules! config_get_field {
    ($self:expr, $path:expr,
     $(display: $($d_field:ident),* ;)?
     $(clone: $($c_field:ident),* ;)?
     $(int: $($i_field:ident),* ;)?
     $(fmt: $($f_field:ident),* ;)?
    ) => {
        match $path {
            $($(
                stringify!($d_field) => return Some($self.$d_field.to_string()),
            )*)?
            $($(
                stringify!($c_field) => return Some($self.$c_field.clone()),
            )*)?
            $($(
                stringify!($i_field) => return Some(format!("{}", $self.$i_field as i32)),
            )*)?
            $($(
                stringify!($f_field) => return Some(format!("{}", $self.$f_field)),
            )*)?
            _ => {} // fall through to caller's match
        }
    };
}

/// Early-return from `set_field` for simple config fields.
///
/// Parses `$value` into the appropriate type and assigns it, returning `Ok(())`.
/// Falls through for fields not listed here (handled by the caller's match block).
///
/// Parse modes:
/// - `bool`: parses "true"/"false"
/// - `usize`: parses via `str::parse::<usize>()`
/// - `u64`: parses via `str::parse::<u64>()`
/// - `f32`: parses via `str::parse::<f32>()`
/// - `string`: clones directly
macro_rules! config_set_field {
    ($self:expr, $path:expr, $value:expr,
     $(bool: $($b_field:ident),* ;)?
     $(usize: $($u_field:ident),* ;)?
     $(u64: $($u64_field:ident),* ;)?
     $(f32: $($f_field:ident),* ;)?
     $(string: $($s_field:ident),* ;)?
    ) => {
        match $path {
            $($( stringify!($b_field) => {
                $self.$b_field = $value.parse::<bool>()
                    .map_err(|_| format!("invalid bool for '{}': {}", $path, $value))?;
                return Ok(());
            }, )*)?
            $($( stringify!($u_field) => {
                $self.$u_field = $value.parse::<usize>()
                    .map_err(|_| format!("invalid usize for '{}': {}", $path, $value))?;
                return Ok(());
            }, )*)?
            $($( stringify!($u64_field) => {
                $self.$u64_field = $value.parse::<u64>()
                    .map_err(|_| format!("invalid u64 for '{}': {}", $path, $value))?;
                return Ok(());
            }, )*)?
            $($( stringify!($f_field) => {
                $self.$f_field = $value.parse::<f32>()
                    .map_err(|_| format!("invalid f32 for '{}': {}", $path, $value))?;
                return Ok(());
            }, )*)?
            $($( stringify!($s_field) => {
                $self.$s_field = $value.to_string();
                return Ok(());
            }, )*)?
            _ => {} // fall through to caller's match
        }
    };
}

// ============================================================================
// API Parameters Types
// ============================================================================

/// Reasoning effort level for models that support it (e.g., OpenAI o3, Grok)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
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
#[derive(Deserialize, Serialize, JsonSchema)]
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

/// Delegate JsonSchema to ReasoningConfigRaw (the canonical serde shape).
impl JsonSchema for ReasoningConfig {
    fn schema_name() -> String {
        "ReasoningConfig".to_string()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        ReasoningConfigRaw::json_schema(generator)
    }
}

/// Tool choice mode for the API
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceMode {
    Auto,
    None,
    Required,
}

/// Specific function to call
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Tool choice - either a mode or a specific function
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ToolsConfig {
    /// Allowlist - only these tools are available (if set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Blocklist - these tools are excluded
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    /// Exclude entire tool categories: "builtin", "file", "agent", "coding"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_categories: Option<Vec<String>>,
}

/// Merge two optional string vecs: append `local` to `global`, deduplicating entries.
fn merge_option_vecs(
    global: &Option<Vec<String>>,
    local: &Option<Vec<String>>,
) -> Option<Vec<String>> {
    match (global, local) {
        (Some(g), Some(l)) => {
            let mut merged = g.clone();
            for item in l {
                if !merged.contains(item) {
                    merged.push(item.clone());
                }
            }
            Some(merged)
        }
        (None, Some(l)) => Some(l.clone()),
        (Some(g), None) => Some(g.clone()),
        (None, None) => None,
    }
}

impl ToolsConfig {
    /// Merge a local (per-context) tools config on top of this (global) config.
    ///
    /// - `include`: local overrides global entirely if set
    /// - `exclude`: local appends to global (deduplicated)
    /// - `exclude_categories`: local appends to global (deduplicated)
    pub fn merge_local(&self, local: &ToolsConfig) -> ToolsConfig {
        let include = if local.include.is_some() {
            local.include.clone()
        } else {
            self.include.clone()
        };

        ToolsConfig {
            include,
            exclude: merge_option_vecs(&self.exclude, &local.exclude),
            exclude_categories: merge_option_vecs(
                &self.exclude_categories,
                &local.exclude_categories,
            ),
        }
    }
}

// ============================================================================
// Default Values
// ============================================================================

/// Central source of truth for all configuration defaults.
pub struct ConfigDefaults;

impl ConfigDefaults {
    // Boolean defaults
    pub const VERBOSE: bool = false;
    pub const HIDE_TOOL_CALLS: bool = false;
    pub const NO_TOOL_CALLS: bool = false;
    pub const SHOW_THINKING: bool = true;
    pub const AUTO_COMPACT: bool = false;
    pub const REFLECTION_ENABLED: bool = true;
    pub const AUTO_CLEANUP_CACHE: bool = true;

    // Numeric defaults
    pub const AUTO_COMPACT_THRESHOLD: f32 = 80.0;
    pub const WARN_THRESHOLD_PERCENT: f32 = 80.0;
    /// Sentinel: 0 = fetch from ratatoskr at runtime
    pub const CONTEXT_WINDOW_LIMIT: usize = 0;
    pub const REFLECTION_CHARACTER_LIMIT: usize = 10_000;
    pub const FUEL: usize = 30;
    pub const FUEL_EMPTY_RESPONSE_COST: usize = 15;
    pub const LOCK_HEARTBEAT_SECONDS: u64 = 30;
    pub const ROLLING_COMPACT_DROP_PERCENTAGE: f32 = 50.0;
    pub const TOOL_OUTPUT_CACHE_THRESHOLD: usize = 4_000;
    pub const TOOL_CACHE_MAX_AGE_DAYS: u64 = 7;
    pub const TOOL_CACHE_PREVIEW_CHARS: usize = 500;

    // String defaults
    pub const USERNAME: &'static str = "user";
    pub const FALLBACK_TOOL: &'static str = "call_user";
    /// Default model: ratatoskr free-tier agentic preset
    pub const MODEL: &'static str = "ratatoskr:free/agentic";
}

// Thin wrappers for serde's #[serde(default = "...")] requirement
fn default_verbose() -> bool {
    ConfigDefaults::VERBOSE
}
fn default_hide_tool_calls() -> bool {
    ConfigDefaults::HIDE_TOOL_CALLS
}
fn default_no_tool_calls() -> bool {
    ConfigDefaults::NO_TOOL_CALLS
}
fn default_show_thinking() -> bool {
    ConfigDefaults::SHOW_THINKING
}
fn default_auto_compact() -> bool {
    ConfigDefaults::AUTO_COMPACT
}
fn default_auto_compact_threshold() -> f32 {
    ConfigDefaults::AUTO_COMPACT_THRESHOLD
}
fn default_reflection_enabled() -> bool {
    ConfigDefaults::REFLECTION_ENABLED
}
fn default_reflection_character_limit() -> usize {
    ConfigDefaults::REFLECTION_CHARACTER_LIMIT
}
fn default_fuel() -> usize {
    ConfigDefaults::FUEL
}
fn default_fuel_empty_response_cost() -> usize {
    ConfigDefaults::FUEL_EMPTY_RESPONSE_COST
}
fn default_lock_heartbeat_seconds() -> u64 {
    ConfigDefaults::LOCK_HEARTBEAT_SECONDS
}
fn default_username() -> String {
    ConfigDefaults::USERNAME.to_string()
}
fn default_rolling_compact_drop_percentage() -> f32 {
    ConfigDefaults::ROLLING_COMPACT_DROP_PERCENTAGE
}
fn default_tool_output_cache_threshold() -> usize {
    ConfigDefaults::TOOL_OUTPUT_CACHE_THRESHOLD
}
fn default_tool_cache_max_age_days() -> u64 {
    ConfigDefaults::TOOL_CACHE_MAX_AGE_DAYS
}
fn default_auto_cleanup_cache() -> bool {
    ConfigDefaults::AUTO_CLEANUP_CACHE
}
fn default_tool_cache_preview_chars() -> usize {
    ConfigDefaults::TOOL_CACHE_PREVIEW_CHARS
}
fn default_fallback_tool() -> String {
    ConfigDefaults::FALLBACK_TOOL.to_string()
}
fn default_warn_threshold_percent() -> f32 {
    ConfigDefaults::WARN_THRESHOLD_PERCENT
}

// ============================================================================
// Configuration Structs
// ============================================================================

/// Global config from ~/.chibi/config.toml
/// Note: This is the core config. Presentation fields (image, markdown_style,
/// render_markdown) are handled by the CLI layer.
/// All fields are optional with sensible defaults — config.toml itself is optional.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context_window_limit: Option<usize>,
    #[serde(default = "default_warn_threshold_percent")]
    pub warn_threshold_percent: f32,
    /// Enable verbose output (equivalent to -v flag)
    #[serde(default = "default_verbose")]
    pub verbose: bool,
    /// Hide tool call display by default (verbose overrides)
    #[serde(default = "default_hide_tool_calls")]
    pub hide_tool_calls: bool,
    /// Omit tools from API requests entirely (pure text mode)
    #[serde(default = "default_no_tool_calls")]
    pub no_tool_calls: bool,
    /// Show thinking/reasoning content (default: false, verbose overrides)
    #[serde(default = "default_show_thinking")]
    pub show_thinking: bool,
    #[serde(default = "default_auto_compact")]
    pub auto_compact: bool,
    #[serde(default = "default_auto_compact_threshold")]
    pub auto_compact_threshold: f32,
    #[serde(default = "default_reflection_enabled")]
    pub reflection_enabled: bool,
    /// Maximum characters for reflection tool output
    #[serde(default = "default_reflection_character_limit")]
    pub reflection_character_limit: usize,
    /// Total fuel budget for the agentic loop (tool rounds, continuations, empty responses).
    /// Set to `0` to disable fuel tracking entirely (unlimited mode).
    #[serde(default = "default_fuel")]
    pub fuel: usize,
    /// Fuel cost of an empty response (high cost prevents infinite empty loops).
    /// Ignored when `fuel = 0` (unlimited mode).
    #[serde(default = "default_fuel_empty_response_cost")]
    pub fuel_empty_response_cost: usize,
    #[serde(default = "default_username")]
    pub username: String,
    /// Lock heartbeat interval in seconds. Intentionally global-only (not in ResolvedConfig)
    /// since lock behaviour must be consistent regardless of active context.
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
    /// Paths allowed for file tools (empty = defaults to cwd at runtime)
    #[serde(default)]
    pub file_tools_allowed_paths: Vec<String>,
    /// API parameters (temperature, max_tokens, etc.)
    #[serde(default)]
    pub api: ApiParams,
    /// Storage configuration for partitioned context storage
    #[serde(default)]
    pub storage: StorageConfig,
    /// Fallback tool when LLM doesn't call call_agent/call_user explicitly
    #[serde(default = "default_fallback_tool")]
    pub fallback_tool: String,
    /// Global tool filtering configuration (include/exclude/exclude_categories)
    #[serde(default)]
    pub tools: ToolsConfig,
    /// URL security policy for sensitive URL handling
    #[serde(default)]
    pub url_policy: Option<UrlPolicy>,
}

/// Per-context config from `~/.chibi/contexts/<name>/local.toml`
/// Note: Core fields only. Presentation overrides are in CLI layer.
#[derive(Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct LocalConfig {
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub username: Option<String>,
    /// Per-context verbose override
    pub verbose: Option<bool>,
    /// Per-context hide tool calls override
    pub hide_tool_calls: Option<bool>,
    /// Per-context no tool calls override
    pub no_tool_calls: Option<bool>,
    /// Per-context show thinking override
    pub show_thinking: Option<bool>,
    pub auto_compact: Option<bool>,
    pub auto_compact_threshold: Option<f32>,
    /// Per-context fuel budget override. `0` means unlimited.
    pub fuel: Option<usize>,
    /// Per-context fuel cost for empty responses. Ignored when `fuel = 0`.
    pub fuel_empty_response_cost: Option<usize>,
    pub warn_threshold_percent: Option<f32>,
    pub context_window_limit: Option<usize>,
    pub reflection_enabled: Option<bool>,
    pub reflection_character_limit: Option<usize>,
    pub rolling_compact_drop_percentage: Option<f32>,
    /// Threshold (in chars) above which tool output is cached
    pub tool_output_cache_threshold: Option<usize>,
    /// Maximum age in days for cached tool outputs
    pub tool_cache_max_age_days: Option<u64>,
    /// Automatically cleanup old cache entries on exit
    pub auto_cleanup_cache: Option<bool>,
    /// Number of preview characters to show in truncated message
    pub tool_cache_preview_chars: Option<usize>,
    /// Paths allowed for file tools (empty = defaults to cwd at runtime)
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
    /// Override fallback tool for this context
    pub fallback_tool: Option<String>,
    /// URL security policy override
    pub url_policy: Option<UrlPolicy>,
}

impl LocalConfig {
    /// Apply all simple-override fields from this local config onto a resolved config.
    ///
    /// Fields with custom merge semantics (api, storage, tools) are NOT handled here —
    /// those are applied separately in `resolve_config`.
    ///
    /// ## Adding a new config field
    ///
    /// 1. Add the field to `Config` (with `#[serde(default = "...")]`)
    /// 2. Add `Option<T>` field to `LocalConfig`
    /// 3. Add the concrete field to `ResolvedConfig`
    /// 4. Add the field name to the `apply_option_overrides!` list below
    /// 5. Add to `ResolvedConfig::get_field()` (macro or hand-written match arm)
    /// 6. Add to `ResolvedConfig::list_fields()`
    /// 7. Initialise from `self.config` in `resolve_config()` struct literal
    pub fn apply_overrides(&self, resolved: &mut ResolvedConfig) {
        // api_key is Option<String> on both sides, needs special handling
        if let Some(ref api_key) = self.api_key {
            resolved.api_key = Some(api_key.clone());
        }
        // All other simple fields: local Some(v) overrides resolved value
        apply_option_overrides!(
            self,
            resolved,
            model,
            username,
            fallback_tool,
            file_tools_allowed_paths,
            verbose,
            hide_tool_calls,
            no_tool_calls,
            show_thinking,
            auto_compact,
            auto_compact_threshold,
            fuel,
            fuel_empty_response_cost,
            warn_threshold_percent,
            context_window_limit,
            reflection_enabled,
            reflection_character_limit,
            rolling_compact_drop_percentage,
            tool_output_cache_threshold,
            tool_cache_max_age_days,
            auto_cleanup_cache,
            tool_cache_preview_chars,
        );
        // url_policy: whole-object override (not merge)
        if self.url_policy.is_some() {
            resolved.url_policy = self.url_policy.clone();
        }
    }
}

/// Model metadata from ~/.chibi/models.toml.
///
/// Contains only per-model API parameter overrides. Model capabilities
/// (context window, tool call support) come from ratatoskr's registry.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelMetadata {
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
    /// API key for the provider. `None` = keyless (free-tier openrouter).
    pub api_key: Option<String>,
    pub model: String,
    pub context_window_limit: usize,
    pub warn_threshold_percent: f32,
    /// Verbose output (from config, may be overridden by CLI flag)
    pub verbose: bool,
    /// Hide tool call display (from config, may be overridden by CLI flag)
    pub hide_tool_calls: bool,
    /// Omit tools from API requests (pure text mode, from config/flag)
    pub no_tool_calls: bool,
    /// Show thinking/reasoning content (default: false, verbose overrides)
    pub show_thinking: bool,
    pub auto_compact: bool,
    pub auto_compact_threshold: f32,
    /// Total fuel budget for the agentic loop. `0` means unlimited (no tracking).
    pub fuel: usize,
    /// Fuel cost of an empty response. Ignored when `fuel = 0`.
    pub fuel_empty_response_cost: usize,
    pub username: String,
    pub reflection_enabled: bool,
    /// Character limit for reflection output
    pub reflection_character_limit: usize,
    /// Percentage of messages to drop during rolling compaction
    pub rolling_compact_drop_percentage: f32,
    /// Threshold (in chars) above which tool output is cached
    pub tool_output_cache_threshold: usize,
    /// Maximum age in days for cached tool outputs
    pub tool_cache_max_age_days: u64,
    /// Automatically cleanup old cache entries on exit
    pub auto_cleanup_cache: bool,
    /// Number of preview characters to show in truncated message
    pub tool_cache_preview_chars: usize,
    /// Paths allowed for file tools (empty = defaults to cwd at runtime).
    /// Reads inside these paths are auto-allowed; reads outside require permission.
    pub file_tools_allowed_paths: Vec<String>,
    /// Resolved API parameters (merged from all layers)
    pub api: ApiParams,
    /// Tool filtering configuration (include/exclude lists)
    pub tools: ToolsConfig,
    /// Fallback tool (call_agent or call_user)
    pub fallback_tool: String,
    /// Storage configuration for partitioned context storage
    pub storage: StorageConfig,
    /// URL security policy (None = use permission handler fallback)
    pub url_policy: Option<UrlPolicy>,
    /// Arbitrary per-invocation key-value overrides (freeform escape hatch).
    /// Unknown field paths in `set_field` land here; `get_field` falls through to here.
    pub extra: BTreeMap<String, String>,
}

impl ResolvedConfig {
    /// Get a config field value by path (e.g., "model", "api.temperature", "api.reasoning.effort").
    /// Returns None if the field doesn't exist or has no value set.
    /// Note: api_key shows presence only ("(set)"/"(unset)"), never the actual value.
    pub fn get_field(&self, path: &str) -> Option<String> {
        // Macro handles standard fields with uniform display logic
        config_get_field!(self, path,
            display: verbose, hide_tool_calls, no_tool_calls, show_thinking, auto_compact,
                     reflection_enabled, auto_cleanup_cache,
                     context_window_limit, reflection_character_limit,
                     fuel, fuel_empty_response_cost,
                     tool_output_cache_threshold, tool_cache_preview_chars,
                     tool_cache_max_age_days;
            clone: model, username, fallback_tool;
            int: warn_threshold_percent, auto_compact_threshold;
            fmt: rolling_compact_drop_percentage;
        );

        // Fields with custom display logic
        match path {
            "api_key" => Some(
                if self.api_key.is_some() {
                    "(set)"
                } else {
                    "(unset)"
                }
                .to_string(),
            ),
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

            // Storage config (storage.*)
            "storage.partition_max_entries" => {
                self.storage.partition_max_entries.map(|v| v.to_string())
            }
            "storage.partition_max_age_seconds" => self
                .storage
                .partition_max_age_seconds
                .map(|v| v.to_string()),
            "storage.partition_max_tokens" => {
                self.storage.partition_max_tokens.map(|v| v.to_string())
            }
            "storage.bytes_per_token" => self.storage.bytes_per_token.map(|v| v.to_string()),
            "storage.enable_bloom_filters" => {
                self.storage.enable_bloom_filters.map(|v| v.to_string())
            }

            // URL policy
            "url_policy" => Some(
                if self.url_policy.is_some() {
                    "(set)"
                } else {
                    "(unset)"
                }
                .to_string(),
            ),

            // Fall through to extra (freeform per-invocation overrides)
            _ => self.extra.get(path).cloned(),
        }
    }

    /// List all inspectable config field paths.
    /// Note: api_key shows presence only, not the actual value.
    pub fn list_fields() -> &'static [&'static str] {
        &[
            // Top-level fields
            "api_key",
            "verbose",
            "hide_tool_calls",
            "no_tool_calls",
            "show_thinking",
            "model",
            "username",
            "context_window_limit",
            "warn_threshold_percent",
            "auto_compact",
            "auto_compact_threshold",
            "fuel",
            "fuel_empty_response_cost",
            "reflection_enabled",
            "reflection_character_limit",
            "rolling_compact_drop_percentage",
            "fallback_tool",
            "tool_output_cache_threshold",
            "tool_cache_max_age_days",
            "auto_cleanup_cache",
            "tool_cache_preview_chars",
            "file_tools_allowed_paths",
            "url_policy",
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
            // Storage
            "storage.partition_max_entries",
            "storage.partition_max_age_seconds",
            "storage.partition_max_tokens",
            "storage.bytes_per_token",
            "storage.enable_bloom_filters",
        ]
    }

    /// Set a config field by path (e.g., "model", "api.temperature", "fuel").
    ///
    /// Known fields are parsed into their native types; unknown paths are stored
    /// in `extra` as freeform string key-value pairs.
    /// Returns `Err` on parse failure for known fields.
    pub fn set_field(&mut self, path: &str, value: &str) -> Result<(), String> {
        // Macro handles standard top-level fields
        config_set_field!(self, path, value,
            bool: verbose, hide_tool_calls, no_tool_calls, show_thinking, auto_compact,
                  reflection_enabled, auto_cleanup_cache;
            usize: context_window_limit, reflection_character_limit,
                   fuel, fuel_empty_response_cost,
                   tool_output_cache_threshold, tool_cache_preview_chars;
            u64: tool_cache_max_age_days;
            f32: warn_threshold_percent, auto_compact_threshold,
                 rolling_compact_drop_percentage;
            string: model, username, fallback_tool;
        );

        // Fields with custom parsing
        match path {
            // API params (api.*)
            "api.temperature" => {
                self.api.temperature = Some(
                    value
                        .parse::<f32>()
                        .map_err(|_| format!("invalid f32 for '{}': {}", path, value))?,
                );
            }
            "api.max_tokens" => {
                self.api.max_tokens = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid usize for '{}': {}", path, value))?,
                );
            }
            "api.top_p" => {
                self.api.top_p = Some(
                    value
                        .parse::<f32>()
                        .map_err(|_| format!("invalid f32 for '{}': {}", path, value))?,
                );
            }
            "api.prompt_caching" => {
                self.api.prompt_caching = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool for '{}': {}", path, value))?,
                );
            }
            "api.parallel_tool_calls" => {
                self.api.parallel_tool_calls = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool for '{}': {}", path, value))?,
                );
            }
            "api.frequency_penalty" => {
                self.api.frequency_penalty = Some(
                    value
                        .parse::<f32>()
                        .map_err(|_| format!("invalid f32 for '{}': {}", path, value))?,
                );
            }
            "api.presence_penalty" => {
                self.api.presence_penalty = Some(
                    value
                        .parse::<f32>()
                        .map_err(|_| format!("invalid f32 for '{}': {}", path, value))?,
                );
            }
            "api.seed" => {
                self.api.seed = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid u64 for '{}': {}", path, value))?,
                );
            }

            // Reasoning config (api.reasoning.*)
            "api.reasoning.effort" => {
                let effort: ReasoningEffort = serde_json::from_str(&format!("\"{}\"", value))
                    .map_err(|_| format!("invalid reasoning effort for '{}': {}", path, value))?;
                self.api.reasoning.effort = Some(effort);
            }
            "api.reasoning.max_tokens" => {
                self.api.reasoning.max_tokens = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid usize for '{}': {}", path, value))?,
                );
            }
            "api.reasoning.exclude" => {
                self.api.reasoning.exclude = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool for '{}': {}", path, value))?,
                );
            }
            "api.reasoning.enabled" => {
                self.api.reasoning.enabled = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool for '{}': {}", path, value))?,
                );
            }

            // Storage config (storage.*)
            "storage.partition_max_entries" => {
                self.storage.partition_max_entries = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid usize for '{}': {}", path, value))?,
                );
            }
            "storage.partition_max_age_seconds" => {
                self.storage.partition_max_age_seconds = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid u64 for '{}': {}", path, value))?,
                );
            }
            "storage.partition_max_tokens" => {
                self.storage.partition_max_tokens = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid usize for '{}': {}", path, value))?,
                );
            }
            "storage.bytes_per_token" => {
                self.storage.bytes_per_token = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid usize for '{}': {}", path, value))?,
                );
            }
            "storage.enable_bloom_filters" => {
                self.storage.enable_bloom_filters = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool for '{}': {}", path, value))?,
                );
            }

            // Unknown paths → freeform extra
            _ => {
                self.extra.insert(path.to_string(), value.to_string());
            }
        }
        Ok(())
    }

    /// Apply a sequence of key-value overrides, short-circuiting on the first error.
    pub fn apply_overrides_from_pairs(&mut self, pairs: &[(String, String)]) -> Result<(), String> {
        for (key, value) in pairs {
            self.set_field(key, value)
                .map_err(|e| format!("override '{}={}': {}", key, value, e))?;
        }
        Ok(())
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
            api_key: Some("secret".to_string()),
            model: "test-model".to_string(),
            context_window_limit: 4096,
            warn_threshold_percent: 80.0,
            verbose: false,
            hide_tool_calls: false,
            no_tool_calls: false,
            show_thinking: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            fuel: 30,
            fuel_empty_response_cost: 15,
            username: "testuser".to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec!["/tmp".to_string()],
            api: ApiParams::defaults(),
            tools: ToolsConfig::default(),
            fallback_tool: "call_user".to_string(),
            storage: StorageConfig {
                partition_max_entries: Some(500),
                partition_max_tokens: Some(100_000),
                ..Default::default()
            },
            url_policy: None,
            extra: BTreeMap::new(),
        };

        assert_eq!(config.get_field("model"), Some("test-model".to_string()));
        assert_eq!(config.get_field("username"), Some("testuser".to_string()));
        assert_eq!(config.get_field("api_key"), Some("(set)".to_string()));
        assert_eq!(
            config.get_field("file_tools_allowed_paths"),
            Some("/tmp".to_string())
        );

        // Storage fields
        assert_eq!(
            config.get_field("storage.partition_max_entries"),
            Some("500".to_string())
        );
        assert_eq!(
            config.get_field("storage.partition_max_tokens"),
            Some("100000".to_string())
        );
        assert_eq!(config.get_field("storage.bytes_per_token"), None); // Not set
        assert_eq!(config.get_field("storage.enable_bloom_filters"), None); // Not set
    }

    #[test]
    fn test_tools_config_deserialize_with_categories() {
        let toml_str = r#"
            include = ["shell_exec", "file_edit"]
            exclude = ["spawn_agent"]
            exclude_categories = ["builtin"]
        "#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.include,
            Some(vec!["shell_exec".to_string(), "file_edit".to_string()])
        );
        assert_eq!(config.exclude, Some(vec!["spawn_agent".to_string()]));
        assert_eq!(config.exclude_categories, Some(vec!["builtin".to_string()]));
    }

    #[test]
    fn test_tools_config_merge_local_appends_exclude() {
        let global = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool_a".to_string()]),
            exclude_categories: Some(vec!["builtin".to_string()]),
        };
        let local = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool_b".to_string()]),
            exclude_categories: Some(vec!["agent".to_string()]),
        };
        let merged = global.merge_local(&local);
        assert_eq!(
            merged.exclude,
            Some(vec!["tool_a".to_string(), "tool_b".to_string()])
        );
        assert_eq!(
            merged.exclude_categories,
            Some(vec!["builtin".to_string(), "agent".to_string()])
        );
    }

    #[test]
    fn test_tools_config_merge_local_include_overrides() {
        let global = ToolsConfig {
            include: Some(vec!["tool_a".to_string(), "tool_b".to_string()]),
            exclude: None,
            exclude_categories: None,
        };
        let local = ToolsConfig {
            include: Some(vec!["tool_c".to_string()]),
            exclude: None,
            exclude_categories: None,
        };
        let merged = global.merge_local(&local);
        assert_eq!(merged.include, Some(vec!["tool_c".to_string()]));
    }

    #[test]
    fn test_tools_config_merge_local_deduplicates() {
        let global = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool_a".to_string()]),
            exclude_categories: None,
        };
        let local = ToolsConfig {
            include: None,
            exclude: Some(vec!["tool_a".to_string(), "tool_b".to_string()]),
            exclude_categories: None,
        };
        let merged = global.merge_local(&local);
        assert_eq!(
            merged.exclude,
            Some(vec!["tool_a".to_string(), "tool_b".to_string()])
        );
    }

    /// Helper: minimal ResolvedConfig for set_field tests
    fn test_resolved_config() -> ResolvedConfig {
        ResolvedConfig {
            api_key: None,
            model: "test-model".to_string(),
            context_window_limit: 4096,
            warn_threshold_percent: 80.0,
            verbose: false,
            hide_tool_calls: false,
            no_tool_calls: false,
            show_thinking: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            fuel: 30,
            fuel_empty_response_cost: 15,
            username: "testuser".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::defaults(),
            tools: ToolsConfig::default(),
            fallback_tool: "call_user".to_string(),
            storage: StorageConfig::default(),
            url_policy: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn test_set_field_bool() {
        let mut config = test_resolved_config();
        config.set_field("verbose", "true").unwrap();
        assert_eq!(config.get_field("verbose"), Some("true".to_string()));
        config.set_field("verbose", "false").unwrap();
        assert_eq!(config.get_field("verbose"), Some("false".to_string()));
    }

    #[test]
    fn test_set_field_usize() {
        let mut config = test_resolved_config();
        config.set_field("fuel", "50").unwrap();
        assert_eq!(config.get_field("fuel"), Some("50".to_string()));
        config.set_field("fuel_empty_response_cost", "5").unwrap();
        assert_eq!(
            config.get_field("fuel_empty_response_cost"),
            Some("5".to_string())
        );
    }

    #[test]
    fn test_set_field_f32() {
        let mut config = test_resolved_config();
        config.set_field("auto_compact_threshold", "0.9").unwrap();
        assert_eq!(config.auto_compact_threshold, 0.9);
        config.set_field("warn_threshold_percent", "75.5").unwrap();
        assert_eq!(config.warn_threshold_percent, 75.5);
    }

    #[test]
    fn test_set_field_string() {
        let mut config = test_resolved_config();
        config
            .set_field("model", "claude-sonnet-4-5-20250929")
            .unwrap();
        assert_eq!(
            config.get_field("model"),
            Some("claude-sonnet-4-5-20250929".to_string())
        );
        config.set_field("username", "alice").unwrap();
        assert_eq!(config.get_field("username"), Some("alice".to_string()));
        config.set_field("fallback_tool", "call_agent").unwrap();
        assert_eq!(
            config.get_field("fallback_tool"),
            Some("call_agent".to_string())
        );
    }

    #[test]
    fn test_set_field_nested() {
        let mut config = test_resolved_config();
        config.set_field("api.temperature", "0.7").unwrap();
        assert_eq!(config.api.temperature, Some(0.7));
        config.set_field("api.reasoning.effort", "high").unwrap();
        assert_eq!(config.api.reasoning.effort, Some(ReasoningEffort::High));
        config
            .set_field("storage.partition_max_entries", "1000")
            .unwrap();
        assert_eq!(config.storage.partition_max_entries, Some(1000));
    }

    #[test]
    fn test_set_field_unknown_goes_to_extra() {
        let mut config = test_resolved_config();
        config
            .set_field("my_custom_key", "my_custom_value")
            .unwrap();
        assert_eq!(
            config.get_field("my_custom_key"),
            Some("my_custom_value".to_string())
        );
        assert_eq!(
            config.extra.get("my_custom_key"),
            Some(&"my_custom_value".to_string())
        );
    }

    #[test]
    fn test_set_field_parse_error() {
        let mut config = test_resolved_config();
        let result = config.set_field("fuel", "notanumber");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid usize"));
    }

    #[test]
    fn test_apply_overrides_from_pairs() {
        let mut config = test_resolved_config();
        let pairs = vec![
            ("fuel".to_string(), "50".to_string()),
            ("model".to_string(), "gpt-4".to_string()),
            ("verbose".to_string(), "true".to_string()),
        ];
        config.apply_overrides_from_pairs(&pairs).unwrap();
        assert_eq!(config.fuel, 50);
        assert_eq!(config.model, "gpt-4");
        assert!(config.verbose);
    }

    #[test]
    fn test_apply_overrides_from_pairs_stops_on_error() {
        let mut config = test_resolved_config();
        let pairs = vec![
            ("fuel".to_string(), "50".to_string()),
            ("verbose".to_string(), "notabool".to_string()),
            ("model".to_string(), "should-not-reach".to_string()),
        ];
        let result = config.apply_overrides_from_pairs(&pairs);
        assert!(result.is_err());
        // first pair applied, second failed, third not reached
        assert_eq!(config.fuel, 50);
        assert_eq!(config.model, "test-model"); // unchanged
    }
}
