//! Virtual VFS backend for `/tools/sys/`.
//!
//! `ToolsBackend` synthesises read-only JSON schema files from the
//! `ToolRegistry` on demand. Each tool appears as a file named after the tool:
//! reading `/shell_exec` returns the shell_exec schema as JSON.
//!
//! All write operations are rejected — `/tools/sys/` is virtual and has no
//! on-disk representation. The permission layer already enforces this (see
//! `permissions.rs`), but the backend also rejects writes explicitly so that
//! callers bypassing the permission layer also get an error.
//!
//! This backend is mounted at `/tools/sys/` by `Chibi::load_with_options()`.

use std::io;
use std::sync::{Arc, RwLock};

use serde_json::json;

use super::backend::{BoxFuture, ReadOnlyVfsBackend};
use super::path::VfsPath;
use super::types::{VfsEntry, VfsEntryKind, VfsMetadata};
use crate::tools::ToolRegistry;

/// Read-only VFS backend backed by the `ToolRegistry`.
///
/// Receives a stripped path (the `/tools/sys` prefix has already been removed
/// by `Vfs::resolve_backend`). Root `/` lists all tools; `/<name>` reads the
/// JSON schema for that tool.
pub struct ToolsBackend {
    registry: Arc<RwLock<ToolRegistry>>,
}

impl ToolsBackend {
    pub fn new(registry: Arc<RwLock<ToolRegistry>>) -> Self {
        Self { registry }
    }

    fn lock_err() -> io::Error {
        io::Error::other("ToolsBackend: registry lock poisoned")
    }

}

impl ReadOnlyVfsBackend for ToolsBackend {
    fn backend_name(&self) -> &str {
        "virtual tool registry"
    }

    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            // strip leading '/' to get tool name
            let name = path.as_str().trim_start_matches('/');
            if name.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot read directory as file; use list() instead",
                ));
            }
            let reg = self.registry.read().map_err(|_| Self::lock_err())?;
            let tool = reg.get(name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no tool: {name}"))
            })?;
            let schema = json!({
                "name": tool.name,
                "description": tool.description,
                "category": tool.category.as_str(),
                "parameters": tool.parameters,
            });
            Ok(serde_json::to_vec_pretty(&schema).expect("schema serialisation cannot fail"))
        })
    }

    fn list<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        Box::pin(async move {
            let reg = self.registry.read().map_err(|_| Self::lock_err())?;
            Ok(reg
                .all()
                .map(|t| VfsEntry {
                    name: t.name.clone(),
                    kind: VfsEntryKind::File,
                })
                .collect())
        })
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            let name = path.as_str().trim_start_matches('/');
            if name.is_empty() {
                return Ok(true); // root always exists
            }
            let reg = self.registry.read().map_err(|_| Self::lock_err())?;
            Ok(reg.get(name).is_some())
        })
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        Box::pin(async move {
            let name = path.as_str().trim_start_matches('/');
            if name.is_empty() {
                return Ok(VfsMetadata {
                    size: 0,
                    created: None,
                    modified: None,
                    kind: VfsEntryKind::Directory,
                });
            }
            let reg = self.registry.read().map_err(|_| Self::lock_err())?;
            if reg.get(name).is_some() {
                Ok(VfsMetadata {
                    size: 0,
                    created: None,
                    modified: None,
                    kind: VfsEntryKind::File,
                })
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no tool: {name}"),
                ))
            }
        })
    }

}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::tools::{Tool, ToolCategory, ToolHandler, ToolImpl, ToolMetadata, ToolRegistry};

    fn make_registry_with(name: &str, category: ToolCategory) -> Arc<RwLock<ToolRegistry>> {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        let tool = Tool {
            name: name.into(),
            description: format!("description for {name}"),
            parameters: serde_json::json!({}),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: ToolImpl::Builtin(handler),
            category,
        };
        registry.write().unwrap().register(tool);
        registry
    }

    #[tokio::test]
    async fn test_tools_backend_list_root() {
        let registry = make_registry_with("shell_exec", ToolCategory::Shell);
        let backend = ToolsBackend::new(registry);
        let root = VfsPath::new("/").unwrap();
        let entries = backend.list(&root).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "shell_exec");
        assert_eq!(entries[0].kind, VfsEntryKind::File);
    }

    #[tokio::test]
    async fn test_tools_backend_read_tool_schema() {
        let registry = make_registry_with("my_tool", ToolCategory::Network);
        let backend = ToolsBackend::new(registry);
        let path = VfsPath::new("/my_tool").unwrap();
        let data = backend.read(&path).await.unwrap();
        let schema: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(schema["name"], "my_tool");
        assert_eq!(schema["description"], "description for my_tool");
        assert_eq!(schema["category"], "network");
    }

    #[tokio::test]
    async fn test_tools_backend_read_unknown_tool() {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let backend = ToolsBackend::new(registry);
        let path = VfsPath::new("/nonexistent").unwrap();
        let err = backend.read(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_tools_backend_write_rejected() {
        #[allow(unused_imports)]
        use crate::vfs::VfsBackend;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let backend: Box<dyn VfsBackend> = Box::new(ToolsBackend::new(registry));
        let path = VfsPath::new("/anything").unwrap();
        let err = backend.write(&path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_tools_backend_exists() {
        let registry = make_registry_with("fetch_url", ToolCategory::Network);
        let backend = ToolsBackend::new(registry);
        assert!(
            backend
                .exists(&VfsPath::new("/fetch_url").unwrap())
                .await
                .unwrap()
        );
        assert!(
            !backend
                .exists(&VfsPath::new("/nope").unwrap())
                .await
                .unwrap()
        );
        assert!(backend.exists(&VfsPath::new("/").unwrap()).await.unwrap()); // root exists
    }

    #[tokio::test]
    async fn test_tools_backend_metadata_dir_at_root() {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let backend = ToolsBackend::new(registry);
        let meta = backend.metadata(&VfsPath::new("/").unwrap()).await.unwrap();
        assert_eq!(meta.kind, VfsEntryKind::Directory);
    }
}
