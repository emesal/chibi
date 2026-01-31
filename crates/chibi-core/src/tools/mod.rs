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
// Other tool name constants are used internally via execute_builtin_tool and check_recurse_signal

// Re-export built-in tool API format functions
pub use builtin::{
    call_agent_tool_to_api_format, call_user_tool_to_api_format, goals_tool_to_api_format,
    reflection_tool_to_api_format, send_message_tool_to_api_format, todos_tool_to_api_format,
};

// Re-export built-in tool execution functions
pub use builtin::{check_recurse_signal, execute_builtin_tool};

// Re-export file tool API format functions
pub use file_tools::{
    cache_list_tool_to_api_format, file_grep_tool_to_api_format, file_head_tool_to_api_format,
    file_lines_tool_to_api_format, file_tail_tool_to_api_format,
};

// Re-export file tool execution and utilities
pub use file_tools::{execute_file_tool, is_file_tool};

/// Represents a tool that can be called by the LLM
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub path: PathBuf,
    pub hooks: Vec<HookPoint>,
}

// Tests for Tool struct are in plugins.rs
