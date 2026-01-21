use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

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
    15
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
}

/// Model metadata from ~/.chibi/models.toml
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelMetadata {
    #[serde(default)]
    pub context_window: Option<usize>,
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
}

use crate::input::PartialRuntimeConfig;

impl Config {
    /// Resolve configuration with runtime overrides.
    ///
    /// Priority order (highest to lowest):
    /// 1. Runtime overrides (from CLI flags or JSON input)
    /// 2. Local config (per-context)
    /// 3. Global config (config.toml)
    /// 4. Model metadata (for context_window_limit)
    /// 5. Defaults
    pub fn resolve_with_runtime(
        &self,
        runtime: &PartialRuntimeConfig,
        local: &LocalConfig,
        models: &ModelsConfig,
    ) -> ResolvedConfig {
        let mut resolved = ResolvedConfig {
            // Start with global config values
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            context_window_limit: self.context_window_limit,
            warn_threshold_percent: self.warn_threshold_percent,
            base_url: self.base_url.clone(),
            auto_compact: self.auto_compact,
            auto_compact_threshold: self.auto_compact_threshold,
            max_recursion_depth: self.max_recursion_depth,
            username: self.username.clone(),
            reflection_enabled: self.reflection_enabled,
        };

        // Layer 2: Local config overrides
        if let Some(ref v) = local.api_key {
            resolved.api_key = v.clone();
        }
        if let Some(ref v) = local.model {
            resolved.model = v.clone();
        }
        if let Some(ref v) = local.base_url {
            resolved.base_url = v.clone();
        }
        if let Some(v) = local.context_window_limit {
            resolved.context_window_limit = v;
        }
        if let Some(v) = local.warn_threshold_percent {
            resolved.warn_threshold_percent = v;
        }
        if let Some(v) = local.auto_compact {
            resolved.auto_compact = v;
        }
        if let Some(v) = local.auto_compact_threshold {
            resolved.auto_compact_threshold = v;
        }
        if let Some(v) = local.max_recursion_depth {
            resolved.max_recursion_depth = v;
        }
        if let Some(ref v) = local.username {
            resolved.username = v.clone();
        }
        if let Some(v) = local.reflection_enabled {
            resolved.reflection_enabled = v;
        }

        // Layer 3: Runtime overrides (highest priority)
        if let Some(ref v) = runtime.api_key {
            resolved.api_key = v.clone();
        }
        if let Some(ref v) = runtime.model {
            resolved.model = v.clone();
        }
        if let Some(ref v) = runtime.base_url {
            resolved.base_url = v.clone();
        }
        if let Some(v) = runtime.context_window_limit {
            resolved.context_window_limit = v;
        }
        if let Some(v) = runtime.warn_threshold_percent {
            resolved.warn_threshold_percent = v;
        }
        if let Some(v) = runtime.auto_compact {
            resolved.auto_compact = v;
        }
        if let Some(v) = runtime.auto_compact_threshold {
            resolved.auto_compact_threshold = v;
        }
        if let Some(v) = runtime.max_recursion_depth {
            resolved.max_recursion_depth = v;
        }
        if let Some(v) = runtime.reflection_enabled {
            resolved.reflection_enabled = v;
        }

        // Layer 4: Model metadata for context_window (only if not overridden)
        // This is applied after runtime since we want explicit overrides to win
        if runtime.context_window_limit.is_none() && local.context_window_limit.is_none() {
            if let Some(meta) = models.models.get(&resolved.model) {
                if let Some(window) = meta.context_window {
                    resolved.context_window_limit = window;
                }
            }
        }

        resolved
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
        assert_eq!(default_max_recursion_depth(), 15);
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
        assert_eq!(config.max_recursion_depth, 15);
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

    // Helper to create a test config
    fn make_test_config() -> Config {
        Config {
            api_key: "global-key".to_string(),
            model: "gpt-4".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            base_url: DEFAULT_API_URL.to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            max_recursion_depth: 15,
            username: "user".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
        }
    }

    #[test]
    fn test_resolve_with_runtime_defaults() {
        let config = make_test_config();
        let local = LocalConfig::default();
        let models = ModelsConfig::default();
        let runtime = PartialRuntimeConfig::default();

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        assert_eq!(resolved.api_key, "global-key");
        assert_eq!(resolved.model, "gpt-4");
        assert_eq!(resolved.context_window_limit, 8000);
        assert_eq!(resolved.warn_threshold_percent, 75.0);
        assert!(!resolved.auto_compact);
        assert!(resolved.reflection_enabled);
    }

    #[test]
    fn test_resolve_with_runtime_local_overrides() {
        let config = make_test_config();
        let local = LocalConfig {
            model: Some("claude-3".to_string()),
            username: Some("localuser".to_string()),
            auto_compact: Some(true),
            ..Default::default()
        };
        let models = ModelsConfig::default();
        let runtime = PartialRuntimeConfig::default();

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        assert_eq!(resolved.api_key, "global-key"); // unchanged
        assert_eq!(resolved.model, "claude-3"); // local override
        assert_eq!(resolved.username, "localuser"); // local override
        assert!(resolved.auto_compact); // local override
    }

    #[test]
    fn test_resolve_with_runtime_runtime_overrides() {
        let config = make_test_config();
        let local = LocalConfig {
            model: Some("claude-3".to_string()),
            ..Default::default()
        };
        let models = ModelsConfig::default();
        let runtime = PartialRuntimeConfig {
            model: Some("gpt-4-turbo".to_string()),
            auto_compact: Some(true),
            ..Default::default()
        };

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        // Runtime should win over local
        assert_eq!(resolved.model, "gpt-4-turbo");
        assert!(resolved.auto_compact);
    }

    #[test]
    fn test_resolve_with_runtime_model_metadata() {
        let config = make_test_config();
        let local = LocalConfig::default();
        let mut models = ModelsConfig::default();
        models.models.insert("gpt-4".to_string(), ModelMetadata {
            context_window: Some(128000),
        });
        let runtime = PartialRuntimeConfig::default();

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        // Model metadata should set context_window_limit
        assert_eq!(resolved.context_window_limit, 128000);
    }

    #[test]
    fn test_resolve_with_runtime_explicit_window_overrides_metadata() {
        let config = make_test_config();
        let local = LocalConfig {
            context_window_limit: Some(16000),
            ..Default::default()
        };
        let mut models = ModelsConfig::default();
        models.models.insert("gpt-4".to_string(), ModelMetadata {
            context_window: Some(128000),
        });
        let runtime = PartialRuntimeConfig::default();

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        // Explicit local override should win over model metadata
        assert_eq!(resolved.context_window_limit, 16000);
    }

    #[test]
    fn test_resolve_with_runtime_priority_chain() {
        let config = make_test_config();
        let local = LocalConfig {
            warn_threshold_percent: Some(60.0),
            ..Default::default()
        };
        let models = ModelsConfig::default();
        let runtime = PartialRuntimeConfig {
            warn_threshold_percent: Some(50.0),
            ..Default::default()
        };

        let resolved = config.resolve_with_runtime(&runtime, &local, &models);

        // Runtime (50.0) should win over local (60.0) should win over global (75.0)
        assert_eq!(resolved.warn_threshold_percent, 50.0);
    }
}
