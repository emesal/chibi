//! VFS backend trait: the storage abstraction.
//!
//! Backends are intentionally simple — just storage, no permission logic.
//! The `Vfs` struct handles permissions and delegates to the backend.
//!
//! # Implementing a backend
//!
//! Implement all methods on `VfsBackend`. The `VfsPath` values you receive
//! are already validated; you only need to map them to your storage.
//!
//! # Future: middleware layers
//!
//! The current design (approach A) has `Vfs` call the backend directly.
//! A future evolution (approach C) would wrap backends in composable
//! middleware layers (logging, caching, etc.) a la tower. The trait
//! signature is designed to be compatible with that transition.
//!
//! # Dyn-compatibility
//!
//! Methods return `Pin<Box<dyn Future>>` instead of `impl Future` so that
//! `Box<dyn VfsBackend>` works. This enables multi-backend mounting where
//! `Vfs` selects a backend at runtime via longest-prefix match.
//!
//! All input references share a single lifetime `'a` so the returned
//! future can borrow from both `&self` and any path arguments.

use std::future::Future;
use std::io;
use std::pin::Pin;

use super::path::VfsPath;
use super::types::{VfsEntry, VfsMetadata};

/// Boxed, Send future — the return type for all backend methods.
pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Storage backend for the virtual file system.
///
/// All methods receive validated `VfsPath` values. Backends translate these
/// to their native addressing (file paths, database keys, XMPP nodes, etc.).
///
/// All methods are async (returning boxed futures) to accommodate
/// network-backed implementations and dyn-compatibility.
pub trait VfsBackend: Send + Sync {
    /// Read the full contents of a file.
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>>;

    /// Write (create or overwrite) a file with the given contents.
    fn write<'a>(&'a self, path: &'a VfsPath, data: &'a [u8]) -> BoxFuture<'a, io::Result<()>>;

    /// Append data to an existing file, creating it if it doesn't exist.
    fn append<'a>(&'a self, path: &'a VfsPath, data: &'a [u8]) -> BoxFuture<'a, io::Result<()>>;

    /// Delete a file. Returns `NotFound` if the path doesn't exist.
    fn delete<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>>;

    /// List entries in a directory. Returns empty vec if path doesn't exist.
    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>>;

    /// Check whether a path exists.
    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>>;

    /// Create a directory (and parents if needed).
    fn mkdir<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>>;

    /// Copy a file from src to dst. Both paths are within this backend.
    fn copy<'a>(&'a self, src: &'a VfsPath, dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>>;

    /// Rename (move) a file from src to dst. Both paths are within this backend.
    fn rename<'a>(&'a self, src: &'a VfsPath, dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>>;

    /// Get metadata for a path.
    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>>;
}

/// Sub-trait for read-only virtual VFS backends.
///
/// Implementors provide only the read operations (`read`, `list`, `exists`,
/// `metadata`). The blanket `VfsBackend` impl fills in all write operations
/// (`write`, `append`, `delete`, `mkdir`, `copy`, `rename`) with
/// `PermissionDenied` errors that include `backend_name()` for diagnostics.
pub trait ReadOnlyVfsBackend: Send + Sync {
    /// Human-readable name for error messages (e.g. "virtual tool registry").
    fn backend_name(&self) -> &str;

    /// Read the full contents of a virtual file.
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>>;

    /// List entries in a virtual directory.
    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>>;

    /// Check whether a virtual path exists.
    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>>;

    /// Get metadata for a virtual path.
    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>>;
}

impl<T: ReadOnlyVfsBackend> VfsBackend for T {
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        ReadOnlyVfsBackend::read(self, path)
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        ReadOnlyVfsBackend::list(self, path)
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        ReadOnlyVfsBackend::exists(self, path)
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        ReadOnlyVfsBackend::metadata(self, path)
    }

    fn write<'a>(&'a self, path: &'a VfsPath, _data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let path_str = path.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path_str, name),
            ))
        })
    }

    fn append<'a>(&'a self, path: &'a VfsPath, _data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let path_str = path.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path_str, name),
            ))
        })
    }

    fn delete<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let path_str = path.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path_str, name),
            ))
        })
    }

    fn mkdir<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let path_str = path.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path_str, name),
            ))
        })
    }

    fn copy<'a>(&'a self, src: &'a VfsPath, _dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let src_str = src.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", src_str, name),
            ))
        })
    }

    fn rename<'a>(&'a self, src: &'a VfsPath, _dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name().to_string();
        let src_str = src.to_string();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", src_str, name),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::types::{VfsEntry, VfsEntryKind, VfsMetadata};
    use crate::vfs::path::VfsPath;

    /// Minimal read-only backend for testing the blanket impl.
    struct StubBackend;

    impl ReadOnlyVfsBackend for StubBackend {
        fn backend_name(&self) -> &str {
            "test stub"
        }

        fn read<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
            Box::pin(async { Ok(b"hello".to_vec()) })
        }

        fn list<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn exists<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
            Box::pin(async { Ok(true) })
        }

        fn metadata<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
            Box::pin(async {
                Ok(VfsMetadata {
                    size: 5,
                    created: None,
                    modified: None,
                    kind: VfsEntryKind::File,
                })
            })
        }
    }

    #[tokio::test]
    async fn test_read_only_backend_read_delegates() {
        let backend: &dyn VfsBackend = &StubBackend;
        let path = VfsPath::new("/test").unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn test_read_only_backend_write_rejected() {
        let backend: &dyn VfsBackend = &StubBackend;
        let path = VfsPath::new("/test").unwrap();
        let err = backend.write(&path, b"data").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("test stub"));
    }

    #[tokio::test]
    async fn test_read_only_backend_all_writes_rejected() {
        let backend: &dyn VfsBackend = &StubBackend;
        let p = VfsPath::new("/x").unwrap();
        let p2 = VfsPath::new("/y").unwrap();

        assert_eq!(backend.append(&p, b"d").await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.delete(&p).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.mkdir(&p).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.copy(&p, &p2).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.rename(&p, &p2).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
