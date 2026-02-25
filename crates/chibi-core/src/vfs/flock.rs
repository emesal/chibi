use serde::{Deserialize, Serialize};
use std::io;

use crate::vfs::VfsPath;

/// A member of a flock, identified by context name and site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlockMember {
    pub context: String,
    pub site: String,
}

/// A flock entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlockEntry {
    pub name: String,
    pub created_at: u64,
    pub members: Vec<FlockMember>,
}

/// The flock registry, serialised to `/flocks/registry.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlockRegistry {
    pub flocks: Vec<FlockEntry>,
}

/// Validate a flock name: non-empty, lowercase alphanumeric + hyphens.
///
/// Reserved prefix `site:` is rejected — the site flock is implicit.
pub fn validate_flock_name(name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "flock name cannot be empty",
        ));
    }
    if name.starts_with("site:") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "flock name '{}' is reserved (site: prefix is not allowed)",
                name
            ),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "flock name '{}' must be lowercase alphanumeric + hyphens",
                name
            ),
        ));
    }
    Ok(())
}

/// Resolve a flock name to its VFS root path.
///
/// The site flock (`site:<site_id>`) maps to `/site/`.
/// Explicit named flocks map to `/flocks/<name>/`.
///
/// The `site_id` parameter is only used to compare against `flock_name`.
pub fn resolve_flock_vfs_root(flock_name: &str, site_id: &str) -> io::Result<VfsPath> {
    let site_flock = format!("site:{}", site_id);
    if flock_name == site_flock || flock_name == "site" {
        VfsPath::new("/site")
    } else {
        VfsPath::new(&format!("/flocks/{}", flock_name))
    }
}

impl FlockRegistry {
    /// Add a member to a flock, creating the flock if it doesn't exist.
    pub fn add_member(&mut self, flock: &str, context: &str, site: &str) {
        let member = FlockMember {
            context: context.to_string(),
            site: site.to_string(),
        };
        if let Some(entry) = self.flocks.iter_mut().find(|f| f.name == flock) {
            if !entry.members.contains(&member) {
                entry.members.push(member);
            }
        } else {
            self.flocks.push(FlockEntry {
                name: flock.to_string(),
                created_at: crate::context::now_timestamp(),
                members: vec![member],
            });
        }
    }

    /// Remove a member from a flock.
    pub fn remove_member(&mut self, flock: &str, context: &str, site: &str) {
        if let Some(entry) = self.flocks.iter_mut().find(|f| f.name == flock) {
            entry
                .members
                .retain(|m| m.context != context || m.site != site);
        }
    }

    /// Check if a (context, site) is a member of a flock.
    pub fn is_member(&self, flock: &str, context: &str, site: &str) -> bool {
        self.flocks
            .iter()
            .find(|f| f.name == flock)
            .map(|f| {
                f.members
                    .iter()
                    .any(|m| m.context == context && m.site == site)
            })
            .unwrap_or(false)
    }

    /// List all flocks a (context, site) belongs to, sorted by name.
    pub fn flocks_for(&self, context: &str, site: &str) -> Vec<String> {
        let mut result: Vec<String> = self
            .flocks
            .iter()
            .filter(|f| {
                f.members
                    .iter()
                    .any(|m| m.context == context && m.site == site)
            })
            .map(|f| f.name.clone())
            .collect();
        result.sort();
        result
    }

    /// Delete a flock entirely (removes all members).
    pub fn delete_flock(&mut self, flock: &str) {
        self.flocks.retain(|f| f.name != flock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_flock_name_valid() {
        assert!(validate_flock_name("frontend").is_ok());
        assert!(validate_flock_name("my-flock").is_ok());
        assert!(validate_flock_name("team123").is_ok());
    }

    #[test]
    fn test_validate_flock_name_invalid() {
        assert!(validate_flock_name("").is_err());
        assert!(validate_flock_name("has:colon").is_err());
        assert!(validate_flock_name("HAS_UPPER").is_err());
        assert!(validate_flock_name("has space").is_err());
        assert!(validate_flock_name("site:thing").is_err());
    }

    #[test]
    fn test_resolve_flock_vfs_root_site() {
        let root = resolve_flock_vfs_root("site:my-laptop-abc123", "my-laptop-abc123");
        assert_eq!(root.unwrap().as_str(), "/site");
    }

    #[test]
    fn test_resolve_flock_vfs_root_explicit() {
        let root = resolve_flock_vfs_root("frontend", "my-laptop-abc123");
        assert_eq!(root.unwrap().as_str(), "/flocks/frontend");
    }

    #[test]
    fn test_registry_add_member_creates_flock() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        assert_eq!(reg.flocks.len(), 1);
        assert_eq!(reg.flocks[0].members.len(), 1);
    }

    #[test]
    fn test_registry_add_member_idempotent() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        reg.add_member("frontend", "ui-dev", "site:abc");
        assert_eq!(reg.flocks[0].members.len(), 1);
    }

    #[test]
    fn test_registry_remove_member() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        reg.remove_member("frontend", "ui-dev", "site:abc");
        assert_eq!(reg.flocks[0].members.len(), 0);
    }

    #[test]
    fn test_registry_is_member() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        assert!(reg.is_member("frontend", "ui-dev", "site:abc"));
        assert!(!reg.is_member("frontend", "coder", "site:abc"));
        assert!(!reg.is_member("backend", "ui-dev", "site:abc"));
    }

    #[test]
    fn test_registry_flocks_for_member() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        reg.add_member("backend", "ui-dev", "site:abc");
        let flocks = reg.flocks_for("ui-dev", "site:abc");
        assert_eq!(flocks, vec!["backend", "frontend"]); // sorted
    }

    #[test]
    fn test_registry_delete_flock() {
        let mut reg = FlockRegistry::default();
        reg.add_member("frontend", "ui-dev", "site:abc");
        reg.add_member("frontend", "coder", "site:abc");
        reg.delete_flock("frontend");
        assert!(reg.flocks.is_empty());
    }
}
