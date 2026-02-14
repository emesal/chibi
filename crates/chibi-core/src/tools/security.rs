//! Security utilities for path validation and URL safety classification.
//!
//! Single source of truth for file path allowlist checks (used by both
//! `file_tools` and `agent_tools`) and URL SSRF classification.

use crate::config::ResolvedConfig;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

// ============================================================================
// File Path Validation
// ============================================================================

/// Resolve and validate a file path against `file_tools_allowed_paths`.
///
/// Performs tilde expansion, canonicalization, and allowlist checking.
/// Returns the canonical path on success, or `PermissionDenied` on failure.
pub fn validate_file_path(path: &str, config: &ResolvedConfig) -> io::Result<PathBuf> {
    let resolved = if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?;
        home.join(rest)
    } else if path == "~" {
        dirs_next::home_dir().ok_or_else(|| {
            io::Error::new(ErrorKind::NotFound, "Could not determine home directory")
        })?
    } else {
        PathBuf::from(path)
    };

    let canonical = resolved.canonicalize().map_err(|e| {
        io::Error::new(
            ErrorKind::NotFound,
            format!("Could not resolve path '{}': {}", path, e),
        )
    })?;

    if config.file_tools_allowed_paths.is_empty() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "File path access is not allowed. Use cache_id to access cached tool outputs, or configure file_tools_allowed_paths.",
        ));
    }

    let allowed = config.file_tools_allowed_paths.iter().any(|allowed_path| {
        let allowed_resolved = if let Some(rest) = allowed_path.strip_prefix("~/") {
            dirs_next::home_dir().map(|home| home.join(rest))
        } else if allowed_path == "~" {
            dirs_next::home_dir()
        } else {
            Some(PathBuf::from(allowed_path))
        };

        allowed_resolved
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|allowed_canonical| canonical.starts_with(&allowed_canonical))
    });

    if !allowed {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "Path '{}' is not under any allowed path. Allowed: {:?}",
                path, config.file_tools_allowed_paths
            ),
        ));
    }

    Ok(canonical)
}

// ============================================================================
// URL Safety Classification (SSRF Protection)
// ============================================================================

/// URL safety classification for SSRF protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlSafety {
    /// URL points to public internet — safe to fetch.
    Safe,
    /// URL points to a sensitive target — requires permission.
    Sensitive(String),
}

/// Classify a URL as safe or sensitive for SSRF protection.
///
/// Sensitive targets: loopback, private RFC 1918, link-local, cloud metadata,
/// `localhost` hostname. Invalid URLs are classified as sensitive (fail-safe).
pub fn classify_url(url: &str) -> UrlSafety {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return UrlSafety::Sensitive("could not parse URL".to_string()),
    };

    match parsed.host() {
        None => UrlSafety::Sensitive("URL has no host".to_string()),
        Some(url::Host::Ipv4(ip)) => classify_ipv4(ip),
        Some(url::Host::Ipv6(ip)) => classify_ipv6(ip),
        Some(url::Host::Domain(domain)) => {
            if domain.eq_ignore_ascii_case("localhost") {
                UrlSafety::Sensitive("loopback address (localhost)".to_string())
            } else {
                // Regular hostname — safe (DNS resolution happens at fetch time)
                UrlSafety::Safe
            }
        }
    }
}

fn classify_ipv4(ip: Ipv4Addr) -> UrlSafety {
    if ip.is_loopback() {
        UrlSafety::Sensitive("loopback address".to_string())
    } else if ip.octets()[0] == 169 && ip.octets()[1] == 254 {
        // Cloud metadata endpoint or link-local
        if ip == Ipv4Addr::new(169, 254, 169, 254) {
            UrlSafety::Sensitive("cloud metadata endpoint".to_string())
        } else {
            UrlSafety::Sensitive("link-local address".to_string())
        }
    } else if ip.is_private() {
        UrlSafety::Sensitive("private network address".to_string())
    } else {
        UrlSafety::Safe
    }
}

fn classify_ipv6(ip: Ipv6Addr) -> UrlSafety {
    if ip.is_loopback() {
        UrlSafety::Sensitive("loopback address".to_string())
    } else if (ip.segments()[0] & 0xffc0) == 0xfe80 {
        UrlSafety::Sensitive("link-local address".to_string())
    } else {
        UrlSafety::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiParams, ToolsConfig};
    use crate::partition::StorageConfig;

    fn make_test_config(allowed_paths: Vec<String>) -> ResolvedConfig {
        ResolvedConfig {
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            context_window_limit: 128000,
            warn_threshold_percent: 0.8,
            verbose: false,
            hide_tool_calls: false,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 0.9,
            fuel: 5,
            fuel_empty_response_cost: 15,
            username: "user".to_string(),
            reflection_enabled: false,
            reflection_character_limit: 10000,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 5000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: allowed_paths,
            api: ApiParams::default(),
            tools: ToolsConfig::default(),
            fallback_tool: "call_agent".to_string(),
            storage: StorageConfig::default(),
        }
    }

    // === validate_file_path ===

    #[test]
    fn test_validate_path_under_allowed_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();
        let config = make_test_config(vec![dir.path().to_string_lossy().to_string()]);
        let result = validate_file_path(file.to_str().unwrap(), &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_outside_allowed_dir() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, "secret").unwrap();
        let config = make_test_config(vec![allowed.path().to_string_lossy().to_string()]);
        let result = validate_file_path(file.to_str().unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_validate_path_empty_allowlist_denies() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();
        let config = make_test_config(vec![]);
        let result = validate_file_path(file.to_str().unwrap(), &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_validate_path_nonexistent_file() {
        let config = make_test_config(vec!["/tmp".to_string()]);
        let result = validate_file_path("/tmp/nonexistent_abc123.txt", &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    // === classify_url ===

    #[test]
    fn test_classify_url_public_safe() {
        assert!(matches!(
            classify_url("https://example.com"),
            UrlSafety::Safe
        ));
        assert!(matches!(
            classify_url("https://api.github.com/repos"),
            UrlSafety::Safe
        ));
        assert!(matches!(
            classify_url("http://1.2.3.4/path"),
            UrlSafety::Safe
        ));
    }

    #[test]
    fn test_classify_url_localhost_sensitive() {
        match classify_url("http://localhost:8080/admin") {
            UrlSafety::Sensitive(reason) => {
                assert!(reason.contains("loopback"), "reason: {}", reason)
            }
            UrlSafety::Safe => panic!("localhost should be sensitive"),
        }
    }

    #[test]
    fn test_classify_url_loopback_ip_sensitive() {
        match classify_url("http://127.0.0.1:9200/_cat/indices") {
            UrlSafety::Sensitive(reason) => {
                assert!(reason.contains("loopback"), "reason: {}", reason)
            }
            UrlSafety::Safe => panic!("127.0.0.1 should be sensitive"),
        }
        match classify_url("http://127.0.0.42/test") {
            UrlSafety::Sensitive(_) => {}
            UrlSafety::Safe => panic!("127.x.x.x should be sensitive"),
        }
    }

    #[test]
    fn test_classify_url_private_rfc1918_sensitive() {
        for url in &[
            "http://10.0.0.1/internal",
            "http://172.16.0.1/api",
            "http://192.168.1.1/router",
        ] {
            match classify_url(url) {
                UrlSafety::Sensitive(reason) => assert!(
                    reason.contains("private"),
                    "url: {}, reason: {}",
                    url,
                    reason
                ),
                UrlSafety::Safe => panic!("{} should be sensitive", url),
            }
        }
    }

    #[test]
    fn test_classify_url_link_local_sensitive() {
        match classify_url("http://169.254.169.254/latest/meta-data/") {
            UrlSafety::Sensitive(reason) => assert!(
                reason.contains("metadata") || reason.contains("link-local"),
                "reason: {}",
                reason
            ),
            UrlSafety::Safe => panic!("metadata endpoint should be sensitive"),
        }
    }

    #[test]
    fn test_classify_url_ipv6_loopback_sensitive() {
        match classify_url("http://[::1]:3000/") {
            UrlSafety::Sensitive(reason) => {
                assert!(reason.contains("loopback"), "reason: {}", reason)
            }
            UrlSafety::Safe => panic!("::1 should be sensitive"),
        }
    }

    #[test]
    fn test_classify_url_invalid() {
        match classify_url("not-a-url") {
            UrlSafety::Sensitive(reason) => assert!(
                reason.contains("parse") || reason.contains("invalid"),
                "reason: {}",
                reason
            ),
            UrlSafety::Safe => panic!("invalid URL should be sensitive"),
        }
    }
}
