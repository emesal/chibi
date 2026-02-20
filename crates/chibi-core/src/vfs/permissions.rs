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
