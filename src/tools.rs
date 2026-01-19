use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Represents a tool that can be called by the LLM
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub path: PathBuf,
}

/// Load all tools from the tools directory by calling each with --schema
pub fn load_tools(tools_dir: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let mut tools = Vec::new();

    if !tools_dir.exists() {
        return Ok(tools);
    }

    let entries = fs::read_dir(tools_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip directories and non-executable files
        if path.is_dir() {
            continue;
        }

        // Check if executable (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = path.metadata() {
                if metadata.permissions().mode() & 0o111 == 0 {
                    continue; // Not executable
                }
            }
        }

        // Try to get schema from the tool
        match get_tool_schema(&path) {
            Ok(tool) => tools.push(tool),
            Err(e) => {
                if verbose {
                    eprintln!("[WARN] Failed to load tool {:?}: {}", path.file_name(), e);
                }
            }
        }
    }

    Ok(tools)
}

/// Get tool schema by calling it with --schema
fn get_tool_schema(path: &PathBuf) -> io::Result<Tool> {
    let output = Command::new(path)
        .arg("--schema")
        .output()
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to execute tool: {}", e)))?;
    
    if !output.status.success() {
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("Tool returned error: {}", String::from_utf8_lossy(&output.stderr)),
        ));
    }
    
    let schema_str = String::from_utf8(output.stdout)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Invalid UTF-8 in schema: {}", e)))?;
    
    let schema: serde_json::Value = serde_json::from_str(&schema_str)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Invalid JSON schema: {}", e)))?;
    
    let name = schema["name"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Schema missing 'name' field"))?
        .to_string();
    
    let description = schema["description"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Schema missing 'description' field"))?
        .to_string();
    
    let parameters = schema["parameters"].clone();
    
    Ok(Tool {
        name,
        description,
        parameters,
        path: path.clone(),
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

/// Execute a tool with the given arguments (as JSON)
pub fn execute_tool(tool: &Tool, arguments: &serde_json::Value) -> io::Result<String> {
    let mut child = Command::new(&tool.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to spawn tool: {}", e)))?;
    
    // Write arguments as JSON to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let json_str = serde_json::to_string(arguments)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to serialize arguments: {}", e)))?;
        stdin.write_all(json_str.as_bytes())?;
    }
    
    let output = child.wait_with_output()
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to wait for tool: {}", e)))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("Tool execution failed: {}", stderr),
        ));
    }
    
    String::from_utf8(output.stdout)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Invalid UTF-8 in tool output: {}", e)))
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
    prompts_dir: &PathBuf,
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

    Ok(format!("Reflection updated successfully ({} characters).", content.len()))
}
