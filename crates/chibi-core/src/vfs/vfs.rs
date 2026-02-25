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

use std::cell::RefCell;
use std::io;

use super::backend::VfsBackend;
use super::flock::{FlockEntry, FlockRegistry, resolve_flock_vfs_root, validate_flock_name};
use super::path::VfsPath;
use super::permissions;
use super::types::{VfsEntry, VfsMetadata};
use crate::vfs::caller::VfsCaller;

/// Path to the flock registry file within the VFS.
const REGISTRY_PATH: &str = "/flocks/registry.json";

/// Core VFS router and permission enforcer.
///
/// All public methods take a `caller` (`VfsCaller`) and enforce zone-based
/// permissions before delegating to the backend.
pub struct Vfs {
    backend: Box<dyn VfsBackend>,
    /// Site identifier for this installation (e.g. `"myhost-a1b2c3d4"`).
    /// Used for flock permission checks and registry membership.
    site_id: String,
    /// Cached flock registry. Loaded lazily and invalidated on registry writes.
    registry_cache: RefCell<Option<FlockRegistry>>,
}

impl Vfs {
    /// Create a new VFS wrapping the given backend.
    ///
    /// `site_id` is the stable site identifier used for flock permission checks.
    /// Pass `"test-site-0000"` in tests.
    pub fn new(backend: Box<dyn VfsBackend>, site_id: impl Into<String>) -> Self {
        Self {
            backend,
            site_id: site_id.into(),
            registry_cache: RefCell::new(None),
        }
    }

    /// Return the site identifier for this installation.
    pub fn site_id(&self) -> &str {
        &self.site_id
    }

    /// Load the flock registry from the backend, using the in-memory cache when available.
    ///
    /// Returns `Ok(FlockRegistry::default())` if the registry file doesn't exist yet.
    pub async fn load_registry(&self) -> io::Result<FlockRegistry> {
        if let Some(cached) = self.registry_cache.borrow().as_ref() {
            return Ok(cached.clone());
        }
        let path = VfsPath::new(REGISTRY_PATH)?;
        let registry = match self.backend.read(&path).await {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => FlockRegistry::default(),
            Err(e) => return Err(e),
        };
        *self.registry_cache.borrow_mut() = Some(registry.clone());
        Ok(registry)
    }

    /// Invalidate the registry cache (called after writing to `/flocks/registry.json`).
    pub fn invalidate_registry_cache(&self) {
        *self.registry_cache.borrow_mut() = None;
    }

    /// Return a snapshot of the flock registry.
    pub async fn registry(&self) -> io::Result<FlockRegistry> {
        self.load_registry().await
    }

    /// Load the registry for use in a permission check, returning `None` on error.
    async fn flock_ctx_for_check(&self) -> Option<FlockRegistry> {
        self.load_registry().await.ok()
    }

    // -- read operations (always allowed) --

    pub async fn read(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<Vec<u8>> {
        permissions::check_read(caller, path)?;
        self.backend.read(path).await
    }

    pub async fn list(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<Vec<VfsEntry>> {
        permissions::check_read(caller, path)?;
        self.backend.list(path).await
    }

    pub async fn exists(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<bool> {
        permissions::check_read(caller, path)?;
        self.backend.exists(path).await
    }

    pub async fn metadata(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<VfsMetadata> {
        permissions::check_read(caller, path)?;
        self.backend.metadata(path).await
    }

    // -- write operations (permission-checked) --

    pub async fn write(
        &self,
        caller: VfsCaller<'_>,
        path: &VfsPath,
        data: &[u8],
    ) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        self.backend.write(path, data).await?;
        if path.as_str() == REGISTRY_PATH {
            self.invalidate_registry_cache();
        }
        Ok(())
    }

    pub async fn append(
        &self,
        caller: VfsCaller<'_>,
        path: &VfsPath,
        data: &[u8],
    ) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        self.backend.append(path, data).await
    }

    pub async fn delete(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        self.backend.delete(path).await
    }

    pub async fn mkdir(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        self.backend.mkdir(path).await
    }

    /// Copy a file. Caller must have read on src and write on dst.
    pub async fn copy(
        &self,
        caller: VfsCaller<'_>,
        src: &VfsPath,
        dst: &VfsPath,
    ) -> io::Result<()> {
        permissions::check_read(caller, src)?;
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, dst, flock_ctx)?;
        self.backend.copy(src, dst).await
    }

    /// Rename (move) a file. Caller must have write on both src and dst.
    pub async fn rename(
        &self,
        caller: VfsCaller<'_>,
        src: &VfsPath,
        dst: &VfsPath,
    ) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        // Clone site_id to avoid double-borrow with two check_write calls.
        let site_id = self.site_id.clone();
        let flock_ctx_src = registry.as_ref().map(|r| (r, site_id.as_str()));
        let flock_ctx_dst = registry.as_ref().map(|r| (r, site_id.as_str()));
        permissions::check_write(caller, src, flock_ctx_src)?;
        permissions::check_write(caller, dst, flock_ctx_dst)?;
        self.backend.rename(src, dst).await
    }

    // -- flock management --

    /// Join a flock (auto-creates if it doesn't exist).
    ///
    /// Errors if `flock` is not a valid flock name. Uses `System` authority
    /// to update the registry.
    pub async fn flock_join(&self, flock: &str, context: &str) -> io::Result<()> {
        validate_flock_name(flock)?;
        let mut reg = self.load_registry().await.unwrap_or_default();
        reg.add_member(flock, context, &format!("site:{}", self.site_id));
        self.save_registry(&reg).await?;
        // Ensure the flock directory exists.
        let dir = resolve_flock_vfs_root(flock, &self.site_id)?;
        let _ = self.backend.mkdir(&dir).await; // ignore already-exists
        Ok(())
    }

    /// Leave a flock. Errors if `flock` is the site flock (`site:*`).
    pub async fn flock_leave(&self, flock: &str, context: &str) -> io::Result<()> {
        if flock.starts_with("site:") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot leave the site flock",
            ));
        }
        let mut reg = self.load_registry().await.unwrap_or_default();
        reg.remove_member(flock, context, &format!("site:{}", self.site_id));
        self.save_registry(&reg).await
    }

    /// Delete a flock entirely (removes all members and the registry entry).
    pub async fn flock_delete(&self, flock: &str) -> io::Result<()> {
        validate_flock_name(flock)?;
        let mut reg = self.load_registry().await.unwrap_or_default();
        reg.delete_flock(flock);
        self.save_registry(&reg).await
    }

    /// List all explicit flocks a context belongs to (site flock not included).
    pub async fn flock_list_for(&self, context: &str) -> io::Result<Vec<String>> {
        let reg = self.load_registry().await.unwrap_or_default();
        Ok(reg.flocks_for(context, &format!("site:{}", self.site_id)))
    }

    /// List all flocks in the registry.
    pub async fn flock_list_all(&self) -> io::Result<Vec<FlockEntry>> {
        let reg = self.load_registry().await.unwrap_or_default();
        Ok(reg.flocks)
    }

    /// Write the flock registry to the backend using `System` authority.
    async fn save_registry(&self, reg: &FlockRegistry) -> io::Result<()> {
        let path = VfsPath::new(REGISTRY_PATH)?;
        let data = serde_json::to_string_pretty(reg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.backend.write(&path, data.as_bytes()).await?;
        self.invalidate_registry_cache();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{LocalBackend, VfsCaller};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let vfs = Vfs::new(Box::new(backend), "test-site-0000");
        (dir, vfs)
    }

    #[tokio::test]
    async fn test_write_to_shared_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/shared/test.txt").unwrap();
        vfs.write(VfsCaller::Context("ctx"), &path, b"hi")
            .await
            .unwrap();
        let data = vfs.read(VfsCaller::Context("ctx"), &path).await.unwrap();
        assert_eq!(data, b"hi");
    }

    #[tokio::test]
    async fn test_write_to_own_home_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/planner/file.md").unwrap();
        vfs.write(VfsCaller::Context("planner"), &path, b"ok")
            .await
            .unwrap();
        assert_eq!(
            vfs.read(VfsCaller::Context("planner"), &path)
                .await
                .unwrap(),
            b"ok"
        );
    }

    #[tokio::test]
    async fn test_write_to_other_home_denied() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.md").unwrap();
        let err = vfs
            .write(VfsCaller::Context("planner"), &path, b"nope")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_write_to_sys_denied() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/data").unwrap();
        let err = vfs
            .write(VfsCaller::Context("ctx"), &path, b"nope")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_system_can_write_sys() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/data").unwrap();
        vfs.write(VfsCaller::System, &path, b"ok").await.unwrap();
        assert_eq!(
            vfs.read(VfsCaller::Context("anyctx"), &path).await.unwrap(),
            b"ok"
        );
    }

    #[tokio::test]
    async fn test_read_other_home_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.md").unwrap();
        vfs.write(VfsCaller::Context("coder"), &path, b"public")
            .await
            .unwrap();
        let data = vfs
            .read(VfsCaller::Context("planner"), &path)
            .await
            .unwrap();
        assert_eq!(data, b"public");
    }

    #[tokio::test]
    async fn test_copy_checks_dst_permissions() {
        let (_dir, vfs) = setup();
        let src = VfsPath::new("/shared/src.txt").unwrap();
        let dst = VfsPath::new("/home/coder/dst.txt").unwrap();
        vfs.write(VfsCaller::Context("ctx"), &src, b"data")
            .await
            .unwrap();
        let err = vfs
            .copy(VfsCaller::Context("planner"), &src, &dst)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_rename_checks_both_permissions() {
        let (_dir, vfs) = setup();
        let src = VfsPath::new("/home/planner/file.txt").unwrap();
        let dst = VfsPath::new("/home/coder/file.txt").unwrap();
        vfs.write(VfsCaller::Context("planner"), &src, b"data")
            .await
            .unwrap();
        let err = vfs
            .rename(VfsCaller::Context("planner"), &src, &dst)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_delete_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/file.txt").unwrap();
        vfs.write(VfsCaller::Context("coder"), &path, b"data")
            .await
            .unwrap();
        let err = vfs
            .delete(VfsCaller::Context("planner"), &path)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_mkdir_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/sys/forbidden").unwrap();
        let err = vfs
            .mkdir(VfsCaller::Context("ctx"), &path)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_append_checks_write_permission() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder/log.txt").unwrap();
        let err = vfs
            .append(VfsCaller::Context("planner"), &path, b"nope")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_list_always_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/home/coder").unwrap();
        let entries = vfs
            .list(VfsCaller::Context("planner"), &path)
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_metadata_always_allowed() {
        let (_dir, vfs) = setup();
        let path = VfsPath::new("/shared/m.txt").unwrap();
        vfs.write(VfsCaller::Context("ctx"), &path, b"data")
            .await
            .unwrap();
        let meta = vfs
            .metadata(VfsCaller::Context("othercxt"), &path)
            .await
            .unwrap();
        assert_eq!(meta.size, 4);
    }

    // -- flock management tests --

    fn setup_with_site(site_id: &str) -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let vfs = Vfs::new(Box::new(backend), site_id);
        (dir, vfs)
    }

    #[tokio::test]
    async fn test_flock_join_creates_flock() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        let reg = vfs.registry().await.unwrap();
        assert!(reg.is_member("frontend", "ui-dev", "site:test-0000"));
    }

    #[tokio::test]
    async fn test_flock_join_idempotent() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        let reg = vfs.registry().await.unwrap();
        assert_eq!(reg.flocks[0].members.len(), 1);
    }

    #[tokio::test]
    async fn test_flock_leave() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        vfs.flock_leave("frontend", "ui-dev").await.unwrap();
        let reg = vfs.registry().await.unwrap();
        assert!(!reg.is_member("frontend", "ui-dev", "site:test-0000"));
    }

    #[tokio::test]
    async fn test_flock_leave_site_flock_fails() {
        let (_dir, vfs) = setup_with_site("test-0000");
        let result = vfs.flock_leave("site:test-0000", "ctx").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_flock_delete() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        vfs.flock_join("frontend", "coder").await.unwrap();
        vfs.flock_delete("frontend").await.unwrap();
        let reg = vfs.registry().await.unwrap();
        assert!(reg.flocks.is_empty());
    }

    #[tokio::test]
    async fn test_flock_member_can_write_goals() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        let path = VfsPath::new("/flocks/frontend/goals.md").unwrap();
        vfs.write(VfsCaller::Context("ui-dev"), &path, b"- ship it")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_non_member_cannot_write_flock_goals() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        let path = VfsPath::new("/flocks/frontend/goals.md").unwrap();
        let result = vfs
            .write(VfsCaller::Context("outsider"), &path, b"nope")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_flock_list_for() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        vfs.flock_join("backend", "ui-dev").await.unwrap();
        let flocks = vfs.flock_list_for("ui-dev").await.unwrap();
        assert_eq!(flocks, vec!["backend", "frontend"]);
    }

    #[tokio::test]
    async fn test_flock_list_all() {
        let (_dir, vfs) = setup_with_site("test-0000");
        vfs.flock_join("frontend", "ui-dev").await.unwrap();
        vfs.flock_join("backend", "coder").await.unwrap();
        let all = vfs.flock_list_all().await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
