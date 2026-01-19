use serde::{Deserialize, Serialize};

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
}
