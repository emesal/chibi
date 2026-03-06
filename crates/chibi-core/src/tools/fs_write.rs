//!
//! fs_write tools: write access to OS and VFS paths.
//! write_file, file_edit.
//! Callers must fire PreFileWrite hook before invoking these.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use super::fs_read::vfs_block_on;
use super::paths::{ResolvedPath, resolve_tool_path};
use super::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::config::ResolvedConfig;
use crate::vfs::{Vfs, VfsCaller, VfsPath};

// === Tool Name Constants ===

pub const WRITE_FILE_TOOL_NAME: &str = "write_file";
pub const FILE_EDIT_TOOL_NAME: &str = "file_edit";

// === Tool Definition Registry ===

/// All fs_write tool definitions
pub static FS_WRITE_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: WRITE_FILE_TOOL_NAME,
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Requires user permission.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to write to, or a vfs:/// URI for VFS storage",
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
];

// === Registry Helpers ===

/// Convert all fs_write tools to API format
pub fn all_fs_write_tools_to_api_format() -> Vec<serde_json::Value> {
    FS_WRITE_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the fs_write group
pub fn is_fs_write_tool(name: &str) -> bool {
    FS_WRITE_TOOL_DEFS.iter().any(|d| d.name == name)
}

// === Tool Execution ===

/// Execute an fs_write tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
/// Note: permission gating (PreFileWrite hook) must be applied by the caller.
pub fn execute_fs_write_tool(
    tool_name: &str,
    args: &serde_json::Value,
    project_root: &Path,
    config: &ResolvedConfig,
    vfs: &Vfs,
    caller: VfsCaller<'_>,
) -> Option<io::Result<String>> {
    match tool_name {
        WRITE_FILE_TOOL_NAME => {
            let path = require_str_param(args, "path");
            let content = require_str_param(args, "content");
            match (path, content) {
                (Ok(p), Ok(c)) => Some(execute_write_file(&p, &c, Some((vfs, caller)))),
                (Err(e), _) | (_, Err(e)) => Some(Err(e)),
            }
        }
        FILE_EDIT_TOOL_NAME => Some(execute_file_edit(args, project_root, config, vfs, caller)),
        _ => None,
    }
}

// === write_file ===

/// Execute write_file: write content to a file or VFS path.
///
/// Note: Permission check via pre_file_write hook happens in send.rs before this is called.
///
/// When `path` starts with `vfs:///`, the write is routed through the VFS.
/// The `vfs` parameter must be `Some((vfs, caller))` for VFS writes.
pub fn execute_write_file(
    path: &str,
    content: &str,
    vfs: Option<(&Vfs, VfsCaller<'_>)>,
) -> io::Result<String> {
    if VfsPath::is_vfs_uri(path) {
        let vfs_path = VfsPath::from_uri(path)?;
        let (vfs, caller) =
            vfs.ok_or_else(|| io::Error::other("VFS not available for vfs:// path"))?;
        vfs_block_on(vfs.write(caller, &vfs_path, content.as_bytes()))?;
        return Ok(format!(
            "File written successfully: {} ({} bytes)",
            vfs_path,
            content.len()
        ));
    }

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

// === file_edit ===

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
///
/// For VFS paths (`vfs:///...`), reads via `vfs.read()`, applies the edit
/// in memory, and writes back via `vfs.write()`.
pub fn execute_file_edit(
    args: &serde_json::Value,
    project_root: &Path,
    config: &ResolvedConfig,
    vfs: &Vfs,
    caller: VfsCaller<'_>,
) -> io::Result<String> {
    let path_str = require_str_param(args, "path")?;
    let operation_str = require_str_param(args, "operation")?;
    let op = parse_edit_operation(&operation_str, args)?;

    let resolved = resolve_tool_path(&path_str, project_root, config)?;

    match resolved {
        ResolvedPath::Os(file_path) => execute_file_edit_os(&file_path, op),
        ResolvedPath::Vfs(vfs_path) => execute_file_edit_vfs(vfs, caller, &vfs_path, op),
    }
}

/// Apply an edit operation to an OS file.
fn execute_file_edit_os(file_path: &Path, op: EditOperation) -> io::Result<String> {
    let content = std::fs::read_to_string(file_path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Failed to read '{}': {}", file_path.display(), e),
        )
    })?;

    let (new_content, summary) = apply_edit(op, &content, &file_path.display().to_string())?;
    crate::safe_io::atomic_write_text(file_path, &new_content)?;
    Ok(summary)
}

/// Apply an edit operation to a VFS file.
///
/// Read is world-readable by design (all zones can read all zones — see
/// `Vfs::check_read`). Write is zone-checked: only the owning zone or
/// callers with explicit cross-zone write access may write. This asymmetry
/// is intentional and documented in `docs/vfs.md`.
fn execute_file_edit_vfs(
    vfs: &Vfs,
    caller: VfsCaller<'_>,
    vfs_path: &VfsPath,
    op: EditOperation,
) -> io::Result<String> {
    let data = vfs_block_on(vfs.read(caller, vfs_path))?;
    let content = String::from_utf8_lossy(&data).into_owned();

    let (new_content, summary) = apply_edit(op, &content, vfs_path.as_str())?;
    vfs_block_on(vfs.write(caller, vfs_path, new_content.as_bytes()))?;
    Ok(summary)
}

/// Apply an edit operation to content, returning (new_content, summary).
///
/// Shared by both OS and VFS edit paths — the edit logic is identical,
/// only read/write differs.
fn apply_edit(
    op: EditOperation,
    content: &str,
    display_path: &str,
) -> io::Result<(String, String)> {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let had_trailing_newline = content.ends_with('\n');

    match op {
        EditOperation::ReplaceLines {
            start,
            end,
            content: new_content,
        } => {
            validate_line_range(start, end, lines.len())?;
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
            if !content.contains(&find) {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("String not found in file: {:?}", find),
                ));
            }
            // `replacen` operates on the raw string, so the trailing newline is preserved
            // naturally if the original had one. Early return is correct here; the line-based
            // newline restoration below is not needed.
            let new_content = content.replacen(&find, &replace, 1);
            return Ok((
                new_content,
                format!(
                    "Replaced string in {} ({} → {} bytes)",
                    display_path,
                    find.len(),
                    replace.len()
                ),
            ));
        }
    }

    // Reassemble lines and restore trailing newline
    let line_count = lines.len();
    let mut new_content = lines.join("\n");
    if had_trailing_newline || !new_content.is_empty() {
        new_content.push('\n');
    }

    Ok((
        new_content,
        format!("Edited {} ({} lines)", display_path, line_count),
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
    use crate::json_ext::JsonExt;
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

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
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

    fn make_test_vfs(dir: &TempDir) -> Vfs {
        let backend = LocalBackend::new(dir.path().to_path_buf());
        Vfs::new(Box::new(backend), "test-site-0000")
    }

    // === Registry tests ===

    #[test]
    fn test_fs_write_tool_defs_api_format() {
        for def in FS_WRITE_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
            assert!(api["function"]["description"].is_string());
        }
    }

    #[test]
    fn test_is_fs_write_tool() {
        assert!(is_fs_write_tool(WRITE_FILE_TOOL_NAME));
        assert!(is_fs_write_tool(FILE_EDIT_TOOL_NAME));
        assert!(!is_fs_write_tool("file_head"));
        assert!(!is_fs_write_tool("shell_exec"));
        assert!(!is_fs_write_tool("unknown"));
    }

    #[test]
    fn test_fs_write_tool_registry_count() {
        assert_eq!(FS_WRITE_TOOL_DEFS.len(), 2);
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(WRITE_FILE_TOOL_NAME, "write_file");
        assert_eq!(FILE_EDIT_TOOL_NAME, "file_edit");
    }

    // === write_file tests ===

    #[test]
    fn test_execute_write_file_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");

        let result = execute_write_file(path.to_str().unwrap(), "hello world", None);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("written successfully"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_execute_write_file_creates_parent_dirs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("nested/dir/test.txt");

        let result = execute_write_file(path.to_str().unwrap(), "content", None);
        assert!(result.is_ok());
        assert!(path.exists());
    }

    #[test]
    fn test_execute_write_file_overwrites() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");
        fs::write(&path, "old content").unwrap();

        let result = execute_write_file(path.to_str().unwrap(), "new content", None);
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_write_file_vfs() {
        let home = tempfile::tempdir().unwrap();
        let vfs = make_test_vfs(&home);

        let result = execute_write_file(
            "vfs:///shared/doc.txt",
            "hello vfs",
            Some((&vfs, VfsCaller::Context("ctx"))),
        );
        assert!(result.is_ok());
        assert!(result.unwrap().contains("written successfully"));

        let vfs_path = VfsPath::new("/shared/doc.txt").unwrap();
        let data = vfs
            .read(VfsCaller::Context("ctx"), &vfs_path)
            .await
            .unwrap();
        assert_eq!(data, b"hello vfs");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_write_file_vfs_permission_denied() {
        let home = tempfile::tempdir().unwrap();
        let vfs = make_test_vfs(&home);

        let result = execute_write_file(
            "vfs:///sys/config.txt",
            "forbidden",
            Some((&vfs, VfsCaller::Context("ctx"))),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }

    // === file_edit tests ===

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
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        )
        .unwrap();
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
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        )
        .unwrap();
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
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        )
        .unwrap();
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
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        )
        .unwrap();
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
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        )
        .unwrap();
        let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
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
        let result = execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        );
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
        let result = execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &make_test_vfs(&dir),
            VfsCaller::Context("test"),
        );
        assert!(result.is_err());
    }

    // === VFS file_edit tests ===

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_edit_replace_lines_vfs() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = make_test_vfs(&dir);
        let path = VfsPath::new("/shared/f.txt").unwrap();
        vfs.write(VfsCaller::Context("ctx"), &path, b"aaa\nbbb\nccc\n")
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/f.txt")),
            ("operation", serde_json::json!("replace_lines")),
            ("line_start", serde_json::json!(2)),
            ("line_end", serde_json::json!(2)),
            ("content", serde_json::json!("BBB")),
        ]);
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &vfs,
            VfsCaller::Context("ctx"),
        )
        .unwrap();
        let data = vfs.read(VfsCaller::Context("ctx"), &path).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&data), "aaa\nBBB\nccc\n");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_edit_replace_string_vfs() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = make_test_vfs(&dir);
        let path = VfsPath::new("/shared/f.txt").unwrap();
        vfs.write(
            VfsCaller::Context("ctx"),
            &path,
            b"hello world\nhello again\n",
        )
        .await
        .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///shared/f.txt")),
            ("operation", serde_json::json!("replace_string")),
            ("find", serde_json::json!("hello")),
            ("replace", serde_json::json!("goodbye")),
        ]);
        execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &vfs,
            VfsCaller::Context("ctx"),
        )
        .unwrap();
        let data = vfs.read(VfsCaller::Context("ctx"), &path).await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&data),
            "goodbye world\nhello again\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_edit_vfs_permission_denied() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = make_test_vfs(&dir);
        let path = VfsPath::new("/home/coder/f.txt").unwrap();
        vfs.write(VfsCaller::Context("coder"), &path, b"data\n")
            .await
            .unwrap();

        let a = args(&[
            ("path", serde_json::json!("vfs:///home/coder/f.txt")),
            ("operation", serde_json::json!("replace_lines")),
            ("line_start", serde_json::json!(1)),
            ("line_end", serde_json::json!(1)),
            ("content", serde_json::json!("hacked")),
        ]);
        let result = execute_file_edit(
            &a,
            dir.path(),
            &make_config_for(&dir),
            &vfs,
            VfsCaller::Context("planner"),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }
}
