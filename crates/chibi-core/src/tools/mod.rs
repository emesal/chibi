//! Tools and plugins module.
//!
//! This module provides the tool system that extends chibi's capabilities:
//! - Plugin loading and execution from the plugins directory
//! - Built-in tools for reflection, todos, goals, and messaging
//! - File tools for examining cached tool outputs
//! - Hook system for plugin lifecycle events

mod builtin;
pub mod file_tools;
mod hooks;
mod plugins;

use std::path::PathBuf;

pub use hooks::HookPoint;

// Re-export hook execution
pub use hooks::execute_hook;

// Re-export plugin functions
pub use plugins::{execute_tool, find_tool, load_tools, tools_to_api_format};

// Re-export built-in tool constants (used by api module)
pub use builtin::{CALL_AGENT_TOOL_NAME, CALL_USER_TOOL_NAME};
pub use builtin::{REFLECTION_TOOL_NAME, SEND_MESSAGE_TOOL_NAME};

// Re-export handoff types for control flow
pub use builtin::{Handoff, HandoffTarget};

// Re-export built-in tool API format functions (legacy wrappers)
pub use builtin::{
    call_agent_tool_to_api_format, call_user_tool_to_api_format, goals_tool_to_api_format,
    reflection_tool_to_api_format, send_message_tool_to_api_format, todos_tool_to_api_format,
};

// Re-export registry-based tool generation
pub use builtin::{all_builtin_tools_to_api_format, builtin_tools_to_api_format};

// Re-export built-in tool execution functions
pub use builtin::execute_builtin_tool;

// Re-export tool metadata functions
pub use builtin::builtin_tool_metadata;

// Re-export file tool API format functions
pub use file_tools::{
    cache_list_tool_to_api_format, file_grep_tool_to_api_format, file_head_tool_to_api_format,
    file_lines_tool_to_api_format, file_tail_tool_to_api_format,
};

// Re-export file tool execution and utilities
pub use file_tools::{execute_file_tool, is_file_tool};

/// Metadata for tool behavior in the agentic loop
#[derive(Debug, Clone, Default)]
pub struct ToolMetadata {
    /// Can this tool run in parallel with others? (default: true)
    /// NOTE: Parallel execution not yet implemented - see #101
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
}
