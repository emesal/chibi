//! VFS permission enforcement.
//!
//! Permissions are zone-based and determined entirely by path structure:
//! - `/shared/` — all contexts can read and write
//! - `/home/<context>/` — owner has read+write, others read-only
//! - `/sys/` — read-only (only SYSTEM can write)
//! - `/site/` — all contexts can read and write
//! - `/flocks/registry.json` — SYSTEM only
//! - `/flocks/<name>/*` — flock members + SYSTEM
//! - `/tools/sys/` — read-only, never writable (virtual; even SYSTEM cannot write)
//! - `/tools/shared/` — world-writable
//! - `/tools/home/<context>/` — owner-writable
//! - `/tools/flocks/<name>/` — flock members only
//! - everything else at root level — read-only (only SYSTEM can write)

use std::io::{self, ErrorKind};

use super::caller::VfsCaller;
use super::flock::{FlockRegistry, site_flock_name};
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
/// `VfsCaller::Context` is subject to zone-based rules:
/// - `/shared/` — world-writable
/// - `/home/<ctx>/` — owner-writable
/// - `/site/` — world-writable
/// - `/flocks/registry.json` — System only
/// - `/flocks/<name>/*` — flock members (requires `flock_ctx`)
///
/// `flock_ctx` is `Some((registry, site_id))` when the caller has flock membership data
/// available. Pass `None` to skip flock membership checks (permission denied for `/flocks/`).
pub fn check_write(
    caller: VfsCaller<'_>,
    path: &VfsPath,
    flock_ctx: Option<(&FlockRegistry, &str)>,
) -> io::Result<()> {
    let p = path.as_str();

    // /tools/sys/ — never writable; not backed by real storage, even SYSTEM cannot write.
    // This check must precede the System early-return below.
    if p == "/tools/sys" || p.starts_with("/tools/sys/") {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("'{}' is read-only (virtual tool registry)", path),
        ));
    }

    if caller == VfsCaller::System {
        return Ok(());
    }

    let VfsCaller::Context(name) = caller else {
        unreachable!()
    };

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

    // /site/ — world-writable (shared site-flock area)
    if p == "/site" || p.starts_with("/site/") {
        return Ok(());
    }

    // /flocks/registry.json — System only (already handled above)
    if p == "/flocks/registry.json" {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "context '{}' cannot write to flock registry (System only)",
                name
            ),
        ));
    }

    // /flocks/<name>/* — flock members only
    if let Some(rest) = p.strip_prefix("/flocks/") {
        let flock_name = rest.split('/').next().unwrap_or("");
        if !flock_name.is_empty() {
            if let Some((registry, site_id)) = flock_ctx
                && registry.is_member(flock_name, name, &site_flock_name(site_id))
            {
                return Ok(());
            }
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "context '{}' is not a member of flock '{}'",
                    name, flock_name
                ),
            ));
        }
    }

    // /tools/shared/ — world-writable (synthesised tools shared across all contexts)
    if p == "/tools/shared" || p.starts_with("/tools/shared/") {
        return Ok(());
    }

    // /tools/home/<ctx>/ — owner-writable
    if let Some(rest) = p.strip_prefix("/tools/home/") {
        let owner = rest.split('/').next().unwrap_or("");
        if owner == name {
            return Ok(());
        }
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("context '{}' cannot write to /tools/home/{}/", name, owner),
        ));
    }

    // /tools/flocks/<name>/ — flock members only
    if let Some(rest) = p.strip_prefix("/tools/flocks/") {
        let flock_name = rest.split('/').next().unwrap_or("");
        if !flock_name.is_empty() {
            if let Some((registry, site_id)) = flock_ctx
                && registry.is_member(flock_name, name, &site_flock_name(site_id))
            {
                return Ok(());
            }
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "context '{}' is not a member of flock '{}'",
                    name, flock_name
                ),
            ));
        }
    }

    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "context '{}' cannot write to '{}' (writable zones: /shared/, /home/{}/, /site/, /flocks/<joined-flock>/, /tools/shared/, /tools/home/{}/, /tools/flocks/<joined-flock>/)",
            name, path, name, name
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{VfsPath, flock::FlockRegistry};

    #[test]
    fn test_system_caller_can_write_anywhere() {
        let sys = VfsPath::new("/sys/config").unwrap();
        let shared = VfsPath::new("/shared/foo").unwrap();
        let home = VfsPath::new("/home/ctx/file").unwrap();
        let flocks_reg = VfsPath::new("/flocks/registry.json").unwrap();
        assert!(check_write(VfsCaller::System, &sys, None).is_ok());
        assert!(check_write(VfsCaller::System, &shared, None).is_ok());
        assert!(check_write(VfsCaller::System, &home, None).is_ok());
        assert!(check_write(VfsCaller::System, &flocks_reg, None).is_ok());
    }

    #[test]
    fn test_context_can_write_shared() {
        let p = VfsPath::new("/shared/tasks.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p, None).is_ok());
    }

    #[test]
    fn test_context_can_write_own_home() {
        let p = VfsPath::new("/home/planner/notes.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p, None).is_ok());
    }

    #[test]
    fn test_context_cannot_write_other_home() {
        let p = VfsPath::new("/home/coder/notes.md").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p, None).is_err());
    }

    #[test]
    fn test_context_cannot_write_sys() {
        let p = VfsPath::new("/sys/info").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p, None).is_err());
    }

    #[test]
    fn test_context_cannot_write_root() {
        let p = VfsPath::new("/random_file").unwrap();
        assert!(check_write(VfsCaller::Context("planner"), &p, None).is_err());
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

    // --- /site/ zone ---

    #[test]
    fn test_context_can_write_site() {
        let path = VfsPath::new("/site/goals.md").unwrap();
        assert!(check_write(VfsCaller::Context("any"), &path, None).is_ok());
    }

    #[test]
    fn test_context_can_write_site_root() {
        let path = VfsPath::new("/site").unwrap();
        assert!(check_write(VfsCaller::Context("any"), &path, None).is_ok());
    }

    // --- /flocks/ zone ---

    #[test]
    fn test_context_cannot_write_flocks_registry() {
        let path = VfsPath::new("/flocks/registry.json").unwrap();
        assert!(check_write(VfsCaller::Context("ctx"), &path, None).is_err());
    }

    #[test]
    fn test_system_can_write_flocks_registry() {
        let path = VfsPath::new("/flocks/registry.json").unwrap();
        assert!(check_write(VfsCaller::System, &path, None).is_ok());
    }

    #[test]
    fn test_flock_member_can_write() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        let path = VfsPath::new("/flocks/frontend/goals.md").unwrap();
        assert!(check_write(VfsCaller::Context("ui-dev"), &path, Some((&reg, "abc"))).is_ok());
    }

    #[test]
    fn test_non_member_cannot_write_flock() {
        let reg = FlockRegistry::default();
        let path = VfsPath::new("/flocks/frontend/goals.md").unwrap();
        assert!(check_write(VfsCaller::Context("outsider"), &path, Some((&reg, "abc"))).is_err());
    }

    #[test]
    fn test_flock_write_without_registry_denied() {
        let path = VfsPath::new("/flocks/frontend/goals.md").unwrap();
        // No registry provided → denied
        assert!(check_write(VfsCaller::Context("ui-dev"), &path, None).is_err());
    }

    // --- /tools/ zones ---

    #[test]
    fn test_tools_sys_read_allowed() {
        let path = VfsPath::new("/tools/sys/shell_exec").unwrap();
        assert!(check_read(VfsCaller::Context("any"), &path).is_ok());
    }

    #[test]
    fn test_tools_sys_write_denied_for_context() {
        let path = VfsPath::new("/tools/sys/shell_exec").unwrap();
        assert!(check_write(VfsCaller::Context("any"), &path, None).is_err());
    }

    #[test]
    fn test_tools_sys_write_denied_for_system() {
        // /tools/sys/ is virtual — even SYSTEM cannot write
        let path = VfsPath::new("/tools/sys/anything").unwrap();
        assert!(check_write(VfsCaller::System, &path, None).is_err());
    }

    #[test]
    fn test_tools_sys_root_write_denied_for_system() {
        let path = VfsPath::new("/tools/sys").unwrap();
        assert!(check_write(VfsCaller::System, &path, None).is_err());
    }

    #[test]
    fn test_tools_shared_writable_by_context() {
        let path = VfsPath::new("/tools/shared/my_tool.scm").unwrap();
        assert!(check_write(VfsCaller::Context("any"), &path, None).is_ok());
    }

    #[test]
    fn test_tools_shared_root_writable() {
        let path = VfsPath::new("/tools/shared").unwrap();
        assert!(check_write(VfsCaller::Context("any"), &path, None).is_ok());
    }

    #[test]
    fn test_tools_home_owner_writable() {
        let path = VfsPath::new("/tools/home/alice/my_tool.scm").unwrap();
        assert!(check_write(VfsCaller::Context("alice"), &path, None).is_ok());
    }

    #[test]
    fn test_tools_home_non_owner_denied() {
        let path = VfsPath::new("/tools/home/alice/my_tool.scm").unwrap();
        assert!(check_write(VfsCaller::Context("bob"), &path, None).is_err());
    }

    #[test]
    fn test_tools_flocks_member_writable() {
        let mut reg = FlockRegistry::default();
        reg.add_member("devteam", "alice", "site:abc");
        let path = VfsPath::new("/tools/flocks/devteam/shared_tool.scm").unwrap();
        assert!(check_write(VfsCaller::Context("alice"), &path, Some((&reg, "abc"))).is_ok());
    }

    #[test]
    fn test_tools_flocks_non_member_denied() {
        let reg = FlockRegistry::default();
        let path = VfsPath::new("/tools/flocks/devteam/shared_tool.scm").unwrap();
        assert!(check_write(VfsCaller::Context("bob"), &path, Some((&reg, "abc"))).is_err());
    }
}
