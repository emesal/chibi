//! Tools and plugins module.
//!
//! This module provides the extensible tool system:
//! - Plugin loading and execution from the plugins directory
//! - MCP bridge client for tools from remote MCP servers
//! - Built-in tools for reflection, todos, goals, and messaging
//! - Coding tools for shell execution, file editing, and codebase indexing
//! - File tools for examining cached tool outputs
//! - Agent tools for sub-agent spawning and content retrieval
//! - URL and file path security policies
//! - Hook system for plugin lifecycle events (31 hook points)

pub mod agent_tools;
mod builtin;
pub mod coding_tools;
pub mod file_tools;
mod hooks;
pub mod mcp;
mod plugins;
pub mod security;

use std::path::PathBuf;

pub use hooks::HookPoint;

// Re-export hook execution
pub use hooks::execute_hook;

// Re-export plugin functions
pub use plugins::{execute_tool, find_tool, load_tools, tools_to_api_format};

// Re-export built-in tool constants (used by api module)
pub use builtin::{CALL_AGENT_TOOL_NAME, CALL_USER_TOOL_NAME};
pub use builtin::{
    MODEL_INFO_TOOL_NAME, READ_CONTEXT_TOOL_NAME, REFLECTION_TOOL_NAME, SEND_MESSAGE_TOOL_NAME,
};

// Re-export handoff types for control flow
pub use builtin::{Handoff, HandoffTarget};

// Re-export builtin tool registry lookup
pub use builtin::{get_builtin_tool_def, is_builtin_tool};

// Re-export registry-based tool generation
pub use builtin::{all_builtin_tools_to_api_format, builtin_tools_to_api_format};

// Re-export built-in tool execution functions
pub use builtin::execute_builtin_tool;

// Re-export tool metadata functions
pub use builtin::builtin_tool_metadata;

// Re-export summary_params lookup
pub use builtin::builtin_summary_params;

// Re-export coding tool registry functions and execution
pub use coding_tools::{
    CODING_TOOL_DEFS, all_coding_tools_to_api_format, execute_coding_tool, is_coding_tool,
};
pub use coding_tools::{
    DIR_LIST_TOOL_NAME, FETCH_URL_TOOL_NAME, FILE_EDIT_TOOL_NAME, GLOB_FILES_TOOL_NAME,
    GREP_FILES_TOOL_NAME, INDEX_QUERY_TOOL_NAME, INDEX_STATUS_TOOL_NAME, INDEX_UPDATE_TOOL_NAME,
    SHELL_EXEC_TOOL_NAME,
};

// Re-export file tool registry functions
pub use file_tools::{all_file_tools_to_api_format, get_file_tool_def};

// Re-export file tool execution and utilities
pub use file_tools::{execute_file_tool, is_file_tool};

// Re-export file write tool names for permission gating
pub use file_tools::WRITE_FILE_TOOL_NAME;

// Re-export agent tool registry functions
pub use agent_tools::{all_agent_tools_to_api_format, get_agent_tool_def};

// Re-export agent tool execution and utilities
pub use agent_tools::{execute_agent_tool, is_agent_tool, spawn_agent};

// Re-export agent tool types and constants
pub use agent_tools::{SPAWN_AGENT_TOOL_NAME, SUMMARIZE_CONTENT_TOOL_NAME, SpawnOptions};

// Re-export security utilities
pub use security::{
    FilePathAccess, UrlAction, UrlCategory, UrlPolicy, UrlRule, UrlSafety, classify_file_path,
    classify_url, evaluate_url_policy, validate_file_path,
};

/// Metadata for tool behavior in the agentic loop
#[derive(Debug, Clone, Default)]
pub struct ToolMetadata {
    /// Can this tool run in parallel with others? (default: true)
    /// Tools with parallel=true are executed concurrently via join_all.
    /// Flow control tools are always sequential regardless of this flag.
    pub parallel: bool,

    /// Is this a flow control tool? (default: false)
    pub flow_control: bool,

    /// If flow_control=true, does invoking end the turn? (default: false)
    /// true = return to user (like call_user)
    /// false = continue processing (like call_agent)
    pub ends_turn: bool,
}

impl ToolMetadata {
    /// Default metadata for regular tools
    pub fn new() -> Self {
        Self {
            parallel: true,
            flow_control: false,
            ends_turn: false,
        }
    }
}

/// Represents a tool that can be called by the LLM
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub path: PathBuf,
    pub hooks: Vec<HookPoint>,
    pub metadata: ToolMetadata,
    /// Parameter names whose values should appear in tool-call notices.
    pub summary_params: Vec<String>,
}

/// Collect names of all built-in tools (core, file, agent).
///
/// Returns a flat list of tool names from all internal registries.
/// New tool categories should be added here when introduced.
pub fn builtin_tool_names() -> Vec<&'static str> {
    builtin::BUILTIN_TOOL_DEFS
        .iter()
        .chain(file_tools::FILE_TOOL_DEFS.iter())
        .chain(agent_tools::AGENT_TOOL_DEFS.iter())
        .chain(coding_tools::CODING_TOOL_DEFS.iter())
        .map(|def| def.name)
        .collect()
}

/// Get metadata for any tool (plugin or builtin)
///
/// Checks plugins first, then falls back to builtin_tool_metadata for known builtins.
pub fn get_tool_metadata(tools: &[Tool], name: &str) -> ToolMetadata {
    if let Some(tool) = tools.iter().find(|t| t.name == name) {
        return tool.metadata.clone();
    }
    builtin_tool_metadata(name)
}

/// Build a concise summary string from a tool's declared summary_params and actual arguments.
///
/// Checks plugins first, then falls back to builtin_summary_params. Extracts
/// string values for each declared param from the JSON args and joins them
/// with spaces. Returns None if no params are declared or no values found.
pub fn tool_call_summary(tools: &[Tool], name: &str, args_json: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;

    // Check plugins first, then builtins
    let params: Vec<&str> = if let Some(tool) = tools.iter().find(|t| t.name == name) {
        tool.summary_params.iter().map(|s| s.as_str()).collect()
    } else {
        builtin_summary_params(name).to_vec()
    };

    if params.is_empty() {
        return None;
    }

    let parts: Vec<String> = params
        .iter()
        .filter_map(|p| args[*p].as_str().map(String::from))
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Shared HTTP client for URL fetching (agent_tools, coding_tools).
///
/// Reuses a single connection pool across all fetch_url calls within a process.
/// Per-request timeouts can be set via `RequestBuilder::timeout()`.
pub(crate) fn http_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build HTTP client")
    })
}

/// Fetch a URL with streaming size limit and timeout.
///
/// Validates the URL scheme (http/https only), checks `Content-Length` for
/// early rejection, then reads the body in chunks up to `max_bytes`.
/// Returns the body as a UTF-8 string (lossy). Responses exceeding
/// `max_bytes` are truncated with a notice appended.
pub(crate) async fn fetch_url_with_limit(
    url: &str,
    max_bytes: usize,
    timeout_secs: u64,
) -> std::io::Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("URL must start with http:// or https://, got: {}", url),
        ));
    }

    let response = http_client()
        .get(url)
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| std::io::Error::other(format!("Request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        return Err(std::io::Error::other(format!(
            "HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown"),
        )));
    }

    // Early reject if Content-Length exceeds limit
    if let Some(content_length) = response.content_length()
        && content_length as usize > max_bytes
    {
        return Err(std::io::Error::other(format!(
            "Response too large: Content-Length {} exceeds limit of {} bytes",
            content_length, max_bytes,
        )));
    }

    // Stream body in chunks up to max_bytes
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut buf = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut truncated = false;

    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| std::io::Error::other(format!("Failed to read response: {}", e)))?;
        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        if chunk.len() > remaining {
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }

    let body = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        Ok(format!(
            "{}\n\n[Truncated: response exceeded limit of {} bytes]",
            body, max_bytes,
        ))
    } else {
        Ok(body)
    }
}

// Tests for Tool struct are in plugins.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_get_tool_metadata_from_plugin() {
        let tools = vec![Tool {
            name: "custom_flow".to_string(),
            description: "A custom flow control tool".to_string(),
            parameters: serde_json::json!({}),
            path: PathBuf::from("/bin/custom"),
            hooks: vec![],
            metadata: ToolMetadata {
                parallel: false,
                flow_control: true,
                ends_turn: true,
            },
            summary_params: vec![],
        }];

        let meta = get_tool_metadata(&tools, "custom_flow");
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(meta.ends_turn);
    }

    #[test]
    fn test_get_tool_metadata_fallback_to_builtin() {
        let tools: Vec<Tool> = vec![]; // Empty plugin list

        // Should fall back to builtin metadata
        let agent_meta = get_tool_metadata(&tools, CALL_AGENT_TOOL_NAME);
        assert!(agent_meta.flow_control);
        assert!(!agent_meta.ends_turn);

        let user_meta = get_tool_metadata(&tools, CALL_USER_TOOL_NAME);
        assert!(user_meta.flow_control);
        assert!(user_meta.ends_turn);
    }

    #[test]
    fn test_get_tool_metadata_unknown_tool() {
        let tools: Vec<Tool> = vec![];

        // Unknown tool should get default metadata
        let meta = get_tool_metadata(&tools, "unknown_tool");
        assert!(meta.parallel);
        assert!(!meta.flow_control);
        assert!(!meta.ends_turn);
    }

    #[test]
    fn test_builtin_tool_names_includes_all_registries() {
        let names = builtin_tool_names();

        // Should include tools from all four registries
        assert!(names.contains(&"update_reflection")); // core builtin
        assert!(names.contains(&"model_info")); // core builtin
        assert!(names.contains(&"file_head")); // file tool
        assert!(names.contains(&"spawn_agent")); // agent tool
        assert!(names.contains(&"shell_exec")); // coding tool
        assert!(names.contains(&"file_edit")); // coding tool

        // Should be the sum of all registries
        let expected_count = builtin::BUILTIN_TOOL_DEFS.len()
            + file_tools::FILE_TOOL_DEFS.len()
            + agent_tools::AGENT_TOOL_DEFS.len()
            + coding_tools::CODING_TOOL_DEFS.len();
        assert_eq!(names.len(), expected_count);
    }

    #[test]
    fn test_builtin_tool_names_no_duplicates() {
        let names = builtin_tool_names();
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(name), "duplicate built-in tool name: {}", name);
        }
    }

    #[test]
    fn test_tool_call_summary_with_plugin() {
        let tools = vec![Tool {
            name: "my_plugin".to_string(),
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            path: PathBuf::from("/bin/test"),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec!["path".to_string(), "pattern".to_string()],
        }];

        let summary = tool_call_summary(
            &tools,
            "my_plugin",
            r#"{"path": "src/main.rs", "pattern": "TODO"}"#,
        );
        assert_eq!(summary, Some("src/main.rs TODO".to_string()));
    }

    #[test]
    fn test_tool_call_summary_builtin_fallback() {
        let tools: Vec<Tool> = vec![];

        // shell_exec has summary_params: &["command"]
        let summary = tool_call_summary(&tools, "shell_exec", r#"{"command": "cargo test"}"#);
        assert_eq!(summary, Some("cargo test".to_string()));
    }

    #[test]
    fn test_tool_call_summary_no_params() {
        let tools: Vec<Tool> = vec![];

        // update_reflection has summary_params: &[]
        let summary = tool_call_summary(
            &tools,
            "update_reflection",
            r#"{"content": "some reflection"}"#,
        );
        assert_eq!(summary, None);
    }

    #[test]
    fn test_tool_call_summary_missing_values() {
        let tools: Vec<Tool> = vec![];

        // file_head has summary_params: &["path"], but no path in args
        let summary = tool_call_summary(&tools, "file_head", r#"{"cache_id": "abc123"}"#);
        assert_eq!(summary, None);
    }
}
