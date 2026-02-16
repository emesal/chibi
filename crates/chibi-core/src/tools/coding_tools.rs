//! Coding tools for shell execution, file system navigation, file editing, and codebase indexing.
//!
//! These tools equip an LLM with the core primitives it needs to do software
//! development work: run commands, explore directory trees, search by glob or
//! regex, edit files in a structured way, and query a codebase index.
//!
//! All path parameters are resolved relative to `project_root`, which is
//! supplied to the executor from the `CHIBI_PROJECT_ROOT` environment variable.

use std::io::{self, BufRead, ErrorKind};
use std::path::{Path, PathBuf};

use super::builtin::{BuiltinToolDef, ToolPropertyDef};
use crate::json_ext::JsonExt;

// === Tool Name Constants ===

pub const SHELL_EXEC_TOOL_NAME: &str = "shell_exec";
pub const DIR_LIST_TOOL_NAME: &str = "dir_list";
pub const GLOB_FILES_TOOL_NAME: &str = "glob_files";
pub const GREP_FILES_TOOL_NAME: &str = "grep_files";
pub const FILE_EDIT_TOOL_NAME: &str = "file_edit";
pub const INDEX_UPDATE_TOOL_NAME: &str = "index_update";
pub const INDEX_QUERY_TOOL_NAME: &str = "index_query";
pub const INDEX_STATUS_TOOL_NAME: &str = "index_status";
pub const FETCH_URL_TOOL_NAME: &str = "fetch_url";

// === Tool Definition Registry ===

/// All coding tool definitions
pub static CODING_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
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
    },
    BuiltinToolDef {
        name: DIR_LIST_TOOL_NAME,
        description: "List a directory tree with file sizes and type indicators. Respects depth limit. Paths are relative to project_root unless absolute.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Directory to list (default: project root)",
                default: None,
            },
            ToolPropertyDef {
                name: "depth",
                prop_type: "integer",
                description: "Maximum recursion depth (default: 1)",
                default: Some(1),
            },
            ToolPropertyDef {
                name: "show_hidden",
                prop_type: "string",
                description: "Include hidden files and directories (\"true\"/\"false\", default: \"false\")",
                default: None,
            },
        ],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: GLOB_FILES_TOOL_NAME,
        description: "Find files matching a glob pattern under a directory, honouring .gitignore. Returns relative paths, one per line.",
        properties: &[
            ToolPropertyDef {
                name: "pattern",
                prop_type: "string",
                description: "Glob pattern to match (e.g. \"**/*.rs\")",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Root directory to search in (default: \".\")",
                default: None,
            },
            ToolPropertyDef {
                name: "max_results",
                prop_type: "integer",
                description: "Maximum number of paths to return (default: 100)",
                default: Some(100),
            },
        ],
        required: &["pattern"],
        summary_params: &["pattern"],
    },
    BuiltinToolDef {
        name: GREP_FILES_TOOL_NAME,
        description: "Search files for a regex pattern, honouring .gitignore. Output format: `file:line: content` with optional context lines.",
        properties: &[
            ToolPropertyDef {
                name: "pattern",
                prop_type: "string",
                description: "Regular expression pattern to search for",
                default: None,
            },
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Root directory to search in (default: \".\")",
                default: None,
            },
            ToolPropertyDef {
                name: "file_pattern",
                prop_type: "string",
                description: "Optional glob to restrict which files are searched (e.g. \"*.rs\")",
                default: None,
            },
            ToolPropertyDef {
                name: "context_lines",
                prop_type: "integer",
                description: "Lines of context to show before and after each match (default: 2)",
                default: Some(2),
            },
            ToolPropertyDef {
                name: "max_results",
                prop_type: "integer",
                description: "Maximum number of matches to return (default: 50)",
                default: Some(50),
            },
        ],
        required: &["pattern"],
        summary_params: &["pattern", "path"],
    },
    BuiltinToolDef {
        name: FILE_EDIT_TOOL_NAME,
        description: "Edit a file using structured operations: replace_lines, insert_before, insert_after, delete_lines, replace_string. All line numbers are 1-indexed. Writes are atomic.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to the file to edit",
                default: None,
            },
            ToolPropertyDef {
                name: "operation",
                prop_type: "string",
                description: "Edit operation: replace_lines | insert_before | insert_after | delete_lines | replace_string",
                default: None,
            },
            ToolPropertyDef {
                name: "line_start",
                prop_type: "integer",
                description: "First line number (1-indexed, required for line operations)",
                default: None,
            },
            ToolPropertyDef {
                name: "line_end",
                prop_type: "integer",
                description: "Last line number (1-indexed, inclusive, required for replace_lines and delete_lines)",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "Text to insert or use as replacement (required for replace_lines, insert_before, insert_after)",
                default: None,
            },
            ToolPropertyDef {
                name: "find",
                prop_type: "string",
                description: "Exact string to find (required for replace_string)",
                default: None,
            },
            ToolPropertyDef {
                name: "replace",
                prop_type: "string",
                description: "Replacement string (required for replace_string)",
                default: None,
            },
        ],
        required: &["path", "operation"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: INDEX_UPDATE_TOOL_NAME,
        description: "Trigger a codebase index update. Walks the project tree, detects changed files, and dispatches to language plugins for symbol extraction. Returns a summary of what was indexed.",
        properties: &[ToolPropertyDef {
            name: "force",
            prop_type: "string",
            description: "Force re-index of all files, ignoring change detection (\"true\"/\"false\", default: \"false\")",
            default: None,
        }],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: INDEX_QUERY_TOOL_NAME,
        description: "Search the codebase index for symbols or references. Use `name`/`kind`/`file` to query symbols, or `refs_to` to find references to a name. Returns formatted results.",
        properties: &[
            ToolPropertyDef {
                name: "name",
                prop_type: "string",
                description: "Filter symbols by name (substring match, case-insensitive)",
                default: None,
            },
            ToolPropertyDef {
                name: "kind",
                prop_type: "string",
                description: "Filter symbols by kind (exact match, e.g. \"function\", \"struct\")",
                default: None,
            },
            ToolPropertyDef {
                name: "file",
                prop_type: "string",
                description: "Filter symbols by file path (substring match)",
                default: None,
            },
            ToolPropertyDef {
                name: "refs_to",
                prop_type: "string",
                description: "Find references to this name (substring match). When set, name/kind/file are ignored.",
                default: None,
            },
            ToolPropertyDef {
                name: "limit",
                prop_type: "integer",
                description: "Maximum number of results to return (default: 50)",
                default: Some(50),
            },
        ],
        required: &[],
        summary_params: &["name", "kind"],
    },
    BuiltinToolDef {
        name: INDEX_STATUS_TOOL_NAME,
        description: "Show a summary of the codebase index: file counts, language breakdown, symbol and reference totals.",
        properties: &[],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: FETCH_URL_TOOL_NAME,
        description: "Fetch content from a URL via HTTP GET and return the response body. Follows redirects. Use for retrieving web pages, API responses, or raw file content.",
        properties: &[
            ToolPropertyDef {
                name: "url",
                prop_type: "string",
                description: "URL to fetch (must start with http:// or https://)",
                default: None,
            },
            ToolPropertyDef {
                name: "max_bytes",
                prop_type: "integer",
                description: "Maximum response body size in bytes (default: 1048576 = 1MB)",
                default: Some(1_048_576),
            },
            ToolPropertyDef {
                name: "timeout_secs",
                prop_type: "integer",
                description: "Request timeout in seconds (default: 30)",
                default: Some(30),
            },
        ],
        required: &["url"],
        summary_params: &["url"],
    },
];

// === Registry Helpers ===

/// Convert all coding tools to API format
pub fn all_coding_tools_to_api_format() -> Vec<serde_json::Value> {
    CODING_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the coding tool set
pub fn is_coding_tool(name: &str) -> bool {
    matches!(
        name,
        SHELL_EXEC_TOOL_NAME
            | DIR_LIST_TOOL_NAME
            | GLOB_FILES_TOOL_NAME
            | GREP_FILES_TOOL_NAME
            | FILE_EDIT_TOOL_NAME
            | INDEX_UPDATE_TOOL_NAME
            | INDEX_QUERY_TOOL_NAME
            | INDEX_STATUS_TOOL_NAME
            | FETCH_URL_TOOL_NAME
    )
}

// === Path Resolution ===

/// Resolve a path string relative to project_root.
///
/// Absolute paths are returned as-is. Relative paths are joined with project_root.
fn resolve_path(project_root: &Path, path_str: &str) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        project_root.join(p)
    }
}

// === Parameter Helpers ===

/// Extract a required string parameter, returning a helpful error if missing.
fn require_str_param(args: &serde_json::Value, name: &str) -> io::Result<String> {
    args.get_str(name).map(String::from).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Missing '{}' parameter", name),
        )
    })
}

// === Tool Execution ===

/// Execute a coding tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
/// The `project_root` is used to resolve relative paths in all tools.
/// The `tools` slice is forwarded to `index_update` for language plugin dispatch;
/// pass `&[]` when the plugin list is unavailable.
pub async fn execute_coding_tool(
    tool_name: &str,
    args: &serde_json::Value,
    project_root: &Path,
    tools: &[super::Tool],
) -> Option<io::Result<String>> {
    match tool_name {
        SHELL_EXEC_TOOL_NAME => Some(execute_shell_exec(args, project_root).await),
        DIR_LIST_TOOL_NAME => Some(execute_dir_list(args, project_root)),
        GLOB_FILES_TOOL_NAME => Some(execute_glob_files(args, project_root)),
        GREP_FILES_TOOL_NAME => Some(execute_grep_files(args, project_root)),
        FILE_EDIT_TOOL_NAME => Some(execute_file_edit(args, project_root)),
        INDEX_UPDATE_TOOL_NAME => Some(execute_index_update(args, project_root, tools)),
        INDEX_QUERY_TOOL_NAME => Some(execute_index_query(args, project_root)),
        INDEX_STATUS_TOOL_NAME => Some(execute_index_status(project_root)),
        FETCH_URL_TOOL_NAME => Some(execute_fetch_url(args).await),
        _ => None,
    }
}

// --- shell_exec ---

/// Execute shell_exec: run a command with a timeout and return structured JSON output.
///
/// Spawns `sh -c <command>` in `project_root` and waits up to `timeout_secs`
/// seconds. If the timeout expires the child is killed. The JSON result always
/// contains the fields `stdout`, `stderr`, `exit_code`, and `timed_out`.
async fn execute_shell_exec(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
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

    // wait_with_output takes ownership — timeout wrapping handles the cancel
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
            // Timeout expired. The future was dropped, which drops the child process,
            // sending SIGKILL on unix when the Child handle is dropped.
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

// --- dir_list ---

/// Execute dir_list: produce an indented directory tree with sizes and type tags.
fn execute_dir_list(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    let path_str = args.get_str_or("path", ".");
    let depth = args.get_u64_or("depth", 1) as usize;
    let show_hidden = args.get_str_or("show_hidden", "false") == "true";

    let root = resolve_path(project_root, path_str);

    if !root.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {}", root.display()),
        ));
    }
    if !root.is_dir() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Path is not a directory: {}", root.display()),
        ));
    }

    let mut output = format!("{}/\n", root.display());
    collect_dir_tree(&root, &mut output, 0, depth, show_hidden, "")?;
    Ok(output)
}

/// Recursively collect a directory tree into `output`.
fn collect_dir_tree(
    dir: &Path,
    output: &mut String,
    current_depth: usize,
    max_depth: usize,
    show_hidden: bool,
    prefix: &str,
) -> io::Result<()> {
    if current_depth >= max_depth {
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            if show_hidden {
                return true;
            }
            // Skip entries whose name starts with '.'
            e.file_name()
                .to_str()
                .map(|n| !n.starts_with('.'))
                .unwrap_or(true)
        })
        .collect();

    // Directories first, then files; each group sorted by name
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.file_name().cmp(&b.file_name()),
        }
    });

    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i + 1 == count;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });

        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if file_type.is_dir() {
            output.push_str(&format!("{}{}{}/\n", prefix, connector, name_str));
            collect_dir_tree(
                &entry.path(),
                output,
                current_depth + 1,
                max_depth,
                show_hidden,
                &child_prefix,
            )?;
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            output.push_str(&format!(
                "{}{}{} ({})\n",
                prefix,
                connector,
                name_str,
                format_size(size),
            ));
        }
    }
    Ok(())
}

/// Format a byte count into a human-readable string.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// --- glob_files ---

/// Execute glob_files: walk the directory tree respecting .gitignore and match a glob.
fn execute_glob_files(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let pattern = require_str_param(args, "pattern")?;
    let path_str = args.get_str_or("path", ".");
    let max_results = args.get_u64_or("max_results", 100) as usize;

    let root = resolve_path(project_root, path_str);

    if !root.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {}", root.display()),
        ));
    }

    // Build an override matcher that implements the glob pattern
    let mut overrides = OverrideBuilder::new(&root);
    overrides.add(&pattern).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid glob pattern '{}': {}", pattern, e),
        )
    })?;
    let overrides = overrides.build().map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Failed to build glob matcher: {}", e),
        )
    })?;

    let walker = WalkBuilder::new(&root).overrides(overrides).build();

    let mut matches = Vec::new();
    for entry in walker {
        if matches.len() >= max_results {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        // Skip directories; we only report files
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        // Return paths relative to the search root
        let rel = path.strip_prefix(&root).unwrap_or(path);
        matches.push(rel.to_string_lossy().into_owned());
    }

    if matches.is_empty() {
        Ok(format!("No files found matching pattern: {}", pattern))
    } else {
        Ok(matches.join("\n"))
    }
}

// --- grep_files ---

/// Execute grep_files: regex-search files under a directory with optional context.
fn execute_grep_files(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;
    use regex::Regex;

    let pattern = require_str_param(args, "pattern")?;
    let path_str = args.get_str_or("path", ".");
    let file_pattern = args.get_str("file_pattern").map(String::from);
    let context_lines = args.get_u64_or("context_lines", 2) as usize;
    let max_results = args.get_u64_or("max_results", 50) as usize;

    let regex = Regex::new(&pattern).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid regex pattern '{}': {}", pattern, e),
        )
    })?;

    let root = resolve_path(project_root, path_str);

    if !root.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {}", root.display()),
        ));
    }

    // Optionally restrict to a file glob pattern
    let mut builder = WalkBuilder::new(&root);
    if let Some(ref fp) = file_pattern {
        let mut overrides = OverrideBuilder::new(&root);
        overrides.add(fp).map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid file pattern '{}': {}", fp, e),
            )
        })?;
        let ov = overrides.build().map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("Failed to build file pattern matcher: {}", e),
            )
        })?;
        builder.overrides(ov);
    }

    let mut output = String::new();
    let mut total_matches = 0;

    'walk: for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(true) {
            continue;
        }

        let file_path = entry.path();
        let rel_path = file_path
            .strip_prefix(&root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .into_owned();

        // Read all lines; skip unreadable files silently
        let file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap_or_default()).collect();

        // Collect 0-based indices of matching lines
        let match_indices: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if match_indices.is_empty() {
            continue;
        }

        // Merge overlapping context windows into contiguous ranges
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for &idx in &match_indices {
            let start = idx.saturating_sub(context_lines);
            let end = (idx + context_lines + 1).min(lines.len());
            if let Some(last) = ranges.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
                continue;
            }
            ranges.push((start, end));
        }

        let match_set: std::collections::HashSet<usize> = match_indices.into_iter().collect();

        for (start, end) in ranges {
            #[allow(clippy::needless_range_loop)]
            for i in start..end {
                let line_num = i + 1; // convert to 1-indexed for display
                if match_set.contains(&i) {
                    output.push_str(&format!("{}:{}: {}\n", rel_path, line_num, lines[i]));
                    total_matches += 1;
                    if total_matches >= max_results {
                        output.push_str(&format!(
                            "\n[Truncated: {} results limit reached]\n",
                            max_results
                        ));
                        break 'walk;
                    }
                } else {
                    // Context line — use '-' separator to distinguish from match lines
                    output.push_str(&format!("{}:{}-{}\n", rel_path, line_num, lines[i]));
                }
            }
            output.push('\n');
        }
    }

    if output.is_empty() {
        Ok(format!("No matches found for pattern: {}", pattern))
    } else {
        Ok(output.trim_end().to_string())
    }
}

// --- file_edit ---

/// Edit operation variants
enum EditOperation {
    ReplaceLines {
        start: usize,
        end: usize,
        content: String,
    },
    InsertBefore {
        line: usize,
        content: String,
    },
    InsertAfter {
        line: usize,
        content: String,
    },
    DeleteLines {
        start: usize,
        end: usize,
    },
    ReplaceString {
        find: String,
        replace: String,
    },
}

/// Execute file_edit: apply a structured edit to a file atomically.
fn execute_file_edit(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    let path_str = require_str_param(args, "path")?;
    let operation_str = require_str_param(args, "operation")?;

    let file_path = resolve_path(project_root, &path_str);

    let op = parse_edit_operation(&operation_str, args)?;

    // Read current content
    let content = std::fs::read_to_string(&file_path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Failed to read '{}': {}", file_path.display(), e),
        )
    })?;
    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    // Preserve trailing newline so we don't silently strip it
    let had_trailing_newline = content.ends_with('\n');

    match op {
        EditOperation::ReplaceLines {
            start,
            end,
            content: new_content,
        } => {
            validate_line_range(start, end, lines.len())?;
            // splice is exclusive on the right; end is 1-indexed inclusive → index end
            let new_lines: Vec<String> = new_content.lines().map(String::from).collect();
            lines.splice((start - 1)..end, new_lines);
        }
        EditOperation::InsertBefore {
            line,
            content: new_content,
        } => {
            validate_line_number(line, lines.len())?;
            let new_lines: Vec<String> = new_content.lines().map(String::from).collect();
            let insert_at = line - 1;
            for (i, l) in new_lines.into_iter().enumerate() {
                lines.insert(insert_at + i, l);
            }
        }
        EditOperation::InsertAfter {
            line,
            content: new_content,
        } => {
            validate_line_number(line, lines.len())?;
            let new_lines: Vec<String> = new_content.lines().map(String::from).collect();
            // After line N (1-indexed) means index N (0-based insert position)
            let insert_at = line;
            for (i, l) in new_lines.into_iter().enumerate() {
                lines.insert(insert_at + i, l);
            }
        }
        EditOperation::DeleteLines { start, end } => {
            validate_line_range(start, end, lines.len())?;
            lines.drain((start - 1)..end);
        }
        EditOperation::ReplaceString { find, replace } => {
            // Operate on full content string to support multiline find/replace
            if !content.contains(&find) {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("String not found in file: {:?}", find),
                ));
            }
            let new_content = content.replacen(&find, &replace, 1);
            crate::safe_io::atomic_write_text(&file_path, &new_content)?;
            return Ok(format!(
                "Replaced string in {} ({} → {} bytes)",
                file_path.display(),
                find.len(),
                replace.len()
            ));
        }
    }

    // Reassemble lines and restore trailing newline
    let mut new_content = lines.join("\n");
    if had_trailing_newline || !new_content.is_empty() {
        new_content.push('\n');
    }
    crate::safe_io::atomic_write_text(&file_path, &new_content)?;

    Ok(format!(
        "Edited {} ({} lines)",
        file_path.display(),
        lines.len()
    ))
}

/// Parse the operation string and required parameters into an `EditOperation`.
fn parse_edit_operation(op: &str, args: &serde_json::Value) -> io::Result<EditOperation> {
    match op {
        "replace_lines" => {
            let start = require_line_param(args, "line_start")?;
            let end = require_line_param(args, "line_end")?;
            let content = require_str_param(args, "content")?;
            Ok(EditOperation::ReplaceLines {
                start,
                end,
                content,
            })
        }
        "insert_before" => {
            let line = require_line_param(args, "line_start")?;
            let content = require_str_param(args, "content")?;
            Ok(EditOperation::InsertBefore { line, content })
        }
        "insert_after" => {
            let line = require_line_param(args, "line_start")?;
            let content = require_str_param(args, "content")?;
            Ok(EditOperation::InsertAfter { line, content })
        }
        "delete_lines" => {
            let start = require_line_param(args, "line_start")?;
            let end = require_line_param(args, "line_end")?;
            Ok(EditOperation::DeleteLines { start, end })
        }
        "replace_string" => {
            let find = require_str_param(args, "find")?;
            let replace = require_str_param(args, "replace")?;
            Ok(EditOperation::ReplaceString { find, replace })
        }
        other => Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Unknown operation '{}'. Valid: replace_lines, insert_before, insert_after, delete_lines, replace_string",
                other
            ),
        )),
    }
}

/// Extract a required 1-indexed line number parameter.
fn require_line_param(args: &serde_json::Value, name: &str) -> io::Result<usize> {
    let n = args.get_u64(name).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Missing '{}' parameter", name),
        )
    })? as usize;
    if n == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("'{}' must be >= 1 (line numbers are 1-indexed)", name),
        ));
    }
    Ok(n)
}

/// Validate that a single 1-indexed line number is within the file.
fn validate_line_number(line: usize, total: usize) -> io::Result<()> {
    if line > total {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Line {} is out of bounds (file has {} lines)", line, total),
        ));
    }
    Ok(())
}

/// Validate that a 1-indexed inclusive line range is valid for the given file.
fn validate_line_range(start: usize, end: usize, total: usize) -> io::Result<()> {
    if end < start {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("line_end ({}) must be >= line_start ({})", end, start),
        ));
    }
    if end > total {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "line_end {} is out of bounds (file has {} lines)",
                end, total
            ),
        ));
    }
    Ok(())
}

// --- index_update ---

/// Execute index_update: walk the project, detect changes, dispatch to language plugins.
fn execute_index_update(
    args: &serde_json::Value,
    project_root: &Path,
    tools: &[super::Tool],
) -> io::Result<String> {
    use crate::index::{self, IndexOptions, open_db};

    let force = args.get_str_or("force", "false") == "true";
    let db_path = crate::project_index_db_path(project_root);

    // Ensure .chibi directory exists
    crate::project_chibi_dir(project_root)?;

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    let options = IndexOptions {
        force,
        verbose: false,
    };

    let stats = index::update_index(&conn, project_root, &options, tools)?;
    Ok(stats.to_string())
}

// --- index_query ---

/// Execute index_query: search for symbols or references in the codebase index.
fn execute_index_query(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    use crate::index::{SymbolQuery, open_db, query_refs, query_symbols};

    let db_path = crate::project_index_db_path(project_root);
    if !db_path.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            "No codebase index found. Run index_update first.",
        ));
    }

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    let limit = args.get_u64_or("limit", 50) as u32;

    // If refs_to is set, query references instead of symbols
    if let Some(refs_to) = args.get_str("refs_to") {
        let rows = query_refs(&conn, refs_to, limit);
        if rows.is_empty() {
            return Ok(format!("No references found matching: {}", refs_to));
        }
        let formatted: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
        return Ok(formatted.join("\n"));
    }

    // Otherwise, query symbols
    let opts = SymbolQuery {
        name: args.get_str("name").map(String::from),
        kind: args.get_str("kind").map(String::from),
        file: args.get_str("file").map(String::from),
        limit,
    };

    let rows = query_symbols(&conn, &opts);
    if rows.is_empty() {
        return Ok("No symbols found matching query.".to_string());
    }
    let formatted: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    Ok(formatted.join("\n"))
}

// --- index_status ---

/// Execute index_status: return a human-readable summary of the codebase index.
fn execute_index_status(project_root: &Path) -> io::Result<String> {
    use crate::index::{index_status, open_db};

    let db_path = crate::project_index_db_path(project_root);
    if !db_path.exists() {
        return Ok("No codebase index found. Run index_update to create one.".to_string());
    }

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    Ok(index_status(&conn, project_root))
}

// --- fetch_url ---

/// Execute fetch_url: HTTP GET a URL and return the response body.
///
/// Delegates to `fetch_url_with_limit` for streaming size-limited fetching.
/// URL policy gating is handled by the caller in `send.rs`.
async fn execute_fetch_url(args: &serde_json::Value) -> io::Result<String> {
    let url = require_str_param(args, "url")?;
    let max_bytes = args.get_u64_or("max_bytes", 1_048_576) as usize;
    let timeout_secs = args.get_u64_or("timeout_secs", 30);

    super::fetch_url_with_limit(&url, max_bytes, timeout_secs).await
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // --- Helpers ---

    fn make_temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    // --- Registry ---

    #[test]
    fn test_coding_tool_defs_api_format() {
        // All defs must convert to valid API objects without panicking
        for def in CODING_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
            assert!(api["function"]["description"].is_string());
        }
    }

    #[test]
    fn test_is_coding_tool() {
        assert!(is_coding_tool(SHELL_EXEC_TOOL_NAME));
        assert!(is_coding_tool(DIR_LIST_TOOL_NAME));
        assert!(is_coding_tool(GLOB_FILES_TOOL_NAME));
        assert!(is_coding_tool(GREP_FILES_TOOL_NAME));
        assert!(is_coding_tool(FILE_EDIT_TOOL_NAME));
        assert!(is_coding_tool(INDEX_UPDATE_TOOL_NAME));
        assert!(is_coding_tool(INDEX_QUERY_TOOL_NAME));
        assert!(is_coding_tool(INDEX_STATUS_TOOL_NAME));
        assert!(is_coding_tool(FETCH_URL_TOOL_NAME));
        assert!(!is_coding_tool("file_head"));
        assert!(!is_coding_tool("unknown_tool"));
    }

    #[test]
    fn test_coding_tool_registry_contains_all_tools() {
        assert_eq!(CODING_TOOL_DEFS.len(), 9);
        let names: Vec<_> = CODING_TOOL_DEFS.iter().map(|d| d.name).collect();
        assert!(names.contains(&SHELL_EXEC_TOOL_NAME));
        assert!(names.contains(&DIR_LIST_TOOL_NAME));
        assert!(names.contains(&GLOB_FILES_TOOL_NAME));
        assert!(names.contains(&GREP_FILES_TOOL_NAME));
        assert!(names.contains(&FILE_EDIT_TOOL_NAME));
        assert!(names.contains(&INDEX_UPDATE_TOOL_NAME));
        assert!(names.contains(&INDEX_QUERY_TOOL_NAME));
        assert!(names.contains(&INDEX_STATUS_TOOL_NAME));
        assert!(names.contains(&FETCH_URL_TOOL_NAME));
    }

    // --- Path Resolution ---

    #[test]
    fn test_resolve_path_absolute() {
        let root = Path::new("/some/root");
        let result = resolve_path(root, "/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_resolve_path_relative() {
        let root = Path::new("/some/root");
        let result = resolve_path(root, "relative/path");
        assert_eq!(result, PathBuf::from("/some/root/relative/path"));
    }

    // --- dir_list ---

    #[test]
    fn test_dir_list_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("subdir").join("nested.txt"), "world").unwrap();

        let a = args(&[
            ("path", serde_json::json!(".")),
            ("depth", serde_json::json!(2)),
        ]);
        let result = execute_dir_list(&a, dir.path()).unwrap();

        assert!(result.contains("file.txt"));
        assert!(result.contains("subdir/"));
        assert!(result.contains("nested.txt"));
    }

    #[test]
    fn test_dir_list_respects_depth() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("subdir").join("deep.txt"), "x").unwrap();

        // depth=1 should not recurse into subdir
        let a = args(&[
            ("path", serde_json::json!(".")),
            ("depth", serde_json::json!(1)),
        ]);
        let result = execute_dir_list(&a, dir.path()).unwrap();

        assert!(result.contains("subdir/"));
        assert!(!result.contains("deep.txt"));
    }

    #[test]
    fn test_dir_list_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden"), "secret").unwrap();
        fs::write(dir.path().join("visible.txt"), "public").unwrap();

        let hidden_off = args(&[
            ("path", serde_json::json!(".")),
            ("show_hidden", serde_json::json!("false")),
        ]);
        let result = execute_dir_list(&hidden_off, dir.path()).unwrap();
        assert!(!result.contains(".hidden"));
        assert!(result.contains("visible.txt"));

        let hidden_on = args(&[
            ("path", serde_json::json!(".")),
            ("show_hidden", serde_json::json!("true")),
        ]);
        let result = execute_dir_list(&hidden_on, dir.path()).unwrap();
        assert!(result.contains(".hidden"));
    }

    // --- glob_files ---

    #[test]
    fn test_glob_files_finds_matching() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("lib.rs"), "pub fn foo() {}").unwrap();
        fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let a = args(&[("pattern", serde_json::json!("*.rs"))]);
        let result = execute_glob_files(&a, dir.path()).unwrap();

        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
        assert!(!result.contains("README.md"));
    }

    #[test]
    fn test_glob_files_max_results() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file{}.txt", i)), "x").unwrap();
        }

        let a = args(&[
            ("pattern", serde_json::json!("*.txt")),
            ("max_results", serde_json::json!(3)),
        ]);
        let result = execute_glob_files(&a, dir.path()).unwrap();
        assert_eq!(result.lines().count(), 3);
    }

    #[test]
    fn test_glob_files_no_match() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let a = args(&[("pattern", serde_json::json!("*.py"))]);
        let result = execute_glob_files(&a, dir.path()).unwrap();
        assert!(result.contains("No files found"));
    }

    // --- grep_files ---

    #[test]
    fn test_grep_files_returns_matches() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(
            &dir,
            "src.rs",
            "fn hello() {\n    println!(\"hello\");\n}\n",
        );

        let a = args(&[("pattern", serde_json::json!("println"))]);
        let result = execute_grep_files(&a, dir.path()).unwrap();

        assert!(result.contains("println"));
        assert!(result.contains("src.rs"));
    }

    #[test]
    fn test_grep_files_no_match() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "code.rs", "fn nothing() {}\n");

        let a = args(&[("pattern", serde_json::json!("NONEXISTENT_PATTERN_XYZ"))]);
        let result = execute_grep_files(&a, dir.path()).unwrap();
        assert!(result.contains("No matches found"));
    }

    #[test]
    fn test_grep_files_file_pattern() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "code.rs", "search_term here\n");
        make_temp_file(&dir, "note.txt", "search_term here\n");

        // Only search .rs files
        let a = args(&[
            ("pattern", serde_json::json!("search_term")),
            ("file_pattern", serde_json::json!("*.rs")),
        ]);
        let result = execute_grep_files(&a, dir.path()).unwrap();

        assert!(result.contains("code.rs"));
        assert!(!result.contains("note.txt"));
    }

    #[test]
    fn test_grep_files_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "code.rs", "line1\nline2\nMATCH\nline4\nline5\n");

        let a = args(&[
            ("pattern", serde_json::json!("MATCH")),
            ("context_lines", serde_json::json!(1)),
        ]);
        let result = execute_grep_files(&a, dir.path()).unwrap();

        // Context should include surrounding lines
        assert!(result.contains("line2"));
        assert!(result.contains("MATCH"));
        assert!(result.contains("line4"));
    }

    // --- file_edit ---

    #[test]
    fn test_file_edit_replace_lines() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\nbbb\nccc\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("replace_lines")),
            ("line_start", serde_json::json!(2)),
            ("line_end", serde_json::json!(2)),
            ("content", serde_json::json!("BBB")),
        ]);
        execute_file_edit(&a, dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(content, "aaa\nBBB\nccc\n");
    }

    #[test]
    fn test_file_edit_insert_before() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\nbbb\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("insert_before")),
            ("line_start", serde_json::json!(2)),
            ("content", serde_json::json!("NEW")),
        ]);
        execute_file_edit(&a, dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(content, "aaa\nNEW\nbbb\n");
    }

    #[test]
    fn test_file_edit_insert_after() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\nbbb\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("insert_after")),
            ("line_start", serde_json::json!(1)),
            ("content", serde_json::json!("NEW")),
        ]);
        execute_file_edit(&a, dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(content, "aaa\nNEW\nbbb\n");
    }

    #[test]
    fn test_file_edit_delete_lines() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\nbbb\nccc\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("delete_lines")),
            ("line_start", serde_json::json!(2)),
            ("line_end", serde_json::json!(2)),
        ]);
        execute_file_edit(&a, dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(content, "aaa\nccc\n");
    }

    #[test]
    fn test_file_edit_replace_string() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "hello world\nhello again\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("replace_string")),
            ("find", serde_json::json!("hello")),
            ("replace", serde_json::json!("goodbye")),
        ]);
        execute_file_edit(&a, dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
        // Only first occurrence replaced
        assert_eq!(content, "goodbye world\nhello again\n");
    }

    #[test]
    fn test_file_edit_out_of_bounds() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\nbbb\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("replace_lines")),
            ("line_start", serde_json::json!(5)),
            ("line_end", serde_json::json!(6)),
            ("content", serde_json::json!("x")),
        ]);
        let result = execute_file_edit(&a, dir.path());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn test_file_edit_invalid_operation() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "f.txt", "aaa\n");

        let a = args(&[
            ("path", serde_json::json!("f.txt")),
            ("operation", serde_json::json!("teleport")),
        ]);
        let result = execute_file_edit(&a, dir.path());
        assert!(result.is_err());
    }

    // --- shell_exec ---

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
        // Canonicalize both sides for macOS /private/tmp symlink
        let expected = dir.path().canonicalize().unwrap();
        let actual = PathBuf::from(stdout).canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    // --- index tools ---

    #[test]
    fn test_index_status_no_db() {
        let dir = tempfile::tempdir().unwrap();
        let result = execute_index_status(dir.path()).unwrap();
        assert!(result.contains("No codebase index found"));
    }

    #[test]
    fn test_index_query_no_db() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("name", serde_json::json!("foo"))]);
        let result = execute_index_query(&a, dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No codebase index")
        );
    }

    #[test]
    fn test_index_update_creates_db_and_indexes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("lib.py"), "def hello(): pass\n").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("util.rs"), "pub fn util() {}\n").unwrap();

        let a = args(&[]);
        let result = execute_index_update(&a, dir.path(), &[]).unwrap();

        // Should report scan results
        assert!(result.contains("scanned:"));

        // DB should now exist
        let db_path = crate::project_index_db_path(dir.path());
        assert!(db_path.exists());
    }

    #[test]
    fn test_index_update_then_status() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        // Update index
        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        // Now status should show indexed files
        let result = execute_index_status(dir.path()).unwrap();
        assert!(result.contains("file"));
    }

    #[test]
    fn test_index_update_then_query_no_symbols() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        // Update index (no language plugins, so no symbols extracted)
        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        // Query symbols — should find nothing since no plugins extracted symbols
        let q = args(&[("name", serde_json::json!("main"))]);
        let result = execute_index_query(&q, dir.path()).unwrap();
        assert!(result.contains("No symbols found"));
    }

    #[test]
    fn test_index_query_refs_no_refs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        let q = args(&[("refs_to", serde_json::json!("nonexistent"))]);
        let result = execute_index_query(&q, dir.path()).unwrap();
        assert!(result.contains("No references found"));
    }

    // --- fetch_url ---

    #[tokio::test]
    async fn test_fetch_url_invalid_scheme() {
        let a = args(&[("url", serde_json::json!("ftp://example.com"))]);
        let result = execute_fetch_url(&a).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http://"));
    }

    #[tokio::test]
    async fn test_fetch_url_missing_param() {
        let a = args(&[]);
        let result = execute_fetch_url(&a).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing"));
    }

    #[test]
    fn test_index_update_force_reindex() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        // First index
        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        // Second index without force should skip unchanged files
        let result = execute_index_update(&a, dir.path(), &[]).unwrap();
        // "skipped (unchanged): N" where N > 0
        assert!(result.contains("skipped (unchanged): 1"));

        // Force re-index should re-process all files
        let a_force = args(&[("force", serde_json::json!("true"))]);
        let result = execute_index_update(&a_force, dir.path(), &[]).unwrap();
        assert!(result.contains("indexed: 1"));
    }
}
