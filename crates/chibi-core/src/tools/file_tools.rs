//! File access tools for examining cached tool outputs and files.
//!
//! These tools provide surgical access to cached tool outputs, allowing the LLM
//! to examine large outputs without overwhelming the context window.

use super::builtin::{BuiltinToolDef, ToolPropertyDef};
use crate::cache;
use crate::config::ResolvedConfig;
use crate::json_ext::JsonExt;
use crate::state::{AppState, StatePaths};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

// === Tool Name Constants ===

pub const FILE_HEAD_TOOL_NAME: &str = "file_head";
pub const FILE_TAIL_TOOL_NAME: &str = "file_tail";
pub const FILE_LINES_TOOL_NAME: &str = "file_lines";
pub const FILE_GREP_TOOL_NAME: &str = "file_grep";
pub const CACHE_LIST_TOOL_NAME: &str = "cache_list";

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
    },
    BuiltinToolDef {
        name: CACHE_LIST_TOOL_NAME,
        description: "List all cached tool outputs for this context. Shows cache IDs, tool names, sizes, and timestamps.",
        properties: &[],
        required: &[],
    },
];

/// Convert all file tools to API format
pub fn all_file_tools_to_api_format() -> Vec<serde_json::Value> {
    FILE_TOOL_DEFS.iter().map(|def| def.to_api_format()).collect()
}

// === Legacy Tool API Format Functions (thin wrappers for backwards compatibility) ===

/// Legacy wrapper - delegates to registry
pub fn file_head_tool_to_api_format() -> serde_json::Value {
    FILE_TOOL_DEFS.iter()
        .find(|d| d.name == FILE_HEAD_TOOL_NAME)
        .unwrap()
        .to_api_format()
}

/// Legacy wrapper - delegates to registry
pub fn file_tail_tool_to_api_format() -> serde_json::Value {
    FILE_TOOL_DEFS.iter()
        .find(|d| d.name == FILE_TAIL_TOOL_NAME)
        .unwrap()
        .to_api_format()
}

/// Legacy wrapper - delegates to registry
pub fn file_lines_tool_to_api_format() -> serde_json::Value {
    FILE_TOOL_DEFS.iter()
        .find(|d| d.name == FILE_LINES_TOOL_NAME)
        .unwrap()
        .to_api_format()
}

/// Legacy wrapper - delegates to registry
pub fn file_grep_tool_to_api_format() -> serde_json::Value {
    FILE_TOOL_DEFS.iter()
        .find(|d| d.name == FILE_GREP_TOOL_NAME)
        .unwrap()
        .to_api_format()
}

/// Legacy wrapper - delegates to registry
pub fn cache_list_tool_to_api_format() -> serde_json::Value {
    FILE_TOOL_DEFS.iter()
        .find(|d| d.name == CACHE_LIST_TOOL_NAME)
        .unwrap()
        .to_api_format()
}

// === Path Resolution ===

/// Resolve a file path from either cache_id or path argument
fn resolve_file_path(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
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
            // Validate path against allowed paths
            let resolved = resolve_and_validate_path(p, config)?;
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

/// Resolve and validate a file path against allowed paths
fn resolve_and_validate_path(path: &str, config: &ResolvedConfig) -> io::Result<PathBuf> {
    let resolved = if let Some(rest) = path.strip_prefix("~/") {
        // Expand home directory with path suffix
        let home = dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?;
        home.join(rest)
    } else if path == "~" {
        // Bare ~ means home directory itself
        dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?
    } else {
        PathBuf::from(path)
    };

    let canonical = resolved.canonicalize().map_err(|e| {
        io::Error::new(
            ErrorKind::NotFound,
            format!("Could not resolve path '{}': {}", path, e),
        )
    })?;

    // If file_tools_allowed_paths is empty, reject all file paths
    // (only cache_id is allowed)
    if config.file_tools_allowed_paths.is_empty() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "File path access is not allowed. Use cache_id to access cached tool outputs, or configure file_tools_allowed_paths.",
        ));
    }

    // Check if path is under any allowed path
    let allowed = config.file_tools_allowed_paths.iter().any(|allowed_path| {
        let allowed_resolved = if let Some(rest) = allowed_path.strip_prefix("~/") {
            dirs_next::home_dir().map(|home| home.join(rest))
        } else if allowed_path == "~" {
            dirs_next::home_dir()
        } else {
            Some(PathBuf::from(allowed_path))
        };

        allowed_resolved
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|allowed_canonical| canonical.starts_with(&allowed_canonical))
    });

    if !allowed {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "Path '{}' is not under any allowed path. Allowed: {:?}",
                path, config.file_tools_allowed_paths
            ),
        ));
    }

    Ok(canonical)
}

// === Tool Execution ===

/// Execute file_head tool
pub fn execute_file_head(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config)?;
    let lines = args.get_u64_or("lines", 50) as usize;

    cache::read_cache_head(&path, lines)
}

/// Execute file_tail tool
pub fn execute_file_tail(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config)?;
    let lines = args.get_u64_or("lines", 50) as usize;

    cache::read_cache_tail(&path, lines)
}

/// Execute file_lines tool
pub fn execute_file_lines(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config)?;

    let start = args
        .get_u64("start")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing 'start' parameter"))?
        as usize;

    let end = args
        .get_u64("end")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing 'end' parameter"))?
        as usize;

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
) -> io::Result<String> {
    let path = resolve_file_path(app, context_name, args, config)?;

    let pattern = args
        .get_str("pattern")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing 'pattern' parameter"))?;

    let context_before = args.get_u64_or("context_before", 2) as usize;
    let context_after = args.get_u64_or("context_after", 2) as usize;

    let result = cache::read_cache_grep(&path, pattern, context_before, context_after)?;

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

/// Check if a tool name is a file tool
pub fn is_file_tool(name: &str) -> bool {
    matches!(
        name,
        FILE_HEAD_TOOL_NAME
            | FILE_TAIL_TOOL_NAME
            | FILE_LINES_TOOL_NAME
            | FILE_GREP_TOOL_NAME
            | CACHE_LIST_TOOL_NAME
    )
}

/// Execute a file tool by name
pub fn execute_file_tool(
    app: &AppState,
    context_name: &str,
    tool_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
) -> Option<io::Result<String>> {
    match tool_name {
        FILE_HEAD_TOOL_NAME => Some(execute_file_head(app, context_name, args, config)),
        FILE_TAIL_TOOL_NAME => Some(execute_file_tail(app, context_name, args, config)),
        FILE_LINES_TOOL_NAME => Some(execute_file_lines(app, context_name, args, config)),
        FILE_GREP_TOOL_NAME => Some(execute_file_grep(app, context_name, args, config)),
        CACHE_LIST_TOOL_NAME => Some(execute_cache_list(app, context_name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_head_tool_api_format() {
        let tool = file_head_tool_to_api_format();
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], FILE_HEAD_TOOL_NAME);
    }

    #[test]
    fn test_file_tail_tool_api_format() {
        let tool = file_tail_tool_to_api_format();
        assert_eq!(tool["function"]["name"], FILE_TAIL_TOOL_NAME);
    }

    #[test]
    fn test_file_lines_tool_api_format() {
        let tool = file_lines_tool_to_api_format();
        assert_eq!(tool["function"]["name"], FILE_LINES_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("start")));
        assert!(required.contains(&serde_json::json!("end")));
    }

    #[test]
    fn test_file_grep_tool_api_format() {
        let tool = file_grep_tool_to_api_format();
        assert_eq!(tool["function"]["name"], FILE_GREP_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }

    #[test]
    fn test_cache_list_tool_api_format() {
        let tool = cache_list_tool_to_api_format();
        assert_eq!(tool["function"]["name"], CACHE_LIST_TOOL_NAME);
    }

    #[test]
    fn test_is_file_tool() {
        assert!(is_file_tool(FILE_HEAD_TOOL_NAME));
        assert!(is_file_tool(FILE_TAIL_TOOL_NAME));
        assert!(is_file_tool(FILE_LINES_TOOL_NAME));
        assert!(is_file_tool(FILE_GREP_TOOL_NAME));
        assert!(is_file_tool(CACHE_LIST_TOOL_NAME));
        assert!(!is_file_tool("other_tool"));
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(FILE_HEAD_TOOL_NAME, "file_head");
        assert_eq!(FILE_TAIL_TOOL_NAME, "file_tail");
        assert_eq!(FILE_LINES_TOOL_NAME, "file_lines");
        assert_eq!(FILE_GREP_TOOL_NAME, "file_grep");
        assert_eq!(CACHE_LIST_TOOL_NAME, "cache_list");
    }

    // === Registry Tests ===

    #[test]
    fn test_file_tool_registry_contains_all_tools() {
        assert_eq!(FILE_TOOL_DEFS.len(), 5);
        let names: Vec<_> = FILE_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&FILE_HEAD_TOOL_NAME));
        assert!(names.contains(&FILE_TAIL_TOOL_NAME));
        assert!(names.contains(&FILE_LINES_TOOL_NAME));
        assert!(names.contains(&FILE_GREP_TOOL_NAME));
        assert!(names.contains(&CACHE_LIST_TOOL_NAME));
    }

    #[test]
    fn test_all_file_tools_to_api_format() {
        let tools = all_file_tools_to_api_format();
        assert_eq!(tools.len(), 5);
        for tool in &tools {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].is_string());
        }
    }

    #[test]
    fn test_file_tool_defaults() {
        // file_head/file_tail have lines default: 50
        let head = file_head_tool_to_api_format();
        assert_eq!(head["function"]["parameters"]["properties"]["lines"]["default"], 50);

        let tail = file_tail_tool_to_api_format();
        assert_eq!(tail["function"]["parameters"]["properties"]["lines"]["default"], 50);

        // file_grep has context defaults: 2
        let grep = file_grep_tool_to_api_format();
        assert_eq!(grep["function"]["parameters"]["properties"]["context_before"]["default"], 2);
        assert_eq!(grep["function"]["parameters"]["properties"]["context_after"]["default"], 2);
    }

    #[test]
    fn test_file_tool_required_fields() {
        // file_lines requires start and end
        let lines = file_lines_tool_to_api_format();
        let required = lines["function"]["parameters"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&serde_json::json!("start")));
        assert!(required.contains(&serde_json::json!("end")));

        // file_grep requires pattern
        let grep = file_grep_tool_to_api_format();
        let required = grep["function"]["parameters"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert!(required.contains(&serde_json::json!("pattern")));

        // cache_list has no required
        let cache = cache_list_tool_to_api_format();
        let required = cache["function"]["parameters"]["required"].as_array().unwrap();
        assert!(required.is_empty());
    }
}
