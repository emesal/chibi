//! Configuration resolution for AppState.
//!
//! Methods for loading, saving, and resolving local configs and model names.

use crate::config::{ApiParams, LocalConfig, ResolvedConfig, ToolsConfig};
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
    /// 1. Runtime override (passed as parameter)
    /// 2. Context-local config (local.toml)
    /// 3. Global config (config.toml)
    /// 4. Models.toml (for model expansion)
    /// 5. Defaults
    pub fn resolve_config(
        &self,
        context_name: &str,
        username_override: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        let local = self.load_local_config(context_name)?;

        // Start with defaults, then merge global config
        let mut api_params = ApiParams::defaults();
        api_params = api_params.merge_with(&self.config.api);

        // Start with global config values
        let mut resolved = ResolvedConfig {
            api_key: self.config.api_key.clone(),
            model: self.config.model.clone(),
            context_window_limit: self.config.context_window_limit,
            warn_threshold_percent: self.config.warn_threshold_percent,
            base_url: self.config.base_url.clone(),
            auto_compact: self.config.auto_compact,
            auto_compact_threshold: self.config.auto_compact_threshold,
            max_recursion_depth: self.config.max_recursion_depth,
            username: self.config.username.clone(),
            reflection_enabled: self.config.reflection_enabled,
            tool_output_cache_threshold: self.config.tool_output_cache_threshold,
            tool_cache_max_age_days: self.config.tool_cache_max_age_days,
            auto_cleanup_cache: self.config.auto_cleanup_cache,
            tool_cache_preview_chars: self.config.tool_cache_preview_chars,
            file_tools_allowed_paths: self.config.file_tools_allowed_paths.clone(),
            api: api_params,
            tools: ToolsConfig::default(),
            fallback_tool: self.config.fallback_tool.clone(),
        };

        // Apply local config overrides
        if let Some(ref api_key) = local.api_key {
            resolved.api_key = api_key.clone();
        }
        if let Some(ref model) = local.model {
            resolved.model = model.clone();
        }
        if let Some(ref base_url) = local.base_url {
            resolved.base_url = base_url.clone();
        }
        if let Some(context_window_limit) = local.context_window_limit {
            resolved.context_window_limit = context_window_limit;
        }
        if let Some(warn_threshold_percent) = local.warn_threshold_percent {
            resolved.warn_threshold_percent = warn_threshold_percent;
        }
        if let Some(auto_compact) = local.auto_compact {
            resolved.auto_compact = auto_compact;
        }
        if let Some(auto_compact_threshold) = local.auto_compact_threshold {
            resolved.auto_compact_threshold = auto_compact_threshold;
        }
        if let Some(max_recursion_depth) = local.max_recursion_depth {
            resolved.max_recursion_depth = max_recursion_depth;
        }
        if let Some(ref username) = local.username {
            resolved.username = username.clone();
        }
        if let Some(reflection_enabled) = local.reflection_enabled {
            resolved.reflection_enabled = reflection_enabled;
        }
        if let Some(tool_output_cache_threshold) = local.tool_output_cache_threshold {
            resolved.tool_output_cache_threshold = tool_output_cache_threshold;
        }
        if let Some(tool_cache_max_age_days) = local.tool_cache_max_age_days {
            resolved.tool_cache_max_age_days = tool_cache_max_age_days;
        }
        if let Some(auto_cleanup_cache) = local.auto_cleanup_cache {
            resolved.auto_cleanup_cache = auto_cleanup_cache;
        }
        if let Some(tool_cache_preview_chars) = local.tool_cache_preview_chars {
            resolved.tool_cache_preview_chars = tool_cache_preview_chars;
        }
        if let Some(ref file_tools_allowed_paths) = local.file_tools_allowed_paths {
            resolved.file_tools_allowed_paths = file_tools_allowed_paths.clone();
        }
        if let Some(ref fallback_tool) = local.fallback_tool {
            resolved.fallback_tool = fallback_tool.clone();
        }

        // Apply context-level API params (Layer 3)
        if let Some(ref local_api) = local.api {
            resolved.api = resolved.api.merge_with(local_api);
        }

        // Apply context-level tool filtering config
        if let Some(ref local_tools) = local.tools {
            resolved.tools = local_tools.clone();
        }

        // Apply runtime username override (highest priority)
        if let Some(username) = username_override {
            resolved.username = username.to_string();
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

            if let Some(context_window) = model_meta.context_window {
                resolved.context_window_limit = context_window;
            }
        }

        Ok(resolved)
    }
}
