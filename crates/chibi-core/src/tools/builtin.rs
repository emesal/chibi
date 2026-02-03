//! Built-in tools provided by chibi.
//!
//! These tools are always available and handle core functionality:
//! - update_reflection: Persistent memory across all contexts
//! - update_todos: Per-context task tracking
//! - update_goals: Per-context goal tracking
//! - send_message: Inter-context messaging
//! - recurse: Signal to continue processing

use crate::state::AppState;
use std::io::{self, ErrorKind};
use std::path::Path;

// === Tool Name Constants ===

/// Name of the built-in reflection tool
pub const REFLECTION_TOOL_NAME: &str = "update_reflection";

/// Name of the built-in todos tool
pub const TODOS_TOOL_NAME: &str = "update_todos";

/// Name of the built-in goals tool
pub const GOALS_TOOL_NAME: &str = "update_goals";

/// Name of the built-in send_message tool for inter-context messaging
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

/// Name of the built-in call_agent tool for control handoff
pub const CALL_AGENT_TOOL_NAME: &str = "call_agent";

/// Name of the built-in call_user tool for control handoff
pub const CALL_USER_TOOL_NAME: &str = "call_user";

// === Tool Definition Registry ===

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
}

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

/// All built-in tool definitions
pub static BUILTIN_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: REFLECTION_TOOL_NAME,
        description: "Update your persistent reflection/memory that persists across all contexts and sessions. Use this to store anything you want to remember: insights about the user, preferences, important facts, or notes to your future self. Keep it concise and organized. The content will completely replace the previous reflection.",
        properties: &[ToolPropertyDef {
            name: "content",
            prop_type: "string",
            description: "The new reflection content. This replaces the entire previous reflection.",
            default: None,
        }],
        required: &["content"],
    },
    BuiltinToolDef {
        name: TODOS_TOOL_NAME,
        description: "Update the todo list for this context. Use this to track tasks you need to complete during this conversation. Todos persist across messages but are specific to this context. Format as markdown checklist.",
        properties: &[ToolPropertyDef {
            name: "content",
            prop_type: "string",
            description: "The todo list content (markdown format, e.g., '- [ ] Task 1\\n- [x] Completed task')",
            default: None,
        }],
        required: &["content"],
    },
    BuiltinToolDef {
        name: GOALS_TOOL_NAME,
        description: "Update the goals for this context. Goals are high-level objectives that persist between conversation rounds and guide your work. Use goals to track what you're trying to achieve overall.",
        properties: &[ToolPropertyDef {
            name: "content",
            prop_type: "string",
            description: "The goals content (markdown format)",
            default: None,
        }],
        required: &["content"],
    },
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
    },
    BuiltinToolDef {
        name: CALL_AGENT_TOOL_NAME,
        description: "recurse to do more work before handing control back to the user. Use this to continue processing when you have more steps to complete.",
        properties: &[ToolPropertyDef {
            name: "prompt",
            prop_type: "string",
            description: "Focus for the next turn",
            default: None,
        }],
        required: &["prompt"],
    },
    BuiltinToolDef {
        name: CALL_USER_TOOL_NAME,
        description: "Return control to user.",
        properties: &[ToolPropertyDef {
            name: "message",
            prop_type: "string",
            description: "Optional message to display",
            default: None,
        }],
        required: &[],
    },
];

/// Look up a specific builtin tool definition by name.
/// Returns None if not found. Use this for testing or conditional tool access.
pub fn get_builtin_tool_def(name: &str) -> Option<&'static BuiltinToolDef> {
    BUILTIN_TOOL_DEFS.iter().find(|def| def.name == name)
}

/// Convert all built-in tools to API format
pub fn all_builtin_tools_to_api_format() -> Vec<serde_json::Value> {
    BUILTIN_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Convert built-in tools to API format, optionally excluding reflection tool
pub fn builtin_tools_to_api_format(include_reflection: bool) -> Vec<serde_json::Value> {
    BUILTIN_TOOL_DEFS
        .iter()
        .filter(|def| include_reflection || def.name != REFLECTION_TOOL_NAME)
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
}

// === Tool Execution ===

/// Execute the built-in update_reflection tool
pub fn execute_reflection_tool(
    prompts_dir: &Path,
    arguments: &serde_json::Value,
    character_limit: usize,
) -> io::Result<String> {
    let content = arguments["content"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing 'content' parameter"))?;

    // Check character limit
    if content.len() > character_limit {
        return Ok(format!(
            "Error: Content exceeds the {} character limit ({} characters provided). Please shorten your reflection.",
            character_limit,
            content.len()
        ));
    }

    let reflection_path = prompts_dir.join("reflection.md");
    crate::safe_io::atomic_write_text(&reflection_path, content)?;

    Ok(format!(
        "Reflection updated successfully ({} characters).",
        content.len()
    ))
}

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
) -> Option<io::Result<String>> {
    match tool_name {
        TODOS_TOOL_NAME => {
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.save_todos(context_name, content)
                    .map(|_| format!("Todos updated ({} characters).", content.len())),
            )
        }
        GOALS_TOOL_NAME => {
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.save_goals(context_name, content)
                    .map(|_| format!("Goals updated ({} characters).", content.len())),
            )
        }
        REFLECTION_TOOL_NAME => Some(execute_reflection_tool(
            &app.prompts_dir,
            args,
            app.config.reflection_character_limit,
        )),
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

    // === Helper for tests: get API format for a specific tool ===
    fn get_tool_api(name: &str) -> serde_json::Value {
        get_builtin_tool_def(name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Individual Tool Tests (using registry lookup) ===

    #[test]
    fn test_reflection_tool_api_format() {
        let tool = get_tool_api(REFLECTION_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], REFLECTION_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("reflection")
        );
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("content"))
        );
    }

    #[test]
    fn test_todos_tool_api_format() {
        let tool = get_tool_api(TODOS_TOOL_NAME);
        assert_eq!(tool["function"]["name"], TODOS_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("todo")
        );
    }

    #[test]
    fn test_goals_tool_api_format() {
        let tool = get_tool_api(GOALS_TOOL_NAME);
        assert_eq!(tool["function"]["name"], GOALS_TOOL_NAME);
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("goal")
        );
    }

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
    fn test_tool_constants() {
        assert_eq!(REFLECTION_TOOL_NAME, "update_reflection");
        assert_eq!(TODOS_TOOL_NAME, "update_todos");
        assert_eq!(GOALS_TOOL_NAME, "update_goals");
        assert_eq!(SEND_MESSAGE_TOOL_NAME, "send_message");
    }

    #[test]
    fn test_builtin_tool_metadata_call_agent() {
        let meta = builtin_tool_metadata(CALL_AGENT_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(!meta.ends_turn); // call_agent continues processing
    }

    #[test]
    fn test_builtin_tool_metadata_call_user() {
        let meta = builtin_tool_metadata(CALL_USER_TOOL_NAME);
        assert!(!meta.parallel);
        assert!(meta.flow_control);
        assert!(meta.ends_turn); // call_user ends turn
    }

    #[test]
    fn test_builtin_tool_metadata_other() {
        let meta = builtin_tool_metadata("update_todos");
        assert!(meta.parallel);
        assert!(!meta.flow_control);
        assert!(!meta.ends_turn);
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
                .contains("recurse")
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
        assert!(
            tool["function"]["description"]
                .as_str()
                .unwrap()
                .contains("user")
        );
        // message is optional, so required should be empty
        assert!(
            tool["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_get_builtin_tool_def() {
        assert!(get_builtin_tool_def(REFLECTION_TOOL_NAME).is_some());
        assert!(get_builtin_tool_def("nonexistent_tool").is_none());
    }

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

        // Set explicit agent call
        handoff.set_agent("explicit prompt".to_string());

        // Take should return the explicit value
        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "explicit prompt"),
            _ => panic!("Expected Agent variant"),
        }

        // Next take should return fallback
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_last_wins() {
        let fallback = HandoffTarget::Agent {
            prompt: String::new(),
        };
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
    fn test_handoff_constants() {
        assert_eq!(CALL_AGENT_TOOL_NAME, "call_agent");
        assert_eq!(CALL_USER_TOOL_NAME, "call_user");
    }

    #[test]
    fn test_handoff_set_fallback() {
        // Start with agent fallback
        let fallback = HandoffTarget::Agent {
            prompt: "original".to_string(),
        };
        let mut handoff = Handoff::new(fallback);

        // Without explicit call, should use original fallback
        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "original"),
            _ => panic!("Expected Agent variant"),
        }

        // Override fallback to user
        handoff.set_fallback(HandoffTarget::User {
            message: "new fallback".to_string(),
        });

        // Now take should return the new fallback
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "new fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    #[test]
    fn test_handoff_explicit_still_beats_set_fallback() {
        let fallback = HandoffTarget::Agent {
            prompt: String::new(),
        };
        let mut handoff = Handoff::new(fallback);

        // Override fallback
        handoff.set_fallback(HandoffTarget::User {
            message: "fallback".to_string(),
        });

        // But also set an explicit call
        handoff.set_agent("explicit".to_string());

        // Explicit should still win
        match handoff.take() {
            HandoffTarget::Agent { prompt } => assert_eq!(prompt, "explicit"),
            _ => panic!("Expected Agent variant"),
        }

        // But next take uses the new fallback
        match handoff.take() {
            HandoffTarget::User { message } => assert_eq!(message, "fallback"),
            _ => panic!("Expected User variant"),
        }
    }

    // === Registry Tests ===

    #[test]
    fn test_registry_contains_all_tools() {
        assert_eq!(BUILTIN_TOOL_DEFS.len(), 6);
        let names: Vec<_> = BUILTIN_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&REFLECTION_TOOL_NAME));
        assert!(names.contains(&TODOS_TOOL_NAME));
        assert!(names.contains(&GOALS_TOOL_NAME));
        assert!(names.contains(&SEND_MESSAGE_TOOL_NAME));
        assert!(names.contains(&CALL_AGENT_TOOL_NAME));
        assert!(names.contains(&CALL_USER_TOOL_NAME));
    }

    #[test]
    fn test_all_builtin_tools_to_api_format() {
        let tools = all_builtin_tools_to_api_format();
        assert_eq!(tools.len(), 6);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].as_str().is_some());
            assert!(tool["function"]["description"].as_str().is_some());
        }
    }

    #[test]
    fn test_builtin_tools_to_api_format_with_reflection() {
        let tools = builtin_tools_to_api_format(true);
        assert_eq!(tools.len(), 6);
    }

    #[test]
    fn test_builtin_tools_to_api_format_without_reflection() {
        let tools = builtin_tools_to_api_format(false);
        assert_eq!(tools.len(), 5);
        for tool in &tools {
            assert_ne!(tool["function"]["name"], REFLECTION_TOOL_NAME);
        }
    }
}
