//! Vfs: the permission-enforcing router.
//!
//! This is the single entry point for all VFS operations. It validates
//! permissions based on caller identity and path zone, then delegates
//! to the appropriate backend via longest-prefix mount matching.
//!
//! # Multi-backend mounting
//!
//! Use `Vfs::builder(site_id)` to mount different backends at different
//! path prefixes. The backend with the longest matching prefix handles the
//! operation; the mount prefix is stripped from the path before delegation.
//!
//! `Vfs::new(backend, site_id)` mounts a single backend at `/` (convenience
//! for the common single-backend case).
//!
//! # Middleware layers
//!
//! Future evolution: composable tower-style layers wrapping the backend
//! (approach C). The public API on `Vfs` stays unchanged.

use std::cell::RefCell;
use std::io;
use std::sync::Arc;

use super::backend::VfsBackend;
use super::flock::{
    FlockEntry, FlockRegistry, resolve_flock_vfs_root, site_flock_name, validate_flock_name,
};
use super::path::VfsPath;
use super::permissions;
use super::types::{VfsEntry, VfsMetadata};
use crate::vfs::caller::VfsCaller;

/// Path to the flock registry file within the VFS.
const REGISTRY_PATH: &str = "/flocks/registry.json";

/// The kind of change that triggered a scheme-tool change callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmChangeKind {
    /// A `.scm` file was written (created or overwritten).
    Write,
    /// A `.scm` file was deleted.
    Delete,
}

/// Callback type for `.scm` file changes under `/tools/`.
///
/// Fired after a successful write or delete on any path matching
/// `/tools/{shared,home/**,flocks/**}/*.scm`. On write, `content` contains
/// the bytes that were written; on delete, `content` is `None`.
pub type ScmChangeCallback = Arc<dyn Fn(&VfsPath, ScmChangeKind, Option<&[u8]>) + Send + Sync>;

/// Core VFS router and permission enforcer.
///
/// All public methods take a `caller` (`VfsCaller`) and enforce zone-based
/// permissions before delegating to the backend.
///
/// Backends are sorted by mount-prefix length (longest first) so that the
/// most-specific prefix wins. The prefix is stripped from the path before
/// delegation: `/tools/sys/shell_exec` on a `/tools/sys` mount becomes
/// `/shell_exec` in the backend.
pub struct Vfs {
    /// Backends sorted by prefix length descending (longest first).
    mounts: Vec<(String, Box<dyn VfsBackend>)>,
    /// Site identifier for this installation (e.g. `"myhost-a1b2c3d4"`).
    /// Used for flock permission checks and registry membership.
    site_id: String,
    /// Cached flock registry. Loaded lazily and invalidated on registry writes.
    // NOTE: RefCell is !Sync. Safe here because borrows never span .await points
    // in load_registry(). If adding new async methods that touch this cache,
    // ensure the RefCell borrow is dropped before any .await.
    registry_cache: RefCell<Option<FlockRegistry>>,
    /// Optional callback fired after a successful write or delete on any path
    /// matching `/tools/{shared,home/**,flocks/**}/*.scm`.
    ///
    /// Set via `Vfs::set_scm_change_callback`. The callback is synchronous
    /// and must not block; use `Handle::current().block_on(...)` for async work.
    pub on_scm_change: Option<ScmChangeCallback>,
}

/// Builder for `Vfs` with multiple backend mounts.
pub struct VfsBuilder {
    mounts: Vec<(String, Box<dyn VfsBackend>)>,
    site_id: String,
}

impl VfsBuilder {
    fn new(site_id: impl Into<String>) -> Self {
        Self {
            mounts: Vec::new(),
            site_id: site_id.into(),
        }
    }

    /// Mount a backend at the given path prefix (e.g. `"/"`, `"/tools/sys"`).
    pub fn mount(mut self, prefix: &str, backend: Box<dyn VfsBackend>) -> Self {
        self.mounts.push((prefix.to_string(), backend));
        self
    }

    /// Build the `Vfs`. Mounts are sorted by prefix length descending so
    /// the most-specific prefix wins at dispatch time.
    pub fn build(mut self) -> Vfs {
        self.mounts.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Vfs {
            mounts: self.mounts,
            site_id: self.site_id,
            registry_cache: RefCell::new(None),
            on_scm_change: None,
        }
    }
}

impl Vfs {
    /// Create a new VFS wrapping the given backend at the root `/` mount.
    ///
    /// `site_id` is the stable site identifier used for flock permission checks.
    /// Pass `"test-site-0000"` in tests.
    pub fn new(backend: Box<dyn VfsBackend>, site_id: impl Into<String>) -> Self {
        Vfs::builder(site_id).mount("/", backend).build()
    }

    /// Start a builder for a multi-backend VFS.
    ///
    /// Call `.mount(prefix, backend)` for each backend, then `.build()`.
    pub fn builder(site_id: impl Into<String>) -> VfsBuilder {
        VfsBuilder::new(site_id)
    }

    /// Set a callback to fire whenever a `.scm` file under `/tools/` is
    /// written or deleted.
    ///
    /// The callback receives the full VFS path, a `ScmChangeKind`, and
    /// (on write) the bytes that were written. On delete, the content is `None`.
    /// The callback is called synchronously after the backend operation succeeds.
    pub fn set_scm_change_callback(&mut self, cb: ScmChangeCallback) {
        self.on_scm_change = Some(cb);
    }

    /// Returns `true` if `path` is a `.scm` file in a writable tools zone.
    fn is_scm_tool_path(path: &VfsPath) -> bool {
        let s = path.as_str();
        s.ends_with(".scm")
            && (s.starts_with("/tools/shared/")
                || s.starts_with("/tools/home/")
                || s.starts_with("/tools/flocks/"))
    }

    /// Resolve the backend and stripped path for the given VFS path.
    ///
    /// Selects the backend whose mount prefix is the longest prefix of `path`.
    /// The prefix is stripped from the path before returning: a path
    /// `/tools/sys/foo` on mount `/tools/sys` returns `/foo`; the root `/`
    /// mount returns the path unchanged.
    ///
    /// Panics if no backend matches (always has a root mount in practice).
    fn resolve_backend(&self, path: &VfsPath) -> (&dyn VfsBackend, VfsPath) {
        let p = path.as_str();
        for (prefix, backend) in &self.mounts {
            if prefix == "/" || p == prefix || p.starts_with(&format!("{}/", prefix)) {
                let stripped = if prefix == "/" {
                    p.to_string()
                } else {
                    let rest = &p[prefix.len()..];
                    if rest.is_empty() {
                        "/".to_string()
                    } else {
                        rest.to_string()
                    }
                };
                // stripped is guaranteed valid: starts with '/' or is "/"
                let stripped_path =
                    VfsPath::new(&stripped).expect("stripped VFS path must be valid");
                return (backend.as_ref(), stripped_path);
            }
        }
        panic!(
            "no VFS backend matched path '{}' — ensure a root '/' mount exists",
            p
        );
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
        let (backend, stripped) = self.resolve_backend(&path);
        let registry = match backend.read(&stripped).await {
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
        match self.load_registry().await {
            Ok(reg) => Some(reg),
            Err(e) => {
                eprintln!("[vfs] warning: failed to load flock registry for permission check: {e}");
                None
            }
        }
    }

    // -- read operations (always allowed) --

    pub async fn read(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<Vec<u8>> {
        permissions::check_read(caller, path)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.read(&stripped).await
    }

    pub async fn list(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<Vec<VfsEntry>> {
        permissions::check_read(caller, path)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.list(&stripped).await
    }

    pub async fn exists(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<bool> {
        permissions::check_read(caller, path)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.exists(&stripped).await
    }

    pub async fn metadata(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<VfsMetadata> {
        permissions::check_read(caller, path)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.metadata(&stripped).await
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
        let (backend, stripped) = self.resolve_backend(path);
        backend.write(&stripped, data).await?;
        if path.as_str() == REGISTRY_PATH {
            self.invalidate_registry_cache();
        }
        if Self::is_scm_tool_path(path)
            && let Some(cb) = &self.on_scm_change
        {
            cb(path, ScmChangeKind::Write, Some(data));
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
        let (backend, stripped) = self.resolve_backend(path);
        backend.append(&stripped, data).await
    }

    pub async fn delete(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.delete(&stripped).await?;
        if Self::is_scm_tool_path(path)
            && let Some(cb) = &self.on_scm_change
        {
            cb(path, ScmChangeKind::Delete, None);
        }
        Ok(())
    }

    pub async fn mkdir(&self, caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, path, flock_ctx)?;
        let (backend, stripped) = self.resolve_backend(path);
        backend.mkdir(&stripped).await
    }

    /// Copy a file. Caller must have read on src and write on dst.
    ///
    /// Both src and dst must resolve to the same backend. Cross-backend copies
    /// are not supported (use read + write instead).
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
        let (src_backend, src_stripped) = self.resolve_backend(src);
        let (dst_backend, dst_stripped) = self.resolve_backend(dst);
        // Backends are trait objects; compare via raw pointer identity.
        if !std::ptr::eq(src_backend, dst_backend) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cross-backend copy not supported; use read + write",
            ));
        }
        src_backend.copy(&src_stripped, &dst_stripped).await
    }

    /// Rename (move) a file. Caller must have write on both src and dst.
    ///
    /// Both src and dst must resolve to the same backend. Cross-backend renames
    /// are not supported (use read + write + delete instead).
    pub async fn rename(
        &self,
        caller: VfsCaller<'_>,
        src: &VfsPath,
        dst: &VfsPath,
    ) -> io::Result<()> {
        let registry = self.flock_ctx_for_check().await;
        let flock_ctx_src = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        let flock_ctx_dst = registry.as_ref().map(|r| (r, self.site_id.as_str()));
        permissions::check_write(caller, src, flock_ctx_src)?;
        permissions::check_write(caller, dst, flock_ctx_dst)?;
        let (src_backend, src_stripped) = self.resolve_backend(src);
        let (dst_backend, dst_stripped) = self.resolve_backend(dst);
        if !std::ptr::eq(src_backend, dst_backend) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cross-backend rename not supported; use read + write + delete",
            ));
        }
        src_backend.rename(&src_stripped, &dst_stripped).await
    }

    // -- flock management --

    /// Join a flock (auto-creates if it doesn't exist).
    ///
    /// Errors if `flock` is not a valid flock name. Uses `System` authority
    /// to update the registry.
    pub async fn flock_join(&self, flock: &str, context: &str) -> io::Result<()> {
        validate_flock_name(flock)?;
        let mut reg = self.load_registry().await.unwrap_or_default();
        reg.add_member(flock, context, &site_flock_name(&self.site_id));
        self.save_registry(&reg).await?;
        // Ensure the flock directory exists.
        let dir = resolve_flock_vfs_root(flock, &self.site_id)?;
        let (backend, stripped) = self.resolve_backend(&dir);
        let _ = backend.mkdir(&stripped).await; // ignore already-exists
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
        reg.remove_member(flock, context, &site_flock_name(&self.site_id));
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
        Ok(reg.flocks_for(context, &site_flock_name(&self.site_id)))
    }

    /// List all flocks in the registry.
    pub async fn flock_list_all(&self) -> io::Result<Vec<FlockEntry>> {
        let reg = self.load_registry().await.unwrap_or_default();
        Ok(reg.flocks)
    }

    /// Write the flock registry to the backend via the permission-checked write path.
    async fn save_registry(&self, reg: &FlockRegistry) -> io::Result<()> {
        let path = VfsPath::new(REGISTRY_PATH)?;
        let data = serde_json::to_string_pretty(reg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.write(VfsCaller::System, &path, data.as_bytes()).await
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
    async fn test_multi_backend_routing() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        let backend1 = LocalBackend::new(dir1.path().to_path_buf());
        let backend2 = LocalBackend::new(dir2.path().to_path_buf());

        let vfs = Vfs::builder("test-site-0000")
            .mount("/", Box::new(backend1))
            .mount("/tools/shared", Box::new(backend2))
            .build();

        // write to /tools/shared/ goes to backend2 (dir2)
        let path1 = VfsPath::new("/tools/shared/test.txt").unwrap();
        vfs.write(VfsCaller::System, &path1, b"hello")
            .await
            .unwrap();
        assert_eq!(vfs.read(VfsCaller::System, &path1).await.unwrap(), b"hello");

        // verify the file landed in dir2, not dir1
        assert!(dir2.path().join("test.txt").exists());
        assert!(
            !dir1
                .path()
                .join("tools")
                .join("shared")
                .join("test.txt")
                .exists()
        );

        // write to /shared/ goes to backend1 (dir1, root mount)
        let path2 = VfsPath::new("/shared/other.txt").unwrap();
        vfs.write(VfsCaller::System, &path2, b"world")
            .await
            .unwrap();
        assert_eq!(vfs.read(VfsCaller::System, &path2).await.unwrap(), b"world");
        assert!(dir1.path().join("shared").join("other.txt").exists());
    }

    #[tokio::test]
    async fn test_multi_backend_list_mount_root() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        let backend1 = LocalBackend::new(dir1.path().to_path_buf());
        let backend2 = LocalBackend::new(dir2.path().to_path_buf());

        let vfs = Vfs::builder("test-site-0000")
            .mount("/", Box::new(backend1))
            .mount("/tools/shared", Box::new(backend2))
            .build();

        // write a file to the backend2 mount
        let path = VfsPath::new("/tools/shared/mytool.scm").unwrap();
        vfs.write(VfsCaller::System, &path, b"(define x 1)")
            .await
            .unwrap();

        // listing /tools/shared should go to backend2 and see the file
        let mount_root = VfsPath::new("/tools/shared").unwrap();
        let entries = vfs.list(VfsCaller::System, &mount_root).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "mytool.scm");
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
