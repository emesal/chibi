//! Built-in tools provided by chibi.
//!
//! These tools are always available and handle core functionality:
//! - update_reflection: Persistent memory across all contexts
//! - update_todos: Per-context task tracking
//! - update_goals: Per-context goal tracking
//! - send_message: Inter-context messaging
//! - call_agent/call_user: Control flow handoff
//! - model_info: Model metadata lookup (async, dispatched in api/send.rs)

use crate::config::ResolvedConfig;
use crate::state::AppState;
use std::io;

// === Tool Name Constants ===

/// Name of the built-in send_message tool for inter-context messaging
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

/// Name of the built-in call_agent tool for control handoff
pub const CALL_AGENT_TOOL_NAME: &str = "call_agent";

/// Name of the built-in call_user tool for control handoff
pub const CALL_USER_TOOL_NAME: &str = "call_user";

/// Name of the built-in model_info tool for metadata lookup
pub const MODEL_INFO_TOOL_NAME: &str = "model_info";


// === Tool Definition Registry ===

pub use super::{BuiltinToolDef, ToolPropertyDef};

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

/// Built-in tool definitions: flow tools (call_agent, call_user, send_message, model_info, read_context).
///
/// Memory tools (reflection, todos, goals, read_context) have moved to `memory::MEMORY_TOOL_DEFS`.
pub static BUILTIN_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: SEND_MESSAGE_TOOL_NAME,
        description: "Send a message to another context's inbox. The message will be delivered to the target context and shown to them before their next prompt.",
        properties: &[
            ToolPropertyDef {
                name: "to",
                prop_type: "string",
                description: "Target context name",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "Message content",
                default: None,
            },
            ToolPropertyDef {
                name: "from",
                prop_type: "string",
                description: "Optional sender name (defaults to current context)",
                default: None,
            },
        ],
        required: &["to", "content"],
        summary_params: &["to"],
    },
    BuiltinToolDef {
        name: CALL_AGENT_TOOL_NAME,
        description: "Continue in a new turn before returning to the user. Use when you have more steps to complete.",
        properties: &[ToolPropertyDef {
            name: "prompt",
            prop_type: "string",
            description: "Focus for the next turn",
            default: None,
        }],
        required: &["prompt"],
        summary_params: &["prompt"],
    },
    BuiltinToolDef {
        name: CALL_USER_TOOL_NAME,
        description: "End your turn immediately and return control to the user.",
        properties: &[ToolPropertyDef {
            name: "message",
            prop_type: "string",
            description: "Final message to show the user.",
            default: None,
        }],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: MODEL_INFO_TOOL_NAME,
        description: "Look up metadata for a model: context window, max output tokens, pricing, capabilities, and parameter ranges. Use this to check model specifications before making recommendations or decisions about model selection.",
        properties: &[ToolPropertyDef {
            name: "model",
            prop_type: "string",
            description: "Model identifier (e.g. 'anthropic/claude-sonnet-4')",
            default: None,
        }],
        required: &["model"],
        summary_params: &["model"],
    },
];

/// Look up summary_params for a built-in tool by name.
///
/// Searches all built-in registries (memory, flow, file, coding, agent, VFS). Returns
/// an empty slice if the tool is not found.
pub fn builtin_summary_params(name: &str) -> &'static [&'static str] {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(BUILTIN_TOOL_DEFS.iter())
        .chain(super::file_tools::FILE_TOOL_DEFS.iter())
        .chain(super::coding_tools::CODING_TOOL_DEFS.iter())
        .chain(super::agent_tools::AGENT_TOOL_DEFS.iter())
        .chain(super::vfs_tools::VFS_TOOL_DEFS.iter())
        .find(|def| def.name == name)
        .map(|def| def.summary_params)
        .unwrap_or(&[])
}

/// Check if a tool name is a core builtin tool (memory or flow group).
pub fn is_builtin_tool(name: &str) -> bool {
    super::memory::is_memory_tool(name) || BUILTIN_TOOL_DEFS.iter().any(|def| def.name == name)
}

/// Look up a specific builtin tool definition by name.
///
/// Searches memory and flow registries. Returns None if not found.
pub fn get_builtin_tool_def(name: &str) -> Option<&'static BuiltinToolDef> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(BUILTIN_TOOL_DEFS.iter())
        .find(|def| def.name == name)
}

/// Convert all built-in tools to API format (memory + flow groups).
pub fn all_builtin_tools_to_api_format() -> Vec<serde_json::Value> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .chain(BUILTIN_TOOL_DEFS.iter())
        .map(|def| def.to_api_format())
        .collect()
}

/// Convert built-in tools to API format, optionally excluding the reflection tool.
pub fn builtin_tools_to_api_format(include_reflection: bool) -> Vec<serde_json::Value> {
    super::memory::MEMORY_TOOL_DEFS
        .iter()
        .filter(|def| include_reflection || def.name != super::memory::REFLECTION_TOOL_NAME)
        .chain(BUILTIN_TOOL_DEFS.iter())
        .map(|def| def.to_api_format())
        .collect()
}

use super::ToolMetadata;

/// Get metadata for builtin tools
pub fn builtin_tool_metadata(name: &str) -> ToolMetadata {
    match name {
        CALL_AGENT_TOOL_NAME => ToolMetadata {
            parallel: false,
            flow_control: true,
            ends_turn: false,
        },
        CALL_USER_TOOL_NAME => ToolMetadata {
            parallel: false,
            flow_control: true,
            ends_turn: true,
        },
        "shell_exec" | "spawn_agent" => ToolMetadata {
            parallel: false,
            flow_control: false,
            ends_turn: false,
        },
        _ => ToolMetadata::new(),
    }
}

// === Handoff Types ===

/// Target for control handoff after tool execution
#[derive(Debug, Clone)]
pub enum HandoffTarget {
    /// Continue with LLM processing
    Agent { prompt: String },
    /// Return control to user
    User { message: String },
}

impl Default for HandoffTarget {
    fn default() -> Self {
        Self::Agent {
            prompt: String::new(),
        }
    }
}

/// Tracks handoff decision during tool execution.
/// Last explicit call wins; falls back to configured default.
#[derive(Debug)]
pub struct Handoff {
    next: Option<HandoffTarget>,
    fallback: HandoffTarget,
}

impl Handoff {
    pub fn new(fallback: HandoffTarget) -> Self {
        Self {
            next: None,
            fallback,
        }
    }

    pub fn set_agent(&mut self, prompt: String) {
        self.next = Some(HandoffTarget::Agent { prompt });
    }

    pub fn set_user(&mut self, message: String) {
        self.next = Some(HandoffTarget::User { message });
    }

    /// Take the handoff decision, resetting to fallback for next use
    pub fn take(&mut self) -> HandoffTarget {
        self.next.take().unwrap_or_else(|| self.fallback.clone())
    }

    /// Override the fallback target (used by hooks)
    pub fn set_fallback(&mut self, target: HandoffTarget) {
        self.fallback = target;
    }

    /// Check if an explicit end-turn (call_user) has been requested
    pub fn ends_turn_requested(&self) -> bool {
        matches!(self.next, Some(HandoffTarget::User { .. }))
    }
}

// === Tool Execution ===

/// Execute a built-in tool by name.
/// Returns Some(result) if the tool exists and was executed, None if not a built-in tool.
///
/// Note: When called from api.rs, send_message is handled separately to support
/// hook integration. This function provides a basic send_message implementation
/// for CLI usage (main.rs).
pub fn execute_builtin_tool(
    app: &AppState,
    context_name: &str,
    tool_name: &str,
    args: &serde_json::Value,
    resolved_config: Option<&ResolvedConfig>,
) -> Option<io::Result<String>> {
    // Memory tools are handled by the memory module
    if let Some(result) = super::memory::execute_memory_tool(app, context_name, tool_name, args, resolved_config) {
        return Some(result);
    }
    match tool_name {
        SEND_MESSAGE_TOOL_NAME => {
            let to = args.get("to").and_then(|v| v.as_str())?;
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.send_inbox_message_from(context_name, to, content)
                    .map(|_| format!("Message sent to '{}'", to)),
            )
        }
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn get_tool_api(name: &str) -> serde_json::Value {
        get_builtin_tool_def(name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Flow Tool API Format Tests ===

    #[test]
    fn test_send_message_tool_api_format() {
        let tool = get_tool_api(SEND_MESSAGE_TOOL_NAME);
        assert_eq!(tool["function"]["name"], SEND_MESSAGE_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("message")
        );
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("to")));
        assert!(required.contains(&serde_json::json!("content")));
    }

    #[test]
    fn test_call_agent_tool_api_format() {
        let tool = get_tool_api(CALL_AGENT_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], CALL_AGENT_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("Continue")
        );
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("prompt"))
        );
    }

    #[test]
    fn test_call_user_tool_api_format() {
        let tool = get_tool_api(CALL_USER_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], CALL_USER_TOOL_NAME);
        // message is optional, so required should be empty
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_model_info_tool_api_format() {
        let tool = get_tool_api(MODEL_INFO_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], MODEL_INFO_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("metadata")
        );
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("model"))
        );
    }

    #[test]
    fn test_flow_tool_constants() {
        assert_eq!(SEND_MESSAGE_TOOL_NAME, "send_message");
        assert_eq!(CALL_AGENT_TOOL_NAME, "call_agent");
        assert_eq!(CALL_USER_TOOL_NAME, "call_user");
        assert_eq!(MODEL_INFO_TOOL_NAME, "model_info");
    }

    // === Metadata Tests ===

    #[test]
    fn test_builtin_tool_metadata_call_agent() {
        let meta = builtin_tool_metadata(CALL_AGENT_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(!meta.ends_turn);
    }

    #[test]
    fn test_builtin_tool_metadata_call_user() {
        let meta = builtin_tool_metadata(CALL_USER_TOOL_NAME);
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
    fn test_builtin_tool_metadata_shell_exec() {
        let meta = builtin_tool_metadata("shell_exec");
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

    // === Handoff Type Tests ===

    #[test]
    fn test_handoff_default() {
        let target = HandoffTarget::default();
        match target {
            HandoffTarget::Agent { prompt } => assert!(prompt.is_empty()),
            _ => panic!("Expected Agent variant"),
        }
    }

    #[test]
    fn test_handoff_explicit_takes_precedence() {
        let fallback = HandoffTarget::User {
            message: "fallback".to_string(),
        };
        let mut handoff = Handoff::new(fallback);
        handoff.set_agent("explicit prompt".to_string());

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "explicit prompt"),
            _ => panic!("Expected Agent variant"),
        }
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_last_wins() {
        let fallback = HandoffTarget::Agent { prompt: String::new() };
        let mut handoff = Handoff::new(fallback);
        handoff.set_agent("first".to_string());
        handoff.set_user("second".to_string());
        handoff.set_agent("third".to_string());

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "third"),
            _ => panic!("Expected Agent variant"),
        }
    }

    #[test]
    fn test_handoff_set_fallback() {
        let fallback = HandoffTarget::Agent { prompt: "original".to_string() };
        let mut handoff = Handoff::new(fallback);

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "original"),
            _ => panic!("Expected Agent variant"),
        }

        handoff.set_fallback(HandoffTarget::User { message: "new fallback".to_string() });

        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "new fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_explicit_still_beats_set_fallback() {
        let fallback = HandoffTarget::Agent { prompt: String::new() };
        let mut handoff = Handoff::new(fallback);
        handoff.set_fallback(HandoffTarget::User { message: "fallback".to_string() });
        handoff.set_agent("explicit".to_string());

        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "explicit"),
            _ => panic!("Expected Agent variant"),
        }
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_ends_turn_requested_none() {
        let handoff = Handoff::new(HandoffTarget::Agent { prompt: String::new() });
        assert!(!handoff.ends_turn_requested());
    }

    #[test]
    fn test_handoff_ends_turn_requested_user() {
        let mut handoff = Handoff::new(HandoffTarget::Agent { prompt: String::new() });
        handoff.set_user("bye".to_string());
        assert!(handoff.ends_turn_requested());
    }

    #[test]
    fn test_handoff_ends_turn_requested_agent() {
        let mut handoff = Handoff::new(HandoffTarget::User { message: String::new() });
        handoff.set_agent("continue".to_string());
        assert!(!handoff.ends_turn_requested());
    }

    // === Registry Tests ===

    #[test]
    fn test_flow_registry_contains_expected_tools() {
        // BUILTIN_TOOL_DEFS now only has flow tools (memory moved to memory::MEMORY_TOOL_DEFS)
        assert_eq!(BUILTIN_TOOL_DEFS.len(), 4);
        let names: Vec<_> = BUILTIN_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&SEND_MESSAGE_TOOL_NAME));
        assert!(names.contains(&CALL_AGENT_TOOL_NAME));
        assert!(names.contains(&CALL_USER_TOOL_NAME));
        assert!(names.contains(&MODEL_INFO_TOOL_NAME));
    }

    #[test]
    fn test_all_builtin_tools_to_api_format_includes_memory() {
        // all_builtin_tools_to_api_format covers memory (4) + flow (4) = 8
        let tools = all_builtin_tools_to_api_format();
        assert_eq!(tools.len(), 8);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].as_str().is_some());
            assert!(tool["function"]["description"].as_str().is_some());
        }
    }

    #[test]
    fn test_builtin_tools_to_api_format_with_reflection() {
        let tools = builtin_tools_to_api_format(true);
        assert_eq!(tools.len(), 8);
    }

    #[test]
    fn test_builtin_tools_to_api_format_without_reflection() {
        let tools = builtin_tools_to_api_format(false);
        assert_eq!(tools.len(), 7);
        for tool in &tools {
            assert_ne!(
                tool["function"]["name"],
                super::super::memory::REFLECTION_TOOL_NAME
            );
        }
    }

    #[test]
    fn test_get_builtin_tool_def_finds_memory_and_flow() {
        // Memory tools
        assert!(get_builtin_tool_def("update_reflection").is_some());
        assert!(get_builtin_tool_def("read_context").is_some());
        // Flow tools
        assert!(get_builtin_tool_def(CALL_AGENT_TOOL_NAME).is_some());
        assert!(get_builtin_tool_def(SEND_MESSAGE_TOOL_NAME).is_some());
        // Unknown
        assert!(get_builtin_tool_def("nonexistent_tool").is_none());
    }
}
