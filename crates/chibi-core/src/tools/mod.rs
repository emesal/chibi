//! Tools and plugins module.
//!
//! This module provides the tool system that extends chibi's capabilities:
//! - Plugin loading and execution from the plugins directory
//! - Built-in tools for reflection, todos, goals, and messaging
//! - File tools for examining cached tool outputs
//! - Hook system for plugin lifecycle events

pub mod agent_tools;
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
pub use builtin::{MODEL_INFO_TOOL_NAME, REFLECTION_TOOL_NAME, SEND_MESSAGE_TOOL_NAME};

// Re-export handoff types for control flow
pub use builtin::{Handoff, HandoffTarget};

// Re-export builtin tool registry lookup
pub use builtin::get_builtin_tool_def;

// Re-export registry-based tool generation
pub use builtin::{all_builtin_tools_to_api_format, builtin_tools_to_api_format};

// Re-export built-in tool execution functions
pub use builtin::execute_builtin_tool;

// Re-export tool metadata functions
pub use builtin::builtin_tool_metadata;

// Re-export file tool registry functions
pub use file_tools::{all_file_tools_to_api_format, get_file_tool_def};

// Re-export file tool execution and utilities
pub use file_tools::{execute_file_tool, is_file_tool};

// Re-export file write tool names for permission gating
pub use file_tools::{PATCH_FILE_TOOL_NAME, WRITE_FILE_TOOL_NAME};

// Re-export agent tool registry functions
pub use agent_tools::{all_agent_tools_to_api_format, get_agent_tool_def};

// Re-export agent tool execution and utilities
pub use agent_tools::{execute_agent_tool, is_agent_tool, spawn_agent};

// Re-export agent tool types and constants
pub use agent_tools::{RETRIEVE_CONTENT_TOOL_NAME, SPAWN_AGENT_TOOL_NAME, SpawnOptions};

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

    #[test]
    fn test_builtin_tool_names_includes_all_registries() {
        let names = builtin_tool_names();

        // Should include tools from all three registries
        assert!(names.contains(&"update_reflection")); // core builtin
        assert!(names.contains(&"model_info")); // core builtin
        assert!(names.contains(&"file_head")); // file tool
        assert!(names.contains(&"spawn_agent")); // agent tool

        // Should be the sum of all registries
        let expected_count = builtin::BUILTIN_TOOL_DEFS.len()
            + file_tools::FILE_TOOL_DEFS.len()
            + agent_tools::AGENT_TOOL_DEFS.len();
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
}
