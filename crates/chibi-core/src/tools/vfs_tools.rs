//! VFS tools for virtual filesystem operations.
//!
//! Thin async wrappers around `Vfs` methods, exposed as LLM-callable tools.
//! Each tool parses `vfs://` URIs from JSON args and delegates to the
//! appropriate `Vfs` method, returning human-readable result strings.

use super::builtin::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::vfs::{Vfs, VfsEntryKind, VfsPath};
use std::io::{self, ErrorKind};

// === Tool Name Constants ===

pub const VFS_LIST_TOOL_NAME: &str = "vfs_list";
pub const VFS_INFO_TOOL_NAME: &str = "vfs_info";
pub const VFS_COPY_TOOL_NAME: &str = "vfs_copy";
pub const VFS_MOVE_TOOL_NAME: &str = "vfs_move";
pub const VFS_MKDIR_TOOL_NAME: &str = "vfs_mkdir";
pub const VFS_DELETE_TOOL_NAME: &str = "vfs_delete";

// === Tool Definition Registry ===

/// All VFS tool definitions.
pub static VFS_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: VFS_LIST_TOOL_NAME,
        description: "List entries in a VFS directory. Returns names and types of children.",
        properties: &[ToolPropertyDef {
            name: "path",
            prop_type: "string",
            description: "VFS URI of the directory to list (e.g. vfs:///shared)",
            default: None,
        }],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: VFS_INFO_TOOL_NAME,
        description: "Get metadata (size, kind, timestamps) for a VFS path.",
        properties: &[ToolPropertyDef {
            name: "path",
            prop_type: "string",
            description: "VFS URI of the entry to inspect (e.g. vfs:///shared/file.txt)",
            default: None,
        }],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: VFS_COPY_TOOL_NAME,
        description: "Copy a file within the VFS.",
        properties: &[
            ToolPropertyDef {
                name: "src",
                prop_type: "string",
                description: "VFS URI of the source file",
                default: None,
            },
            ToolPropertyDef {
                name: "dst",
                prop_type: "string",
                description: "VFS URI of the destination",
                default: None,
            },
        ],
        required: &["src", "dst"],
        summary_params: &["src", "dst"],
    },
    BuiltinToolDef {
        name: VFS_MOVE_TOOL_NAME,
        description: "Move (rename) a file within the VFS.",
        properties: &[
            ToolPropertyDef {
                name: "src",
                prop_type: "string",
                description: "VFS URI of the source file",
                default: None,
            },
            ToolPropertyDef {
                name: "dst",
                prop_type: "string",
                description: "VFS URI of the destination",
                default: None,
            },
        ],
        required: &["src", "dst"],
        summary_params: &["src", "dst"],
    },
    BuiltinToolDef {
        name: VFS_MKDIR_TOOL_NAME,
        description: "Create a directory in the VFS.",
        properties: &[ToolPropertyDef {
            name: "path",
            prop_type: "string",
            description: "VFS URI of the directory to create (e.g. vfs:///shared/newdir)",
            default: None,
        }],
        required: &["path"],
        summary_params: &["path"],
    },
    BuiltinToolDef {
        name: VFS_DELETE_TOOL_NAME,
        description: "Delete a file or directory from the VFS.",
        properties: &[ToolPropertyDef {
            name: "path",
            prop_type: "string",
            description: "VFS URI of the entry to delete",
            default: None,
        }],
        required: &["path"],
        summary_params: &["path"],
    },
];

// === Helpers ===

/// Parse a `vfs://` URI parameter into a `VfsPath`.
fn require_vfs_path(args: &serde_json::Value, name: &str) -> io::Result<VfsPath> {
    let uri = require_str_param(args, name)?;
    VfsPath::from_uri(&uri)
}

// === Tool Implementations ===

/// List entries in a VFS directory.
pub async fn execute_vfs_list(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    let entries = vfs.list(caller, &path).await?;
    if entries.is_empty() {
        return Ok("No entries found.".to_string());
    }
    let lines: Vec<String> = entries
        .iter()
        .map(|e| {
            let kind = match e.kind {
                VfsEntryKind::File => "file",
                VfsEntryKind::Directory => "dir",
            };
            format!("{} ({})", e.name, kind)
        })
        .collect();
    Ok(lines.join("\n"))
}

/// Get metadata for a VFS path.
pub async fn execute_vfs_info(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    let meta = vfs.metadata(caller, &path).await?;
    let kind = match meta.kind {
        VfsEntryKind::File => "file",
        VfsEntryKind::Directory => "directory",
    };
    let mut parts = vec![
        format!("kind: {}", kind),
        format!("size: {} bytes", meta.size),
    ];
    if let Some(created) = meta.created {
        parts.push(format!("created: {}", created));
    }
    if let Some(modified) = meta.modified {
        parts.push(format!("modified: {}", modified));
    }
    Ok(parts.join("\n"))
}

/// Create a directory in the VFS.
pub async fn execute_vfs_mkdir(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    vfs.mkdir(caller, &path).await?;
    Ok(format!("Created {}", path.as_str()))
}

/// Delete a file or directory from the VFS.
pub async fn execute_vfs_delete(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    vfs.delete(caller, &path).await?;
    Ok(format!("Deleted {}", path.as_str()))
}

/// Copy a file within the VFS.
pub async fn execute_vfs_copy(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let src = require_vfs_path(args, "src")?;
    let dst = require_vfs_path(args, "dst")?;
    vfs.copy(caller, &src, &dst).await?;
    Ok(format!("Copied {} -> {}", src.as_str(), dst.as_str()))
}

/// Move (rename) a file within the VFS.
pub async fn execute_vfs_move(
    vfs: &Vfs,
    caller: &str,
    args: &serde_json::Value,
) -> io::Result<String> {
    let src = require_vfs_path(args, "src")?;
    let dst = require_vfs_path(args, "dst")?;
    vfs.rename(caller, &src, &dst).await?;
    Ok(format!("Moved {} -> {}", src.as_str(), dst.as_str()))
}

// === Dispatch ===

/// Check if a tool name is a VFS tool.
pub fn is_vfs_tool(name: &str) -> bool {
    matches!(
        name,
        VFS_LIST_TOOL_NAME
            | VFS_INFO_TOOL_NAME
            | VFS_COPY_TOOL_NAME
            | VFS_MOVE_TOOL_NAME
            | VFS_MKDIR_TOOL_NAME
            | VFS_DELETE_TOOL_NAME
    )
}

/// Execute a VFS tool by name.
///
/// Returns `None` if `tool_name` is not a VFS tool, allowing callers to
/// chain with other tool dispatchers.
pub async fn execute_vfs_tool(
    vfs: &Vfs,
    caller: &str,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<io::Result<String>> {
    match tool_name {
        VFS_LIST_TOOL_NAME => Some(execute_vfs_list(vfs, caller, args).await),
        VFS_INFO_TOOL_NAME => Some(execute_vfs_info(vfs, caller, args).await),
        VFS_COPY_TOOL_NAME => Some(execute_vfs_copy(vfs, caller, args).await),
        VFS_MOVE_TOOL_NAME => Some(execute_vfs_move(vfs, caller, args).await),
        VFS_MKDIR_TOOL_NAME => Some(execute_vfs_mkdir(vfs, caller, args).await),
        VFS_DELETE_TOOL_NAME => Some(execute_vfs_delete(vfs, caller, args).await),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{LocalBackend, Vfs, VfsPath};
    use tempfile::TempDir;

    fn setup_vfs() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        (dir, Vfs::new(Box::new(backend)))
    }

    #[tokio::test]
    async fn test_execute_vfs_list() {
        let (_dir, vfs) = setup_vfs();
        vfs.write("ctx", &VfsPath::new("/shared/a.txt").unwrap(), b"a")
            .await
            .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared"});
        let result = execute_vfs_list(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("a.txt"));
    }

    #[tokio::test]
    async fn test_execute_vfs_list_nonexistent() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///shared/nope"});
        let result = execute_vfs_list(&vfs, "ctx", &args).await.unwrap();
        assert!(
            result.contains("empty")
                || result.contains("no entries")
                || result.is_empty()
                || result.contains("No entries")
        );
    }

    #[tokio::test]
    async fn test_execute_vfs_info() {
        let (_dir, vfs) = setup_vfs();
        vfs.write("ctx", &VfsPath::new("/shared/f.txt").unwrap(), b"hello")
            .await
            .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared/f.txt"});
        let result = execute_vfs_info(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("5")); // 5 bytes
        assert!(result.contains("file"));
    }

    #[tokio::test]
    async fn test_execute_vfs_mkdir() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///shared/newdir"});
        let result = execute_vfs_mkdir(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("Created"));
    }

    #[tokio::test]
    async fn test_execute_vfs_delete() {
        let (_dir, vfs) = setup_vfs();
        vfs.write("ctx", &VfsPath::new("/shared/del.txt").unwrap(), b"x")
            .await
            .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared/del.txt"});
        let result = execute_vfs_delete(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("Deleted"));
    }

    #[tokio::test]
    async fn test_execute_vfs_copy() {
        let (_dir, vfs) = setup_vfs();
        vfs.write("ctx", &VfsPath::new("/shared/src.txt").unwrap(), b"data")
            .await
            .unwrap();
        let args = serde_json::json!({
            "src": "vfs:///shared/src.txt",
            "dst": "vfs:///shared/dst.txt"
        });
        let result = execute_vfs_copy(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("Copied"));
    }

    #[tokio::test]
    async fn test_execute_vfs_move() {
        let (_dir, vfs) = setup_vfs();
        vfs.write("ctx", &VfsPath::new("/shared/old.txt").unwrap(), b"data")
            .await
            .unwrap();
        let args = serde_json::json!({
            "src": "vfs:///shared/old.txt",
            "dst": "vfs:///shared/new.txt"
        });
        let result = execute_vfs_move(&vfs, "ctx", &args).await.unwrap();
        assert!(result.contains("Moved"));
    }

    #[tokio::test]
    async fn test_vfs_tool_permission_denied() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///sys/forbidden"});
        let result = execute_vfs_mkdir(&vfs, "ctx", &args).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_is_vfs_tool() {
        assert!(is_vfs_tool("vfs_list"));
        assert!(is_vfs_tool("vfs_info"));
        assert!(is_vfs_tool("vfs_copy"));
        assert!(is_vfs_tool("vfs_move"));
        assert!(is_vfs_tool("vfs_mkdir"));
        assert!(is_vfs_tool("vfs_delete"));
        assert!(!is_vfs_tool("file_head"));
    }

    #[test]
    fn test_vfs_tool_defs_have_descriptions() {
        for def in VFS_TOOL_DEFS {
            assert!(
                !def.description.is_empty(),
                "{} missing description",
                def.name
            );
        }
    }
}
