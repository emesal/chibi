//! Local filesystem backend for the VFS.
//!
//! Maps `VfsPath` values to OS paths under a root directory (typically
//! `~/.chibi/vfs/`). Uses `safe_io::atomic_write` for write operations.

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use tokio::fs;

use super::backend::VfsBackend;
use super::path::VfsPath;
use super::types::{VfsEntry, VfsEntryKind, VfsMetadata};

/// Boxed, Send future — matches the backend trait's return type.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
    async fn ensure_parent(&self, os_path: &std::path::Path) -> io::Result<()> {
        if let Some(parent) = os_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(())
    }
}

impl VfsBackend for LocalBackend {
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move { fs::read(self.os_path(path)).await })
    }

    fn write<'a>(&'a self, path: &'a VfsPath, data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let os_path = self.os_path(path);
            self.ensure_parent(&os_path).await?;
            // Clone data for the blocking closure (needs 'static)
            let data = data.to_vec();
            tokio::task::spawn_blocking(move || crate::safe_io::atomic_write(&os_path, &data))
                .await
                .map_err(|e| io::Error::other(format!("join error: {}", e)))?
        })
    }

    fn append<'a>(&'a self, path: &'a VfsPath, data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
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
        })
    }

    fn delete<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let os_path = self.os_path(path);
            let meta = fs::metadata(&os_path).await?; // propagates NotFound cleanly
            if meta.is_dir() {
                fs::remove_dir_all(os_path).await
            } else {
                fs::remove_file(os_path).await
            }
        })
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        Box::pin(async move {
            let os_path = self.os_path(path);
            if fs::metadata(&os_path).await.is_err() {
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
        })
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        Box::pin(async move { Ok(fs::metadata(self.os_path(path)).await.is_ok()) })
    }

    fn mkdir<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move { fs::create_dir_all(self.os_path(path)).await })
    }

    fn copy<'a>(&'a self, src: &'a VfsPath, dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let src_os = self.os_path(src);
            let dst_os = self.os_path(dst);
            self.ensure_parent(&dst_os).await?;
            fs::copy(src_os, dst_os).await?;
            Ok(())
        })
    }

    fn rename<'a>(&'a self, src: &'a VfsPath, dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let src_os = self.os_path(src);
            let dst_os = self.os_path(dst);
            self.ensure_parent(&dst_os).await?;
            fs::rename(src_os, dst_os).await
        })
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        Box::pin(async move {
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
        })
    }
}

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
    async fn test_delete_directory() {
        let (_dir, backend) = setup();
        let dir_path = VfsPath::new("/shared/subdir").unwrap();
        backend.mkdir(&dir_path).await.unwrap();
        backend
            .write(&VfsPath::new("/shared/subdir/file.txt").unwrap(), b"x")
            .await
            .unwrap();
        backend.delete(&dir_path).await.unwrap();
        assert!(!backend.exists(&dir_path).await.unwrap());
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
        backend
            .mkdir(&VfsPath::new("/shared/sub").unwrap())
            .await
            .unwrap();

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
        assert!(backend.exists(&src).await.unwrap());
    }

    #[tokio::test]
    async fn test_rename() {
        let (_dir, backend) = setup();
        let src = VfsPath::new("/shared/old.txt").unwrap();
        let dst = VfsPath::new("/shared/new.txt").unwrap();
        backend.write(&src, b"moved").await.unwrap();
        backend.rename(&src, &dst).await.unwrap();
        assert_eq!(backend.read(&dst).await.unwrap(), b"moved");
        assert!(!backend.exists(&src).await.unwrap());
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
