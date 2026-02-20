//! Configuration resolution for AppState.
//!
//! Methods for loading, saving, and resolving local configs and model names.

use crate::config::{ApiParams, ConfigDefaults, LocalConfig, ResolvedConfig};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, ErrorKind};

use super::{AppState, StatePaths};

impl AppState {
    /// Load local config for a context (returns default if doesn't exist)
    pub fn load_local_config(&self, context_name: &str) -> io::Result<LocalConfig> {
        let path = self.local_config_file(context_name);
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse local.toml: {}", e),
                )
            })
        } else {
            Ok(LocalConfig::default())
        }
    }

    /// Save local config for a context (atomic write)
    pub fn save_local_config(
        &self,
        context_name: &str,
        local_config: &LocalConfig,
    ) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let path = self.local_config_file(context_name);
        let content = toml::to_string_pretty(local_config)
            .map_err(|e| io::Error::other(format!("Failed to serialize local.toml: {}", e)))?;
        crate::safe_io::atomic_write_text(&path, &content)
    }

    /// Resolve model name using models.toml aliases
    /// If the model is an alias defined in models.toml, return the full model name
    /// Otherwise return the original model name
    pub fn resolve_model_name(&self, model: &str) -> String {
        if self.models_config.models.contains_key(model) {
            // The model name itself is a key in models.toml, use it as-is
            // (models.toml maps alias -> metadata, not alias -> full name)
            model.to_string()
        } else {
            model.to_string()
        }
    }

    /// Resolve the full configuration, applying overrides in order:
    /// 1. Runtime override (passed as parameter, highest priority)
    /// 2. Context-local config (local.toml)
    /// 3. Models.toml (per-model API params)
    /// 4. Environment variables (`CHIBI_API_KEY`, `CHIBI_MODEL`)
    /// 5. Global config (config.toml)
    /// 6. Defaults
    pub fn resolve_config(
        &self,
        context_name: &str,
        username_override: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        let local = self.load_local_config(context_name)?;

        // Start with defaults, then merge global config
        let mut api_params = ApiParams::defaults();
        api_params = api_params.merge_with(&self.config.api);

        // Start with global config values, applying defaults for optional fields
        let mut resolved = ResolvedConfig {
            api_key: self.config.api_key.clone(),
            model: self
                .config
                .model
                .clone()
                .unwrap_or_else(|| ConfigDefaults::MODEL.to_string()),
            context_window_limit: self
                .config
                .context_window_limit
                .unwrap_or(ConfigDefaults::CONTEXT_WINDOW_LIMIT),
            warn_threshold_percent: self.config.warn_threshold_percent,
            no_tool_calls: self.config.no_tool_calls,
            auto_compact: self.config.auto_compact,
            auto_compact_threshold: self.config.auto_compact_threshold,
            fuel: self.config.fuel,
            fuel_empty_response_cost: self.config.fuel_empty_response_cost,
            username: self.config.username.clone(),
            reflection_enabled: self.config.reflection_enabled,
            reflection_character_limit: self.config.reflection_character_limit,
            rolling_compact_drop_percentage: self.config.rolling_compact_drop_percentage,
            tool_output_cache_threshold: self.config.tool_output_cache_threshold,
            tool_cache_max_age_days: self.config.tool_cache_max_age_days,
            auto_cleanup_cache: self.config.auto_cleanup_cache,
            tool_cache_preview_chars: self.config.tool_cache_preview_chars,
            file_tools_allowed_paths: self.config.file_tools_allowed_paths.clone(),
            api: api_params,
            tools: self.config.tools.clone(),
            fallback_tool: self.config.fallback_tool.clone(),
            storage: self.config.storage.clone(),
            url_policy: self.config.url_policy.clone(),
            subagent_cost_tier: self.config.subagent_cost_tier.clone(),
            extra: BTreeMap::new(),
        };

        // Apply environment variable overrides (between global config and local.toml)
        apply_env_overrides(&mut resolved);

        // Apply local config overrides (simple fields via macro, see LocalConfig::apply_overrides)
        local.apply_overrides(&mut resolved);

        // Apply context-level storage config overrides
        resolved.storage = resolved.storage.merge(&local.storage);

        // Apply context-level API params (Layer 3)
        if let Some(ref local_api) = local.api {
            resolved.api = resolved.api.merge_with(local_api);
        }

        // Apply context-level tool filtering config (merge local on top of global)
        if let Some(ref local_tools) = local.tools {
            resolved.tools = resolved.tools.merge_local(local_tools);
        }

        // Apply runtime username override (highest priority)
        if let Some(username) = username_override {
            resolved.username = username.to_string();
        }

        // Default file_tools_allowed_paths to cwd when empty (after all overrides).
        // This ensures project files are readable without explicit config.
        if resolved.file_tools_allowed_paths.is_empty()
            && let Ok(cwd) = std::env::current_dir()
        {
            resolved.file_tools_allowed_paths = vec![cwd.to_string_lossy().to_string()];
        }

        // Resolve model name and potentially override context window + API params
        resolved.model = self.resolve_model_name(&resolved.model);
        if let Some(model_meta) = self.models_config.models.get(&resolved.model) {
            // Apply model-level API params (Layer 2 - after global, before context)
            // Note: We merge model params before context params because context should override model
            // But we do this after context-level override for the rest of config, so we need to
            // re-merge context params on top
            let model_api = resolved.api.merge_with(&model_meta.api);
            // Re-apply context-level API params on top of model params
            resolved.api = if let Some(ref local_api) = local.api {
                model_api.merge_with(local_api)
            } else {
                model_api
            };
        }

        Ok(resolved)
    }

    /// Validate resolved config against loaded tools
    ///
    /// Checks that fallback_tool exists and has flow_control=true metadata.
    pub fn validate_config(
        &self,
        resolved: &ResolvedConfig,
        tools: &[crate::tools::Tool],
    ) -> io::Result<()> {
        let fallback = &resolved.fallback_tool;

        // Get metadata (checks plugins then builtins)
        let meta = crate::tools::get_tool_metadata(tools, fallback);

        // Verify tool exists: must be in plugins OR be a known builtin
        let is_builtin = matches!(
            fallback.as_str(),
            crate::tools::CALL_AGENT_TOOL_NAME | crate::tools::CALL_USER_TOOL_NAME
        );
        let in_plugins = tools.iter().any(|t| t.name == *fallback);

        if !is_builtin && !in_plugins {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("fallback_tool '{}' not found", fallback),
            ));
        }

        // Enforce: fallback must be a flow_control tool
        if !meta.flow_control {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "fallback_tool '{}' must have flow_control=true metadata",
                    fallback
                ),
            ));
        }

        Ok(())
    }
}

/// Environment variable names for config overrides.
pub const ENV_API_KEY: &str = "CHIBI_API_KEY";
pub const ENV_MODEL: &str = "CHIBI_MODEL";

/// Apply environment variable overrides onto a resolved config.
///
/// Priority: global config.toml < **env vars** < context local.toml.
fn apply_env_overrides(resolved: &mut ResolvedConfig) {
    if let Ok(key) = env::var(ENV_API_KEY) {
        resolved.api_key = Some(key);
    }
    if let Ok(model) = env::var(ENV_MODEL) {
        resolved.model = model;
    }
}
