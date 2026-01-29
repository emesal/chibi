//! Plugin loading and execution.
//!
//! Plugins are executable scripts in the plugins directory that provide tools for the LLM.
//! They output JSON schema when called with --schema and receive arguments via CHIBI_TOOL_ARGS.

use super::Tool;
use super::hooks::HookPoint;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Load all tools from the plugins directory by calling each with --schema
pub fn load_tools(plugins_dir: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let mut tools = Vec::new();

    if !plugins_dir.exists() {
        return Ok(tools);
    }

    // Canonicalize plugins directory for path traversal protection
    let plugins_dir_canonical = plugins_dir.canonicalize()?;

    let entries = fs::read_dir(plugins_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

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

        // Security: Verify the executable path is within the plugins directory
        // This prevents symlink attacks that could escape the plugins directory.
        // We store and use the canonical path to prevent TOCTOU attacks where
        // a symlink could be modified between verification and execution.
        let canonical_exec = match exec_path.canonicalize() {
            Ok(canonical) => {
                if !canonical.starts_with(&plugins_dir_canonical) {
                    if verbose {
                        eprintln!(
                            "[WARN] Skipping plugin outside plugins directory: {:?}",
                            exec_path
                        );
                    }
                    continue;
                }
                canonical
            }
            Err(e) => {
                if verbose {
                    eprintln!("[WARN] Cannot verify plugin path {:?}: {}", exec_path, e);
                }
                continue;
            }
        };

        // Check if executable (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = canonical_exec.metadata()
                && metadata.permissions().mode() & 0o111 == 0
            {
                continue; // Not executable
            }
        }

        // Try to get schema(s) from the tool (using canonical path)
        match get_tool_schemas(&canonical_exec, verbose) {
            Ok(new_tools) => tools.extend(new_tools),
            Err(e) => {
                if verbose {
                    eprintln!(
                        "[WARN] Failed to load tool {:?}: {}",
                        exec_path.file_name(),
                        e
                    );
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
    path: &Path,
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
                match hook_str.parse::<HookPoint>() {
                    Ok(hook) => Some(hook),
                    Err(_) => {
                        if verbose {
                            eprintln!("[WARN] Unknown hook '{}' in tool '{}'", hook_str, name);
                        }
                        None
                    }
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Tool {
        name,
        description,
        parameters,
        path: path.to_path_buf(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
}
