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
}

/// Per-context config from ~/.chibi/contexts/<name>/local.toml
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct LocalConfig {
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub username: Option<String>,
    pub auto_compact: Option<bool>,
    pub max_recursion_depth: Option<usize>,
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
    pub base_url: String,
    pub auto_compact: bool,
    pub auto_compact_threshold: f32,
    pub max_recursion_depth: usize,
    pub username: String,
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
}
