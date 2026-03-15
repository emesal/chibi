//! Tools and plugins module.
//!
//! This module provides the extensible tool system:
//! - Plugin loading and execution from the plugins directory
//! - MCP bridge client for tools from remote MCP servers
//! - Built-in tools organised by permission/capability group:
//!   - `memory`: reflection, goals, flock_list, read_context
//!   - `fs_read`: read-only file and directory access
//!   - `fs_write`: file write and edit (triggers PreFileWrite hooks)
//!   - `shell`: OS command execution (triggers PreShellExec hooks)
//!   - `network`: outbound HTTP (triggers PreFetchUrl hooks)
//!   - `index`: codebase index management
//!   - `flow`: control flow, spawning, coordination, model introspection
//!   - `vfs_tools`: virtual filesystem operations
//! - URL and file path security policies
//! - Hook system for plugin lifecycle events

mod eval;
mod flow;
mod fs_read;
mod fs_write;
mod hooks;
mod index;
pub mod mcp;
mod memory;
mod network;
pub(crate) mod paths;
mod plugins;
pub mod registry;
pub mod security;
mod shell;
pub mod synthesised;
pub mod vfs_tools;

use std::io::{self, ErrorKind};

pub use hooks::HookPoint;
pub use registry::{ToolCall, ToolCallContext, ToolCategory, ToolHandler, ToolImpl, ToolRegistry};

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
    /// Convert this tool definition's parameters to JSON Schema format.
    ///
    /// Used by `Tool::from_builtin_def` to populate `Tool.parameters`.
    pub fn to_json_schema(&self) -> serde_json::Value {
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
            "type": "object",
            "properties": props,
            "required": self.required,
        })
    }

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

// Re-export hook execution and docs generation
#[cfg(feature = "synthesised-tools")]
pub use hooks::generate_hooks_markdown;
pub use hooks::{TeinHookContext, execute_hook};

// Re-export plugin functions
pub use plugins::{execute_tool, execute_tool_by_path, load_tools};

// Re-export memory tool constants and functions
pub use memory::{
    GOALS_TOOL_NAME, MEMORY_TOOL_DEFS, READ_CONTEXT_TOOL_NAME, REFLECTION_TOOL_NAME,
    execute_memory_tool, register_memory_tools,
};

// Re-export flow tool constants, types and functions
pub use flow::{
    CALL_AGENT_TOOL_NAME, CALL_USER_TOOL_NAME, FLOW_TOOL_DEFS, Handoff, HandoffTarget,
    MODEL_INFO_TOOL_NAME, SEND_MESSAGE_TOOL_NAME, SPAWN_AGENT_TOOL_NAME,
    SUMMARIZE_CONTENT_TOOL_NAME, SpawnOptions, execute_flow_tool, flow_tool_metadata, is_url,
    register_flow_tools, spawn_agent, spawn_agent_preset_description,
};

// Re-export fs_read tool registry functions and execution
pub use fs_read::{
    DIR_LIST_TOOL_NAME, FILE_GREP_TOOL_NAME, FILE_HEAD_TOOL_NAME, FILE_LINES_TOOL_NAME,
    FILE_TAIL_TOOL_NAME, FS_READ_TOOL_DEFS, GLOB_FILES_TOOL_NAME, GREP_FILES_TOOL_NAME,
    execute_fs_read_tool, register_fs_read_tools,
};

// Bridge for blocking async VFS calls from synchronous contexts.
pub(crate) use fs_read::vfs_block_on;

// Re-export fs_write tool registry functions and execution
pub use fs_write::{
    FILE_EDIT_TOOL_NAME, FS_WRITE_TOOL_DEFS, WRITE_FILE_TOOL_NAME, execute_fs_write_tool,
    execute_write_file, register_fs_write_tools,
};

// Re-export shell tool registry functions and execution
pub use shell::{SHELL_EXEC_TOOL_NAME, SHELL_TOOL_DEFS, execute_shell_tool, register_shell_tools};

// Re-export network tool registry functions and execution
pub use network::{
    FETCH_URL_TOOL_NAME, NETWORK_TOOL_DEFS, execute_network_tool, register_network_tools,
};

// Re-export index tool registry functions and execution
pub use index::{
    INDEX_QUERY_TOOL_NAME, INDEX_STATUS_TOOL_NAME, INDEX_TOOL_DEFS, INDEX_UPDATE_TOOL_NAME,
    execute_index_tool, register_index_tools,
};

// Re-export VFS tool registry functions and execution
pub use vfs_tools::{execute_vfs_tool, register_vfs_tools};

// Re-export eval tool constants and registration
pub use eval::{EVAL_TOOL_DEFS, SCHEME_EVAL_TOOL_NAME, evict_eval_context, register_eval_tools};

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

/// Represents a tool that can be called by the LLM.
///
/// `r#impl` carries typed dispatch info; `category` enables filtering and
/// permission routing without per-tool predicate functions.
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub hooks: Vec<HookPoint>,
    pub metadata: ToolMetadata,
    /// Parameter names whose values should appear in tool-call notices.
    pub summary_params: Vec<String>,
    /// Typed dispatch discriminant.
    pub r#impl: registry::ToolImpl,
    /// Category for filtering and permission routing.
    pub category: registry::ToolCategory,
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool")
            .field("name", &self.name)
            .field("category", &self.category)
            .field("impl", &"<handler>")
            .finish()
    }
}

impl Tool {
    /// Serialise to OpenAI-style function definition for the LLM API.
    ///
    /// Single source of truth — replaces the per-module `*_to_api_format()` helpers
    /// once the send.rs tool-building loop is migrated to iterate over the registry.
    pub fn to_api_format(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }

    /// Returns `true` if this tool can participate in hook dispatch.
    ///
    /// Plugin tools always register hooks via their `--schema` output.
    /// Synthesised tools participate only when they called `register-hook`.
    /// All other categories (builtin, MCP) never register hooks.
    pub fn is_hook_eligible(&self) -> bool {
        use registry::ToolCategory;
        self.category == ToolCategory::Plugin
            || (self.category == ToolCategory::Synthesised && !self.hooks.is_empty())
    }

    /// Construct a Tool from a `BuiltinToolDef`, a shared handler, and a category.
    ///
    /// Reduces boilerplate in the `register_*_tools()` functions across all modules.
    pub fn from_builtin_def(
        def: &BuiltinToolDef,
        handler: registry::ToolHandler,
        category: registry::ToolCategory,
    ) -> Self {
        Self {
            name: def.name.to_string(),
            description: def.description.to_string(),
            parameters: def.to_json_schema(),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: def.summary_params.iter().map(|s| s.to_string()).collect(),
            r#impl: registry::ToolImpl::Builtin(handler),
            category,
        }
    }
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
        .chain(eval::EVAL_TOOL_DEFS.iter())
        .map(|def| def.name)
        .collect()
}

/// Get metadata for any tool (builtin, plugin, or MCP).
///
/// Looks up by name in the registry; falls back to `flow_tool_metadata` for
/// unregistered flow tools (e.g. call_agent, which is retained for the fallback
/// mechanism but not registered as a callable tool).
pub fn get_tool_metadata(registry: &ToolRegistry, name: &str) -> ToolMetadata {
    if let Some(tool) = registry.get(name) {
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
        .chain(eval::EVAL_TOOL_DEFS.iter())
        .find(|def| def.name == name)
        .map(|def| def.summary_params)
        .unwrap_or(&[])
}

/// Build a concise summary string from a tool's declared summary_params and actual arguments.
///
/// Looks up summary_params from the registry; falls back to builtin_summary_params for
/// tools not yet in the registry. Extracts string values for each declared param from
/// the JSON args and joins them with spaces. Returns None if no params are declared or
/// no values found.
pub fn tool_call_summary(registry: &ToolRegistry, name: &str, args_json: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;

    // Check registry first, then builtins
    let params: Vec<&str> = if let Some(tool) = registry.get(name) {
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
        let mut reg = ToolRegistry::new();
        reg.register(Tool {
            name: "custom_flow".to_string(),
            description: "A custom flow control tool".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![],
            metadata: ToolMetadata {
                parallel: false,
                flow_control: true,
                ends_turn: true,
            },
            summary_params: vec![],
            r#impl: registry::ToolImpl::Plugin(PathBuf::from("/bin/custom")),
            category: registry::ToolCategory::Plugin,
        });

        let meta = get_tool_metadata(&reg, "custom_flow");
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(meta.ends_turn);
    }

    #[test]
    fn test_get_tool_metadata_fallback_to_builtin() {
        let reg = ToolRegistry::new(); // Empty registry — falls back to flow_tool_metadata

        // Should fall back to builtin metadata
        let agent_meta = get_tool_metadata(&reg, CALL_AGENT_TOOL_NAME);
        assert!(agent_meta.flow_control);
        assert!(!agent_meta.ends_turn);

        let user_meta = get_tool_metadata(&reg, CALL_USER_TOOL_NAME);
        assert!(user_meta.flow_control);
        assert!(user_meta.ends_turn);
    }

    #[test]
    fn test_get_tool_metadata_unknown_tool() {
        let reg = ToolRegistry::new();

        // Unknown tool should get default metadata
        let meta = get_tool_metadata(&reg, "unknown_tool");
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

        // Should be: memory + flow + fs_read + fs_write + shell + network + index + vfs + eval
        let expected_count = memory::MEMORY_TOOL_DEFS.len()
            + flow::FLOW_TOOL_DEFS.len()
            + fs_read::FS_READ_TOOL_DEFS.len()
            + fs_write::FS_WRITE_TOOL_DEFS.len()
            + shell::SHELL_TOOL_DEFS.len()
            + network::NETWORK_TOOL_DEFS.len()
            + index::INDEX_TOOL_DEFS.len()
            + vfs_tools::VFS_TOOL_DEFS.len()
            + eval::EVAL_TOOL_DEFS.len();
        assert_eq!(names.len(), expected_count);
        assert!(names.contains(&"scheme_eval")); // eval tool
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
        let mut reg = ToolRegistry::new();
        reg.register(Tool {
            name: "my_plugin".to_string(),
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec!["path".to_string(), "pattern".to_string()],
            r#impl: registry::ToolImpl::Plugin(PathBuf::from("/bin/test")),
            category: registry::ToolCategory::Plugin,
        });

        let summary = tool_call_summary(
            &reg,
            "my_plugin",
            r#"{"path": "src/main.rs", "pattern": "TODO"}"#,
        );
        assert_eq!(summary, Some("src/main.rs TODO".to_string()));
    }

    #[test]
    fn test_tool_call_summary_builtin_fallback() {
        let reg = ToolRegistry::new();

        // shell_exec has summary_params: &["command"]
        let summary = tool_call_summary(&reg, "shell_exec", r#"{"command": "cargo test"}"#);
        assert_eq!(summary, Some("cargo test".to_string()));
    }

    #[test]
    fn test_tool_call_summary_no_params() {
        let reg = ToolRegistry::new();

        // update_reflection has summary_params: &[]
        let summary = tool_call_summary(
            &reg,
            "update_reflection",
            r#"{"content": "some reflection"}"#,
        );
        assert_eq!(summary, None);
    }

    #[test]
    fn test_tool_call_summary_missing_values() {
        let reg = ToolRegistry::new();

        // file_head has summary_params: &["path"], but no path in args
        let summary = tool_call_summary(&reg, "file_head", r#"{"cache_id": "abc123"}"#);
        assert_eq!(summary, None);
    }
}
