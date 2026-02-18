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

use std::io;

use super::path::VfsPath;
use super::types::{VfsEntry, VfsMetadata};

/// Storage backend for the virtual file system.
///
/// All methods receive validated `VfsPath` values. Backends translate these
/// to their native addressing (file paths, database keys, XMPP nodes, etc.).
///
/// All methods are async to accommodate network-backed implementations.
pub trait VfsBackend: Send + Sync {
    /// Read the full contents of a file.
    fn read(&self, path: &VfsPath) -> impl Future<Output = io::Result<Vec<u8>>> + Send;

    /// Write (create or overwrite) a file with the given contents.
    fn write(&self, path: &VfsPath, data: &[u8]) -> impl Future<Output = io::Result<()>> + Send;

    /// Append data to an existing file, creating it if it doesn't exist.
    fn append(&self, path: &VfsPath, data: &[u8]) -> impl Future<Output = io::Result<()>> + Send;

    /// Delete a file. Returns `NotFound` if the path doesn't exist.
    fn delete(&self, path: &VfsPath) -> impl Future<Output = io::Result<()>> + Send;

    /// List entries in a directory. Returns empty vec if path doesn't exist.
    fn list(&self, path: &VfsPath) -> impl Future<Output = io::Result<Vec<VfsEntry>>> + Send;

    /// Check whether a path exists.
    fn exists(&self, path: &VfsPath) -> impl Future<Output = io::Result<bool>> + Send;

    /// Create a directory (and parents if needed).
    fn mkdir(&self, path: &VfsPath) -> impl Future<Output = io::Result<()>> + Send;

    /// Copy a file from src to dst. Both paths are within this backend.
    fn copy(&self, src: &VfsPath, dst: &VfsPath)
        -> impl Future<Output = io::Result<()>> + Send;

    /// Rename (move) a file from src to dst. Both paths are within this backend.
    fn rename(&self, src: &VfsPath, dst: &VfsPath)
        -> impl Future<Output = io::Result<()>> + Send;

    /// Get metadata for a path.
    fn metadata(&self, path: &VfsPath)
        -> impl Future<Output = io::Result<VfsMetadata>> + Send;
}
