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
use super::permissions;
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
    pub async fn copy(&self, caller: &str, src: &VfsPath, dst: &VfsPath) -> io::Result<()> {
        permissions::check_read(caller, src)?;
        permissions::check_write(caller, dst)?;
        self.backend.copy(src, dst).await
    }

    /// Rename (move) a file. Caller must have write on both src and dst.
    pub async fn rename(&self, caller: &str, src: &VfsPath, dst: &VfsPath) -> io::Result<()> {
        permissions::check_write(caller, src)?;
        permissions::check_write(caller, dst)?;
        self.backend.rename(src, dst).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::LocalBackend;
    use crate::vfs::permissions::SYSTEM_CALLER;
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
        let entries = vfs.list("planner", &path).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_metadata_always_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/shared/m.txt").unwrap();
        vfs.write("ctx", &path, b"data").await.unwrap();
        let meta = vfs.metadata("othercxt", &path).await.unwrap();
        assert_eq!(meta.size, 4);
    }
}
