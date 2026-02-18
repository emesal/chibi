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
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
