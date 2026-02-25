//!
//! shell tools: OS command execution.
//! Callers must fire PreShellExec hook before invoking.

use std::io;
use std::path::Path;

use super::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::json_ext::JsonExt;

// === Tool Name Constants ===

pub const SHELL_EXEC_TOOL_NAME: &str = "shell_exec";

// === Tool Definition Registry ===

/// All shell tool definitions
pub static SHELL_TOOL_DEFS: &[BuiltinToolDef] = &[BuiltinToolDef {
    name: SHELL_EXEC_TOOL_NAME,
    description: "Execute a shell command and return stdout, stderr, exit code, and whether it timed out. Commands run via `sh -c`. Use for build, test, and general shell tasks.",
    properties: &[
        ToolPropertyDef {
            name: "command",
            prop_type: "string",
            description: "Shell command to execute",
            default: None,
        },
        ToolPropertyDef {
            name: "timeout_secs",
            prop_type: "integer",
            description: "Timeout in seconds before the process is killed (default: 30)",
            default: Some(30),
        },
    ],
    required: &["command"],
    summary_params: &["command"],
}];

// === Registry Helpers ===

/// Convert all shell tools to API format
pub fn all_shell_tools_to_api_format() -> Vec<serde_json::Value> {
    SHELL_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the shell group
pub fn is_shell_tool(name: &str) -> bool {
    name == SHELL_EXEC_TOOL_NAME
}

// === Tool Execution ===

/// Execute a shell tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
/// Note: permission gating (PreShellExec hook) must be applied by the caller.
pub async fn execute_shell_tool(
    tool_name: &str,
    args: &serde_json::Value,
    project_root: &Path,
) -> Option<io::Result<String>> {
    match tool_name {
        SHELL_EXEC_TOOL_NAME => Some(execute_shell_exec(args, project_root).await),
        _ => None,
    }
}

// === shell_exec implementation ===

/// Execute shell_exec: run a command with a timeout and return structured JSON output.
pub async fn execute_shell_exec(
    args: &serde_json::Value,
    project_root: &Path,
) -> io::Result<String> {
    use tokio::time::{Duration, timeout};

    let command = require_str_param(args, "command")?;
    let timeout_secs = args.get_u64_or("timeout_secs", 30);

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to spawn command: {}", e)))?;

    let result = timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await;

    let (stdout, stderr, exit_code, timed_out) = match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let exit_code = output.status.code().unwrap_or(-1);
            (stdout, stderr, exit_code, false)
        }
        Ok(Err(e)) => {
            return Err(io::Error::new(
                e.kind(),
                format!("Command wait failed: {}", e),
            ));
        }
        Err(_elapsed) => {
            // Timeout expired — child process is dropped, sending SIGKILL on unix.
            (String::new(), String::new(), -1, true)
        }
    };

    let output = serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "timed_out": timed_out,
    });

    Ok(output.to_string())
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    #[test]
    fn test_shell_tool_defs_api_format() {
        for def in SHELL_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
        }
    }

    #[test]
    fn test_is_shell_tool() {
        assert!(is_shell_tool(SHELL_EXEC_TOOL_NAME));
        assert!(!is_shell_tool("file_head"));
        assert!(!is_shell_tool("fetch_url"));
    }

    #[test]
    fn test_tool_constant() {
        assert_eq!(SHELL_EXEC_TOOL_NAME, "shell_exec");
    }

    #[tokio::test]
    async fn test_shell_exec_basic() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("command", serde_json::json!("echo hello"))]);
        let result = execute_shell_exec(&a, dir.path()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["stdout"].as_str().unwrap().trim(), "hello");
        assert_eq!(parsed["exit_code"], 0);
        assert_eq!(parsed["timed_out"], false);
    }

    #[tokio::test]
    async fn test_shell_exec_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[
            ("command", serde_json::json!("sleep 10")),
            ("timeout_secs", serde_json::json!(1)),
        ]);
        let result = execute_shell_exec(&a, dir.path()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["timed_out"], true);
    }

    #[tokio::test]
    async fn test_shell_exec_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("command", serde_json::json!("exit 42"))]);
        let result = execute_shell_exec(&a, dir.path()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["exit_code"], 42);
        assert_eq!(parsed["timed_out"], false);
    }

    #[tokio::test]
    async fn test_shell_exec_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("command", serde_json::json!("echo error >&2"))]);
        let result = execute_shell_exec(&a, dir.path()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["stderr"].as_str().unwrap().trim(), "error");
    }

    #[tokio::test]
    async fn test_shell_exec_uses_project_root() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("command", serde_json::json!("pwd"))]);
        let result = execute_shell_exec(&a, dir.path()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let stdout = parsed["stdout"].as_str().unwrap().trim();
        let expected = dir.path().canonicalize().unwrap();
        let actual = PathBuf::from(stdout).canonicalize().unwrap();
        assert_eq!(actual, expected);
    }
}
