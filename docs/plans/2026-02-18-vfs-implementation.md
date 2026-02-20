# VFS Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a sandboxed virtual file system to chibi-core, shared between contexts, with zone-based permissions, an async trait-based backend, and `vfs://` prefix integration into existing tools.

**Architecture:** A `Vfs` struct (permission enforcer + router) wraps a `VfsBackend` trait (dumb storage). The local-disk backend maps VFS paths to `~/.chibi/vfs/`. Existing tools detect the `vfs://` prefix and route through the VFS. See `docs/plans/2026-02-18-vfs-design.md` for the full design.

**Tech Stack:** Rust (edition 2024, native async traits), tokio (already in deps), `safe_io::atomic_write` for disk backend writes.

---

### Task 1: VfsPath newtype and validation

**Files:**
- Create: `crates/chibi-core/src/vfs/mod.rs`
- Create: `crates/chibi-core/src/vfs/path.rs`
- Modify: `crates/chibi-core/src/lib.rs` (add `pub mod vfs;`)

**Step 1: Write the failing tests**

In `crates/chibi-core/src/vfs/path.rs`, add a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_paths() {
        assert!(VfsPath::new("/shared/foo.txt").is_ok());
        assert!(VfsPath::new("/home/planner/notes.md").is_ok());
        assert!(VfsPath::new("/sys/info").is_ok());
        assert!(VfsPath::new("/").is_ok());
    }

    #[test]
    fn test_rejects_dotdot() {
        assert!(VfsPath::new("/shared/../etc/passwd").is_err());
        assert!(VfsPath::new("/home/ctx/../../secret").is_err());
    }

    #[test]
    fn test_rejects_double_slash() {
        assert!(VfsPath::new("/shared//foo").is_err());
    }

    #[test]
    fn test_rejects_no_leading_slash() {
        assert!(VfsPath::new("shared/foo").is_err());
        assert!(VfsPath::new("").is_err());
    }

    #[test]
    fn test_rejects_null_bytes() {
        assert!(VfsPath::new("/shared/\0bad").is_err());
    }

    #[test]
    fn test_rejects_trailing_slash_except_root() {
        assert!(VfsPath::new("/shared/").is_err());
        assert!(VfsPath::new("/").is_ok());
    }

    #[test]
    fn test_as_str() {
        let p = VfsPath::new("/shared/foo.txt").unwrap();
        assert_eq!(p.as_str(), "/shared/foo.txt");
    }

    #[test]
    fn test_parent() {
        let p = VfsPath::new("/shared/sub/foo.txt").unwrap();
        assert_eq!(p.parent().unwrap().as_str(), "/shared/sub");
        let root = VfsPath::new("/").unwrap();
        assert!(root.parent().is_none());
    }

    #[test]
    fn test_file_name() {
        let p = VfsPath::new("/shared/foo.txt").unwrap();
        assert_eq!(p.file_name(), Some("foo.txt"));
        let root = VfsPath::new("/").unwrap();
        assert_eq!(root.file_name(), None);
    }

    #[test]
    fn test_join() {
        let base = VfsPath::new("/shared").unwrap();
        let joined = base.join("sub/file.txt").unwrap();
        assert_eq!(joined.as_str(), "/shared/sub/file.txt");
    }

    #[test]
    fn test_join_rejects_absolute() {
        let base = VfsPath::new("/shared").unwrap();
        assert!(base.join("/etc/passwd").is_err());
    }

    #[test]
    fn test_join_rejects_dotdot() {
        let base = VfsPath::new("/shared").unwrap();
        assert!(base.join("../etc/passwd").is_err());
    }

    #[test]
    fn test_from_vfs_uri() {
        let p = VfsPath::from_uri("vfs:///shared/foo.txt").unwrap();
        assert_eq!(p.as_str(), "/shared/foo.txt");
    }

    #[test]
    fn test_from_vfs_uri_rejects_non_vfs() {
        assert!(VfsPath::from_uri("/shared/foo.txt").is_err());
        assert!(VfsPath::from_uri("file:///shared/foo.txt").is_err());
    }

    #[test]
    fn test_is_vfs_uri() {
        assert!(VfsPath::is_vfs_uri("vfs:///shared/foo.txt"));
        assert!(!VfsPath::is_vfs_uri("/shared/foo.txt"));
        assert!(!VfsPath::is_vfs_uri("vfs://shared")); // only two slashes
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core vfs::path::tests`
Expected: compile errors (module doesn't exist yet)

**Step 3: Write the implementation**

Create `crates/chibi-core/src/vfs/mod.rs`:

```rust
//! Virtual file system: sandboxed, shared storage for contexts.
//!
//! The VFS provides a permission-enforced file namespace that contexts can
//! read and write without engaging the OS-level permission system. Paths use
//! a `vfs://` URI scheme and never leak OS `PathBuf` values.
//!
//! # Architecture (Approach A — thin trait, fat router)
//!
//! ```text
//! tool code  ->  Vfs (permissions + path validation)  ->  VfsBackend (dumb storage)
//! ```
//!
//! The `Vfs` struct enforces zone-based permissions and delegates to a
//! `VfsBackend` trait implementation. Backends are trivially simple — just
//! storage, no permission logic.
//!
//! # Future evolution
//!
//! - **Multi-backend mounting**: `Vfs` maps path prefixes to different backends
//!   (e.g. `/shared/` on disk, `/remote/` on XMPP). Longest-prefix match.
//! - **Middleware layers**: Composable tower-style layers (logging, caching)
//!   wrapping backends (approach C in the design doc). Refactor when needed.

pub mod path;

pub use path::VfsPath;
```

Create `crates/chibi-core/src/vfs/path.rs` with the `VfsPath` newtype:

```rust
//! VFS path newtype: the only way to address VFS content.
//!
//! `VfsPath` is an opaque string validated on construction. It rejects `..`,
//! `//`, null bytes, and anything that could escape the VFS sandbox. OS
//! `PathBuf` values never appear in the VFS API — backends translate `VfsPath`
//! to their native addressing internally.

use std::fmt;
use std::io::{self, ErrorKind};

/// Opaque path within the virtual file system.
///
/// Invariants (enforced at construction):
/// - Starts with `/`
/// - No `..` components
/// - No `//` sequences
/// - No null bytes
/// - No trailing `/` (except root `/`)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsPath(String);

/// URI scheme prefix for VFS paths in tool arguments.
const VFS_URI_PREFIX: &str = "vfs://";

impl VfsPath {
    /// Create a new VFS path, validating all invariants.
    pub fn new(path: &str) -> io::Result<Self> {
        if path.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "VFS path cannot be empty"));
        }
        if !path.starts_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path must start with '/': {}", path),
            ));
        }
        if path.contains('\0') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "VFS path cannot contain null bytes",
            ));
        }
        if path != "/" && path.ends_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path cannot have trailing slash: {}", path),
            ));
        }
        if path.contains("//") {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path cannot contain '//': {}", path),
            ));
        }
        // Check for '..' in any component
        for component in path.split('/') {
            if component == ".." {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("VFS path cannot contain '..': {}", path),
                ));
            }
        }
        Ok(Self(path.to_string()))
    }

    /// The path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parent path, or `None` if this is the root.
    pub fn parent(&self) -> Option<VfsPath> {
        if self.0 == "/" {
            return None;
        }
        match self.0.rfind('/') {
            Some(0) => Some(VfsPath("/".to_string())),
            Some(pos) => Some(VfsPath(self.0[..pos].to_string())),
            None => None,
        }
    }

    /// Final component of the path, or `None` for root.
    pub fn file_name(&self) -> Option<&str> {
        if self.0 == "/" {
            return None;
        }
        self.0.rsplit('/').next()
    }

    /// Join a relative path onto this path.
    ///
    /// The segment must not start with `/` or contain `..`.
    pub fn join(&self, segment: &str) -> io::Result<VfsPath> {
        if segment.starts_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "join segment must be relative",
            ));
        }
        if segment.split('/').any(|c| c == "..") {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "join segment cannot contain '..'",
            ));
        }
        let combined = if self.0 == "/" {
            format!("/{}", segment)
        } else {
            format!("{}/{}", self.0, segment)
        };
        VfsPath::new(&combined)
    }

    /// Parse a `vfs:///path` URI into a `VfsPath`.
    pub fn from_uri(uri: &str) -> io::Result<Self> {
        if !Self::is_vfs_uri(uri) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("not a vfs:// URI: {}", uri),
            ));
        }
        // "vfs:///foo" -> "/foo"  (strip "vfs://")
        VfsPath::new(&uri[VFS_URI_PREFIX.len()..])
    }

    /// Check whether a string is a `vfs://` URI.
    ///
    /// Requires `vfs:///` (three slashes) so the path component starts with `/`.
    pub fn is_vfs_uri(s: &str) -> bool {
        s.starts_with("vfs:///")
    }
}

impl fmt::Display for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

Add to `crates/chibi-core/src/lib.rs`:

```rust
pub mod vfs;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core vfs::path::tests`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): add VfsPath newtype with validation and URI parsing
```

---

### Task 2: VfsBackend trait, types, and permission model

**Files:**
- Create: `crates/chibi-core/src/vfs/backend.rs`
- Create: `crates/chibi-core/src/vfs/types.rs`
- Create: `crates/chibi-core/src/vfs/permissions.rs`
- Modify: `crates/chibi-core/src/vfs/mod.rs` (add submodules, re-exports)

**Step 1: Write the failing tests**

In `crates/chibi-core/src/vfs/permissions.rs`, add tests for permission logic:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VfsPath;

    #[test]
    fn test_system_caller_can_write_anywhere() {
        let sys = VfsPath::new("/sys/config").unwrap();
        let shared = VfsPath::new("/shared/foo").unwrap();
        let home = VfsPath::new("/home/ctx/file").unwrap();
        assert!(check_write(SYSTEM_CALLER, &sys).is_ok());
        assert!(check_write(SYSTEM_CALLER, &shared).is_ok());
        assert!(check_write(SYSTEM_CALLER, &home).is_ok());
    }

    #[test]
    fn test_context_can_write_shared() {
        let p = VfsPath::new("/shared/tasks.md").unwrap();
        assert!(check_write("planner", &p).is_ok());
    }

    #[test]
    fn test_context_can_write_own_home() {
        let p = VfsPath::new("/home/planner/notes.md").unwrap();
        assert!(check_write("planner", &p).is_ok());
    }

    #[test]
    fn test_context_cannot_write_other_home() {
        let p = VfsPath::new("/home/coder/notes.md").unwrap();
        assert!(check_write("planner", &p).is_err());
    }

    #[test]
    fn test_context_cannot_write_sys() {
        let p = VfsPath::new("/sys/info").unwrap();
        assert!(check_write("planner", &p).is_err());
    }

    #[test]
    fn test_context_cannot_write_root() {
        let p = VfsPath::new("/random_file").unwrap();
        assert!(check_write("planner", &p).is_err());
    }

    #[test]
    fn test_read_always_allowed() {
        let paths = ["/shared/x", "/home/other/x", "/sys/x", "/root_file"];
        for p in &paths {
            let path = VfsPath::new(p).unwrap();
            assert!(check_read("anyctx", &path).is_ok());
        }
    }

    #[test]
    fn test_is_reserved_caller_name() {
        assert!(is_reserved_caller_name("SYSTEM"));
        assert!(is_reserved_caller_name("system"));
        assert!(is_reserved_caller_name("System"));
        assert!(!is_reserved_caller_name("planner"));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core vfs::permissions::tests`
Expected: compile errors

**Step 3: Write the implementation**

Create `crates/chibi-core/src/vfs/types.rs`:

```rust
//! VFS data types shared across the module.

use chrono::{DateTime, Utc};

/// Kind of entry in the VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsEntryKind {
    File,
    Directory,
}

/// Metadata for a VFS entry.
#[derive(Debug, Clone)]
pub struct VfsMetadata {
    pub size: u64,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub kind: VfsEntryKind,
}

/// A single entry returned by a directory listing.
///
/// Contains only name and kind — no metadata. Callers who need metadata
/// follow up with `VfsBackend::metadata()`. This keeps `list()` cheap for
/// backends where metadata is expensive.
#[derive(Debug, Clone)]
pub struct VfsEntry {
    pub name: String,
    pub kind: VfsEntryKind,
}
```

Create `crates/chibi-core/src/vfs/backend.rs`:

```rust
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
```

Create `crates/chibi-core/src/vfs/permissions.rs`:

```rust
//! VFS permission enforcement.
//!
//! Permissions are zone-based and determined entirely by path structure:
//! - `/shared/` — all contexts can read and write
//! - `/home/<context>/` — owner has read+write, others read-only
//! - `/sys/` — read-only (only SYSTEM can write)
//! - everything else at root level — read-only (only SYSTEM can write)

use std::io::{self, ErrorKind};

use super::path::VfsPath;

/// Reserved caller name with unrestricted write access to all zones.
pub const SYSTEM_CALLER: &str = "SYSTEM";

/// Check whether a caller name is reserved and cannot be used as a context name.
pub fn is_reserved_caller_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(SYSTEM_CALLER)
}

/// Check read permission. Always succeeds — all zones are world-readable.
pub fn check_read(_caller: &str, _path: &VfsPath) -> io::Result<()> {
    Ok(())
}

/// Check write permission based on caller identity and path zone.
pub fn check_write(caller: &str, path: &VfsPath) -> io::Result<()> {
    if caller == SYSTEM_CALLER {
        return Ok(());
    }

    let p = path.as_str();

    // /shared/ — world-writable
    if p == "/shared" || p.starts_with("/shared/") {
        return Ok(());
    }

    // /home/<caller>/ — owner-writable
    if let Some(rest) = p.strip_prefix("/home/") {
        let owner = rest.split('/').next().unwrap_or("");
        if owner == caller {
            return Ok(());
        }
    }

    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "context '{}' cannot write to '{}' (writable zones: /shared/, /home/{}/)",
            caller, path, caller
        ),
    ))
}
```

Update `crates/chibi-core/src/vfs/mod.rs` to add the new submodules:

```rust
//! Virtual file system: sandboxed, shared storage for contexts.
//!
//! The VFS provides a permission-enforced file namespace that contexts can
//! read and write without engaging the OS-level permission system. Paths use
//! a `vfs://` URI scheme and never leak OS `PathBuf` values.
//!
//! # Architecture (Approach A — thin trait, fat router)
//!
//! ```text
//! tool code  ->  Vfs (permissions + path validation)  ->  VfsBackend (dumb storage)
//! ```
//!
//! The `Vfs` struct enforces zone-based permissions and delegates to a
//! `VfsBackend` trait implementation. Backends are trivially simple — just
//! storage, no permission logic.
//!
//! # Future evolution
//!
//! - **Multi-backend mounting**: `Vfs` maps path prefixes to different backends
//!   (e.g. `/shared/` on disk, `/remote/` on XMPP). Longest-prefix match.
//! - **Middleware layers**: Composable tower-style layers (logging, caching)
//!   wrapping backends (approach C in the design doc). Refactor when needed.

pub mod backend;
pub mod path;
pub mod permissions;
pub mod types;

pub use backend::VfsBackend;
pub use path::VfsPath;
pub use permissions::{SYSTEM_CALLER, check_read, check_write, is_reserved_caller_name};
pub use types::{VfsEntry, VfsEntryKind, VfsMetadata};
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core vfs`
Expected: all pass (path tests + permission tests)

**Step 5: Commit**

```
feat(vfs): add backend trait, types, and permission model
```

---

### Task 3: LocalBackend implementation

**Files:**
- Create: `crates/chibi-core/src/vfs/local.rs`
- Modify: `crates/chibi-core/src/vfs/mod.rs` (add submodule, re-export)

**Step 1: Write the failing tests**

In `crates/chibi-core/src/vfs/local.rs`, add tests using `tempfile::TempDir`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, LocalBackend) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        (dir, backend)
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/test.txt").unwrap();
        backend.write(&path, b"hello").await.unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/home/ctx/deep/nested/file.txt").unwrap();
        backend.write(&path, b"nested").await.unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"nested");
    }

    #[tokio::test]
    async fn test_read_nonexistent_returns_not_found() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/nope.txt").unwrap();
        let err = backend.read(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_append() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/log.txt").unwrap();
        backend.append(&path, b"line1\n").await.unwrap();
        backend.append(&path, b"line2\n").await.unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"line1\nline2\n");
    }

    #[tokio::test]
    async fn test_append_creates_file() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/new.txt").unwrap();
        backend.append(&path, b"first").await.unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"first");
    }

    #[tokio::test]
    async fn test_delete() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/del.txt").unwrap();
        backend.write(&path, b"bye").await.unwrap();
        backend.delete(&path).await.unwrap();
        assert!(!backend.exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/nope.txt").unwrap();
        let err = backend.delete(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_list() {
        let (_dir, backend) = setup();
        let dir_path = VfsPath::new("/shared").unwrap();
        backend
            .write(&VfsPath::new("/shared/a.txt").unwrap(), b"a")
            .await
            .unwrap();
        backend
            .write(&VfsPath::new("/shared/b.txt").unwrap(), b"b")
            .await
            .unwrap();
        backend.mkdir(&VfsPath::new("/shared/sub").unwrap()).await.unwrap();

        let mut entries = backend.list(&dir_path).await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].kind, VfsEntryKind::File);
        assert_eq!(entries[2].name, "sub");
        assert_eq!(entries[2].kind, VfsEntryKind::Directory);
    }

    #[tokio::test]
    async fn test_list_nonexistent_returns_empty() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/nope").unwrap();
        let entries = backend.list(&path).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_exists() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/e.txt").unwrap();
        assert!(!backend.exists(&path).await.unwrap());
        backend.write(&path, b"x").await.unwrap();
        assert!(backend.exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn test_mkdir() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/home/ctx/subdir").unwrap();
        backend.mkdir(&path).await.unwrap();
        assert!(backend.exists(&path).await.unwrap());
        let meta = backend.metadata(&path).await.unwrap();
        assert_eq!(meta.kind, VfsEntryKind::Directory);
    }

    #[tokio::test]
    async fn test_copy() {
        let (_dir, backend) = setup();
        let src = VfsPath::new("/shared/orig.txt").unwrap();
        let dst = VfsPath::new("/shared/copy.txt").unwrap();
        backend.write(&src, b"content").await.unwrap();
        backend.copy(&src, &dst).await.unwrap();
        assert_eq!(backend.read(&dst).await.unwrap(), b"content");
        assert!(backend.exists(&src).await.unwrap()); // source still exists
    }

    #[tokio::test]
    async fn test_rename() {
        let (_dir, backend) = setup();
        let src = VfsPath::new("/shared/old.txt").unwrap();
        let dst = VfsPath::new("/shared/new.txt").unwrap();
        backend.write(&src, b"moved").await.unwrap();
        backend.rename(&src, &dst).await.unwrap();
        assert_eq!(backend.read(&dst).await.unwrap(), b"moved");
        assert!(!backend.exists(&src).await.unwrap()); // source gone
    }

    #[tokio::test]
    async fn test_metadata_file() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/meta.txt").unwrap();
        backend.write(&path, b"12345").await.unwrap();
        let meta = backend.metadata(&path).await.unwrap();
        assert_eq!(meta.size, 5);
        assert_eq!(meta.kind, VfsEntryKind::File);
        assert!(meta.modified.is_some());
    }

    #[tokio::test]
    async fn test_metadata_nonexistent() {
        let (_dir, backend) = setup();
        let path = VfsPath::new("/shared/nope").unwrap();
        let err = backend.metadata(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core vfs::local::tests`
Expected: compile errors

**Step 3: Write the implementation**

Create `crates/chibi-core/src/vfs/local.rs`:

```rust
//! Local filesystem backend for the VFS.
//!
//! Maps `VfsPath` values to OS paths under a root directory (typically
//! `~/.chibi/vfs/`). Uses `safe_io::atomic_write` for write operations.

use std::io;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tokio::fs;

use super::backend::VfsBackend;
use super::path::VfsPath;
use super::types::{VfsEntry, VfsEntryKind, VfsMetadata};

/// Filesystem-backed VFS storage.
///
/// All VFS paths are resolved relative to `root`. For example, with
/// `root = ~/.chibi/vfs`, the VFS path `/shared/foo.txt` maps to
/// `~/.chibi/vfs/shared/foo.txt`.
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    /// Create a new local backend rooted at the given directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Map a VFS path to an OS path.
    fn os_path(&self, path: &VfsPath) -> PathBuf {
        // VfsPath always starts with '/'; strip it for joining
        let relative = &path.as_str()[1..];
        if relative.is_empty() {
            self.root.clone()
        } else {
            self.root.join(relative)
        }
    }

    /// Ensure parent directory exists for a file path.
    async fn ensure_parent(&self, os_path: &PathBuf) -> io::Result<()> {
        if let Some(parent) = os_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(())
    }
}

impl VfsBackend for LocalBackend {
    async fn read(&self, path: &VfsPath) -> io::Result<Vec<u8>> {
        fs::read(self.os_path(path)).await
    }

    async fn write(&self, path: &VfsPath, data: &[u8]) -> io::Result<()> {
        let os_path = self.os_path(path);
        self.ensure_parent(&os_path).await?;
        // Use atomic write via spawn_blocking for safety
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            crate::safe_io::atomic_write(&os_path, &data)
        })
        .await
        .map_err(|e| io::Error::other(format!("join error: {}", e)))?
    }

    async fn append(&self, path: &VfsPath, data: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let os_path = self.os_path(path);
        self.ensure_parent(&os_path).await?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(os_path)
            .await?;
        file.write_all(data).await?;
        file.flush().await
    }

    async fn delete(&self, path: &VfsPath) -> io::Result<()> {
        let os_path = self.os_path(path);
        if os_path.is_dir() {
            fs::remove_dir_all(os_path).await
        } else {
            fs::remove_file(os_path).await
        }
    }

    async fn list(&self, path: &VfsPath) -> io::Result<Vec<VfsEntry>> {
        let os_path = self.os_path(path);
        if !os_path.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(os_path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let kind = if entry.file_type().await?.is_dir() {
                VfsEntryKind::Directory
            } else {
                VfsEntryKind::File
            };
            entries.push(VfsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }
        Ok(entries)
    }

    async fn exists(&self, path: &VfsPath) -> io::Result<bool> {
        Ok(self.os_path(path).exists())
    }

    async fn mkdir(&self, path: &VfsPath) -> io::Result<()> {
        fs::create_dir_all(self.os_path(path)).await
    }

    async fn copy(&self, src: &VfsPath, dst: &VfsPath) -> io::Result<()> {
        let src_os = self.os_path(src);
        let dst_os = self.os_path(dst);
        self.ensure_parent(&dst_os).await?;
        fs::copy(src_os, dst_os).await?;
        Ok(())
    }

    async fn rename(&self, src: &VfsPath, dst: &VfsPath) -> io::Result<()> {
        let src_os = self.os_path(src);
        let dst_os = self.os_path(dst);
        self.ensure_parent(&dst_os).await?;
        fs::rename(src_os, dst_os).await
    }

    async fn metadata(&self, path: &VfsPath) -> io::Result<VfsMetadata> {
        let os_path = self.os_path(path);
        let meta = fs::metadata(&os_path).await?;
        let kind = if meta.is_dir() {
            VfsEntryKind::Directory
        } else {
            VfsEntryKind::File
        };
        let created = meta.created().ok().map(DateTime::<Utc>::from);
        let modified = meta.modified().ok().map(DateTime::<Utc>::from);
        Ok(VfsMetadata {
            size: meta.len(),
            created,
            modified,
            kind,
        })
    }
}
```

Add to `crates/chibi-core/src/vfs/mod.rs`:

```rust
pub mod local;
pub use local::LocalBackend;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core vfs::local::tests`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): add LocalBackend (disk-based VFS storage)
```

---

### Task 4: Vfs router struct

**Files:**
- Create: `crates/chibi-core/src/vfs/vfs.rs`
- Modify: `crates/chibi-core/src/vfs/mod.rs` (add submodule, re-export)

**Step 1: Write the failing tests**

In `crates/chibi-core/src/vfs/vfs.rs`, tests that exercise the permission layer through `Vfs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::LocalBackend;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let vfs = Vfs::new(Box::new(backend));
        (dir, vfs)
    }

    #[tokio::test]
    async fn test_write_to_shared_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/shared/test.txt").unwrap();
        vfs.write("ctx", &path, b"hi").await.unwrap();
        let data = vfs.read("ctx", &path).await.unwrap();
        assert_eq!(data, b"hi");
    }

    #[tokio::test]
    async fn test_write_to_own_home_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/planner/file.md").unwrap();
        vfs.write("planner", &path, b"ok").await.unwrap();
        assert_eq!(vfs.read("planner", &path).await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn test_write_to_other_home_denied() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.md").unwrap();
        let err = vfs.write("planner", &path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_write_to_sys_denied() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/data").unwrap();
        let err = vfs.write("ctx", &path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_system_can_write_sys() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/data").unwrap();
        vfs.write(SYSTEM_CALLER, &path, b"ok").await.unwrap();
        assert_eq!(vfs.read("anyctx", &path).await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn test_read_other_home_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.md").unwrap();
        vfs.write("coder", &path, b"public").await.unwrap();
        let data = vfs.read("planner", &path).await.unwrap();
        assert_eq!(data, b"public");
    }

    #[tokio::test]
    async fn test_copy_checks_dst_permissions() {
        let (_dir, vfs) = setup();
        let src = VfsPath::new("/shared/src.txt").unwrap();
        let dst = VfsPath::new("/home/coder/dst.txt").unwrap();
        vfs.write("ctx", &src, b"data").await.unwrap();
        let err = vfs.copy("planner", &src, &dst).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_rename_checks_both_permissions() {
        let (_dir, vfs) = setup();
        let src = VfsPath::new("/home/planner/file.txt").unwrap();
        let dst = VfsPath::new("/home/coder/file.txt").unwrap();
        vfs.write("planner", &src, b"data").await.unwrap();
        // planner can write src but not dst
        let err = vfs.rename("planner", &src, &dst).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_delete_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.txt").unwrap();
        vfs.write("coder", &path, b"data").await.unwrap();
        let err = vfs.delete("planner", &path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_mkdir_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/forbidden").unwrap();
        let err = vfs.mkdir("ctx", &path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_append_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/log.txt").unwrap();
        let err = vfs.append("planner", &path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_list_always_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder").unwrap();
        // list is a read operation, should succeed for any caller
        let entries = vfs.list("planner", &path).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_metadata_always_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/shared/m.txt").unwrap();
        vfs.write("ctx", &path, b"data").await.unwrap();
        // metadata is a read operation
        let meta = vfs.metadata("othercxt", &path).await.unwrap();
        assert_eq!(meta.size, 4);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core vfs::vfs::tests`
Expected: compile errors

**Step 3: Write the implementation**

Create `crates/chibi-core/src/vfs/vfs.rs`:

```rust
//! Vfs: the permission-enforcing router.
//!
//! This is the single entry point for all VFS operations. It validates
//! permissions based on caller identity and path zone, then delegates
//! to the underlying `VfsBackend`.
//!
//! # Future evolution
//!
//! Currently wraps a single backend. Designed to evolve toward:
//! - **Multi-backend mounting**: A `Vec<(VfsPath, Box<dyn VfsBackend>)>` with
//!   longest-prefix match to select the backend and strip the mount prefix.
//! - **Middleware layers**: Composable tower-style layers wrapping the backend
//!   (approach C). The public API on `Vfs` stays unchanged.

use std::io;

use super::backend::VfsBackend;
use super::path::VfsPath;
use super::permissions::{self, SYSTEM_CALLER};
use super::types::{VfsEntry, VfsMetadata};

/// Core VFS router and permission enforcer.
///
/// All public methods take a `caller` (context name or `SYSTEM_CALLER`) and
/// enforce zone-based permissions before delegating to the backend.
pub struct Vfs {
    backend: Box<dyn VfsBackend>,
}

impl Vfs {
    /// Create a new VFS wrapping the given backend.
    pub fn new(backend: Box<dyn VfsBackend>) -> Self {
        Self { backend }
    }

    // -- read operations (always allowed) --

    pub async fn read(&self, caller: &str, path: &VfsPath) -> io::Result<Vec<u8>> {
        permissions::check_read(caller, path)?;
        self.backend.read(path).await
    }

    pub async fn list(&self, caller: &str, path: &VfsPath) -> io::Result<Vec<VfsEntry>> {
        permissions::check_read(caller, path)?;
        self.backend.list(path).await
    }

    pub async fn exists(&self, caller: &str, path: &VfsPath) -> io::Result<bool> {
        permissions::check_read(caller, path)?;
        self.backend.exists(path).await
    }

    pub async fn metadata(&self, caller: &str, path: &VfsPath) -> io::Result<VfsMetadata> {
        permissions::check_read(caller, path)?;
        self.backend.metadata(path).await
    }

    // -- write operations (permission-checked) --

    pub async fn write(&self, caller: &str, path: &VfsPath, data: &[u8]) -> io::Result<()> {
        permissions::check_write(caller, path)?;
        self.backend.write(path, data).await
    }

    pub async fn append(&self, caller: &str, path: &VfsPath, data: &[u8]) -> io::Result<()> {
        permissions::check_write(caller, path)?;
        self.backend.append(path, data).await
    }

    pub async fn delete(&self, caller: &str, path: &VfsPath) -> io::Result<()> {
        permissions::check_write(caller, path)?;
        self.backend.delete(path).await
    }

    pub async fn mkdir(&self, caller: &str, path: &VfsPath) -> io::Result<()> {
        permissions::check_write(caller, path)?;
        self.backend.mkdir(path).await
    }

    /// Copy a file. Caller must have read on src and write on dst.
    pub async fn copy(
        &self,
        caller: &str,
        src: &VfsPath,
        dst: &VfsPath,
    ) -> io::Result<()> {
        permissions::check_read(caller, src)?;
        permissions::check_write(caller, dst)?;
        self.backend.copy(src, dst).await
    }

    /// Rename (move) a file. Caller must have write on both src and dst.
    pub async fn rename(
        &self,
        caller: &str,
        src: &VfsPath,
        dst: &VfsPath,
    ) -> io::Result<()> {
        permissions::check_write(caller, src)?;
        permissions::check_write(caller, dst)?;
        self.backend.rename(src, dst).await
    }
}
```

Add to `crates/chibi-core/src/vfs/mod.rs`:

```rust
mod vfs;
pub use vfs::Vfs;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core vfs`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): add Vfs router with zone-based permission enforcement
```

---

### Task 5: Reject reserved context names

**Files:**
- Modify: `crates/chibi-core/src/context.rs` (update `is_valid_context_name`)

**Step 1: Write the failing test**

Add to existing tests in `crates/chibi-core/src/context.rs`:

```rust
#[test]
fn test_context_name_rejects_system() {
    assert!(!is_valid_context_name("SYSTEM"));
    assert!(!is_valid_context_name("system"));
    assert!(!is_valid_context_name("System"));
    assert!(!is_valid_context_name("SyStEm"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core test_context_name_rejects_system`
Expected: FAIL (currently these names would be accepted)

**Step 3: Write the implementation**

In `is_valid_context_name`, add the reserved name check:

```rust
pub fn is_valid_context_name(name: &str) -> bool {
    if name == "-" {
        return false;
    }
    if crate::vfs::is_reserved_caller_name(name) {
        return false;
    }
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core test_context_name`
Expected: all pass (new test + existing tests)

**Step 5: Commit**

```
fix(context): reject reserved VFS caller names (SYSTEM)
```

---

### Task 6: VFS config and initialization

**Files:**
- Modify: `crates/chibi-core/src/config.rs` (add `VfsConfig` struct and field to `Config`)
- Modify: `crates/chibi-core/src/state/mod.rs` (add `vfs` field to `AppState`, init in `load`)

**Step 1: Write the failing tests**

Add to config tests:

```rust
#[test]
fn test_vfs_config_defaults() {
    let config: Config = toml::from_str("").unwrap();
    assert_eq!(config.vfs.backend, "local");
}

#[test]
fn test_vfs_config_custom_backend() {
    let config: Config = toml::from_str("[vfs]\nbackend = \"fossil\"").unwrap();
    assert_eq!(config.vfs.backend, "fossil");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core test_vfs_config`
Expected: compile errors

**Step 3: Write the implementation**

Add `VfsConfig` to `crates/chibi-core/src/config.rs`:

```rust
/// VFS backend configuration.
///
/// Backend selection is config-level. Plugins can register backends,
/// but which one is active is determined here.
#[derive(Debug, Serialize, Deserialize)]
pub struct VfsConfig {
    /// Backend name. Currently only "local" is built in.
    #[serde(default = "default_vfs_backend")]
    pub backend: String,
}

fn default_vfs_backend() -> String {
    "local".to_string()
}

impl Default for VfsConfig {
    fn default() -> Self {
        Self {
            backend: default_vfs_backend(),
        }
    }
}
```

Add to `Config`:

```rust
    /// Virtual file system configuration
    #[serde(default)]
    pub vfs: VfsConfig,
```

Add `vfs` field to `AppState` in `crates/chibi-core/src/state/mod.rs`:

```rust
use crate::vfs::Vfs;
// ...
pub struct AppState {
    // ... existing fields ...
    /// Shared virtual file system.
    pub vfs: Vfs,
}
```

In the `AppState::load` (or `from_dir`) function, initialize the VFS:

```rust
let vfs_root = chibi_dir.join("vfs");
let vfs_backend = crate::vfs::LocalBackend::new(vfs_root);
let vfs = crate::vfs::Vfs::new(Box::new(vfs_backend));
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core`
Expected: all pass (config tests + no regressions)

**Step 5: Commit**

```
feat(vfs): add VfsConfig and wire Vfs into AppState
```

---

### Task 7: Dedicated VFS tools

**Files:**
- Create: `crates/chibi-core/src/tools/vfs_tools.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs` (add module, register tools)

This task creates the VFS-specific tools: `vfs_list`, `vfs_info`, `vfs_copy`, `vfs_move`, `vfs_mkdir`, `vfs_delete`. These are thin async wrappers around `Vfs` methods.

**Step 1: Write the failing tests**

Create `crates/chibi-core/src/tools/vfs_tools.rs` with tests:

```rust
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
        assert!(result.contains("empty") || result.contains("no entries") || result.is_empty()
            || result.contains("No entries"));
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
            assert!(!def.description.is_empty(), "{} missing description", def.name);
        }
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core tools::vfs_tools::tests`
Expected: compile errors

**Step 3: Write the implementation**

Implement the tool functions, tool definitions (following the pattern in `file_tools.rs` for `ToolDef` structs), and the `is_vfs_tool` / `execute_vfs_tool` dispatch function. Each tool:
1. Parses args (path, src/dst from JSON)
2. Calls `VfsPath::from_uri()` to parse the `vfs://` path
3. Delegates to the appropriate `Vfs` method
4. Returns a human-readable result string

Register in `crates/chibi-core/src/tools/mod.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core tools::vfs_tools::tests`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): add dedicated VFS tools (list, info, copy, move, mkdir, delete)
```

---

### Task 8: Wire `vfs://` prefix into existing file tools

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs` (`resolve_file_path`, `execute_write_file`)
- Modify: `crates/chibi-core/src/tools/coding_tools.rs` (`resolve_path`, `execute_file_edit`)

This is the key integration task. When any existing tool receives a `vfs://` path, it routes through the VFS instead of the OS filesystem.

**Step 1: Write the failing tests**

Add to file_tools tests:

```rust
#[tokio::test]
async fn test_resolve_file_path_vfs_prefix() {
    // When path starts with vfs://, it should be recognized as VFS
    assert!(VfsPath::is_vfs_uri("vfs:///shared/test.txt"));
}
```

Add to coding_tools tests:

```rust
#[test]
fn test_resolve_path_detects_vfs_uri() {
    assert!(VfsPath::is_vfs_uri("vfs:///shared/code.rs"));
    assert!(!VfsPath::is_vfs_uri("/home/user/code.rs"));
}
```

More substantive integration tests should exercise the full tool path with a VFS-backed write and read. These depend on the tool signatures — the implementing agent should add tests that:
- Call `execute_write_file` with a `vfs:///shared/test.txt` path and verify it writes through VFS
- Call `execute_file_head` with a `vfs://` path and verify it reads through VFS
- Call `execute_file_edit` with a `vfs://` path and verify it reads+edits through VFS
- Verify permission denial when writing to `/sys/` via VFS

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core`
Expected: new tests fail

**Step 3: Write the implementation**

The key changes:

**`file_tools::resolve_file_path`**: Add a `vfs://` check at the top of the `(None, Some(p))` branch. If detected, parse `VfsPath::from_uri`, then read from VFS (for read tools) or signal VFS mode for write tools. This may require `resolve_file_path` to return an enum (`OsPath(PathBuf) | VfsPath(VfsPath)`) so callers know which path type they're dealing with.

**`coding_tools::resolve_path`**: Same pattern — detect `vfs://`, return an enum.

**`execute_write_file`**: Handle the VFS case by calling `vfs.write()` instead of `safe_io::atomic_write_text`.

**`execute_file_edit`**: Handle the VFS case by reading via `vfs.read()`, applying the edit in memory, writing back via `vfs.write()`.

**`execute_file_head_or_tail`**: Handle the VFS case by reading via `vfs.read()` and extracting lines.

**`execute_grep_files`**: This one is complex since it walks directories. For VFS, it would need to use `vfs.list()` recursively and `vfs.read()` for each file. Consider whether this is needed in the initial implementation or if grep only works on OS paths for now. Document the decision.

**Important**: The VFS operations are async but the file/coding tool functions are sync (or sync-with-tokio). Use `tokio::runtime::Handle::current().block_on()` at the boundary. The `execute_coding_tool` function is already async, making that integration straightforward.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): wire vfs:// prefix into existing file and coding tools
```

---

### Task 9: Wire VFS tools into tool execution pipeline

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (add VFS tool dispatch)
- Modify: `crates/chibi-core/src/chibi.rs` (if tool dispatch lives here too)

VFS tools bypass the `PreFileWrite`/`PreFileRead` permission hooks — that's the whole point. The VFS's own permission model is sufficient.

**Step 1: Study the tool dispatch in `send.rs`**

The implementing agent should read `send.rs` lines 830-890 (the file tool dispatch section) and the coding tool dispatch section to understand the pattern. VFS tools should be dispatched similarly but without the permission hook checks.

**Step 2: Write tests**

Integration tests that verify:
- VFS tools are dispatched correctly when called by name
- VFS tool calls do NOT trigger `PreFileRead`/`PreFileWrite` hooks
- Permission errors from VFS are surfaced correctly

**Step 3: Add VFS tool dispatch**

In the tool execution match chain in `send.rs`, add a branch for VFS tools:

```rust
} else if tools::is_vfs_tool(&tool_call.name) {
    match tools::execute_vfs_tool(&app.vfs, context_name, &tool_call.name, &args).await {
        Some(Ok(r)) => r,
        Some(Err(e)) => format!("Error: {}", e),
        None => format!("Error: Unknown VFS tool '{}'", tool_call.name),
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core`
Expected: all pass

**Step 5: Commit**

```
feat(vfs): wire VFS tools into tool execution pipeline
```

---

### Task 10: Documentation

**Files:**
- Create: `docs/vfs.md`
- Modify: `docs/architecture.md` (add VFS section)
- Modify: `AGENTS.md` (add VFS doc link)

**Step 1: Write `docs/vfs.md`**

Document:
- Concept and motivation
- Namespace layout (`/shared/`, `/home/<context>/`, `/sys/`)
- Permission model
- `vfs://` URI scheme usage in tools
- Dedicated VFS tools
- Configuration (`[vfs]` section)
- Backend extensibility (how to implement `VfsBackend`)

**Step 2: Update `docs/architecture.md`**

Add VFS to the storage layout diagram and file listing.

**Step 3: Update `AGENTS.md`**

Add `docs/vfs.md` to the documentation list.

**Step 4: Commit**

```
docs: add VFS documentation
```

---

### Task 11: Full integration test

**Files:**
- Create or extend: `crates/chibi-core/src/vfs/mod.rs` (integration test section)

**Step 1: Write a comprehensive integration test**

A test that exercises the full workflow:
1. Create a `Vfs` with `LocalBackend`
2. Write files as SYSTEM to `/sys/`
3. Write files as context "alice" to `/home/alice/` and `/shared/`
4. Verify "bob" can read alice's home but can't write
5. Verify "bob" can write to `/shared/`
6. Copy, move, delete operations with permission checks
7. List and metadata operations

**Step 2: Run the test**

Run: `cargo test -p chibi-core vfs`
Expected: all pass

**Step 3: Commit**

```
test(vfs): add comprehensive integration test
```

---

## Summary

| Task | Description | Depends on |
|------|------------|------------|
| 1 | VfsPath newtype + validation | - |
| 2 | Backend trait, types, permissions | 1 |
| 3 | LocalBackend | 2 |
| 4 | Vfs router struct | 2, 3 |
| 5 | Reject reserved context names | 2 |
| 6 | Config + AppState wiring | 3, 4 |
| 7 | Dedicated VFS tools | 4 |
| 8 | `vfs://` prefix in existing tools | 4, 6 |
| 9 | Tool execution pipeline wiring | 7, 8 |
| 10 | Documentation | 9 |
| 11 | Full integration test | 9 |

Tasks 1-5 can be parallelized somewhat (1 must be first, then 2, then 3-5 in parallel). Tasks 7 and 8 can be parallelized. Tasks 10 and 11 can be parallelized.
