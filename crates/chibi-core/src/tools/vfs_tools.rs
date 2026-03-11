//! VFS tools for virtual filesystem operations.
//!
//! Thin async wrappers around `Vfs` methods, exposed as LLM-callable tools.
//! Each tool parses `vfs://` URIs from JSON args and delegates to the
//! appropriate `Vfs` method, returning human-readable result strings.

use super::{BuiltinToolDef, ToolPropertyDef, require_str_param};
use crate::vfs::{Vfs, VfsCaller, VfsEntryKind, VfsPath};
use std::io;

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
    caller: VfsCaller<'_>,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    let entries = vfs.list(caller, &path).await?;
    if entries.is_empty() {
        return Ok("No entries found.".to_string());
    }
    let lines: Vec<String> = entries
        .iter()
        .filter(|e| !e.name.starts_with('.'))
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
    caller: VfsCaller<'_>,
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
    caller: VfsCaller<'_>,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    vfs.mkdir(caller, &path).await?;
    Ok(format!("Created {}", path.as_str()))
}

/// Delete a file or directory from the VFS.
pub async fn execute_vfs_delete(
    vfs: &Vfs,
    caller: VfsCaller<'_>,
    args: &serde_json::Value,
) -> io::Result<String> {
    let path = require_vfs_path(args, "path")?;
    vfs.delete(caller, &path).await?;
    Ok(format!("Deleted {}", path.as_str()))
}

/// Copy a file within the VFS.
pub async fn execute_vfs_copy(
    vfs: &Vfs,
    caller: VfsCaller<'_>,
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
    caller: VfsCaller<'_>,
    args: &serde_json::Value,
) -> io::Result<String> {
    let src = require_vfs_path(args, "src")?;
    let dst = require_vfs_path(args, "dst")?;
    vfs.rename(caller, &src, &dst).await?;
    Ok(format!("Moved {} -> {}", src.as_str(), dst.as_str()))
}

/// Register all VFS tools into the registry.
pub fn register_vfs_tools(registry: &mut super::registry::ToolRegistry) {
    use super::Tool;
    use super::registry::{ToolCategory, ToolHandler};
    use std::sync::Arc;

    let handler: ToolHandler = Arc::new(|call| {
        // execute_vfs_tool is async. We can't hold &Vfs or VfsCaller<'_> across
        // .await because Vfs contains !Sync RefCell fields. Instead, call the
        // function synchronously-wrapped in an async block — it borrows vfs only
        // within the async fn body and those borrows don't cross an .await in the
        // sense that VfsCaller is passed by value (Copy) and &Vfs is borrowed for
        // the duration of the call, not stored into the future state machine.
        // This works because the future is !Send (BoxFuture is not Send), so the
        // future stays on the same thread as the &Vfs it borrows.
        let ctx = call.context;
        let name = call.name;
        let args = call.args;
        let vfs = ctx.vfs;
        let caller = ctx.vfs_caller;
        Box::pin(async move {
            execute_vfs_tool(vfs, caller, name, args)
                .await
                .unwrap_or_else(|| {
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("unknown vfs tool: {name}"),
                    ))
                })
        })
    });

    for def in VFS_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(
            def,
            handler.clone(),
            ToolCategory::Vfs,
        ));
    }
}

// === Dispatch ===

/// Execute a VFS tool by name.
///
/// Returns `None` if `tool_name` is not a VFS tool, allowing callers to
/// chain with other tool dispatchers.
pub async fn execute_vfs_tool(
    vfs: &Vfs,
    caller: VfsCaller<'_>,
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
    use crate::vfs::{LocalBackend, Vfs, VfsCaller, VfsPath};
    use tempfile::TempDir;

    fn setup_vfs() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        (dir, Vfs::new(Box::new(backend), "test-site-0000"))
    }

    #[tokio::test]
    async fn test_execute_vfs_list() {
        let (_dir, vfs) = setup_vfs();
        vfs.write(
            VfsCaller::Context("ctx"),
            &VfsPath::new("/shared/a.txt").unwrap(),
            b"a",
        )
        .await
        .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared"});
        let result = execute_vfs_list(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("a.txt"));
    }

    #[tokio::test]
    async fn test_execute_vfs_list_nonexistent() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///shared/nope"});
        let result = execute_vfs_list(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
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
        vfs.write(
            VfsCaller::Context("ctx"),
            &VfsPath::new("/shared/f.txt").unwrap(),
            b"hello",
        )
        .await
        .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared/f.txt"});
        let result = execute_vfs_info(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("5")); // 5 bytes
        assert!(result.contains("file"));
    }

    #[tokio::test]
    async fn test_execute_vfs_mkdir() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///shared/newdir"});
        let result = execute_vfs_mkdir(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("Created"));
    }

    #[tokio::test]
    async fn test_execute_vfs_delete() {
        let (_dir, vfs) = setup_vfs();
        vfs.write(
            VfsCaller::Context("ctx"),
            &VfsPath::new("/shared/del.txt").unwrap(),
            b"x",
        )
        .await
        .unwrap();
        let args = serde_json::json!({"path": "vfs:///shared/del.txt"});
        let result = execute_vfs_delete(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("Deleted"));
    }

    #[tokio::test]
    async fn test_execute_vfs_copy() {
        let (_dir, vfs) = setup_vfs();
        vfs.write(
            VfsCaller::Context("ctx"),
            &VfsPath::new("/shared/src.txt").unwrap(),
            b"data",
        )
        .await
        .unwrap();
        let args = serde_json::json!({
            "src": "vfs:///shared/src.txt",
            "dst": "vfs:///shared/dst.txt"
        });
        let result = execute_vfs_copy(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("Copied"));
    }

    #[tokio::test]
    async fn test_execute_vfs_move() {
        let (_dir, vfs) = setup_vfs();
        vfs.write(
            VfsCaller::Context("ctx"),
            &VfsPath::new("/shared/old.txt").unwrap(),
            b"data",
        )
        .await
        .unwrap();
        let args = serde_json::json!({
            "src": "vfs:///shared/old.txt",
            "dst": "vfs:///shared/new.txt"
        });
        let result = execute_vfs_move(&vfs, VfsCaller::Context("ctx"), &args)
            .await
            .unwrap();
        assert!(result.contains("Moved"));
    }

    #[tokio::test]
    async fn test_vfs_tool_permission_denied() {
        let (_dir, vfs) = setup_vfs();
        let args = serde_json::json!({"path": "vfs:///sys/forbidden"});
        let result = execute_vfs_mkdir(&vfs, VfsCaller::Context("ctx"), &args).await;
        assert!(result.is_err());
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

    #[tokio::test]
    async fn test_vfs_list_hides_dotfiles() {
        let (_dir, vfs) = setup_vfs();

        // Use System caller to bypass zone restrictions for setup.
        // Write a regular file and a dotfile directory in /shared/.
        vfs.write(
            VfsCaller::System,
            &VfsPath::new("/shared/tool.scm").unwrap(),
            b"content",
        )
        .await
        .unwrap();
        vfs.write(
            VfsCaller::System,
            &VfsPath::new("/shared/.chibi/history/tool.scm/meta").unwrap(),
            b"meta",
        )
        .await
        .unwrap();

        let args = serde_json::json!({"path": "vfs:///shared"});
        let result = execute_vfs_list(&vfs, VfsCaller::System, &args)
            .await
            .unwrap();

        assert!(result.contains("tool.scm"), "regular file should appear");
        assert!(!result.contains(".chibi"), "dotfile directory should be hidden");
    }
}
