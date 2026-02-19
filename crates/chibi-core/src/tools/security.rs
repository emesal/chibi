//! Security utilities for path validation and URL safety classification.
//!
//! Single source of truth for file path access control (used by both
//! `file_tools` and `agent_tools`) and URL SSRF classification.
//!
//! File path access uses a two-tier model:
//! - Paths under `file_tools_allowed_paths` (auto-populated with cwd if empty)
//!   are allowed directly for reads.
//! - Paths outside allowed paths require user permission via `PreFileRead` hook.
//! - Writes always require permission via `PreFileWrite` hook regardless of path.

use crate::config::ResolvedConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

// ============================================================================
// File Path Validation
// ============================================================================

/// File path access classification result.
///
/// Used by the agentic loop to decide whether a read operation can proceed
/// directly or needs user permission via the `PreFileRead` hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePathAccess {
    /// Path is under an allowed directory — proceed without prompting.
    Allowed(PathBuf),
    /// Path is outside allowed directories — requires user permission.
    NeedsPermission(PathBuf),
}

/// Resolve and classify a file path against `file_tools_allowed_paths`.
///
/// Performs tilde expansion, canonicalization, and allowlist checking.
/// Returns `Allowed` for paths under an allowed directory, or
/// `NeedsPermission` for paths outside (including when the allowlist is empty).
pub fn classify_file_path(path: &str, config: &ResolvedConfig) -> io::Result<FilePathAccess> {
    let canonical = resolve_and_canonicalize(path)?;

    let is_allowed = !config.file_tools_allowed_paths.is_empty()
        && config.file_tools_allowed_paths.iter().any(|allowed_path| {
            resolve_allowed_path(allowed_path)
                .is_some_and(|allowed_canonical| canonical.starts_with(&allowed_canonical))
        });

    if is_allowed {
        Ok(FilePathAccess::Allowed(canonical))
    } else {
        Ok(FilePathAccess::NeedsPermission(canonical))
    }
}

/// Resolve and validate a file path against `file_tools_allowed_paths`.
///
/// Performs tilde expansion, canonicalization, and allowlist checking.
/// Returns the canonical path on success, or `PermissionDenied` on failure.
///
/// This is a strict wrapper around `classify_file_path` that maps
/// `NeedsPermission` to an error. Used by callers that don't handle
/// interactive permission prompting (e.g. agent_tools).
pub fn validate_file_path(path: &str, config: &ResolvedConfig) -> io::Result<PathBuf> {
    match classify_file_path(path, config)? {
        FilePathAccess::Allowed(canonical) => Ok(canonical),
        FilePathAccess::NeedsPermission(_) => Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "Path '{}' is not under any allowed path. Allowed: {:?}",
                path, config.file_tools_allowed_paths
            ),
        )),
    }
}

/// Tilde-expand and canonicalize a file path.
fn resolve_and_canonicalize(path: &str) -> io::Result<PathBuf> {
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

    resolved.canonicalize().map_err(|e| {
        io::Error::new(
            ErrorKind::NotFound,
            format!("Could not resolve path '{}': {}", path, e),
        )
    })
}

/// Resolve an allowed path entry (tilde expansion + canonicalization).
fn resolve_allowed_path(allowed_path: &str) -> Option<PathBuf> {
    let resolved = if let Some(rest) = allowed_path.strip_prefix("~/") {
        dirs_next::home_dir().map(|home| home.join(rest))
    } else if allowed_path == "~" {
        dirs_next::home_dir()
    } else {
        Some(PathBuf::from(allowed_path))
    };

    resolved.and_then(|p| p.canonicalize().ok())
}

/// Ensure `project_root` is included in `file_tools_allowed_paths`.
///
/// If `project_root` is not already covered by an existing allowed path
/// (by canonical prefix comparison), adds it. This ensures files within
/// the project are readable when `project_root` differs from process CWD.
pub fn ensure_project_root_allowed(config: &mut ResolvedConfig, project_root: &Path) {
    let canonical_root = project_root.canonicalize().ok();
    let already_covered = canonical_root.as_ref().is_some_and(|root| {
        config
            .file_tools_allowed_paths
            .iter()
            .any(|p| resolve_allowed_path(p).is_some_and(|allowed| root.starts_with(&allowed)))
    });
    if !already_covered {
        config
            .file_tools_allowed_paths
            .push(project_root.to_string_lossy().to_string());
    }
}

// ============================================================================
// URL Safety Classification (SSRF Protection)
// ============================================================================

/// Built-in URL categories for policy matching.
///
/// Maps 1:1 onto the classification in `classify_url()`. The `Display` impl
/// provides human-readable reason strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlCategory {
    Loopback,
    PrivateNetwork,
    LinkLocal,
    CloudMetadata,
    Unparseable,
}

impl fmt::Display for UrlCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loopback => write!(f, "loopback address"),
            Self::PrivateNetwork => write!(f, "private network address"),
            Self::LinkLocal => write!(f, "link-local address"),
            Self::CloudMetadata => write!(f, "cloud metadata endpoint"),
            Self::Unparseable => write!(f, "could not parse URL"),
        }
    }
}

/// URL safety classification for SSRF protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlSafety {
    /// URL points to public internet — safe to fetch.
    Safe,
    /// URL points to a sensitive target — requires permission.
    Sensitive(UrlCategory),
}

/// Classify a URL as safe or sensitive for SSRF protection.
///
/// Sensitive targets: loopback, private RFC 1918, link-local, cloud metadata,
/// `localhost` hostname. Invalid URLs are classified as sensitive (fail-safe).
pub fn classify_url(url: &str) -> UrlSafety {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return UrlSafety::Sensitive(UrlCategory::Unparseable),
    };

    match parsed.host() {
        None => UrlSafety::Sensitive(UrlCategory::Unparseable),
        Some(url::Host::Ipv4(ip)) => classify_ipv4(ip),
        Some(url::Host::Ipv6(ip)) => classify_ipv6(ip),
        Some(url::Host::Domain(domain)) => {
            if domain.eq_ignore_ascii_case("localhost") {
                UrlSafety::Sensitive(UrlCategory::Loopback)
            } else {
                UrlSafety::Safe
            }
        }
    }
}

fn classify_ipv4(ip: Ipv4Addr) -> UrlSafety {
    if ip.is_loopback() {
        UrlSafety::Sensitive(UrlCategory::Loopback)
    } else if ip.octets()[0] == 169 && ip.octets()[1] == 254 {
        if ip == Ipv4Addr::new(169, 254, 169, 254) {
            UrlSafety::Sensitive(UrlCategory::CloudMetadata)
        } else {
            UrlSafety::Sensitive(UrlCategory::LinkLocal)
        }
    } else if ip.is_private() {
        UrlSafety::Sensitive(UrlCategory::PrivateNetwork)
    } else {
        UrlSafety::Safe
    }
}

fn classify_ipv6(ip: Ipv6Addr) -> UrlSafety {
    if ip.is_loopback() {
        UrlSafety::Sensitive(UrlCategory::Loopback)
    } else if (ip.segments()[0] & 0xffc0) == 0xfe80 {
        UrlSafety::Sensitive(UrlCategory::LinkLocal)
    } else {
        UrlSafety::Safe
    }
}

// ============================================================================
// URL Security Policy
// ============================================================================

/// Action taken by a URL policy rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlAction {
    Allow,
    Deny,
}

/// A single rule entry — preset category or URL glob pattern.
///
/// In config/JSON, presets use `"preset:category_name"` syntax,
/// bare strings are glob patterns (`*`/`?` wildcards, `\*` for literal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub enum UrlRule {
    Preset(UrlCategory),
    Pattern(String),
}

impl TryFrom<String> for UrlRule {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        if let Some(category) = s.strip_prefix("preset:") {
            let cat: UrlCategory = serde_json::from_str(&format!("\"{}\"", category))
                .map_err(|_| format!("unknown URL category: {}", category))?;
            Ok(UrlRule::Preset(cat))
        } else {
            Ok(UrlRule::Pattern(s))
        }
    }
}

impl From<UrlRule> for String {
    fn from(rule: UrlRule) -> String {
        match rule {
            UrlRule::Preset(cat) => {
                format!(
                    "preset:{}",
                    serde_json::to_string(&cat).unwrap().trim_matches('"')
                )
            }
            UrlRule::Pattern(pat) => pat,
        }
    }
}

/// URL security policy with two-tier allow/deny override semantics.
///
/// Evaluation order (first match wins, highest priority first):
/// 1. `deny_override`  — unconditional deny
/// 2. `allow_override` — unconditional allow (except deny_override)
/// 3. `deny`           — standard deny
/// 4. `allow`          — standard allow
/// 5. `default`        — fallback
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UrlPolicy {
    /// Fallback action when no rule matches (default: allow for backwards compat).
    #[serde(default = "default_url_action")]
    pub default: UrlAction,
    #[serde(default)]
    pub allow: Vec<UrlRule>,
    #[serde(default)]
    pub deny: Vec<UrlRule>,
    #[serde(default)]
    pub allow_override: Vec<UrlRule>,
    #[serde(default)]
    pub deny_override: Vec<UrlRule>,
}

fn default_url_action() -> UrlAction {
    UrlAction::Allow
}

/// Evaluate a URL against a policy. Returns the action to take.
///
/// `url` is the original URL string (for glob matching).
/// `safety` is the result of `classify_url()` (for category matching).
///
/// For glob matching, the URL is canonicalized (lowercased, parsed and
/// re-serialized) to prevent bypasses via case or encoding tricks.
pub fn evaluate_url_policy(url: &str, safety: &UrlSafety, policy: &UrlPolicy) -> UrlAction {
    let category = match safety {
        UrlSafety::Sensitive(cat) => Some(cat),
        UrlSafety::Safe => None,
    };

    // canonicalize for glob matching: parse normalizes percent-encoding and
    // lowercases the host; we lowercase the whole URL for consistent matching
    let canonical = url::Url::parse(url)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| url.to_lowercase());

    if rule_matches(&policy.deny_override, category, &canonical) {
        return UrlAction::Deny;
    }
    if rule_matches(&policy.allow_override, category, &canonical) {
        return UrlAction::Allow;
    }
    if rule_matches(&policy.deny, category, &canonical) {
        return UrlAction::Deny;
    }
    if rule_matches(&policy.allow, category, &canonical) {
        return UrlAction::Allow;
    }
    policy.default
}

/// Check if any rule in the list matches the given URL.
fn rule_matches(rules: &[UrlRule], category: Option<&UrlCategory>, url: &str) -> bool {
    rules.iter().any(|rule| match rule {
        UrlRule::Preset(cat) => category == Some(cat),
        UrlRule::Pattern(pattern) => glob_match(pattern, url),
    })
}

/// Simple glob matching: `*` matches any sequence, `?` matches one char,
/// `\*` matches literal `*`, `\?` matches literal `?`.
///
/// Consecutive unescaped `*` are collapsed into a single `*` before matching,
/// so patterns like `****` behave identically to `*`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let raw: Vec<char> = pattern.chars().collect();
    // Collapse consecutive unescaped `*` into a single `*`
    let mut pat = Vec::with_capacity(raw.len());
    for (i, &ch) in raw.iter().enumerate() {
        if ch == '*' && i > 0 && raw[i - 1] == '*' {
            // Skip if previous char was also `*` (and wasn't escaped)
            if i >= 2 && raw[i - 2] == '\\' {
                pat.push(ch); // previous `*` was escaped, keep this one
            }
            // else: consecutive unescaped `*`, skip
        } else {
            pat.push(ch);
        }
    }
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt)
}

fn glob_match_inner(pat: &[char], txt: &[char]) -> bool {
    match (pat.first(), txt.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            glob_match_inner(&pat[1..], txt)
                || (!txt.is_empty() && glob_match_inner(pat, &txt[1..]))
        }
        (Some('?'), Some(_)) => glob_match_inner(&pat[1..], &txt[1..]),
        (Some('\\'), _) => {
            if pat.len() > 1 && !txt.is_empty() && pat[1] == txt[0] {
                glob_match_inner(&pat[2..], &txt[1..])
            } else {
                false
            }
        }
        (Some(p), Some(t)) if *p == *t => glob_match_inner(&pat[1..], &txt[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiParams, ToolsConfig};
    use crate::partition::StorageConfig;
    use std::collections::BTreeMap;

    fn make_test_config(allowed_paths: Vec<String>) -> ResolvedConfig {
        ResolvedConfig {
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            context_window_limit: 128000,
            warn_threshold_percent: 0.8,
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
            url_policy: None,
            extra: BTreeMap::new(),
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

    // === classify_file_path ===

    #[test]
    fn test_classify_path_under_allowed_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();
        let config = make_test_config(vec![dir.path().to_string_lossy().to_string()]);
        let result = classify_file_path(file.to_str().unwrap(), &config);
        assert!(matches!(result, Ok(FilePathAccess::Allowed(_))));
    }

    #[test]
    fn test_classify_path_outside_allowed_dir() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, "secret").unwrap();
        let config = make_test_config(vec![allowed.path().to_string_lossy().to_string()]);
        let result = classify_file_path(file.to_str().unwrap(), &config);
        assert!(matches!(result, Ok(FilePathAccess::NeedsPermission(_))));
    }

    #[test]
    fn test_classify_path_empty_allowlist_needs_permission() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();
        let config = make_test_config(vec![]);
        let result = classify_file_path(file.to_str().unwrap(), &config);
        assert!(matches!(result, Ok(FilePathAccess::NeedsPermission(_))));
    }

    #[test]
    fn test_classify_path_nonexistent_file() {
        let config = make_test_config(vec!["/tmp".to_string()]);
        let result = classify_file_path("/tmp/nonexistent_abc123.txt", &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    // === UrlCategory ===

    #[test]
    fn test_url_category_display() {
        assert_eq!(UrlCategory::Loopback.to_string(), "loopback address");
        assert_eq!(
            UrlCategory::PrivateNetwork.to_string(),
            "private network address"
        );
        assert_eq!(UrlCategory::LinkLocal.to_string(), "link-local address");
        assert_eq!(
            UrlCategory::CloudMetadata.to_string(),
            "cloud metadata endpoint"
        );
        assert_eq!(UrlCategory::Unparseable.to_string(), "could not parse URL");
    }

    #[test]
    fn test_classify_url_returns_category() {
        assert_eq!(
            classify_url("http://localhost/"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
        assert_eq!(
            classify_url("http://127.0.0.1/"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
        assert_eq!(
            classify_url("http://192.168.1.1/"),
            UrlSafety::Sensitive(UrlCategory::PrivateNetwork)
        );
        assert_eq!(
            classify_url("http://169.254.1.1/"),
            UrlSafety::Sensitive(UrlCategory::LinkLocal)
        );
        assert_eq!(
            classify_url("http://169.254.169.254/"),
            UrlSafety::Sensitive(UrlCategory::CloudMetadata)
        );
        assert_eq!(classify_url("https://example.com/"), UrlSafety::Safe);
        assert!(matches!(
            classify_url("not-a-url"),
            UrlSafety::Sensitive(UrlCategory::Unparseable)
        ));
    }

    // === classify_url (original tests, updated for UrlCategory) ===

    #[test]
    fn test_classify_url_public_safe() {
        assert_eq!(classify_url("https://example.com"), UrlSafety::Safe);
        assert_eq!(
            classify_url("https://api.github.com/repos"),
            UrlSafety::Safe
        );
        assert_eq!(classify_url("http://1.2.3.4/path"), UrlSafety::Safe);
    }

    #[test]
    fn test_classify_url_localhost_sensitive() {
        assert_eq!(
            classify_url("http://localhost:8080/admin"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
    }

    #[test]
    fn test_classify_url_loopback_ip_sensitive() {
        assert_eq!(
            classify_url("http://127.0.0.1:9200/_cat/indices"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
        assert_eq!(
            classify_url("http://127.0.0.42/test"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
    }

    #[test]
    fn test_classify_url_private_rfc1918_sensitive() {
        for url in &[
            "http://10.0.0.1/internal",
            "http://172.16.0.1/api",
            "http://192.168.1.1/router",
        ] {
            assert_eq!(
                classify_url(url),
                UrlSafety::Sensitive(UrlCategory::PrivateNetwork),
                "url: {}",
                url,
            );
        }
    }

    #[test]
    fn test_classify_url_link_local_sensitive() {
        assert_eq!(
            classify_url("http://169.254.169.254/latest/meta-data/"),
            UrlSafety::Sensitive(UrlCategory::CloudMetadata)
        );
    }

    #[test]
    fn test_classify_url_ipv6_loopback_sensitive() {
        assert_eq!(
            classify_url("http://[::1]:3000/"),
            UrlSafety::Sensitive(UrlCategory::Loopback)
        );
    }

    #[test]
    fn test_classify_url_invalid() {
        assert!(matches!(
            classify_url("not-a-url"),
            UrlSafety::Sensitive(UrlCategory::Unparseable)
        ));
    }

    // === UrlPolicy evaluation ===

    fn make_policy(default: UrlAction) -> UrlPolicy {
        UrlPolicy {
            default,
            allow: vec![],
            deny: vec![],
            allow_override: vec![],
            deny_override: vec![],
        }
    }

    #[test]
    fn test_policy_default_allow() {
        let policy = make_policy(UrlAction::Allow);
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Allow,
        );
    }

    #[test]
    fn test_policy_default_deny() {
        let policy = make_policy(UrlAction::Deny);
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_allow_category() {
        let mut policy = make_policy(UrlAction::Deny);
        policy.allow.push(UrlRule::Preset(UrlCategory::Loopback));
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Allow,
        );
        assert_eq!(
            evaluate_url_policy(
                "http://192.168.1.1/",
                &classify_url("http://192.168.1.1/"),
                &policy
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_deny_category() {
        let mut policy = make_policy(UrlAction::Allow);
        policy
            .deny
            .push(UrlRule::Preset(UrlCategory::CloudMetadata));
        assert_eq!(
            evaluate_url_policy(
                "http://169.254.169.254/",
                &classify_url("http://169.254.169.254/"),
                &policy,
            ),
            UrlAction::Deny,
        );
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Allow,
        );
    }

    #[test]
    fn test_policy_deny_override_beats_allow_override() {
        let mut policy = make_policy(UrlAction::Allow);
        policy
            .allow_override
            .push(UrlRule::Preset(UrlCategory::CloudMetadata));
        policy
            .deny_override
            .push(UrlRule::Preset(UrlCategory::CloudMetadata));
        assert_eq!(
            evaluate_url_policy(
                "http://169.254.169.254/",
                &classify_url("http://169.254.169.254/"),
                &policy,
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_allow_override_beats_deny() {
        let mut policy = make_policy(UrlAction::Deny);
        policy.deny.push(UrlRule::Preset(UrlCategory::Loopback));
        policy
            .allow_override
            .push(UrlRule::Preset(UrlCategory::Loopback));
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Allow,
        );
    }

    #[test]
    fn test_policy_deny_beats_allow() {
        let mut policy = make_policy(UrlAction::Allow);
        policy.allow.push(UrlRule::Preset(UrlCategory::Loopback));
        policy.deny.push(UrlRule::Preset(UrlCategory::Loopback));
        assert_eq!(
            evaluate_url_policy(
                "http://localhost/",
                &classify_url("http://localhost/"),
                &policy
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_glob_pattern() {
        let mut policy = make_policy(UrlAction::Deny);
        policy
            .allow
            .push(UrlRule::Pattern("https://api.example.com/*".to_string()));
        assert_eq!(
            evaluate_url_policy(
                "https://api.example.com/v1/data",
                &classify_url("https://api.example.com/v1/data"),
                &policy,
            ),
            UrlAction::Allow,
        );
        assert_eq!(
            evaluate_url_policy(
                "https://evil.com/",
                &classify_url("https://evil.com/"),
                &policy,
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_glob_literal_asterisk() {
        let mut policy = make_policy(UrlAction::Deny);
        policy
            .allow
            .push(UrlRule::Pattern("https://example.com/\\*".to_string()));
        assert_eq!(
            evaluate_url_policy(
                "https://example.com/*",
                &classify_url("https://example.com/*"),
                &policy,
            ),
            UrlAction::Allow,
        );
        assert_eq!(
            evaluate_url_policy(
                "https://example.com/foo",
                &classify_url("https://example.com/foo"),
                &policy,
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_safe_url_with_deny_default() {
        let policy = make_policy(UrlAction::Deny);
        assert_eq!(
            evaluate_url_policy(
                "https://example.com/",
                &classify_url("https://example.com/"),
                &policy,
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_policy_evaluation_order_full() {
        let mut policy = make_policy(UrlAction::Allow);
        policy
            .allow
            .push(UrlRule::Preset(UrlCategory::PrivateNetwork));
        policy
            .deny
            .push(UrlRule::Preset(UrlCategory::PrivateNetwork));
        // deny beats allow at same tier
        assert_eq!(
            evaluate_url_policy(
                "http://10.0.0.1/",
                &classify_url("http://10.0.0.1/"),
                &policy
            ),
            UrlAction::Deny,
        );
        // but allow_override beats deny
        policy
            .allow_override
            .push(UrlRule::Preset(UrlCategory::PrivateNetwork));
        assert_eq!(
            evaluate_url_policy(
                "http://10.0.0.1/",
                &classify_url("http://10.0.0.1/"),
                &policy
            ),
            UrlAction::Allow,
        );
        // but deny_override beats allow_override
        policy
            .deny_override
            .push(UrlRule::Preset(UrlCategory::PrivateNetwork));
        assert_eq!(
            evaluate_url_policy(
                "http://10.0.0.1/",
                &classify_url("http://10.0.0.1/"),
                &policy
            ),
            UrlAction::Deny,
        );
    }

    #[test]
    fn test_canonicalize_percent_encoded_host() {
        // %6c%6f%63%61%6c%68%6f%73%74 = "localhost"
        let url = "http://%6c%6f%63%61%6c%68%6f%73%74/path";
        let safety = classify_url(url);
        assert!(
            matches!(safety, UrlSafety::Sensitive(UrlCategory::Loopback)),
            "percent-encoded localhost should be classified as loopback, got: {:?}",
            safety
        );
    }

    #[test]
    fn test_policy_glob_matches_case_insensitive() {
        // url::Url::parse lowercases hosts, so we should normalize for glob matching
        let mut policy = make_policy(UrlAction::Deny);
        policy
            .allow
            .push(UrlRule::Pattern("https://example.com/*".to_string()));
        let url = "https://EXAMPLE.COM/path";
        let safety = classify_url(url);
        // evaluate_url_policy should canonicalize the URL for glob matching
        assert_eq!(evaluate_url_policy(url, &safety, &policy), UrlAction::Allow,);
    }

    #[test]
    fn test_policy_overrides_sensitive_classification() {
        // a sensitive URL with an allow policy should be allowed
        let url = "http://localhost:3000/api";
        let safety = classify_url(url);
        assert!(matches!(
            safety,
            UrlSafety::Sensitive(UrlCategory::Loopback)
        ));

        let mut policy = make_policy(UrlAction::Deny);
        policy.allow.push(UrlRule::Preset(UrlCategory::Loopback));
        assert_eq!(evaluate_url_policy(url, &safety, &policy), UrlAction::Allow);

        // a safe URL with a deny policy should be denied
        let url = "https://example.com/";
        let safety = classify_url(url);
        assert_eq!(safety, UrlSafety::Safe);

        let policy = make_policy(UrlAction::Deny);
        assert_eq!(evaluate_url_policy(url, &safety, &policy), UrlAction::Deny);
    }

    // === serde round-trips ===

    #[test]
    fn test_url_rule_serde_preset() {
        let rule: UrlRule = serde_json::from_str("\"preset:loopback\"").unwrap();
        assert_eq!(rule, UrlRule::Preset(UrlCategory::Loopback));
        let serialized = serde_json::to_string(&rule).unwrap();
        assert_eq!(serialized, "\"preset:loopback\"");
    }

    #[test]
    fn test_url_rule_serde_pattern() {
        let rule: UrlRule = serde_json::from_str("\"https://example.com/*\"").unwrap();
        assert_eq!(rule, UrlRule::Pattern("https://example.com/*".to_string()));
    }

    #[test]
    fn test_url_rule_serde_invalid_preset() {
        let result: Result<UrlRule, _> = serde_json::from_str("\"preset:nonexistent\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_policy_serde_full() {
        let json = r#"{
            "default": "deny",
            "allow": ["preset:loopback", "https://api.example.com/*"],
            "deny_override": ["preset:cloud_metadata"]
        }"#;
        let policy: UrlPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.default, UrlAction::Deny);
        assert_eq!(policy.allow.len(), 2);
        assert_eq!(policy.deny.len(), 0);
        assert_eq!(policy.deny_override.len(), 1);
        assert_eq!(policy.allow_override.len(), 0);
    }

    #[test]
    fn test_url_policy_serde_defaults() {
        let json = "{}";
        let policy: UrlPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.default, UrlAction::Allow);
        assert!(policy.allow.is_empty());
        assert!(policy.deny.is_empty());
    }

    #[test]
    fn test_url_policy_toml() {
        let toml_str = r#"
default = "deny"
allow = ["preset:private_network", "preset:loopback"]
deny_override = ["preset:cloud_metadata"]
"#;
        let policy: UrlPolicy = toml::from_str(toml_str).expect("valid toml");
        assert_eq!(policy.default, UrlAction::Deny);
        assert_eq!(policy.allow.len(), 2);
        assert_eq!(policy.deny_override.len(), 1);
    }

    // === ensure_project_root_allowed ===

    #[test]
    fn test_ensure_project_root_allowed_adds_when_missing() {
        let project = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut config = make_test_config(vec![other.path().to_string_lossy().to_string()]);
        ensure_project_root_allowed(&mut config, project.path());
        assert_eq!(config.file_tools_allowed_paths.len(), 2);
    }

    #[test]
    fn test_ensure_project_root_allowed_skips_when_covered() {
        let project = tempfile::tempdir().unwrap();
        let mut config = make_test_config(vec![project.path().to_string_lossy().to_string()]);
        ensure_project_root_allowed(&mut config, project.path());
        assert_eq!(config.file_tools_allowed_paths.len(), 1);
    }
}
