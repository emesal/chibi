//!
//! network tools: outbound HTTP.
//! Callers must apply URL policy / PreFetchUrl hook before invoking.

use std::io;

use super::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::json_ext::JsonExt;

// === Tool Name Constants ===

pub const FETCH_URL_TOOL_NAME: &str = "fetch_url";

// === Tool Definition Registry ===

/// All network tool definitions
pub static NETWORK_TOOL_DEFS: &[BuiltinToolDef] = &[BuiltinToolDef {
    name: FETCH_URL_TOOL_NAME,
    description: "Fetch content from a URL via HTTP GET and return the response body. Follows redirects. Use for retrieving web pages, API responses, or raw file content.",
    properties: &[
        ToolPropertyDef {
            name: "url",
            prop_type: "string",
            description: "URL to fetch (must start with http:// or https://)",
            default: None,
        },
        ToolPropertyDef {
            name: "max_bytes",
            prop_type: "integer",
            description: "Maximum response body size in bytes (default: 1048576 = 1MB)",
            default: Some(1_048_576),
        },
        ToolPropertyDef {
            name: "timeout_secs",
            prop_type: "integer",
            description: "Request timeout in seconds (default: 30)",
            default: Some(30),
        },
    ],
    required: &["url"],
    summary_params: &["url"],
}];

// === Registry Helpers ===

/// Register all network tools into the registry.
pub fn register_network_tools(registry: &mut super::registry::ToolRegistry) {
    use std::sync::Arc;
    use super::registry::{ToolCategory, ToolHandler};
    use super::Tool;

    let handler: ToolHandler = Arc::new(|call| {
        Box::pin(async move {
            execute_network_tool(call.name, call.args)
                .await
                .unwrap_or_else(|| {
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("unknown network tool: {}", call.name),
                    ))
                })
        })
    });

    for def in NETWORK_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(def, handler.clone(), ToolCategory::Network));
    }
}

/// Convert all network tools to API format
pub fn all_network_tools_to_api_format() -> Vec<serde_json::Value> {
    NETWORK_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the network group
pub fn is_network_tool(name: &str) -> bool {
    name == FETCH_URL_TOOL_NAME
}

// === Tool Execution ===

/// Execute a network tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
/// Note: URL policy gating must be applied by the caller before invoking.
pub async fn execute_network_tool(
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<io::Result<String>> {
    match tool_name {
        FETCH_URL_TOOL_NAME => Some(execute_fetch_url(args).await),
        _ => None,
    }
}

// === fetch_url ===

/// Execute fetch_url: HTTP GET a URL and return the response body.
///
/// Delegates to `fetch_url_with_limit` for streaming size-limited fetching.
/// URL policy gating is handled by the caller.
async fn execute_fetch_url(args: &serde_json::Value) -> io::Result<String> {
    let url = require_str_param(args, "url")?;
    let max_bytes = args.get_u64_or("max_bytes", 1_048_576) as usize;
    let timeout_secs = args.get_u64_or("timeout_secs", 30);

    super::fetch_url_with_limit(&url, max_bytes, timeout_secs).await
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    #[test]
    fn test_network_tool_defs_api_format() {
        for def in NETWORK_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
        }
    }

    #[test]
    fn test_is_network_tool() {
        assert!(is_network_tool(FETCH_URL_TOOL_NAME));
        assert!(!is_network_tool("shell_exec"));
        assert!(!is_network_tool("file_head"));
    }

    #[test]
    fn test_tool_constant() {
        assert_eq!(FETCH_URL_TOOL_NAME, "fetch_url");
    }

    #[tokio::test]
    async fn test_fetch_url_invalid_scheme() {
        let a = args(&[("url", serde_json::json!("ftp://example.com"))]);
        let result = execute_fetch_url(&a).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http://"));
    }

    #[tokio::test]
    async fn test_fetch_url_missing_param() {
        let a = args(&[]);
        let result = execute_fetch_url(&a).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing"));
    }
}
