//!
//! fs_read tools: read-only access to OS and VFS paths.
//! file_head, file_tail, file_lines, file_grep, dir_list, glob_files, grep_files.
//! No PreFileRead/PreFileWrite hooks are fired here; gating is in the dispatcher.

use std::io::{self, BufRead, ErrorKind};
use std::path::Path;

use super::paths::{ResolvedPath, resolve_tool_path};
use super::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::config::ResolvedConfig;
use crate::json_ext::JsonExt;
use crate::state::AppState;
use crate::vfs::VfsCaller;

// === Tool Name Constants ===

pub const FILE_HEAD_TOOL_NAME: &str = "file_head";
pub const FILE_TAIL_TOOL_NAME: &str = "file_tail";
pub const FILE_LINES_TOOL_NAME: &str = "file_lines";
pub const FILE_GREP_TOOL_NAME: &str = "file_grep";
pub const DIR_LIST_TOOL_NAME: &str = "dir_list";
pub const GLOB_FILES_TOOL_NAME: &str = "glob_files";
pub const GREP_FILES_TOOL_NAME: &str = "grep_files";

// === Tool Definition Registry ===

/// All fs_read tool definitions
pub static FS_READ_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: FILE_HEAD_TOOL_NAME,
        description: "Read the first N lines from a file or cached tool output. Use this to examine the beginning of large outputs.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to a file, or a vfs:/// URI for VFS storage",
                default: None,
            },
            ToolPropertyDef {
                name: "lines",
                prop_type: "integer",
                description: "Number of lines to read (default: 50)",
                default: Some(50),
            },
        ],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_TAIL_TOOL_NAME,
        description: "Read the last N lines from a file or cached tool output. Use this to examine the end of large outputs.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to a file, or a vfs:/// URI for VFS storage",
                default: None,
            },
            ToolPropertyDef {
                name: "lines",
                prop_type: "integer",
                description: "Number of lines to read (default: 50)",
                default: Some(50),
            },
        ],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_LINES_TOOL_NAME,
        description: "Read a specific range of lines from a file or cached tool output. Lines are 1-indexed.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to a file, or a vfs:/// URI for VFS storage",
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
        required: &["path", "start", "end"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: FILE_GREP_TOOL_NAME,
        description: "Search for a pattern in a file or cached tool output. Returns matching lines with optional context.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to a file, or a vfs:/// URI for VFS storage",
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
        required: &["path", "pattern"],
        summary_params: &["pattern", "path"],
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
];

// === Registry Helpers ===

/// Register all fs_read tools into the registry.
pub fn register_fs_read_tools(registry: &mut super::registry::ToolRegistry) {
    use std::sync::Arc;
    use super::registry::{ToolCategory, ToolHandler};
    use super::Tool;

    let handler: ToolHandler = Arc::new(|call| {
        // execute_fs_read_tool is sync — extract result before the async block so
        // no !Sync references (&AppState, &Vfs) cross an .await point.
        let ctx = call.context;
        let result = execute_fs_read_tool(
            ctx.app,
            ctx.context_name,
            call.name,
            call.args,
            ctx.config,
            ctx.project_root,
        )
        .unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown fs_read tool: {}", call.name),
            ))
        });
        Box::pin(async move { result })
    });

    for def in FS_READ_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(def, handler.clone(), ToolCategory::FsRead));
    }
}

/// Convert all fs_read tools to API format
pub fn all_fs_read_tools_to_api_format() -> Vec<serde_json::Value> {
    FS_READ_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the fs_read group
pub fn is_fs_read_tool(name: &str) -> bool {
    FS_READ_TOOL_DEFS.iter().any(|d| d.name == name)
}

// === Tool Execution ===

/// Execute an fs_read tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
pub fn execute_fs_read_tool(
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
        DIR_LIST_TOOL_NAME => Some(execute_dir_list(args, project_root, config)),
        GLOB_FILES_TOOL_NAME => Some(execute_glob_files(args, project_root, config)),
        GREP_FILES_TOOL_NAME => Some(execute_grep_files(args, project_root, config)),
        _ => None,
    }
}

// === VFS bridge ===
/// Bridge an async VFS future into synchronous tool dispatch.
///
/// Uses `block_in_place` + `block_on` so that it works from sync code inside a
/// tokio multi-thread runtime (e.g. tool dispatch in `execute_tool_pure`). The
/// `block_in_place` call tells the runtime scheduler to move other tasks off
/// this thread while we block.
///
/// **Runtime requirement:** `block_in_place` panics on `current_thread`
/// runtimes. Any test that calls VFS tools must use:
/// `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`
///
/// If called outside of any tokio runtime (e.g. plain `#[test]`), spins up a
/// temporary current-thread runtime.
pub(crate) fn vfs_block_on<F: std::future::Future>(f: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        Err(_) => {
            // No runtime active — spin up a temporary one.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build temporary tokio runtime")
                .block_on(f)
        }
    }
}

// === file_head / file_tail ===

/// Direction for head/tail reading
enum ReadDirection {
    Head,
    Tail,
}

/// Shared implementation for file_head and file_tail.
fn execute_file_head_or_tail(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    direction: ReadDirection,
    project_root: &Path,
) -> io::Result<String> {
    let path_str = args
        .get_str("path")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Must provide path"))?;
    let path = resolve_tool_path(path_str, project_root, config)?;
    let n = args.get_u64_or("lines", 50) as usize;

    match path {
        ResolvedPath::Os(p) => match direction {
            ReadDirection::Head => {
                let file = std::fs::File::open(&p)?;
                let reader = std::io::BufReader::new(file);
                let lines: Vec<String> = reader.lines().take(n).collect::<Result<_, _>>()?;
                Ok(lines.join("\n"))
            }
            ReadDirection::Tail => {
                let content = std::fs::read_to_string(&p)?;
                let lines: Vec<&str> = content.lines().collect();
                let start = lines.len().saturating_sub(n);
                Ok(lines[start..].join("\n"))
            }
        },
        ResolvedPath::Vfs(vfs_path) => {
            let data = vfs_block_on(app.vfs.read(VfsCaller::Context(context_name), &vfs_path))?;
            let content = String::from_utf8_lossy(&data);
            let all_lines: Vec<&str> = content.lines().collect();
            let selected = match direction {
                ReadDirection::Head => &all_lines[..all_lines.len().min(n)],
                ReadDirection::Tail => {
                    let start = all_lines.len().saturating_sub(n);
                    &all_lines[start..]
                }
            };
            Ok(selected.join("\n"))
        }
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

// === file_lines ===

/// Extract a required u64 parameter, returning a helpful error if missing.
fn require_u64_param(args: &serde_json::Value, name: &str) -> io::Result<u64> {
    args.get_u64(name).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Missing '{}' parameter", name),
        )
    })
}

/// Execute file_lines tool
pub fn execute_file_lines(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    let path_str = args
        .get_str("path")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Must provide path"))?;
    let path = resolve_tool_path(path_str, project_root, config)?;
    let start = require_u64_param(args, "start")? as usize;
    let end = require_u64_param(args, "end")? as usize;

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

    match path {
        ResolvedPath::Os(p) => {
            let file = std::fs::File::open(&p)?;
            let reader = std::io::BufReader::new(file);
            let lines: Vec<String> = reader
                .lines()
                .enumerate()
                .filter(|(i, _)| *i >= start.saturating_sub(1) && *i < end)
                .map(|(_, line)| line)
                .collect::<Result<_, _>>()?;
            Ok(lines.join("\n"))
        }
        ResolvedPath::Vfs(vfs_path) => {
            let data = vfs_block_on(app.vfs.read(VfsCaller::Context(context_name), &vfs_path))?;
            let content = String::from_utf8_lossy(&data);
            let lines: Vec<&str> = content
                .lines()
                .enumerate()
                .filter(|(i, _)| *i >= start.saturating_sub(1) && *i < end)
                .map(|(_, line)| line)
                .collect();
            Ok(lines.join("\n"))
        }
    }
}

// === file_grep ===

/// Execute file_grep tool
pub fn execute_file_grep(
    app: &AppState,
    context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<String> {
    let path_str = args
        .get_str("path")
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Must provide path"))?;
    let path = resolve_tool_path(path_str, project_root, config)?;
    let pattern = require_str_param(args, "pattern")?;
    let context_before = args.get_u64_or("context_before", 2) as usize;
    let context_after = args.get_u64_or("context_after", 2) as usize;

    let result = match path {
        ResolvedPath::Os(p) => {
            let content = std::fs::read_to_string(&p)?;
            grep_in_memory(&content, &pattern, context_before, context_after)?
        }
        ResolvedPath::Vfs(vfs_path) => {
            let data = vfs_block_on(app.vfs.read(VfsCaller::Context(context_name), &vfs_path))?;
            let content = String::from_utf8_lossy(&data).into_owned();
            grep_in_memory(&content, &pattern, context_before, context_after)?
        }
    };

    if result.is_empty() {
        Ok(format!("No matches found for pattern: {}", pattern))
    } else {
        Ok(result)
    }
}

/// In-memory grep with context lines.
fn grep_in_memory(
    content: &str,
    pattern: &str,
    context_before: usize,
    context_after: usize,
) -> io::Result<String> {
    let lines: Vec<&str> = content.lines().collect();
    let regex = regex::Regex::new(pattern)
        .map_err(|e| io::Error::new(ErrorKind::InvalidInput, format!("Invalid regex: {}", e)))?;

    let mut result = Vec::new();
    let mut last_end = 0;

    for (i, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            let start = i.saturating_sub(context_before);
            let end = (i + context_after + 1).min(lines.len());

            if start > last_end && !result.is_empty() {
                result.push("--".to_string());
            }

            let range_start = start.max(last_end);
            for (j, line) in lines.iter().enumerate().take(end).skip(range_start) {
                let prefix = if j == i { ">" } else { " " };
                result.push(format!("{}{}:{}", prefix, j + 1, line));
            }

            last_end = end;
        }
    }

    Ok(result.join("\n"))
}

// === dir_list ===

/// Execute dir_list: produce an indented directory tree with sizes and type tags.
fn execute_dir_list(
    args: &serde_json::Value,
    project_root: &Path,
    config: &ResolvedConfig,
) -> io::Result<String> {
    let path_str = args.get_str_or("path", ".");
    let depth = args.get_u64_or("depth", 1) as usize;
    let show_hidden = args.get_str_or("show_hidden", "false") == "true";

    let root = resolve_tool_path(path_str, project_root, config)?.require_os("dir_list")?;

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

// === glob_files ===

/// Execute glob_files: walk the directory tree respecting .gitignore and match a glob.
fn execute_glob_files(
    args: &serde_json::Value,
    project_root: &Path,
    config: &ResolvedConfig,
) -> io::Result<String> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let pattern = require_str_param(args, "pattern")?;
    let path_str = args.get_str_or("path", ".");
    let max_results = args.get_u64_or("max_results", 100) as usize;

    let root = resolve_tool_path(path_str, project_root, config)?.require_os("glob_files")?;

    if !root.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {}", root.display()),
        ));
    }

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
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(&root).unwrap_or(path);
        matches.push(rel.to_string_lossy().into_owned());
    }

    if matches.is_empty() {
        Ok(format!("No files found matching pattern: {}", pattern))
    } else {
        Ok(matches.join("\n"))
    }
}

// === grep_files ===

/// Execute grep_files: regex-search files under a directory with optional context.
fn execute_grep_files(
    args: &serde_json::Value,
    project_root: &Path,
    config: &ResolvedConfig,
) -> io::Result<String> {
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

    let root = resolve_tool_path(path_str, project_root, config)?.require_os("grep_files")?;

    if !root.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Path not found: {}", root.display()),
        ));
    }

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

        let file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap_or_default()).collect();

        let match_indices: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if match_indices.is_empty() {
            continue;
        }

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
                let line_num = i + 1;
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

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::vfs::{LocalBackend, Vfs, VfsPath};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_config_for(dir: &TempDir) -> ResolvedConfig {
        ResolvedConfig {
            file_tools_allowed_paths: vec![dir.path().to_string_lossy().to_string()],
            ..Default::default()
        }
    }

    fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    fn make_temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn make_vfs_app(home: &TempDir, vfs_dir: &TempDir) -> AppState {
        let mut app = AppState::load(Some(home.path().to_path_buf())).unwrap();
        let backend = LocalBackend::new(vfs_dir.path().to_path_buf());
        app.vfs = Vfs::new(Box::new(backend), "test-site-0000");
        app
    }

    // === Registry tests ===

    #[test]
    fn test_fs_read_tool_defs_api_format() {
        for def in FS_READ_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
            assert!(api["function"]["description"].is_string());
        }
    }

    #[test]
    fn test_is_fs_read_tool() {
        assert!(is_fs_read_tool(FILE_HEAD_TOOL_NAME));
        assert!(is_fs_read_tool(FILE_TAIL_TOOL_NAME));
        assert!(is_fs_read_tool(FILE_LINES_TOOL_NAME));
        assert!(is_fs_read_tool(FILE_GREP_TOOL_NAME));
        assert!(is_fs_read_tool(DIR_LIST_TOOL_NAME));
        assert!(is_fs_read_tool(GLOB_FILES_TOOL_NAME));
        assert!(is_fs_read_tool(GREP_FILES_TOOL_NAME));
        assert!(!is_fs_read_tool("write_file"));
        assert!(!is_fs_read_tool("shell_exec"));
        assert!(!is_fs_read_tool("unknown_tool"));
    }

    #[test]
    fn test_fs_read_tool_registry_count() {
        assert_eq!(FS_READ_TOOL_DEFS.len(), 7);
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(FILE_HEAD_TOOL_NAME, "file_head");
        assert_eq!(FILE_TAIL_TOOL_NAME, "file_tail");
        assert_eq!(FILE_LINES_TOOL_NAME, "file_lines");
        assert_eq!(FILE_GREP_TOOL_NAME, "file_grep");
        assert_eq!(DIR_LIST_TOOL_NAME, "dir_list");
        assert_eq!(GLOB_FILES_TOOL_NAME, "glob_files");
        assert_eq!(GREP_FILES_TOOL_NAME, "grep_files");
    }

    #[test]
    fn test_file_head_tool_api_format() {
        let tool = FS_READ_TOOL_DEFS
            .iter()
            .find(|d| d.name == FILE_HEAD_TOOL_NAME)
            .unwrap()
            .to_api_format();
        assert_eq!(tool["function"]["name"], FILE_HEAD_TOOL_NAME);
        assert_eq!(
            tool["function"]["parameters"]["properties"]["lines"]["default"],
            50
        );
    }

    #[test]
    fn test_file_grep_tool_api_format() {
        let tool = FS_READ_TOOL_DEFS
            .iter()
            .find(|d| d.name == FILE_GREP_TOOL_NAME)
            .unwrap()
            .to_api_format();
        assert_eq!(tool["function"]["name"], FILE_GREP_TOOL_NAME);
        let required = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }

    // === file_head / file_tail OS tests ===

    #[test]
    fn test_execute_file_head_with_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        let file = project_dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let config = make_config_for(&project_dir);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let a = args(&[
            ("path", serde_json::json!("test.txt")),
            ("lines", serde_json::json!(2)),
        ]);
        let result = execute_file_head(&app, "test", &a, &config, project_dir.path());
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(!output.contains("line3"));
    }

    #[test]
    fn test_execute_file_grep_with_project_root() {
        let project_dir = tempfile::tempdir().unwrap();
        let file = project_dir.path().join("code.rs");
        fs::write(&file, "fn hello() {\n    println!(\"world\");\n}\n").unwrap();

        let config = make_config_for(&project_dir);
        let home = tempfile::tempdir().unwrap();
        let app = AppState::load(Some(home.path().to_path_buf())).unwrap();

        let a = args(&[
            ("path", serde_json::json!("code.rs")),
            ("pattern", serde_json::json!("println")),
        ]);
        let result = execute_file_grep(&app, "test", &a, &config, project_dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("println"));
    }

    // === dir_list tests ===

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
        let result = execute_dir_list(&a, dir.path(), &make_config_for(&dir)).unwrap();

        assert!(result.contains("file.txt"));
        assert!(result.contains("subdir/"));
        assert!(result.contains("nested.txt"));
    }

    #[test]
    fn test_dir_list_respects_depth() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("subdir").join("deep.txt"), "x").unwrap();

        let a = args(&[
            ("path", serde_json::json!(".")),
            ("depth", serde_json::json!(1)),
        ]);
        let result = execute_dir_list(&a, dir.path(), &make_config_for(&dir)).unwrap();

        assert!(result.contains("subdir/"));
        assert!(!result.contains("deep.txt"));
    }

    #[test]
    fn test_dir_list_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden"), "secret").unwrap();
        fs::write(dir.path().join("visible.txt"), "public").unwrap();

        let a = args(&[
            ("path", serde_json::json!(".")),
            ("show_hidden", serde_json::json!("false")),
        ]);
        let result = execute_dir_list(&a, dir.path(), &make_config_for(&dir)).unwrap();
        assert!(!result.contains(".hidden"));
        assert!(result.contains("visible.txt"));

        let a = args(&[
            ("path", serde_json::json!(".")),
            ("show_hidden", serde_json::json!("true")),
        ]);
        let result = execute_dir_list(&a, dir.path(), &make_config_for(&dir)).unwrap();
        assert!(result.contains(".hidden"));
    }

    #[test]
    fn test_dir_list_rejects_vfs_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("path", serde_json::json!("vfs:///shared"))]);
        let result = execute_dir_list(&a, dir.path(), &make_config_for(&dir));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not support vfs://")
        );
    }

    // === glob_files tests ===

    #[test]
    fn test_glob_files_finds_matching() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("lib.rs"), "pub fn foo() {}").unwrap();
        fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let a = args(&[("pattern", serde_json::json!("*.rs"))]);
        let result = execute_glob_files(&a, dir.path(), &make_config_for(&dir)).unwrap();

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
        let result = execute_glob_files(&a, dir.path(), &make_config_for(&dir)).unwrap();
        assert_eq!(result.lines().count(), 3);
    }

    #[test]
    fn test_glob_files_no_match() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let a = args(&[("pattern", serde_json::json!("*.py"))]);
        let result = execute_glob_files(&a, dir.path(), &make_config_for(&dir)).unwrap();
        assert!(result.contains("No files found"));
    }

    // === grep_files tests ===

    #[test]
    fn test_grep_files_returns_matches() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(
            &dir,
            "src.rs",
            "fn hello() {\n    println!(\"hello\");\n}\n",
        );

        let a = args(&[("pattern", serde_json::json!("println"))]);
        let result = execute_grep_files(&a, dir.path(), &make_config_for(&dir)).unwrap();

        assert!(result.contains("println"));
        assert!(result.contains("src.rs"));
    }

    #[test]
    fn test_grep_files_no_match() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "code.rs", "fn nothing() {}\n");

        let a = args(&[("pattern", serde_json::json!("NONEXISTENT_PATTERN_XYZ"))]);
        let result = execute_grep_files(&a, dir.path(), &make_config_for(&dir)).unwrap();
        assert!(result.contains("No matches found"));
    }

    #[test]
    fn test_grep_files_file_pattern() {
        let dir = tempfile::tempdir().unwrap();
        make_temp_file(&dir, "code.rs", "search_term here\n");
        make_temp_file(&dir, "note.txt", "search_term here\n");

        let a = args(&[
            ("pattern", serde_json::json!("search_term")),
            ("file_pattern", serde_json::json!("*.rs")),
        ]);
        let result = execute_grep_files(&a, dir.path(), &make_config_for(&dir)).unwrap();

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
        let result = execute_grep_files(&a, dir.path(), &make_config_for(&dir)).unwrap();

        assert!(result.contains("line2"));
        assert!(result.contains("MATCH"));
        assert!(result.contains("line4"));
    }

    // === VFS integration tests ===

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_head_vfs() {
        let home = tempfile::tempdir().unwrap();
        let vfs_dir = tempfile::tempdir().unwrap();
        let app = make_vfs_app(&home, &vfs_dir);
        let config = ResolvedConfig::default();

        let vfs_path = VfsPath::new("/shared/data.txt").unwrap();
        app.vfs
            .write(
                VfsCaller::Context("ctx"),
                &vfs_path,
                b"line1\nline2\nline3\nline4\nline5",
            )
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/data.txt")),
            ("lines", serde_json::json!(2)),
        ]);
        let result = execute_file_head(&app, "ctx", &a, &config, Path::new("/unused"));
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(!output.contains("line3"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_tail_vfs() {
        let home = tempfile::tempdir().unwrap();
        let vfs_dir = tempfile::tempdir().unwrap();
        let app = make_vfs_app(&home, &vfs_dir);
        let config = ResolvedConfig::default();

        let vfs_path = VfsPath::new("/shared/data.txt").unwrap();
        app.vfs
            .write(
                VfsCaller::Context("ctx"),
                &vfs_path,
                b"line1\nline2\nline3\nline4\nline5",
            )
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/data.txt")),
            ("lines", serde_json::json!(2)),
        ]);
        let result = execute_file_tail(&app, "ctx", &a, &config, Path::new("/unused"));
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.contains("line3"));
        assert!(output.contains("line4"));
        assert!(output.contains("line5"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_lines_vfs() {
        let home = tempfile::tempdir().unwrap();
        let vfs_dir = tempfile::tempdir().unwrap();
        let app = make_vfs_app(&home, &vfs_dir);
        let config = ResolvedConfig::default();

        let vfs_path = VfsPath::new("/shared/data.txt").unwrap();
        app.vfs
            .write(
                VfsCaller::Context("ctx"),
                &vfs_path,
                b"line1\nline2\nline3\nline4\nline5",
            )
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/data.txt")),
            ("start", serde_json::json!(2)),
            ("end", serde_json::json!(4)),
        ]);
        let result = execute_file_lines(&app, "ctx", &a, &config, Path::new("/unused"));
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
        assert!(output.contains("line4"));
        assert!(!output.contains("line5"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_grep_vfs() {
        let home = tempfile::tempdir().unwrap();
        let vfs_dir = tempfile::tempdir().unwrap();
        let app = make_vfs_app(&home, &vfs_dir);
        let config = ResolvedConfig::default();

        let vfs_path = VfsPath::new("/shared/code.rs").unwrap();
        app.vfs
            .write(
                VfsCaller::Context("ctx"),
                &vfs_path,
                b"fn hello() {\n    println!(\"world\");\n}\n",
            )
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/code.rs")),
            ("pattern", serde_json::json!("println")),
        ]);
        let result = execute_file_grep(&app, "ctx", &a, &config, Path::new("/unused"));
        assert!(result.is_ok());
        assert!(result.unwrap().contains("println"));
    }
}
