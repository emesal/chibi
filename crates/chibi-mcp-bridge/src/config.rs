use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// MCP server definition
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Summary generation config (used in task 12: LLM summary generation)
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SummaryConfig {
    #[serde(default = "default_summary_model")]
    pub model: String,
}

fn default_summary_model() -> String {
    "ratatoskr:free/text-generation".to_string()
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            model: default_summary_model(),
        }
    }
}

/// Top-level bridge config from mcp-bridge.toml
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_minutes: u64,
    #[serde(default)]
    #[allow(dead_code)] // used in task 12: LLM summary generation
    pub summary: SummaryConfig,
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}

fn default_idle_timeout() -> u64 {
    5
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            idle_timeout_minutes: default_idle_timeout(),
            summary: SummaryConfig::default(),
            servers: HashMap::new(),
        }
    }
}

impl BridgeConfig {
    /// Load config from `<home>/mcp-bridge.toml`, falling back to defaults.
    pub fn load(home: &Path) -> Self {
        let path = home.join("mcp-bridge.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                eprintln!("[mcp-bridge] config parse error: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_one_server() {
        let toml = r#"
[servers.serena]
command = "serena"
args = ["--stdio"]
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        let s = &cfg.servers["serena"];
        assert_eq!(s.command, "serena");
        assert_eq!(s.args, vec!["--stdio"]);
        assert_eq!(cfg.idle_timeout_minutes, 5);
    }

    #[test]
    fn parse_with_idle_timeout() {
        let toml = r#"
idle_timeout_minutes = 10

[servers.test]
command = "test-server"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.idle_timeout_minutes, 10);
    }

    #[test]
    fn parse_with_summary_section() {
        let toml = r#"
[summary]
model = "claude-3-haiku"

[servers.test]
command = "test-server"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.summary.model, "claude-3-haiku");
    }

    #[test]
    fn parse_multiple_servers() {
        let toml = r#"
[servers.alpha]
command = "alpha-cmd"
args = ["-a"]

[servers.beta]
command = "beta-cmd"
args = ["-b", "--verbose"]
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.servers.len(), 2);
        assert!(cfg.servers.contains_key("alpha"));
        assert!(cfg.servers.contains_key("beta"));
    }

    #[test]
    fn missing_file_returns_defaults() {
        let cfg = BridgeConfig::load(Path::new("/nonexistent/path"));
        assert_eq!(cfg.idle_timeout_minutes, 5);
        assert!(cfg.servers.is_empty());
        assert_eq!(cfg.summary.model, "ratatoskr:free/text-generation");
    }

    #[test]
    fn server_args_default_to_empty() {
        let toml = r#"
[servers.minimal]
command = "my-server"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert!(cfg.servers["minimal"].args.is_empty());
    }
}
