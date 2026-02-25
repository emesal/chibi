//! VFS permission enforcement.
//!
//! Permissions are zone-based and determined entirely by path structure:
//! - `/shared/` — all contexts can read and write
//! - `/home/<context>/` — owner has read+write, others read-only
//! - `/sys/` — read-only (only SYSTEM can write)
//! - everything else at root level — read-only (only SYSTEM can write)

use std::io::{self, ErrorKind};

use super::caller::VfsCaller;
use super::path::VfsPath;

/// Check whether a caller name is reserved and cannot be used as a context name.
pub fn is_reserved_caller_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("SYSTEM")
}

/// Check read permission. Always succeeds — all zones are world-readable.
pub fn check_read(_caller: VfsCaller<'_>, _path: &VfsPath) -> io::Result<()> {
    Ok(())
}

/// Check write permission based on caller identity and path zone.
///
/// `VfsCaller::System` has unrestricted write access to all zones.
/// `VfsCaller::Context` is subject to zone-based rules.
pub fn check_write(caller: VfsCaller<'_>, path: &VfsPath) -> io::Result<()> {
    if caller == VfsCaller::System {
        return Ok(());
    }

    let VfsCaller::Context(name) = caller else {
        unreachable!()
    };

    let p = path.as_str();

    // /shared/ — world-writable
    if p == "/shared" || p.starts_with("/shared/") {
        return Ok(());
    }

    // /home/<caller>/ — owner-writable
    if let Some(rest) = p.strip_prefix("/home/") {
        let owner = rest.split('/').next().unwrap_or("");
        if owner == name {
            return Ok(());
        }
    }

    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "context '{}' cannot write to '{}' (writable zones: /shared/, /home/{}/)",
            name, path, name
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VfsPath;

    #[test]
    fn test_system_caller_can_write_anywhere() {
        let sys = VfsPath::new("/sys/config").unwrap();
        let shared = VfsPath::new("/shared/foo").unwrap();
        let home = VfsPath::new("/home/ctx/file").unwrap();
        assert!(check_write(VfsCaller::System, &sys).is_ok());
        assert!(check_write(VfsCaller::System, &shared).is_ok());
        assert!(check_write(VfsCaller::System, &home).is_ok());
    }

    #[test]
    fn test_context_can_write_shared() {
        let p = VfsPath::new("/shared/tasks.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p).is_ok());
    }

    #[test]
    fn test_context_can_write_own_home() {
        let p = VfsPath::new("/home/planner/notes.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p).is_ok());
    }

    #[test]
    fn test_context_cannot_write_other_home() {
        let p = VfsPath::new("/home/coder/notes.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p).is_err());
    }

    #[test]
    fn test_context_cannot_write_sys() {
        let p = VfsPath::new("/sys/info").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p).is_err());
    }

    #[test]
    fn test_context_cannot_write_root() {
        let p = VfsPath::new("/random_file").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p).is_err());
    }

    #[test]
    fn test_read_always_allowed() {
        let paths = ["/shared/x", "/home/other/x", "/sys/x", "/root_file"];
        for p in &paths {
            let path = VfsPath::new(p).unwrap();
            assert!(check_read(VfsCaller::Context("anyctx"), &path).is_ok());
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
