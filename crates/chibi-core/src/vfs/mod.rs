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
pub mod local;
pub mod path;
pub mod permissions;
pub mod types;
mod vfs;

pub use backend::VfsBackend;
pub use local::LocalBackend;
pub use path::VfsPath;
pub use permissions::{SYSTEM_CALLER, check_read, check_write, is_reserved_caller_name};
pub use types::{VfsEntry, VfsEntryKind, VfsMetadata};
pub use vfs::Vfs;

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::io::ErrorKind;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        (dir, Vfs::new(Box::new(backend)))
    }

    #[tokio::test]
    async fn test_vfs_multi_context_integration() {
        let (_dir, vfs) = setup();

        // --- SYSTEM writes to /sys/ ---
        let sys_config = VfsPath::new("/sys/config.json").unwrap();
        vfs.write(SYSTEM_CALLER, &sys_config, b"{\"version\":1}")
            .await
            .unwrap();
        assert!(vfs.exists(SYSTEM_CALLER, &sys_config).await.unwrap());
        assert_eq!(
            vfs.read(SYSTEM_CALLER, &sys_config).await.unwrap(),
            b"{\"version\":1}"
        );

        // --- alice operations ---
        let shared_tasks = VfsPath::new("/shared/tasks.md").unwrap();
        let alice_notes = VfsPath::new("/home/alice/notes.md").unwrap();

        vfs.write("alice", &shared_tasks, b"# tasks\n- implement vfs")
            .await
            .unwrap();
        vfs.write("alice", &alice_notes, b"alice's private notes")
            .await
            .unwrap();

        // alice cannot write to /sys/
        let err = vfs
            .write("alice", &VfsPath::new("/sys/data").unwrap(), b"nope")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // alice cannot write to bob's home
        let err = vfs
            .write(
                "alice",
                &VfsPath::new("/home/bob/secret.md").unwrap(),
                b"nope",
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // --- bob operations ---
        // bob can read alice's home (read is always allowed)
        let data = vfs.read("bob", &alice_notes).await.unwrap();
        assert_eq!(data, b"alice's private notes");

        // bob cannot write to alice's home
        let err = vfs
            .write("bob", &alice_notes, b"hacked")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // bob can write to /shared/
        let bob_file = VfsPath::new("/shared/bob-file.txt").unwrap();
        vfs.write("bob", &bob_file, b"bob's contribution")
            .await
            .unwrap();

        // bob can write to own home
        let bob_private = VfsPath::new("/home/bob/private.md").unwrap();
        vfs.write("bob", &bob_private, b"bob's secret")
            .await
            .unwrap();

        // --- copy/move with permissions ---
        let backup = VfsPath::new("/shared/backup.md").unwrap();
        vfs.copy("alice", &shared_tasks, &backup).await.unwrap();
        assert_eq!(
            vfs.read("alice", &backup).await.unwrap(),
            b"# tasks\n- implement vfs"
        );

        // alice cannot copy to bob's home
        let err = vfs
            .copy(
                "alice",
                &shared_tasks,
                &VfsPath::new("/home/bob/stolen.md").unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // bob moves his file in /shared/
        let renamed = VfsPath::new("/shared/renamed.txt").unwrap();
        vfs.rename("bob", &bob_file, &renamed).await.unwrap();
        assert!(!vfs.exists("bob", &bob_file).await.unwrap());
        assert_eq!(
            vfs.read("bob", &renamed).await.unwrap(),
            b"bob's contribution"
        );

        // --- delete with permissions ---
        // bob cannot delete alice's file
        let err = vfs.delete("bob", &alice_notes).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        // alice deletes her own file
        vfs.delete("alice", &alice_notes).await.unwrap();
        assert!(!vfs.exists("alice", &alice_notes).await.unwrap());

        // --- list and metadata ---
        let mut shared_entries = vfs
            .list("alice", &VfsPath::new("/shared").unwrap())
            .await
            .unwrap();
        shared_entries.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = shared_entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"backup.md"));
        assert!(names.contains(&"renamed.txt"));
        assert!(names.contains(&"tasks.md"));

        let meta = vfs.metadata("alice", &backup).await.unwrap();
        assert_eq!(meta.kind, VfsEntryKind::File);
        assert!(meta.size > 0);

        let mut home_entries = vfs
            .list("alice", &VfsPath::new("/home").unwrap())
            .await
            .unwrap();
        home_entries.sort_by(|a, b| a.name.cmp(&b.name));
        let home_names: Vec<&str> = home_entries.iter().map(|e| e.name.as_str()).collect();
        assert!(home_names.contains(&"alice"));
        assert!(home_names.contains(&"bob"));

        // --- mkdir ---
        let subdir = VfsPath::new("/shared/subdir").unwrap();
        vfs.mkdir("alice", &subdir).await.unwrap();
        assert!(vfs.exists("alice", &subdir).await.unwrap());
        let subdir_meta = vfs.metadata("alice", &subdir).await.unwrap();
        assert_eq!(subdir_meta.kind, VfsEntryKind::Directory);

        // alice cannot mkdir in /sys/
        let err = vfs
            .mkdir("alice", &VfsPath::new("/sys/forbidden").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
}
