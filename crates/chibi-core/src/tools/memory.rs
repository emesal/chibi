//!
//! Memory tools: reflection, todos, goals, read_context.
//! These tools read and write internal context state.

use super::{BuiltinToolDef, ToolPropertyDef, require_str_param, vfs_block_on};
use crate::config::ResolvedConfig;
use crate::state::{AppState, load_flock_contexts};
use crate::vfs::{VfsCaller, VfsPath, flock::resolve_flock_vfs_root};
use std::io::{self, ErrorKind};
use std::path::Path;

pub const REFLECTION_TOOL_NAME: &str = "update_reflection";
pub const TODOS_TOOL_NAME: &str = "update_todos";
pub const GOALS_TOOL_NAME: &str = "update_goals";
pub const READ_CONTEXT_TOOL_NAME: &str = "read_context";
pub const FLOCK_JOIN_TOOL_NAME: &str = "flock_join";
pub const FLOCK_LEAVE_TOOL_NAME: &str = "flock_leave";

pub static MEMORY_TOOL_DEFS: &[BuiltinToolDef] = &[
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
        summary_params: &[],
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
        summary_params: &[],
    },
    BuiltinToolDef {
        name: GOALS_TOOL_NAME,
        description: "Update the goals for a flock. Goals are high-level objectives shared by all contexts in a flock. Use 'site' to update site-wide goals or a specific flock name for team goals.",
        properties: &[
            ToolPropertyDef {
                name: "flock",
                prop_type: "string",
                description: "The flock to update goals for ('site' for site-wide, or a named flock)",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "The goals content (markdown format)",
                default: None,
            },
        ],
        required: &["flock", "content"],
        summary_params: &["flock"],
    },
    BuiltinToolDef {
        name: READ_CONTEXT_TOOL_NAME,
        description: "Read the state of another context (read-only). Returns summary, todos, goals, and recent messages. Useful for inspecting sub-agents or coordinating with related contexts.",
        properties: &[
            ToolPropertyDef {
                name: "context_name",
                prop_type: "string",
                description: "Name of the context to read",
                default: None,
            },
            ToolPropertyDef {
                name: "include_messages",
                prop_type: "string",
                description: "Include recent messages (\"true\"/\"false\", default: \"true\")",
                default: None,
            },
            ToolPropertyDef {
                name: "num_messages",
                prop_type: "integer",
                description: "Number of recent messages to include (default: 5)",
                default: Some(5),
            },
        ],
        required: &["context_name"],
        summary_params: &["context_name"],
    },
    BuiltinToolDef {
        name: FLOCK_JOIN_TOOL_NAME,
        description: "Join a flock (named group of contexts that share goals). Creates the flock if it doesn't exist.",
        properties: &[ToolPropertyDef {
            name: "flock",
            prop_type: "string",
            description: "Name of the flock to join (lowercase alphanumeric + hyphens)",
            default: None,
        }],
        required: &["flock"],
        summary_params: &["flock"],
    },
    BuiltinToolDef {
        name: FLOCK_LEAVE_TOOL_NAME,
        description: "Leave a flock. Cannot leave the site flock.",
        properties: &[ToolPropertyDef {
            name: "flock",
            prop_type: "string",
            description: "Name of the flock to leave",
            default: None,
        }],
        required: &["flock"],
        summary_params: &["flock"],
    },
];

/// Check if a tool name is a memory tool.
pub fn is_memory_tool(name: &str) -> bool {
    MEMORY_TOOL_DEFS.iter().any(|d| d.name == name)
}

/// Convert all memory tools to API format.
pub fn all_memory_tools_to_api_format() -> Vec<serde_json::Value> {
    MEMORY_TOOL_DEFS.iter().map(|d| d.to_api_format()).collect()
}

/// Execute a memory tool by name.
///
/// Returns `Some(result)` if the tool was handled, `None` if the name is not a memory tool.
pub fn execute_memory_tool(
    app: &AppState,
    context_name: &str,
    name: &str,
    args: &serde_json::Value,
    config: Option<&ResolvedConfig>,
) -> Option<io::Result<String>> {
    match name {
        TODOS_TOOL_NAME => {
            let content = args.get("content").and_then(|v| v.as_str())?;
            Some(
                app.save_todos(context_name, content)
                    .map(|_| format!("Todos updated ({} characters).", content.len())),
            )
        }
        GOALS_TOOL_NAME => Some((|| {
            let flock = require_str_param(args, "flock")?;
            let content = require_str_param(args, "content")?;
            let root = resolve_flock_vfs_root(&flock, app.vfs.site_id())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let path = VfsPath::new(&format!("{}/goals.md", root.as_str()))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            vfs_block_on(app.vfs.write(VfsCaller::Context(context_name), &path, content.as_bytes()))
                .map(|_| format!("Goals updated for flock '{}' ({} characters).", flock, content.len()))
        })()),
        REFLECTION_TOOL_NAME => {
            let limit = config
                .map(|c| c.reflection_character_limit)
                .unwrap_or(app.config.reflection_character_limit);
            Some(execute_reflection_tool(&app.prompts_dir, args, limit))
        }
        READ_CONTEXT_TOOL_NAME => {
            let target = args.get("context_name").and_then(|v| v.as_str())?;
            Some(execute_read_context(app, target, args))
        }
        FLOCK_JOIN_TOOL_NAME => Some((|| {
            let flock = require_str_param(args, "flock")?;
            vfs_block_on(app.vfs.flock_join(&flock, context_name))
                .map(|_| format!("Joined flock '{}'.", flock))
        })()),
        FLOCK_LEAVE_TOOL_NAME => Some((|| {
            let flock = require_str_param(args, "flock")?;
            vfs_block_on(app.vfs.flock_leave(&flock, context_name))
                .map(|_| format!("Left flock '{}'.", flock))
        })()),
        _ => None,
    }
}

/// Execute the built-in update_reflection tool.
pub fn execute_reflection_tool(
    prompts_dir: &Path,
    arguments: &serde_json::Value,
    character_limit: usize,
) -> io::Result<String> {
    let content = arguments["content"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing 'content' parameter"))?;

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

/// Execute read_context: read the state of another context (read-only).
///
/// Returns a JSON object with the context's summary, todos, flock_goals, and
/// optionally recent messages from context.jsonl.
fn execute_read_context(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    use crate::StatePaths;
    use crate::json_ext::JsonExt;

    crate::context::validate_context_name(context_name)?;

    if !app.list_contexts().contains(&context_name.to_string()) {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Context '{}' does not exist", context_name),
        ));
    }

    let include_messages = args.get_str_or("include_messages", "true") == "true";
    let num_messages = args.get_u64_or("num_messages", 5) as usize;

    let mut result = serde_json::Map::new();
    result.insert(
        "name".to_string(),
        serde_json::Value::String(context_name.to_string()),
    );

    let summary = std::fs::read_to_string(app.summary_file(context_name)).unwrap_or_default();
    result.insert("summary".to_string(), serde_json::Value::String(summary));

    let todos = app.load_todos_for(context_name).unwrap_or_default();
    result.insert("todos".to_string(), serde_json::Value::String(todos));

    // Goals are flock-scoped: load all flock contexts for the target and format
    // as an attributed flock_goals array for the caller.
    let flock_contexts = load_flock_contexts(&app.vfs, context_name).unwrap_or_default();
    let flock_goals: Vec<serde_json::Value> = flock_contexts
        .iter()
        .filter_map(|fc| {
            fc.goals.as_ref().map(|g| {
                serde_json::json!({
                    "flock": fc.flock_name,
                    "goals": g,
                })
            })
        })
        .collect();
    result.insert(
        "flock_goals".to_string(),
        serde_json::Value::Array(flock_goals),
    );

    if include_messages {
        match app.read_context_entries(context_name) {
            Ok(entries) => {
                let total = entries.len();
                let recent: Vec<_> = entries.into_iter().rev().take(num_messages).collect();
                let recent: Vec<_> = recent.into_iter().rev().collect();
                let messages: Vec<serde_json::Value> = recent
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "from": e.from,
                            "content": e.content,
                            "type": e.entry_type,
                        })
                    })
                    .collect();
                result.insert(
                    "message_count".to_string(),
                    serde_json::Value::Number(total.into()),
                );
                result.insert(
                    "recent_messages".to_string(),
                    serde_json::Value::Array(messages),
                );
            }
            Err(_) => {
                result.insert(
                    "message_count".to_string(),
                    serde_json::Value::Number(0.into()),
                );
                result.insert(
                    "recent_messages".to_string(),
                    serde_json::Value::Array(vec![]),
                );
            }
        }
    }

    serde_json::to_string_pretty(&result)
        .map_err(|e| io::Error::other(format!("Failed to serialize context state: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_tool_api(name: &str) -> serde_json::Value {
        MEMORY_TOOL_DEFS
            .iter()
            .find(|d| d.name == name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

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
    fn test_memory_tool_constants() {
        assert_eq!(REFLECTION_TOOL_NAME, "update_reflection");
        assert_eq!(TODOS_TOOL_NAME, "update_todos");
        assert_eq!(GOALS_TOOL_NAME, "update_goals");
        assert_eq!(READ_CONTEXT_TOOL_NAME, "read_context");
    }

    #[test]
    fn test_is_memory_tool() {
        assert!(is_memory_tool(REFLECTION_TOOL_NAME));
        assert!(is_memory_tool(TODOS_TOOL_NAME));
        assert!(is_memory_tool(GOALS_TOOL_NAME));
        assert!(is_memory_tool(READ_CONTEXT_TOOL_NAME));
        assert!(!is_memory_tool("shell_exec"));
        assert!(!is_memory_tool("call_agent"));
    }

    #[test]
    fn test_memory_defs_count() {
        assert_eq!(MEMORY_TOOL_DEFS.len(), 6);
    }
}
