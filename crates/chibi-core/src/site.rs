use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Site identity: a stable UUID plus a human-readable site_id derived from hostname + UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteIdentity {
    pub uuid: String,
    pub site_id: String,
}

/// Derive site_id from hostname and UUID.
///
/// Uses the first 8 characters of the UUID for brevity.
pub fn generate_site_id(hostname: &str, uuid: &str) -> String {
    let short = &uuid[..8.min(uuid.len())];
    format!("{}-{}", hostname, short)
}

/// Load site identity from disk, or create a new one.
///
/// If `hostname_override` is `Some`, uses that instead of the OS hostname.
/// If the derived site_id doesn't match the cached one (e.g. hostname changed),
/// updates the file.
pub fn load_or_create(
    chibi_dir: &Path,
    hostname_override: Option<&str>,
) -> io::Result<SiteIdentity> {
    let path = chibi_dir.join("site.json");
    let hostname = hostname_override.map(String::from).unwrap_or_else(|| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "chibi".to_string())
    });

    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let mut site: SiteIdentity = serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let expected_id = generate_site_id(&hostname, &site.uuid);
        if site.site_id != expected_id {
            site.site_id = expected_id;
            crate::safe_io::atomic_write_json(&path, &site)?;
        }
        Ok(site)
    } else {
        let uuid = uuid::Uuid::new_v4().to_string();
        let site_id = generate_site_id(&hostname, &uuid);
        let site = SiteIdentity { uuid, site_id };
        crate::safe_io::atomic_write_json(&path, &site)?;
        Ok(site)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_site_id() {
        let id = generate_site_id("fey-laptop", "a1b2c3d4");
        assert_eq!(id, "fey-laptop-a1b2c3d4");
    }

    #[test]
    fn test_generate_site_id_long_uuid() {
        // Uses first 8 chars only
        let id = generate_site_id("host", "a1b2c3d4-0000-0000-0000-000000000000");
        assert_eq!(id, "host-a1b2c3d4");
    }

    #[test]
    fn test_load_or_create_creates_new() {
        let dir = TempDir::new().unwrap();
        let site = load_or_create(dir.path(), Some("test-host")).unwrap();
        assert!(!site.uuid.is_empty());
        assert!(site.site_id.starts_with("test-host-"));
        // File should now exist
        assert!(dir.path().join("site.json").exists());
    }

    #[test]
    fn test_load_or_create_persists() {
        let dir = TempDir::new().unwrap();
        let site1 = load_or_create(dir.path(), Some("test-host")).unwrap();
        let site2 = load_or_create(dir.path(), Some("test-host")).unwrap();
        assert_eq!(site1.uuid, site2.uuid);
        assert_eq!(site1.site_id, site2.site_id);
    }

    #[test]
    fn test_hostname_override_regenerates_site_id() {
        let dir = TempDir::new().unwrap();
        let site1 = load_or_create(dir.path(), Some("old-host")).unwrap();
        let site2 = load_or_create(dir.path(), Some("new-host")).unwrap();
        assert_eq!(site1.uuid, site2.uuid); // UUID unchanged
        assert_ne!(site1.site_id, site2.site_id); // site_id regenerated
        assert!(site2.site_id.starts_with("new-host-"));
    }
}
