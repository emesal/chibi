use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// MCP server definition — either a local stdio process or a remote HTTP endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServerConfig {
    /// Local server spawned as a child process (stdio transport).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Remote server accessed via streamable HTTP transport.
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// Summary generation config.
#[derive(Debug, Clone, Deserialize)]
pub struct SummaryConfig {
    #[serde(default = "default_summary_enabled")]
    pub enabled: bool,
    #[serde(default = "default_summary_model")]
    pub model: String,
}

fn default_summary_model() -> String {
    "ratatoskr:free/summariser".to_string()
}

fn default_summary_enabled() -> bool {
    true
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            enabled: default_summary_enabled(),
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
        match &cfg.servers["serena"] {
            ServerConfig::Stdio { command, args } => {
                assert_eq!(command, "serena");
                assert_eq!(args, &["--stdio"]);
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
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
        assert_eq!(cfg.summary.model, "ratatoskr:free/summariser");
    }

    #[test]
    fn server_args_default_to_empty() {
        let toml = r#"
[servers.minimal]
command = "my-server"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        match &cfg.servers["minimal"] {
            ServerConfig::Stdio { args, .. } => assert!(args.is_empty()),
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn parse_streamable_http_server() {
        let toml = r#"
[servers.remote]
url = "https://my-server.com/mcp"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        match &cfg.servers["remote"] {
            ServerConfig::StreamableHttp { url, headers } => {
                assert_eq!(url, "https://my-server.com/mcp");
                assert!(headers.is_empty());
            }
            other => panic!("expected StreamableHttp, got {other:?}"),
        }
    }

    #[test]
    fn parse_streamable_http_with_headers() {
        let toml = r#"
[servers.remote]
url = "https://my-server.com/mcp"

[servers.remote.headers]
Authorization = "Bearer sk-test"
X-Custom = "value"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        match &cfg.servers["remote"] {
            ServerConfig::StreamableHttp { url, headers } => {
                assert_eq!(url, "https://my-server.com/mcp");
                assert_eq!(headers["Authorization"], "Bearer sk-test");
                assert_eq!(headers["X-Custom"], "value");
            }
            other => panic!("expected StreamableHttp, got {other:?}"),
        }
    }

    #[test]
    fn parse_mixed_servers() {
        let toml = r#"
[servers.local]
command = "serena"
args = ["--stdio"]

[servers.remote]
url = "https://my-server.com/mcp"
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.servers.len(), 2);
        assert!(matches!(cfg.servers["local"], ServerConfig::Stdio { .. }));
        assert!(matches!(
            cfg.servers["remote"],
            ServerConfig::StreamableHttp { .. }
        ));
    }

    #[test]
    fn validate_rejects_empty_server() {
        let toml = r#"
[servers.bad]
args = ["--flag"]
"#;
        let result: Result<BridgeConfig, _> = toml::from_str(toml);
        assert!(
            result.is_err(),
            "server with neither command nor url should fail to parse"
        );
    }

    #[test]
    fn stdio_config_preserves_existing_behaviour() {
        let toml = r#"
[servers.serena]
command = "serena"
args = ["--stdio"]
"#;
        let cfg: BridgeConfig = toml::from_str(toml).unwrap();
        match &cfg.servers["serena"] {
            ServerConfig::Stdio { command, args } => {
                assert_eq!(command, "serena");
                assert_eq!(args, &["--stdio"]);
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }
}
