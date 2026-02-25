//! Built-in tools: compatibility shim delegating to memory and flow groups.
//!
//! This module is a transitional facade. It will be deleted in task 8 once all
//! callers have been updated to use the new per-group modules directly.

use crate::config::ResolvedConfig;
use crate::state::AppState;
use std::io;

pub use super::BuiltinToolDef;

impl BuiltinToolDef {
    /// Convert this tool definition to API format
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

/// Look up summary_params for a built-in tool by name.
///
/// Searches all registries (memory, flow, file, coding, vfs). Returns
/// an empty slice if the tool is not found.
pub fn builtin_summary_params(name: &str) -> &'static [&'static str] {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(super::flow::FLOW_TOOL_DEFS.iter())
        .chain(super::file_tools::FILE_TOOL_DEFS.iter())
        .chain(super::coding_tools::CODING_TOOL_DEFS.iter())
        .chain(super::vfs_tools::VFS_TOOL_DEFS.iter())
        .find(|def| def.name == name)
        .map(|def| def.summary_params)
        .unwrap_or(&[])
}

/// Check if a tool name is a core builtin tool (memory or flow group).
pub fn is_builtin_tool(name: &str) -> bool {
    super::memory::is_memory_tool(name) || super::flow::is_flow_tool(name)
}

/// Look up a specific builtin tool definition by name.
///
/// Searches memory and flow registries. Returns None if not found.
pub fn get_builtin_tool_def(name: &str) -> Option<&'static BuiltinToolDef> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(super::flow::FLOW_TOOL_DEFS.iter())
        .find(|def| def.name == name)
}

/// Convert all built-in tools to API format (memory + flow groups).
pub fn all_builtin_tools_to_api_format() -> Vec<serde_json::Value> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(super::flow::FLOW_TOOL_DEFS.iter())
        .map(|def| def.to_api_format())
        .collect()
}

/// Convert built-in tools to API format, optionally excluding the reflection tool.
pub fn builtin_tools_to_api_format(include_reflection: bool) -> Vec<serde_json::Value> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .filter(|def| include_reflection || def.name != super::memory::REFLECTION_TOOL_NAME)
        .chain(super::flow::FLOW_TOOL_DEFS.iter())
        .map(|def| def.to_api_format())
        .collect()
}

/// Get metadata for builtin tools. Delegates to flow_tool_metadata.
pub fn builtin_tool_metadata(name: &str) -> super::ToolMetadata {
    super::flow::flow_tool_metadata(name)
}

/// Execute a built-in tool by name.
///
/// Returns `Some(result)` if handled, `None` if not a built-in tool.
///
/// Note: When called from api.rs, send_message is handled separately to support
/// hook integration. This function provides a basic send_message implementation
/// for CLI usage (chibi.rs).
pub fn execute_builtin_tool(
    app: &AppState,
    context_name: &str,
    tool_name: &str,
    args: &serde_json::Value,
    resolved_config: Option<&ResolvedConfig>,
) -> Option<io::Result<String>> {
    // Memory tools
    if let Some(result) = super::memory::execute_memory_tool(app, context_name, tool_name, args, resolved_config) {
        return Some(result);
    }
    // send_message (flow tool with sync execution path)
    if tool_name == super::flow::SEND_MESSAGE_TOOL_NAME {
        let to = args.get("to").and_then(|v| v.as_str())?;
        let content = args.get("content").and_then(|v| v.as_str())?;
        return Some(
            app.send_inbox_message_from(context_name, to, content)
                .map(|_| format!("Message sent to '{}'", to)),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_tool_api(name: &str) -> serde_json::Value {
        get_builtin_tool_def(name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Tool API format tests (exercising delegation to memory + flow) ===

    #[test]
    fn test_send_message_tool_api_format() {
        let tool = get_tool_api("send_message");
        assert_eq!(tool["function"]["name"], "send_message");
        let required = tool["function"]["parameters"]["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("to")));
        assert!(required.contains(&serde_json::json!("content")));
    }

    #[test]
    fn test_call_agent_tool_api_format() {
        let tool = get_tool_api("call_agent");
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "call_agent");
    }

    #[test]
    fn test_call_user_tool_api_format() {
        let tool = get_tool_api("call_user");
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_model_info_tool_api_format() {
        let tool = get_tool_api("model_info");
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("metadata")
        );
    }

    // === Metadata ===

    #[test]
    fn test_builtin_tool_metadata_call_agent() {
        let meta = builtin_tool_metadata("call_agent");
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(!meta.ends_turn);
    }

    #[test]
    fn test_builtin_tool_metadata_call_user() {
        let meta = builtin_tool_metadata("call_user");
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(meta.ends_turn);
    }

    #[test]
    fn test_builtin_tool_metadata_spawn_agent() {
        let meta = builtin_tool_metadata("spawn_agent");
        assert!(!meta.parallel);
        assert!(!meta.flow_control);
        assert!(!meta.ends_turn);
    }

    #[test]
    fn test_builtin_tool_metadata_other() {
        let meta = builtin_tool_metadata("update_todos");
        assert!(meta.parallel);
        assert!(!meta.flow_control);
        assert!(!meta.ends_turn);
    }

    // === Registry ===

    #[test]
    fn test_all_builtin_tools_to_api_format_includes_memory_and_flow() {
        // memory (4) + flow (6) = 10
        let tools = all_builtin_tools_to_api_format();
        assert_eq!(tools.len(), 10);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].as_str().is_some());
        }
    }

    #[test]
    fn test_builtin_tools_to_api_format_with_reflection() {
        let tools = builtin_tools_to_api_format(true);
        assert_eq!(tools.len(), 10);
    }

    #[test]
    fn test_builtin_tools_to_api_format_without_reflection() {
        let tools = builtin_tools_to_api_format(false);
        assert_eq!(tools.len(), 9);
        for tool in &tools {
            assert_ne!(tool["function"]["name"], "update_reflection");
        }
    }

    #[test]
    fn test_is_builtin_tool() {
        assert!(is_builtin_tool("update_reflection"));
        assert!(is_builtin_tool("call_agent"));
        assert!(is_builtin_tool("send_message"));
        assert!(is_builtin_tool("spawn_agent"));
        assert!(!is_builtin_tool("shell_exec"));
        assert!(!is_builtin_tool("file_head"));
    }

    #[test]
    fn test_get_builtin_tool_def_finds_all_groups() {
        assert!(get_builtin_tool_def("update_reflection").is_some());
        assert!(get_builtin_tool_def("read_context").is_some());
        assert!(get_builtin_tool_def("call_agent").is_some());
        assert!(get_builtin_tool_def("spawn_agent").is_some());
        assert!(get_builtin_tool_def("nonexistent_tool").is_none());
    }
}
