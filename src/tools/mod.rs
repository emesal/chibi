//! Tools and plugins module.
//!
//! This module provides the tool system that extends chibi's capabilities:
//! - Plugin loading and execution from the plugins directory
//! - Built-in tools for reflection, todos, goals, and messaging
//! - Hook system for plugin lifecycle events

mod builtin;
mod hooks;
mod plugins;

use std::path::PathBuf;

pub use hooks::HookPoint;

// Re-export hook execution
pub use hooks::execute_hook;

// Re-export plugin functions
pub use plugins::{execute_tool, find_tool, load_tools, tools_to_api_format};

// Re-export built-in tool constants (used by api module)
pub use builtin::{REFLECTION_TOOL_NAME, SEND_MESSAGE_TOOL_NAME};
// Other tool name constants are used internally via execute_builtin_tool and check_recurse_signal

// Re-export built-in tool API format functions
pub use builtin::{
    goals_tool_to_api_format, reflection_tool_to_api_format, send_message_tool_to_api_format,
    todos_tool_to_api_format,
};

// Re-export built-in tool execution functions
pub use builtin::{check_recurse_signal, execute_builtin_tool};

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
