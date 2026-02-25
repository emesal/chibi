//! Tools and plugins module.
//!
//! This module provides the extensible tool system:
//! - Plugin loading and execution from the plugins directory
//! - MCP bridge client for tools from remote MCP servers
//! - Built-in tools organised by permission/capability group:
//!   - `memory`: reflection, todos, goals, read_context
//!   - `fs_read`: read-only file and directory access
//!   - `fs_write`: file write and edit (triggers PreFileWrite hooks)
//!   - `shell`: OS command execution (triggers PreShellExec hooks)
//!   - `network`: outbound HTTP (triggers PreFetchUrl hooks)
//!   - `index`: codebase index management
//!   - `flow`: control flow, spawning, coordination, model introspection
//!   - `vfs_tools`: virtual filesystem operations
//! - URL and file path security policies
//! - Hook system for plugin lifecycle events

mod flow;
mod fs_read;
mod fs_write;
mod hooks;
mod index;
mod network;
mod shell;
mod memory;
pub mod mcp;
pub(crate) mod paths;
mod plugins;
pub mod security;
pub mod vfs_tools;

use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub use hooks::HookPoint;

/// Property definition for a tool parameter
pub struct ToolPropertyDef {
    pub name: &'static str,
    pub prop_type: &'static str,
    pub description: &'static str,
    /// Optional default value (for integer defaults only, as used by file tools)
    pub default: Option<i64>,
}

/// Built-in tool definition for declarative registry
pub struct BuiltinToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub properties: &'static [ToolPropertyDef],
    pub required: &'static [&'static str],
    /// Parameter names whose values should appear in tool-call notices.
    pub summary_params: &'static [&'static str],
}

impl BuiltinToolDef {
    /// Convert this tool definition to API format.
    pub fn to_api_format(&self) -> serde_json::Value {
        let mut props = serde_json::Map::new();
        for prop in self.properties {
            let mut prop_obj = serde_json::json!({
                "type": prop.prop_type,
                "description": prop.description,
            });
            if let Some(default) = prop.default {
                prop_obj["default"] = serde_json::json!(default);
            }
            props.insert(prop.name.to_string(), prop_obj);
        }

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": self.required,
                }
            }
        })
    }
}

/// Extract a required string parameter from tool args.
///
/// Shared helper for all tool modules that parse JSON arguments.
pub fn require_str_param(args: &serde_json::Value, name: &str) -> io::Result<String> {
    use crate::json_ext::JsonExt;
    args.get_str(name).map(String::from).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Missing '{}' parameter", name),
        )
    })
}

// Re-export hook execution
pub use hooks::execute_hook;

// Re-export plugin functions
pub use plugins::{execute_tool, find_tool, load_tools, tools_to_api_format};

// Re-export memory tool constants and functions
pub use memory::{
    GOALS_TOOL_NAME, MEMORY_TOOL_DEFS, READ_CONTEXT_TOOL_NAME, REFLECTION_TOOL_NAME,
    TODOS_TOOL_NAME, all_memory_tools_to_api_format, execute_memory_tool, is_memory_tool,
};

// Re-export flow tool constants, types and functions
pub use flow::{
    CALL_AGENT_TOOL_NAME, CALL_USER_TOOL_NAME, FLOW_TOOL_DEFS, MODEL_INFO_TOOL_NAME,
    SEND_MESSAGE_TOOL_NAME, SPAWN_AGENT_TOOL_NAME, SUMMARIZE_CONTENT_TOOL_NAME,
    Handoff, HandoffTarget, SpawnOptions,
    all_flow_tools_to_api_format, execute_flow_tool, flow_tool_metadata, is_flow_tool, is_url,
    spawn_agent,
};

// Re-export fs_read tool registry functions and execution
pub use fs_read::{
    FS_READ_TOOL_DEFS, FILE_HEAD_TOOL_NAME, FILE_TAIL_TOOL_NAME, FILE_LINES_TOOL_NAME,
    FILE_GREP_TOOL_NAME, DIR_LIST_TOOL_NAME, GLOB_FILES_TOOL_NAME, GREP_FILES_TOOL_NAME,
    all_fs_read_tools_to_api_format, execute_fs_read_tool, is_fs_read_tool,
};

// Re-export fs_write tool registry functions and execution
pub use fs_write::{
    FS_WRITE_TOOL_DEFS, FILE_EDIT_TOOL_NAME, WRITE_FILE_TOOL_NAME,
    all_fs_write_tools_to_api_format, execute_fs_write_tool, execute_write_file, is_fs_write_tool,
};

// Re-export shell tool registry functions and execution
pub use shell::{
    SHELL_TOOL_DEFS, SHELL_EXEC_TOOL_NAME,
    all_shell_tools_to_api_format, execute_shell_tool, is_shell_tool,
};

// Re-export network tool registry functions and execution
pub use network::{
    NETWORK_TOOL_DEFS, FETCH_URL_TOOL_NAME,
    all_network_tools_to_api_format, execute_network_tool, is_network_tool,
};

// Re-export index tool registry functions and execution
pub use index::{
    INDEX_TOOL_DEFS, INDEX_UPDATE_TOOL_NAME, INDEX_QUERY_TOOL_NAME, INDEX_STATUS_TOOL_NAME,
    all_index_tools_to_api_format, execute_index_tool, is_index_tool,
};

// Re-export VFS tool registry functions and execution
pub use vfs_tools::{all_vfs_tools_to_api_format, execute_vfs_tool, is_vfs_tool};

// Re-export security utilities
pub use security::{
    FilePathAccess, UrlAction, UrlCategory, UrlPolicy, UrlRule, UrlSafety, classify_file_path,
    classify_url, ensure_project_root_allowed, evaluate_url_policy, validate_file_path,
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

/// Collect names of all built-in tools across all groups.
///
/// Returns a flat list from: memory, fs_read, fs_write, shell, network, index, flow, vfs.
/// Add new groups here when introduced.
pub fn builtin_tool_names() -> Vec<&'static str> {
    memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(fs_read::FS_READ_TOOL_DEFS.iter())
        .chain(fs_write::FS_WRITE_TOOL_DEFS.iter())
        .chain(shell::SHELL_TOOL_DEFS.iter())
        .chain(network::NETWORK_TOOL_DEFS.iter())
        .chain(index::INDEX_TOOL_DEFS.iter())
        .chain(flow::FLOW_TOOL_DEFS.iter())
        .chain(vfs_tools::VFS_TOOL_DEFS.iter())
        .map(|def| def.name)
        .collect()
}

/// Get metadata for any tool (plugin or builtin).
///
/// Checks plugins first, then delegates to flow_tool_metadata for known flow tools.
pub fn get_tool_metadata(tools: &[Tool], name: &str) -> ToolMetadata {
    if let Some(tool) = tools.iter().find(|t| t.name == name) {
        return tool.metadata.clone();
    }
    flow_tool_metadata(name)
}

/// Look up summary_params for a built-in tool by name.
///
/// Searches all tool groups. Returns an empty slice if the tool is not found.
fn builtin_summary_params(name: &str) -> &'static [&'static str] {
    memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(flow::FLOW_TOOL_DEFS.iter())
        .chain(fs_read::FS_READ_TOOL_DEFS.iter())
        .chain(fs_write::FS_WRITE_TOOL_DEFS.iter())
        .chain(shell::SHELL_TOOL_DEFS.iter())
        .chain(network::NETWORK_TOOL_DEFS.iter())
        .chain(index::INDEX_TOOL_DEFS.iter())
        .chain(vfs_tools::VFS_TOOL_DEFS.iter())
        .find(|def| def.name == name)
        .map(|def| def.summary_params)
        .unwrap_or(&[])
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

/// Default timeout for hook and plugin execution (30 seconds).
///
/// Prevents hung plugins from blocking the entire application.
pub(crate) const PLUGIN_TIMEOUT_SECS: u64 = 30;

/// Wait for a child process with a timeout, killing it if exceeded.
///
/// Returns the process output on success, or a timeout error if the process
/// does not complete within the given duration. On timeout, the child process
/// is killed via its PID to prevent zombie accumulation.
pub(crate) fn wait_with_timeout(
    child: std::process::Child,
    timeout: std::time::Duration,
    context: &str,
) -> std::io::Result<std::process::Output> {
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = child.wait_with_output();
        // Send the result back; if the receiver is gone (timeout), the child
        // is already killed and this thread exits cleanly.
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            // Timeout — kill the child by PID. The wait thread will unblock
            // once the process exits and clean up naturally.
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
            Err(std::io::Error::other(format!(
                "Timed out after {}s: {}",
                timeout.as_secs(),
                context,
            )))
        }
    }
}

#[cfg(test)]
pub(super) mod test_helpers {
    use std::path::{Path, PathBuf};

    /// Helper to create a test script and make it executable.
    /// Uses sync_all to ensure the file is fully written before execution.
    #[cfg(unix)]
    pub(super) fn create_test_script(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let script_path = dir.join(name);

        {
            let mut file = std::fs::File::create(&script_path).unwrap();
            file.write_all(content).unwrap();
            file.sync_all().unwrap();
        }

        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        script_path
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

        // Should include tools from all five registries
        assert!(names.contains(&"update_reflection")); // core builtin
        assert!(names.contains(&"model_info")); // core builtin
        assert!(names.contains(&"file_head")); // file tool
        assert!(names.contains(&"spawn_agent")); // agent tool
        assert!(names.contains(&"shell_exec")); // coding tool
        assert!(names.contains(&"file_edit")); // coding tool
        assert!(names.contains(&"vfs_list")); // vfs tool

        // Should be: memory + flow + fs_read + fs_write + shell + network + index + vfs
        let expected_count = memory::MEMORY_TOOL_DEFS.len()
            + flow::FLOW_TOOL_DEFS.len()
            + fs_read::FS_READ_TOOL_DEFS.len()
            + fs_write::FS_WRITE_TOOL_DEFS.len()
            + shell::SHELL_TOOL_DEFS.len()
            + network::NETWORK_TOOL_DEFS.len()
            + index::INDEX_TOOL_DEFS.len()
            + vfs_tools::VFS_TOOL_DEFS.len();
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
