use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Hook points where tools can register to be called
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    PreMessage,
    PostMessage,
    PreTool,
    PostTool,
    OnContextSwitch,
    PreClear,
    PostClear,
    PreCompact,
    PostCompact,
    PreRollingCompact,
    PostRollingCompact,
    OnStart,
    OnEnd,
    PreSystemPrompt,  // Can inject content before system prompt sections
    PostSystemPrompt, // Can inject content after all system prompt sections
    PreSendMessage,   // Can intercept delivery (return {"delivered": true, "via": "..."})
    PostSendMessage,  // Observe delivery (read-only)
}

impl HookPoint {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pre_message" => Some(Self::PreMessage),
            "post_message" => Some(Self::PostMessage),
            "pre_tool" => Some(Self::PreTool),
            "post_tool" => Some(Self::PostTool),
            "on_context_switch" => Some(Self::OnContextSwitch),
            "pre_clear" => Some(Self::PreClear),
            "post_clear" => Some(Self::PostClear),
            "pre_compact" => Some(Self::PreCompact),
            "post_compact" => Some(Self::PostCompact),
            "pre_rolling_compact" => Some(Self::PreRollingCompact),
            "post_rolling_compact" => Some(Self::PostRollingCompact),
            "on_start" => Some(Self::OnStart),
            "on_end" => Some(Self::OnEnd),
            "pre_system_prompt" => Some(Self::PreSystemPrompt),
            "post_system_prompt" => Some(Self::PostSystemPrompt),
            "pre_send_message" => Some(Self::PreSendMessage),
            "post_send_message" => Some(Self::PostSendMessage),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PreMessage => "pre_message",
            Self::PostMessage => "post_message",
            Self::PreTool => "pre_tool",
            Self::PostTool => "post_tool",
            Self::OnContextSwitch => "on_context_switch",
            Self::PreClear => "pre_clear",
            Self::PostClear => "post_clear",
            Self::PreCompact => "pre_compact",
            Self::PostCompact => "post_compact",
            Self::PreRollingCompact => "pre_rolling_compact",
            Self::PostRollingCompact => "post_rolling_compact",
            Self::OnStart => "on_start",
            Self::OnEnd => "on_end",
            Self::PreSystemPrompt => "pre_system_prompt",
            Self::PostSystemPrompt => "post_system_prompt",
            Self::PreSendMessage => "pre_send_message",
            Self::PostSendMessage => "post_send_message",
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
}

/// Load all tools from the plugins directory by calling each with --schema
pub fn load_tools(plugins_dir: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let mut tools = Vec::new();

    if !plugins_dir.exists() {
        return Ok(tools);
    }

    let entries = fs::read_dir(plugins_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Skip .disabled entries
        if file_name.ends_with(".disabled") {
            continue;
        }

        // Determine the executable path
        let exec_path = if path.is_dir() {
            // Directory plugin: look for plugins/[name]/[name]
            let inner = path.join(file_name);
            if !inner.exists() || inner.is_dir() {
                if verbose {
                    eprintln!("[WARN] Plugin directory {:?} missing executable", file_name);
                }
                continue;
            }
            inner
        } else {
            path.clone()
        };

        // Check if executable (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = exec_path.metadata()
                && metadata.permissions().mode() & 0o111 == 0
            {
                continue; // Not executable
            }
        }

        // Try to get schema(s) from the tool
        match get_tool_schemas(&exec_path, verbose) {
            Ok(new_tools) => tools.extend(new_tools),
            Err(e) => {
                if verbose {
                    eprintln!("[WARN] Failed to load tool {:?}: {}", exec_path.file_name(), e);
                }
            }
        }
    }

    Ok(tools)
}

/// Get tool schema(s) by calling plugin with --schema
/// Returns Vec<Tool> to support plugins that provide multiple tools
fn get_tool_schemas(path: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let output = Command::new(path)
        .arg("--schema")
        .output()
        .map_err(|e| io::Error::other(format!("Failed to execute tool: {}", e)))?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Tool returned error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let schema_str = String::from_utf8(output.stdout).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid UTF-8 in schema: {}", e),
        )
    })?;

    let schema: serde_json::Value = serde_json::from_str(&schema_str).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid JSON schema: {}", e),
        )
    })?;

    // Handle array of tools or single tool
    let schemas: Vec<&serde_json::Value> = if let Some(arr) = schema.as_array() {
        arr.iter().collect()
    } else {
        vec![&schema]
    };

    let mut tools = Vec::new();
    for s in schemas {
        match parse_single_tool_schema(s, path, verbose) {
            Ok(tool) => tools.push(tool),
            Err(e) => {
                if verbose {
                    eprintln!(
                        "[WARN] Failed to parse tool in {:?}: {}",
                        path.file_name(),
                        e
                    );
                }
            }
        }
    }

    if tools.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "No valid tools found in schema",
        ));
    }

    Ok(tools)
}

fn parse_single_tool_schema(
    schema: &serde_json::Value,
    path: &PathBuf,
    verbose: bool,
) -> io::Result<Tool> {
    let name = schema["name"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Schema missing 'name' field"))?
        .to_string();

    let description = schema["description"]
        .as_str()
        .ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "Schema missing 'description' field")
        })?
        .to_string();

    let parameters = schema["parameters"].clone();

    // Parse hooks array (optional)
    let hooks = if let Some(hooks_array) = schema["hooks"].as_array() {
        hooks_array
            .iter()
            .filter_map(|v| {
                let hook_str = v.as_str()?;
                let hook = HookPoint::from_str(hook_str);
                if hook.is_none() && verbose {
                    eprintln!("[WARN] Unknown hook '{}' in tool '{}'", hook_str, name);
                }
                hook
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Tool {
        name,
        description,
        parameters,
        path: path.clone(),
        hooks,
    })
}

/// Convert tools to OpenAI-style function definitions for the API
pub fn tools_to_api_format(tools: &[Tool]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

/// Execute a hook on all tools that registered for it
/// Returns a vector of (tool_name, result) for tools that returned non-empty output
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
    verbose: bool,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        if verbose {
            eprintln!("[Hook {}: {}]", hook.as_str(), tool.name);
        }

        let output = Command::new(&tool.path)
            .env("CHIBI_HOOK", hook.as_str())
            .env("CHIBI_HOOK_DATA", data.to_string())
            .env_remove("CHIBI_TOOL_ARGS") // Clear tool args to avoid confusion
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
                io::Error::other(format!(
                    "Failed to execute hook {} on {}: {}",
                    hook.as_str(),
                    tool.name,
                    e
                ))
            })?;

        if !output.status.success() {
            if verbose {
                eprintln!(
                    "[WARN] Hook {} on {} failed (exit code {:?})",
                    hook.as_str(),
                    tool.name,
                    output.status.code()
                );
            }
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Try to parse as JSON, otherwise wrap as string
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|_| serde_json::Value::String(trimmed.to_string()));

        results.push((tool.name.clone(), value));
    }

    Ok(results)
}

/// Execute a tool with the given arguments (as JSON)
///
/// Tools receive arguments via CHIBI_TOOL_ARGS env var, leaving stdin free for user interaction.
/// Tools also receive CHIBI_VERBOSE=1 env var when verbose mode is enabled.
pub fn execute_tool(
    tool: &Tool,
    arguments: &serde_json::Value,
    verbose: bool,
) -> io::Result<String> {
    let mut cmd = Command::new(&tool.path);
    cmd.stdin(Stdio::inherit()) // Let tool read from user's terminal
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()); // Let tool's stderr go directly to terminal (for prompts)

    // Pass arguments via environment variable (frees stdin for user interaction)
    let json_str = serde_json::to_string(arguments)
        .map_err(|e| io::Error::other(format!("Failed to serialize arguments: {}", e)))?;
    cmd.env("CHIBI_TOOL_ARGS", json_str);

    // Pass tool name for multi-tool plugins
    cmd.env("CHIBI_TOOL_NAME", &tool.name);

    // Pass verbosity to tool via environment variable
    if verbose {
        cmd.env("CHIBI_VERBOSE", "1");
    }

    let output = cmd
        .output()
        .map_err(|e| io::Error::other(format!("Failed to execute tool: {}", e)))?;

    if !output.status.success() {
        return Err(io::Error::other(
            "Tool execution failed or was cancelled".to_string(),
        ));
    }

    String::from_utf8(output.stdout).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid UTF-8 in tool output: {}", e),
        )
    })
}

/// Find a tool by name
pub fn find_tool<'a>(tools: &'a [Tool], name: &str) -> Option<&'a Tool> {
    tools.iter().find(|t| t.name == name)
}

/// Name of the built-in reflection tool
pub const REFLECTION_TOOL_NAME: &str = "update_reflection";

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

// --- Todos Tool ---

pub const TODOS_TOOL_NAME: &str = "update_todos";

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

// --- Goals Tool ---

pub const GOALS_TOOL_NAME: &str = "update_goals";

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

// --- Recurse Tool (external noop) ---

/// Name of the external recurse tool that triggers recursion
pub const RECURSE_TOOL_NAME: &str = "recurse";

/// Check if the tool call is for the recurse tool and extract the note
pub fn check_recurse_signal(tool_name: &str, arguments: &serde_json::Value) -> Option<String> {
    if tool_name == RECURSE_TOOL_NAME {
        let note = arguments["note"].as_str().unwrap_or("").to_string();
        Some(note)
    } else {
        None
    }
}

/// Name of the built-in send_message tool for inter-context messaging
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";

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

#[cfg(test)]
mod tests {
    use super::*;

    // All 17 hook points for testing
    const ALL_HOOKS: &[(&str, HookPoint)] = &[
        ("pre_message", HookPoint::PreMessage),
        ("post_message", HookPoint::PostMessage),
        ("pre_tool", HookPoint::PreTool),
        ("post_tool", HookPoint::PostTool),
        ("on_context_switch", HookPoint::OnContextSwitch),
        ("pre_clear", HookPoint::PreClear),
        ("post_clear", HookPoint::PostClear),
        ("pre_compact", HookPoint::PreCompact),
        ("post_compact", HookPoint::PostCompact),
        ("pre_rolling_compact", HookPoint::PreRollingCompact),
        ("post_rolling_compact", HookPoint::PostRollingCompact),
        ("on_start", HookPoint::OnStart),
        ("on_end", HookPoint::OnEnd),
        ("pre_system_prompt", HookPoint::PreSystemPrompt),
        ("post_system_prompt", HookPoint::PostSystemPrompt),
        ("pre_send_message", HookPoint::PreSendMessage),
        ("post_send_message", HookPoint::PostSendMessage),
    ];

    #[test]
    fn test_hook_point_from_str_valid() {
        for (s, expected) in ALL_HOOKS {
            let result = HookPoint::from_str(s);
            assert!(result.is_some(), "from_str failed for '{}'", s);
            assert_eq!(result.unwrap(), *expected);
        }
    }

    #[test]
    fn test_hook_point_from_str_invalid() {
        assert!(HookPoint::from_str("").is_none());
        assert!(HookPoint::from_str("unknown").is_none());
        assert!(HookPoint::from_str("PreMessage").is_none()); // wrong case
        assert!(HookPoint::from_str("pre-message").is_none()); // wrong separator
    }

    #[test]
    fn test_hook_point_as_str() {
        for (expected_str, hook) in ALL_HOOKS {
            assert_eq!(hook.as_str(), *expected_str);
        }
    }

    #[test]
    fn test_hook_point_round_trip() {
        for (s, _) in ALL_HOOKS {
            let hook = HookPoint::from_str(s).unwrap();
            assert_eq!(hook.as_str(), *s);
        }
    }

    #[test]
    fn test_tool_struct() {
        let tool = Tool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
            path: PathBuf::from("/usr/bin/test"),
            hooks: vec![HookPoint::OnStart, HookPoint::OnEnd],
        };
        assert_eq!(tool.name, "test_tool");
        assert_eq!(tool.hooks.len(), 2);
        assert!(tool.hooks.contains(&HookPoint::OnStart));
    }

    #[test]
    fn test_tools_to_api_format() {
        let tools = vec![
            Tool {
                name: "tool_one".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {"arg": {"type": "string"}}}),
                path: PathBuf::from("/bin/one"),
                hooks: vec![],
            },
            Tool {
                name: "tool_two".to_string(),
                description: "Second tool".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
                path: PathBuf::from("/bin/two"),
                hooks: vec![HookPoint::PreTool],
            },
        ];

        let api_format = tools_to_api_format(&tools);
        assert_eq!(api_format.len(), 2);

        // Check first tool
        assert_eq!(api_format[0]["type"], "function");
        assert_eq!(api_format[0]["function"]["name"], "tool_one");
        assert_eq!(api_format[0]["function"]["description"], "First tool");

        // Check second tool
        assert_eq!(api_format[1]["function"]["name"], "tool_two");
    }

    #[test]
    fn test_find_tool() {
        let tools = vec![
            Tool {
                name: "alpha".to_string(),
                description: "Alpha tool".to_string(),
                parameters: serde_json::json!({}),
                path: PathBuf::from("/bin/alpha"),
                hooks: vec![],
            },
            Tool {
                name: "beta".to_string(),
                description: "Beta tool".to_string(),
                parameters: serde_json::json!({}),
                path: PathBuf::from("/bin/beta"),
                hooks: vec![],
            },
        ];

        assert!(find_tool(&tools, "alpha").is_some());
        assert_eq!(find_tool(&tools, "alpha").unwrap().name, "alpha");

        assert!(find_tool(&tools, "beta").is_some());
        assert!(find_tool(&tools, "gamma").is_none());
        assert!(find_tool(&tools, "").is_none());
    }

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
