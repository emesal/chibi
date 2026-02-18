//! File access tools for examining cached tool outputs and files.
//!
//! These tools provide surgical access to cached tool outputs, allowing the LLM
//! to examine large outputs without overwhelming the context window.

use super::builtin::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::cache;
use crate::config::ResolvedConfig;
use crate::json_ext::JsonExt;
use crate::state::{AppState, StatePaths};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

// === Tool Name Constants ===

pub const FILE_HEAD_TOOL_NAME: &str = "file_head";
pub const FILE_TAIL_TOOL_NAME: &str = "file_tail";
pub const FILE_LINES_TOOL_NAME: &str = "file_lines";
pub const FILE_GREP_TOOL_NAME: &str = "file_grep";
pub const CACHE_LIST_TOOL_NAME: &str = "cache_list";
pub const WRITE_FILE_TOOL_NAME: &str = "write_file";

// === Tool Definition Registry ===

/// All file tool definitions
pub static FILE_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: FILE_HEAD_TOOL_NAME,
        description: "Read the first N lines from a cached tool output or file. Use this to examine the beginning of large outputs.",
        properties: &[
            ToolPropertyDef {
                name: "cache_id",
                prop_type: "string",
                description: "ID of a cached tool output (from [Output cached: ID] messages)",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to a file (if not using cache_id)",
                default: None,
            },
            ToolPropertyDef {
                name: "lines",
                prop_type: "integer",
                description: "Number of lines to read (default: 50)",
                default: Some(50),
            },
        ],
        required: &[],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_TAIL_TOOL_NAME,
        description: "Read the last N lines from a cached tool output or file. Use this to examine the end of large outputs.",
        properties: &[
            ToolPropertyDef {
                name: "cache_id",
                prop_type: "string",
                description: "ID of a cached tool output (from [Output cached: ID] messages)",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to a file (if not using cache_id)",
                default: None,
            },
            ToolPropertyDef {
                name: "lines",
                prop_type: "integer",
                description: "Number of lines to read (default: 50)",
                default: Some(50),
            },
        ],
        required: &[],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_LINES_TOOL_NAME,
        description: "Read a specific range of lines from a cached tool output or file. Lines are 1-indexed.",
        properties: &[
            ToolPropertyDef {
                name: "cache_id",
                prop_type: "string",
                description: "ID of a cached tool output (from [Output cached: ID] messages)",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to a file (if not using cache_id)",
                default: None,
            },
            ToolPropertyDef {
                name: "start",
                prop_type: "integer",
                description: "First line number (1-indexed)",
                default: None,
            },
            ToolPropertyDef {
                name: "end",
                prop_type: "integer",
                description: "Last line number (1-indexed, inclusive)",
                default: None,
            },
        ],
        required: &["start", "end"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_GREP_TOOL_NAME,
        description: "Search for a pattern in a cached tool output or file. Returns matching lines with optional context.",
        properties: &[
            ToolPropertyDef {
                name: "cache_id",
                prop_type: "string",
                description: "ID of a cached tool output (from [Output cached: ID] messages)",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to a file (if not using cache_id)",
                default: None,
            },
            ToolPropertyDef {
                name: "pattern",
                prop_type: "string",
                description: "Regular expression pattern to search for",
                default: None,
            },
            ToolPropertyDef {
                name: "context_before",
                prop_type: "integer",
                description: "Number of lines to show before each match (default: 2)",
                default: Some(2),
            },
            ToolPropertyDef {
                name: "context_after",
                prop_type: "integer",
                description: "Number of lines to show after each match (default: 2)",
                default: Some(2),
            },
        ],
        required: &["pattern"],
        summary_params: &["pattern", "path"],
    },
    BuiltinToolDef {
        name: CACHE_LIST_TOOL_NAME,
        description: "List all cached tool outputs for this context. Shows cache IDs, tool names, sizes, and timestamps.",
        properties: &[],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: WRITE_FILE_TOOL_NAME,
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Requires user permission.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to write to",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "Content to write to the file",
                default: None,
            },
        ],
        required: &["path", "content"],
        summary_params: &["path"],
    },
];

/// Convert all file tools to API format
pub fn all_file_tools_to_api_format() -> Vec<serde_json::Value> {
    FILE_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Look up a specific file tool definition by name.
/// Returns None if not found. Use this for testing or conditional tool access.
pub fn get_file_tool_def(name: &str) -> Option<&'static BuiltinToolDef> {
    FILE_TOOL_DEFS.iter().find(|d| d.name == name)
}

// === Path Resolution ===

/// Resolve a file path from either cache_id or path argument.
///
/// When a relative `path` is provided, it is resolved against `project_root`
/// before validation so that tools work correctly even when the process CWD
/// differs from the project root.
fn resolve_file_path(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<PathBuf> {
    let cache_id = args.get_str("cache_id");
    let path = args.get_str("path");

    match (cache_id, path) {
        (Some(id), None) => {
            // Resolve from cache
            let cache_dir = app.tool_cache_dir(context_name);
            cache::resolve_cache_path(&cache_dir, id)
        }
        (None, Some(p)) => {
            // Resolve relative paths against project_root before validation
            let resolved_path = if Path::new(p).is_relative() {
                project_root.join(p).to_string_lossy().to_string()
            } else {
                p.to_string()
            };
            let resolved = super::security::validate_file_path(&resolved_path, config)?;
            Ok(resolved)
        }
        (Some(_), Some(_)) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Provide either cache_id or path, not both",
        )),
        (None, None) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Must provide either cache_id or path",
        )),
    }
}

// === Tool Execution ===

// --- Helpers for parameter extraction ---

/// Extract a required u64 parameter, returning a helpful error if missing.
/// Use this pattern for required numeric params in future file tools.
fn require_u64_param(args: &serde_json::Value, name: &str) -> io::Result<u64> {
    args.get_u64(name).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Missing '{}' parameter", name),
        )
    })
}

// --- Head/Tail shared implementation ---

/// Direction for head/tail reading
enum ReadDirection {
    Head,
    Tail,
}

/// Shared implementation for file_head and file_tail.
/// When adding similar "read N lines from start/end" tools, follow this pattern.
fn execute_file_head_or_tail(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    direction: ReadDirection,
    project_root: &Path,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config, project_root)?;
    let lines = args.get_u64_or("lines", 50) as usize;

    match direction {
        ReadDirection::Head => cache::read_cache_head(&path, lines),
        ReadDirection::Tail => cache::read_cache_tail(&path, lines),
    }
}

/// Execute file_head tool
pub fn execute_file_head(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    execute_file_head_or_tail(
        app,
        context_name,
        args,
        config,
        ReadDirection::Head,
        project_root,
    )
}

/// Execute file_tail tool
pub fn execute_file_tail(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    execute_file_head_or_tail(
        app,
        context_name,
        args,
        config,
        ReadDirection::Tail,
        project_root,
    )
}

/// Execute file_lines tool
pub fn execute_file_lines(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config, project_root)?;
    let start = require_u64_param(args, "start")? as usize;
    let end = require_u64_param(args, "end")? as usize;

    // Validation specific to line ranges
    if start == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Line numbers are 1-indexed, start must be >= 1",
        ));
    }
    if end < start {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "End line must be >= start line",
        ));
    }

    cache::read_cache_lines(&path, start, end)
}

/// Execute file_grep tool
pub fn execute_file_grep(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config, project_root)?;
    let pattern = require_str_param(args, "pattern")?;
    let context_before = args.get_u64_or("context_before", 2) as usize;
    let context_after = args.get_u64_or("context_after", 2) as usize;

    let result = cache::read_cache_grep(&path, &pattern, context_before, context_after)?;

    if result.is_empty() {
        Ok(format!("No matches found for pattern: {}", pattern))
    } else {
        Ok(result)
    }
}

/// Execute cache_list tool
pub fn execute_cache_list(app: &AppState, context_name: &str) -> io::Result<String> {
    let cache_dir = app.tool_cache_dir(context_name);
    let entries = cache::list_cache_entries(&cache_dir)?;

    if entries.is_empty() {
        return Ok("No cached outputs found.".to_string());
    }

    let mut output = String::from("Cached tool outputs:\n");
    for entry in entries {
        // Format timestamp
        let timestamp = chrono::DateTime::from_timestamp(entry.timestamp as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        output.push_str(&format!(
            "\n  {} ({}):\n    Tool: {}\n    Size: {} chars (~{} tokens), {} lines\n    Cached: {}\n",
            entry.id,
            entry.tool_name,
            entry.tool_name,
            entry.char_count,
            entry.token_estimate,
            entry.line_count,
            timestamp
        ));
    }

    Ok(output)
}

/// Execute write_file tool
///
/// Note: Permission check via pre_file_write hook happens in send.rs before this is called.
pub fn execute_write_file(path: &str, content: &str) -> io::Result<String> {
    let path = PathBuf::from(path);

    // Create parent directories if needed
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    crate::safe_io::atomic_write_text(&path, content)?;

    Ok(format!(
        "File written successfully: {} ({} bytes)",
        path.display(),
        content.len()
    ))
}

/// Check if a tool name is a file tool
pub fn is_file_tool(name: &str) -> bool {
    matches!(
        name,
        FILE_HEAD_TOOL_NAME
            | FILE_TAIL_TOOL_NAME
            | FILE_LINES_TOOL_NAME
            | FILE_GREP_TOOL_NAME
            | CACHE_LIST_TOOL_NAME
            | WRITE_FILE_TOOL_NAME
    )
}

/// Execute a file tool by name.
///
/// `project_root` is used to resolve relative paths in file read tools.
/// `CACHE_LIST_TOOL_NAME` and `WRITE_FILE_TOOL_NAME` do not use it.
pub fn execute_file_tool(
    app: &AppState,
    context_name: &str,
    tool_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> Option<io::Result<String>> {
    match tool_name {
        FILE_HEAD_TOOL_NAME => Some(execute_file_head(
            app,
            context_name,
            args,
            config,
            project_root,
        )),
        FILE_TAIL_TOOL_NAME => Some(execute_file_tail(
            app,
            context_name,
            args,
            config,
            project_root,
        )),
        FILE_LINES_TOOL_NAME => Some(execute_file_lines(
            app,
            context_name,
            args,
            config,
            project_root,
        )),
        FILE_GREP_TOOL_NAME => Some(execute_file_grep(
            app,
            context_name,
            args,
            config,
            project_root,
        )),
        CACHE_LIST_TOOL_NAME => Some(execute_cache_list(app, context_name)),
        WRITE_FILE_TOOL_NAME => {
            let path = require_str_param(args, "path");
            let content = require_str_param(args, "content");
            match (path, content) {
                (Ok(p), Ok(c)) => Some(execute_write_file(&p, &c)),
                (Err(e), _) | (_, Err(e)) => Some(Err(e)),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Helper for tests: get API format for a specific tool ===
    fn get_tool_api(name: &str) -> serde_json::Value {
        get_file_tool_def(name)
            .expect("tool should exist in registry")
            .to_api_format()
    }

    // === Individual Tool Tests (using registry lookup) ===

    #[test]
    fn test_file_head_tool_api_format() {
        let tool = get_tool_api(FILE_HEAD_TOOL_NAME);
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], FILE_HEAD_TOOL_NAME);
    }

    #[test]
    fn test_file_tail_tool_api_format() {
        let tool = get_tool_api(FILE_TAIL_TOOL_NAME);
        assert_eq!(tool["function"]["name"], FILE_TAIL_TOOL_NAME);
    }

    #[test]
    fn test_file_lines_tool_api_format() {
        let tool = get_tool_api(FILE_LINES_TOOL_NAME);
        assert_eq!(tool["function"]["name"], FILE_LINES_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("start")));
        assert!(required.contains(&serde_json::json!("end")));
    }

    #[test]
    fn test_file_grep_tool_api_format() {
        let tool = get_tool_api(FILE_GREP_TOOL_NAME);
        assert_eq!(tool["function"]["name"], FILE_GREP_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }

    #[test]
    fn test_cache_list_tool_api_format() {
        let tool = get_tool_api(CACHE_LIST_TOOL_NAME);
        assert_eq!(tool["function"]["name"], CACHE_LIST_TOOL_NAME);
    }

    #[test]
    fn test_is_file_tool() {
        assert!(is_file_tool(FILE_HEAD_TOOL_NAME));
        assert!(is_file_tool(FILE_TAIL_TOOL_NAME));
        assert!(is_file_tool(FILE_LINES_TOOL_NAME));
        assert!(is_file_tool(FILE_GREP_TOOL_NAME));
        assert!(is_file_tool(CACHE_LIST_TOOL_NAME));
        assert!(is_file_tool(WRITE_FILE_TOOL_NAME));
        assert!(!is_file_tool("other_tool"));
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(FILE_HEAD_TOOL_NAME, "file_head");
        assert_eq!(FILE_TAIL_TOOL_NAME, "file_tail");
        assert_eq!(FILE_LINES_TOOL_NAME, "file_lines");
        assert_eq!(FILE_GREP_TOOL_NAME, "file_grep");
        assert_eq!(CACHE_LIST_TOOL_NAME, "cache_list");
        assert_eq!(WRITE_FILE_TOOL_NAME, "write_file");
    }

    // === Registry Tests ===

    #[test]
    fn test_file_tool_registry_contains_all_tools() {
        assert_eq!(FILE_TOOL_DEFS.len(), 6);
        let names: Vec<_> = FILE_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&FILE_HEAD_TOOL_NAME));
        assert!(names.contains(&FILE_TAIL_TOOL_NAME));
        assert!(names.contains(&FILE_LINES_TOOL_NAME));
        assert!(names.contains(&FILE_GREP_TOOL_NAME));
        assert!(names.contains(&CACHE_LIST_TOOL_NAME));
        assert!(names.contains(&WRITE_FILE_TOOL_NAME));
    }

    #[test]
    fn test_get_file_tool_def() {
        assert!(get_file_tool_def(FILE_HEAD_TOOL_NAME).is_some());
        assert!(get_file_tool_def("nonexistent_tool").is_none());
    }

    #[test]
    fn test_all_file_tools_to_api_format() {
        let tools = all_file_tools_to_api_format();
        assert_eq!(tools.len(), 6);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].is_string());
        }
    }

    #[test]
    fn test_write_file_tool_api_format() {
        let tool = get_tool_api(WRITE_FILE_TOOL_NAME);
        assert_eq!(tool["function"]["name"], WRITE_FILE_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("path")));
        assert!(required.contains(&serde_json::json!("content")));
    }

    #[test]
    fn test_file_tool_defaults() {
        // file_head/file_tail have lines default: 50
        let head = get_tool_api(FILE_HEAD_TOOL_NAME);
        assert_eq!(
            head["function"]["parameters"]["properties"]["lines"]["default"],
            50
        );

        let tail = get_tool_api(FILE_TAIL_TOOL_NAME);
        assert_eq!(
            tail["function"]["parameters"]["properties"]["lines"]["default"],
            50
        );

        // file_grep has context defaults: 2
        let grep = get_tool_api(FILE_GREP_TOOL_NAME);
        assert_eq!(
            grep["function"]["parameters"]["properties"]["context_before"]["default"],
            2
        );
        assert_eq!(
            grep["function"]["parameters"]["properties"]["context_after"]["default"],
            2
        );
    }

    #[test]
    fn test_file_tool_required_fields() {
        // file_lines requires start and end
        let lines = get_tool_api(FILE_LINES_TOOL_NAME);
        let required = lines["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&serde_json::json!("start")));
        assert!(required.contains(&serde_json::json!("end")));

        // file_grep requires pattern
        let grep = get_tool_api(FILE_GREP_TOOL_NAME);
        let required = grep["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert_eq!(required.len(), 1);
        assert!(required.contains(&serde_json::json!("pattern")));

        // cache_list has no required
        let cache = get_tool_api(CACHE_LIST_TOOL_NAME);
        let required = cache["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_execute_write_file_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");

        let result = super::execute_write_file(path.to_str().unwrap(), "hello world");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("written successfully"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_execute_write_file_creates_parent_dirs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("nested/dir/test.txt");

        let result = super::execute_write_file(path.to_str().unwrap(), "content");
        assert!(result.is_ok());
        assert!(path.exists());
    }

    #[test]
    fn test_execute_write_file_overwrites() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");
        std::fs::write(&path, "old content").unwrap();

        let result = super::execute_write_file(path.to_str().unwrap(), "new content");
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    // === resolve_file_path project_root tests ===

    use crate::config::{ApiParams, ToolsConfig};
    use crate::partition::StorageConfig;
    use std::collections::BTreeMap;

    fn make_test_config(allowed_paths: Vec<String>) -> ResolvedConfig {
        ResolvedConfig {
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            context_window_limit: 128000,
            warn_threshold_percent: 0.8,
            verbose: false,
            hide_tool_calls: false,
            no_tool_calls: false,
            show_thinking: false,
            auto_compact: false,
            auto_compact_threshold: 0.9,
            fuel: 5,
            fuel_empty_response_cost: 15,
            username: "user".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 5000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: allowed_paths,
            api: ApiParams::default(),
            tools: ToolsConfig::default(),
            fallback_tool: "call_agent".to_string(),
            storage: StorageConfig::default(),
            url_policy: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn test_resolve_file_path_relative_uses_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project_dir.path().join("docs")).unwrap();
        std::fs::write(project_dir.path().join("docs/readme.md"), "hello world").unwrap();

        let config = make_test_config(vec![project_dir.path().to_string_lossy().to_string()]);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let args = serde_json::json!({"path": "docs/readme.md"});
        let result = resolve_file_path(&app, "test", &args, &config, project_dir.path());
        assert!(
            result.is_ok(),
            "relative path should resolve against project_root: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_resolve_file_path_absolute_ignores_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let file = other_dir.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();

        let config = make_test_config(vec![other_dir.path().to_string_lossy().to_string()]);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let args = serde_json::json!({"path": file.to_string_lossy().to_string()});
        let result = resolve_file_path(&app, "test", &args, &config, project_dir.path());
        assert!(
            result.is_ok(),
            "absolute path should resolve without project_root: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap(), file.canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_file_path_relative_outside_allowed_fails() {
        let project_dir = tempfile::tempdir().unwrap();
        let allowed_dir = tempfile::tempdir().unwrap();
        // Create file under project_dir but allowed_dir is different
        std::fs::write(project_dir.path().join("secret.txt"), "secret").unwrap();

        let config = make_test_config(vec![allowed_dir.path().to_string_lossy().to_string()]);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let args = serde_json::json!({"path": "secret.txt"});
        let result = resolve_file_path(&app, "test", &args, &config, project_dir.path());
        assert!(result.is_err(), "path outside allowed dirs should fail");
    }

    // === Integration tests: full execute path with project_root ===

    #[test]
    fn test_execute_file_head_with_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        let file = project_dir.path().join("test.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let config = make_test_config(vec![project_dir.path().to_string_lossy().to_string()]);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let args = serde_json::json!({"path": "test.txt", "lines": 2});
        let result = execute_file_head(&app, "test", &args, &config, project_dir.path());
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
    }

    #[test]
    fn test_execute_file_grep_with_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        let file = project_dir.path().join("code.rs");
        std::fs::write(&file, "fn hello() {\n    println!(\"world\");\n}\n").unwrap();

        let config = make_test_config(vec![project_dir.path().to_string_lossy().to_string()]);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let args = serde_json::json!({"path": "code.rs", "pattern": "println"});
        let result = execute_file_grep(&app, "test", &args, &config, project_dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("println"));
    }
}
