# retrieve_content security boundary fix — implementation plan [DONE]

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Status:** implemented 2026-02-14. all 8 tasks complete — tests pass, clippy clean.

**Goal:** close the security bypass in `retrieve_content` (file path allowlist + URL SSRF protection)

**Architecture:** extract shared `validate_file_path` to `tools/security.rs`, add `classify_url` for SSRF detection, wire sensitive URLs through the existing hook+permission system (`PreFetchUrl`). file path validation is a hard deny; URL classification triggers interactive confirmation via the established `check_permission` pattern.

**Tech Stack:** rust, std::net for IP classification, url crate (already a transitive dep via reqwest)

---

### Task 1: create `tools/security.rs` with `validate_file_path`

**Files:**
- Create: `crates/chibi-core/src/tools/security.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs:9` (add `pub mod security`)
- Modify: `crates/chibi-core/src/tools/mod.rs` (add re-exports)

**Step 1: write the failing test**

in `security.rs`, add the module with tests first:

```rust
//! Security utilities for path validation and URL safety classification.
//!
//! Single source of truth for file path allowlist checks (used by both
//! `file_tools` and `agent_tools`) and URL SSRF classification.

use crate::config::ResolvedConfig;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// Resolve and validate a file path against `file_tools_allowed_paths`.
///
/// Performs tilde expansion, canonicalization, and allowlist checking.
/// Returns the canonical path on success, or `PermissionDenied` on failure.
pub fn validate_file_path(path: &str, config: &ResolvedConfig) -> io::Result<PathBuf> {
    todo!()
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
            warn_threshold_percent: 80.0,
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
}
```

**Step 2: register the module**

in `tools/mod.rs`, add after line 13 (`mod plugins;`):

```rust
pub mod security;
```

**Step 3: run test to verify it fails**

Run: `cargo test -p chibi-core security::tests -- --nocapture`
Expected: FAIL with `not yet implemented`

**Step 4: implement `validate_file_path`**

replace the `todo!()` body with the logic from `file_tools.rs:227-284` (the exact `resolve_and_validate_path` body):

```rust
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
```

**Step 5: run tests to verify they pass**

Run: `cargo test -p chibi-core security::tests`
Expected: all 4 tests PASS

**Step 6: commit**

```bash
git add crates/chibi-core/src/tools/security.rs crates/chibi-core/src/tools/mod.rs
git commit -m "feat(security): add tools/security.rs with validate_file_path"
```

---

### Task 2: migrate `file_tools.rs` to use `security::validate_file_path`

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs:227-284` (remove `resolve_and_validate_path`)
- Modify: `crates/chibi-core/src/tools/file_tools.rs:212` (call `security::validate_file_path`)

**Step 1: run existing file_tools tests as baseline**

Run: `cargo test -p chibi-core file_tools::tests`
Expected: all PASS

**Step 2: replace `resolve_and_validate_path` with import**

in `file_tools.rs`, remove the entire `fn resolve_and_validate_path` function (lines 227-284). then update the call site at line 212 from:

```rust
let resolved = resolve_and_validate_path(p, config)?;
```

to:

```rust
let resolved = super::security::validate_file_path(p, config)?;
```

**Step 3: run file_tools tests to verify no regression**

Run: `cargo test -p chibi-core file_tools::tests`
Expected: all PASS (same results as step 1)

**Step 4: run full test suite**

Run: `cargo test -p chibi-core`
Expected: all PASS

**Step 5: commit**

```bash
git add crates/chibi-core/src/tools/file_tools.rs
git commit -m "refactor(file_tools): use shared validate_file_path from security module"
```

---

### Task 3: wire `validate_file_path` into `agent_tools::read_file`

**Files:**
- Modify: `crates/chibi-core/src/tools/agent_tools.rs:154-169` (change `read_file` signature)
- Modify: `crates/chibi-core/src/tools/agent_tools.rs:272` (pass config to `read_file`)

**Step 1: write the failing test**

add to `agent_tools::tests`:

```rust
#[test]
fn test_read_file_respects_allowed_paths() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("allowed.txt");
    std::fs::write(&file, "allowed content").unwrap();

    // Config allows the temp dir
    let mut config = make_test_config();
    config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
    let result = read_file(file.to_str().unwrap(), &config);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "allowed content");
}

#[test]
fn test_read_file_denies_outside_allowed_paths() {
    let allowed = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("secret.txt");
    std::fs::write(&file, "secret").unwrap();

    let mut config = make_test_config();
    config.file_tools_allowed_paths = vec![allowed.path().to_string_lossy().to_string()];
    let result = read_file(file.to_str().unwrap(), &config);
    assert!(result.is_err());
}

#[test]
fn test_read_file_denies_empty_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "content").unwrap();

    let config = make_test_config(); // empty allowed_paths
    let result = read_file(file.to_str().unwrap(), &config);
    assert!(result.is_err());
}
```

**Step 2: run new tests to verify they fail**

Run: `cargo test -p chibi-core agent_tools::tests::test_read_file_respects -- --nocapture`
Expected: FAIL (wrong number of arguments — `read_file` doesn't take config yet)

**Step 3: implement the change**

replace `read_file` in `agent_tools.rs`:

```rust
/// Read a file, validated against `file_tools_allowed_paths`.
fn read_file(path: &str, config: &ResolvedConfig) -> io::Result<String> {
    let validated = super::security::validate_file_path(path, config)?;
    std::fs::read_to_string(&validated)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to read '{}': {}", path, e)))
}
```

update the call in `retrieve_content` (line 272) from `read_file(source)?` to `read_file(source, config)?`.

update existing `read_file` tests to pass a config:

```rust
#[test]
fn test_read_file_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "hello world").unwrap();
    let mut config = make_test_config();
    config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
    let result = read_file(path.to_str().unwrap(), &config).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn test_read_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = make_test_config();
    config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
    let result = read_file(
        &format!("{}/nonexistent.txt", dir.path().display()),
        &config,
    );
    assert!(result.is_err());
}

#[test]
fn test_read_file_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.txt");
    std::fs::write(&path, "").unwrap();
    let mut config = make_test_config();
    config.file_tools_allowed_paths = vec![dir.path().to_string_lossy().to_string()];
    let result = read_file(path.to_str().unwrap(), &config).unwrap();
    assert_eq!(result, "");
}
```

**Step 4: run all agent_tools tests**

Run: `cargo test -p chibi-core agent_tools::tests`
Expected: all PASS

**Step 5: commit**

```bash
git add crates/chibi-core/src/tools/agent_tools.rs
git commit -m "fix(security): validate file paths in retrieve_content against allowlist"
```

---

### Task 4: add `classify_url` to `security.rs`

**Files:**
- Modify: `crates/chibi-core/src/tools/security.rs` (add `UrlSafety`, `classify_url`)

**Step 1: write the failing tests**

add to `security::tests`:

```rust
// === classify_url ===

#[test]
fn test_classify_url_public_safe() {
    assert!(matches!(classify_url("https://example.com"), UrlSafety::Safe));
    assert!(matches!(classify_url("https://api.github.com/repos"), UrlSafety::Safe));
    assert!(matches!(classify_url("http://1.2.3.4/path"), UrlSafety::Safe));
}

#[test]
fn test_classify_url_localhost_sensitive() {
    match classify_url("http://localhost:8080/admin") {
        UrlSafety::Sensitive(reason) => assert!(reason.contains("loopback"), "reason: {}", reason),
        UrlSafety::Safe => panic!("localhost should be sensitive"),
    }
}

#[test]
fn test_classify_url_loopback_ip_sensitive() {
    match classify_url("http://127.0.0.1:9200/_cat/indices") {
        UrlSafety::Sensitive(reason) => assert!(reason.contains("loopback"), "reason: {}", reason),
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
            UrlSafety::Sensitive(reason) => assert!(reason.contains("private"), "url: {}, reason: {}", url, reason),
            UrlSafety::Safe => panic!("{} should be sensitive", url),
        }
    }
}

#[test]
fn test_classify_url_link_local_sensitive() {
    match classify_url("http://169.254.169.254/latest/meta-data/") {
        UrlSafety::Sensitive(reason) => assert!(reason.contains("metadata") || reason.contains("link-local"), "reason: {}", reason),
        UrlSafety::Safe => panic!("metadata endpoint should be sensitive"),
    }
}

#[test]
fn test_classify_url_ipv6_loopback_sensitive() {
    match classify_url("http://[::1]:3000/") {
        UrlSafety::Sensitive(reason) => assert!(reason.contains("loopback"), "reason: {}", reason),
        UrlSafety::Safe => panic!("::1 should be sensitive"),
    }
}

#[test]
fn test_classify_url_invalid() {
    // invalid URLs should be treated as sensitive (fail-safe)
    match classify_url("not-a-url") {
        UrlSafety::Sensitive(reason) => assert!(reason.contains("parse") || reason.contains("invalid"), "reason: {}", reason),
        UrlSafety::Safe => panic!("invalid URL should be sensitive"),
    }
}
```

**Step 2: run tests to verify they fail**

Run: `cargo test -p chibi-core security::tests::test_classify`
Expected: FAIL (function doesn't exist)

**Step 3: implement `classify_url`**

add to `security.rs`:

```rust
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return UrlSafety::Sensitive("URL has no host".to_string()),
    };

    // Check hostname directly for "localhost"
    if host.eq_ignore_ascii_case("localhost") {
        return UrlSafety::Sensitive("loopback address (localhost)".to_string());
    }

    // Try parsing as IP address
    if let Ok(ip) = host.parse::<IpAddr>() {
        return classify_ip(ip);
    }

    // For bracketed IPv6 like [::1], url crate already strips brackets
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        return classify_ip(IpAddr::V6(ip));
    }

    // Regular hostname — safe (DNS resolution happens at fetch time, not our concern)
    UrlSafety::Safe
}

/// Classify an IP address as safe or sensitive.
fn classify_ip(ip: IpAddr) -> UrlSafety {
    match ip {
        IpAddr::V4(v4) => classify_ipv4(v4),
        IpAddr::V6(v6) => classify_ipv6(v6),
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
```

note: `url` crate is already a transitive dep via reqwest. add it as a direct dep if needed:

Run: `grep '^url' crates/chibi-core/Cargo.toml` — if missing, add `url = "2"` to `[dependencies]`.

**Step 4: run tests**

Run: `cargo test -p chibi-core security::tests`
Expected: all PASS

**Step 5: commit**

```bash
git add crates/chibi-core/src/tools/security.rs crates/chibi-core/Cargo.toml
git commit -m "feat(security): add classify_url for SSRF protection"
```

---

### Task 5: add `PreFetchUrl` hook point

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs:43` (add variant)
- Modify: `crates/chibi-core/src/tools/hooks.rs:122` (fix count comment)
- Modify: `crates/chibi-core/src/tools/hooks.rs:152` (add to `ALL_HOOKS` test array)

**Step 1: add the variant**

in `hooks.rs`, add after `PreShellExec` (line 40):

```rust
PreFetchUrl,     // Before fetching a sensitive URL (can approve/deny, fail-safe deny)
```

**Step 2: fix the count comment**

change line 122 from `// All 26 hook points` to `// All 30 hook points`.

**Step 3: add to `ALL_HOOKS` test array**

add after the `pre_shell_exec` entry (line 149):

```rust
("pre_fetch_url", HookPoint::PreFetchUrl),
```

**Step 4: run hook tests**

Run: `cargo test -p chibi-core hooks::tests`
Expected: all PASS (strum derives handle serialization automatically)

**Step 5: commit**

```bash
git add crates/chibi-core/src/tools/hooks.rs
git commit -m "feat(hooks): add PreFetchUrl hook point (30 total)"
```

---

### Task 6: wire URL permission check into `send.rs`

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:934-938` (add permission check for retrieve_content)
- Modify: `crates/chibi-core/src/tools/mod.rs` (re-export `UrlSafety`, `classify_url`)

**Step 1: add re-exports to `tools/mod.rs`**

add after the agent tool re-exports:

```rust
// Re-export security utilities
pub use security::{UrlSafety, classify_url, validate_file_path};
```

**Step 2: modify agent tool dispatch in `execute_tool_pure`**

replace the agent tool dispatch block (lines 934-938) with:

```rust
    } else if tools::is_agent_tool(&tool_call.name) {
        // URL permission check for retrieve_content with sensitive URLs
        if tool_call.name == tools::RETRIEVE_CONTENT_TOOL_NAME {
            if let Some(source) = args.get_str("source") {
                if tools::agent_tools::is_url(source) {
                    if let tools::UrlSafety::Sensitive(reason) = tools::classify_url(source) {
                        let hook_data = serde_json::json!({
                            "tool_name": tool_call.name,
                            "url": source,
                            "safety": "sensitive",
                            "reason": reason,
                        });
                        match check_permission(
                            tools,
                            tools::HookPoint::PreFetchUrl,
                            &hook_data,
                            permission_handler,
                        )? {
                            Ok(()) => {} // approved, continue to execute
                            Err(reason) => {
                                return Ok(ToolExecutionResult {
                                    result: format!("Permission denied: {}", reason),
                                    diagnostics,
                                    handoff: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        match tools::execute_agent_tool(resolved_config, &tool_call.name, &args, tools).await {
            Ok(r) => r,
            Err(e) => format!("Error: {}", e),
        }
```

note: `is_url` needs to be made `pub` in `agent_tools.rs` (currently private). change `fn is_url` to `pub fn is_url` at line 199.

**Step 3: write tests for the permission check**

add to `send.rs` tests:

```rust
#[test]
fn test_classify_url_integration_with_permission() {
    // Sensitive URL + no permission handler = fail-safe deny
    let hook_data = json!({
        "tool_name": "retrieve_content",
        "url": "http://localhost:8080",
        "safety": "sensitive",
        "reason": "loopback address (localhost)",
    });
    let result = evaluate_permission(&[], &hook_data, None).unwrap();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("fail-safe deny"));
}

#[test]
fn test_classify_url_handler_approves_sensitive() {
    let hook_data = json!({
        "tool_name": "retrieve_content",
        "url": "http://localhost:8080",
        "safety": "sensitive",
        "reason": "loopback address (localhost)",
    });
    let handler: PermissionHandler = Box::new(|_| Ok(true));
    let result = evaluate_permission(&[], &hook_data, Some(&handler)).unwrap();
    assert_eq!(result, Ok(()));
}
```

**Step 4: run tests**

Run: `cargo test -p chibi-core`
Expected: all PASS

**Step 5: commit**

```bash
git add crates/chibi-core/src/api/send.rs crates/chibi-core/src/tools/mod.rs crates/chibi-core/src/tools/agent_tools.rs
git commit -m "feat(security): wire PreFetchUrl permission check for sensitive URLs"
```

---

### Task 7: update docs and AGENTS.md

**Files:**
- Modify: `AGENTS.md` (add `pre_fetch_url` to hooks list, update count to 30)
- Modify: `docs/plans/2026-02-14-pre-release-audit.md` (mark item 1 as done)

**Step 1: update AGENTS.md**

in the hooks section, add `pre_fetch_url` to the hook list and change "29 hook points" to "30 hook points".

**Step 2: mark item 1 as resolved in the audit**

add `[DONE]` prefix to item 1 title.

**Step 3: commit**

```bash
git add AGENTS.md docs/plans/2026-02-14-pre-release-audit.md
git commit -m "docs: update hook count to 30, mark audit item 1 done"
```

---

### Task 8: full test suite + build verification

**Step 1: run full test suite**

Run: `cargo test`
Expected: all PASS across all 3 crates

**Step 2: run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

**Step 3: build both binaries**

Run: `cargo build`
Expected: clean build
