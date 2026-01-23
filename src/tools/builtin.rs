//! Built-in tools provided by chibi.
//!
//! These tools are always available and handle core functionality:
//! - update_reflection: Persistent memory across all contexts
//! - update_todos: Per-context task tracking
//! - update_goals: Per-context goal tracking
//! - send_message: Inter-context messaging
//! - recurse: Signal to continue processing

use crate::state::AppState;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

// === Tool Name Constants ===

/// Name of the built-in reflection tool
pub const REFLECTION_TOOL_NAME: &str = "update_reflection";

/// Name of the built-in todos tool
pub const TODOS_TOOL_NAME: &str = "update_todos";

/// Name of the built-in goals tool
pub const GOALS_TOOL_NAME: &str = "update_goals";

/// Name of the external recurse tool that triggers recursion
pub const RECURSE_TOOL_NAME: &str = "recurse";

/// Name of the built-in send_message tool for inter-context messaging
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

// === Tool API Format Definitions ===

/// Create the built-in update_reflection tool definition for the API
pub fn reflection_tool_to_api_format() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": REFLECTION_TOOL_NAME,
            "description": "Update your persistent reflection/memory that persists across all contexts and sessions. Use this to store anything you want to remember: insights about the user, preferences, important facts, or notes to your future self. Keep it concise and organized. The content will completely replace the previous reflection.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The new reflection content. This replaces the entire previous reflection."
                    }
                },
                "required": ["content"]
            }
        }
    })
}

/// Create the built-in update_todos tool definition for the API
pub fn todos_tool_to_api_format() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": TODOS_TOOL_NAME,
            "description": "Update the todo list for this context. Use this to track tasks you need to complete during this conversation. Todos persist across messages but are specific to this context. Format as markdown checklist.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The todo list content (markdown format, e.g., '- [ ] Task 1\\n- [x] Completed task')"
                    }
                },
                "required": ["content"]
            }
        }
    })
}

/// Create the built-in update_goals tool definition for the API
pub fn goals_tool_to_api_format() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": GOALS_TOOL_NAME,
            "description": "Update the goals for this context. Goals are high-level objectives that persist between conversation rounds and guide your work. Use goals to track what you're trying to achieve overall.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The goals content (markdown format)"
                    }
                },
                "required": ["content"]
            }
        }
    })
}

/// Create the built-in send_message tool definition for the API
pub fn send_message_tool_to_api_format() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": SEND_MESSAGE_TOOL_NAME,
            "description": "Send a message to another context's inbox. The message will be delivered to the target context and shown to them before their next prompt.",
            "parameters": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Target context name"
                    },
                    "content": {
                        "type": "string",
                        "description": "Message content"
                    },
                    "from": {
                        "type": "string",
                        "description": "Optional sender name (defaults to current context)"
                    }
                },
                "required": ["to", "content"]
            }
        }
    })
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
    fs::write(&reflection_path, content)?;

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
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<io::Result<String>> {
    match tool_name {
        TODOS_TOOL_NAME => {
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.save_current_todos(content)
                    .map(|_| format!("Todos updated ({} characters).", content.len())),
            )
        }
        GOALS_TOOL_NAME => {
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.save_current_goals(content)
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
                app.send_inbox_message(to, content)
                    .map(|_| format!("Message sent to '{}'", to)),
            )
        }
        _ => None,
    }
}

/// Check if the tool call is for the recurse tool and extract the note
pub fn check_recurse_signal(tool_name: &str, arguments: &serde_json::Value) -> Option<String> {
    if tool_name == RECURSE_TOOL_NAME {
        let note = arguments["note"].as_str().unwrap_or("").to_string();
        Some(note)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflection_tool_api_format() {
        let tool = reflection_tool_to_api_format();
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
        let tool = todos_tool_to_api_format();
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
        let tool = goals_tool_to_api_format();
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
        let tool = send_message_tool_to_api_format();
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
    fn test_check_recurse_signal() {
        // With recurse tool and note
        let args = serde_json::json!({"note": "test note"});
        let result = check_recurse_signal(RECURSE_TOOL_NAME, &args);
        assert_eq!(result, Some("test note".to_string()));

        // With recurse tool but no note
        let args_empty = serde_json::json!({});
        let result_empty = check_recurse_signal(RECURSE_TOOL_NAME, &args_empty);
        assert_eq!(result_empty, Some("".to_string()));

        // With different tool
        let result_other = check_recurse_signal("other_tool", &args);
        assert!(result_other.is_none());
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(REFLECTION_TOOL_NAME, "update_reflection");
        assert_eq!(TODOS_TOOL_NAME, "update_todos");
        assert_eq!(GOALS_TOOL_NAME, "update_goals");
        assert_eq!(RECURSE_TOOL_NAME, "recurse");
        assert_eq!(SEND_MESSAGE_TOOL_NAME, "send_message");
    }
}
