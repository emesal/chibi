# URL Security Policy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** configurable URL security policy for chibi-json (and chibi-core generally) with two-tier allow/deny rules, preset categories, and glob patterns (#147)

**Architecture:** new types in `security.rs`, config fields at three layers (global/local/per-invocation), policy evaluation intercepts the existing permission check in `send.rs`

**Tech Stack:** rust, serde, schemars, `url` crate (already a dep)

---

### Task 1: add `UrlCategory` enum and refactor `UrlSafety`

**Files:**
- Modify: `crates/chibi-core/src/tools/security.rs:77-136`
- Modify: `crates/chibi-core/src/tools/mod.rs:79` (re-export)

**Step 1: write failing tests**

Add to the `tests` module in `security.rs`, after the existing `classify_url` tests:

```rust
// === UrlCategory ===

#[test]
fn test_url_category_display() {
    assert_eq!(UrlCategory::Loopback.to_string(), "loopback address");
    assert_eq!(UrlCategory::PrivateNetwork.to_string(), "private network address");
    assert_eq!(UrlCategory::LinkLocal.to_string(), "link-local address");
    assert_eq!(UrlCategory::CloudMetadata.to_string(), "cloud metadata endpoint");
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
```

**Step 2: run tests to verify they fail**

Run: `cargo test -p chibi-core -- test_url_category`
Expected: FAIL — `UrlCategory` doesn't exist yet

**Step 3: implement `UrlCategory` and refactor `UrlSafety`**

In `security.rs`, add above the existing `UrlSafety` enum:

```rust
use std::fmt;

/// built-in URL categories for policy matching.
///
/// maps 1:1 onto the classification in `classify_url()`. the `Display` impl
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
```

Refactor `UrlSafety::Sensitive` from `String` to `UrlCategory`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlSafety {
    Safe,
    Sensitive(UrlCategory),
}
```

Update `classify_url`, `classify_ipv4`, `classify_ipv6`:

```rust
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
```

Update `mod.rs:79` re-export to include `UrlCategory`:

```rust
pub use security::{UrlCategory, UrlSafety, classify_url, validate_file_path};
```

**Step 4: fix existing tests**

The existing `classify_url` tests use `Sensitive(reason)` where `reason` is a `String`. Update them to match on `UrlCategory` instead (or use `.to_string()` for the reason assertions). The new tests from step 1 are the canonical tests; the old ones should be updated to use the new enum.

For example, `test_classify_url_localhost_sensitive`:
```rust
#[test]
fn test_classify_url_localhost_sensitive() {
    assert_eq!(
        classify_url("http://localhost:8080/admin"),
        UrlSafety::Sensitive(UrlCategory::Loopback)
    );
}
```

**Step 5: fix `send.rs` callsite**

In `crates/chibi-core/src/api/send.rs:870`, update the destructure:

```rust
&& let tools::UrlSafety::Sensitive(category) = tools::classify_url(source)
```

And update the hook data:
```rust
let hook_data = json!({
    "tool_name": tool_call.name,
    "url": source,
    "safety": "sensitive",
    "reason": category.to_string(),
});
```

**Step 6: run all tests**

Run: `cargo test -p chibi-core`
Expected: all PASS

**Step 7: commit**

```
feat: add UrlCategory enum, refactor UrlSafety (#147)

single source of truth for URL classification categories.
Sensitive(String) → Sensitive(UrlCategory) with Display impl.
```

---

### Task 2: add `UrlPolicy` types and evaluation logic

**Files:**
- Modify: `crates/chibi-core/src/tools/security.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs` (re-exports)

**Step 1: write failing tests**

Add to `security.rs` tests module:

```rust
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
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Allow,
    );
}

#[test]
fn test_policy_default_deny() {
    let policy = make_policy(UrlAction::Deny);
    assert_eq!(
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Deny,
    );
}

#[test]
fn test_policy_allow_category() {
    let mut policy = make_policy(UrlAction::Deny);
    policy.allow.push(UrlRule::Preset(UrlCategory::Loopback));
    assert_eq!(
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Allow,
    );
    // private network still denied
    assert_eq!(
        evaluate_url_policy("http://192.168.1.1/", &classify_url("http://192.168.1.1/"), &policy),
        UrlAction::Deny,
    );
}

#[test]
fn test_policy_deny_category() {
    let mut policy = make_policy(UrlAction::Allow);
    policy.deny.push(UrlRule::Preset(UrlCategory::CloudMetadata));
    assert_eq!(
        evaluate_url_policy(
            "http://169.254.169.254/",
            &classify_url("http://169.254.169.254/"),
            &policy,
        ),
        UrlAction::Deny,
    );
    // loopback still allowed
    assert_eq!(
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Allow,
    );
}

#[test]
fn test_policy_deny_override_beats_allow_override() {
    let mut policy = make_policy(UrlAction::Allow);
    policy.allow_override.push(UrlRule::Preset(UrlCategory::CloudMetadata));
    policy.deny_override.push(UrlRule::Preset(UrlCategory::CloudMetadata));
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
    policy.allow_override.push(UrlRule::Preset(UrlCategory::Loopback));
    assert_eq!(
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Allow,
    );
}

#[test]
fn test_policy_deny_beats_allow() {
    let mut policy = make_policy(UrlAction::Allow);
    policy.allow.push(UrlRule::Preset(UrlCategory::Loopback));
    policy.deny.push(UrlRule::Preset(UrlCategory::Loopback));
    assert_eq!(
        evaluate_url_policy("http://localhost/", &classify_url("http://localhost/"), &policy),
        UrlAction::Deny,
    );
}

#[test]
fn test_policy_glob_pattern() {
    let mut policy = make_policy(UrlAction::Deny);
    policy.allow.push(UrlRule::Pattern("https://api.example.com/*".to_string()));
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
    // match literal asterisk with backslash escape
    policy.allow.push(UrlRule::Pattern("https://example.com/\\*".to_string()));
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
    // safe (public) URLs are also subject to policy
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
    // deny_override > allow_override > deny > allow > default
    let mut policy = make_policy(UrlAction::Allow);
    policy.allow.push(UrlRule::Preset(UrlCategory::PrivateNetwork));
    policy.deny.push(UrlRule::Preset(UrlCategory::PrivateNetwork));
    // deny beats allow at same tier
    assert_eq!(
        evaluate_url_policy("http://10.0.0.1/", &classify_url("http://10.0.0.1/"), &policy),
        UrlAction::Deny,
    );
    // but allow_override beats deny
    policy.allow_override.push(UrlRule::Preset(UrlCategory::PrivateNetwork));
    assert_eq!(
        evaluate_url_policy("http://10.0.0.1/", &classify_url("http://10.0.0.1/"), &policy),
        UrlAction::Allow,
    );
    // but deny_override beats allow_override
    policy.deny_override.push(UrlRule::Preset(UrlCategory::PrivateNetwork));
    assert_eq!(
        evaluate_url_policy("http://10.0.0.1/", &classify_url("http://10.0.0.1/"), &policy),
        UrlAction::Deny,
    );
}
```

**Step 2: run tests to verify they fail**

Run: `cargo test -p chibi-core -- test_policy`
Expected: FAIL — types and function don't exist

**Step 3: implement types and evaluation**

Add to `security.rs` (below the `UrlCategory` and `UrlSafety` types):

```rust
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

/// action taken by a URL policy rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlAction {
    Allow,
    Deny,
}

/// a single rule entry — preset category or URL glob pattern.
///
/// in config/JSON, presets use `"preset:category_name"` syntax,
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
            UrlRule::Preset(cat) => format!("preset:{}", serde_json::to_string(&cat)
                .unwrap().trim_matches('"')),
            UrlRule::Pattern(pat) => pat,
        }
    }
}

/// URL security policy with two-tier allow/deny override semantics.
///
/// evaluation order (first match wins, highest priority first):
/// 1. `deny_override`  — unconditional deny
/// 2. `allow_override` — unconditional allow (except deny_override)
/// 3. `deny`           — standard deny
/// 4. `allow`          — standard allow
/// 5. `default`        — fallback
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UrlPolicy {
    /// fallback action when no rule matches (default: allow for backwards compat)
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

/// evaluate a URL against a policy. returns the action to take.
///
/// `url` is the original URL string (for glob matching).
/// `safety` is the result of `classify_url()` (for category matching).
pub fn evaluate_url_policy(url: &str, safety: &UrlSafety, policy: &UrlPolicy) -> UrlAction {
    let category = match safety {
        UrlSafety::Sensitive(cat) => Some(cat),
        UrlSafety::Safe => None,
    };

    // deny_override (highest priority)
    if rule_matches(&policy.deny_override, category, url) {
        return UrlAction::Deny;
    }
    // allow_override
    if rule_matches(&policy.allow_override, category, url) {
        return UrlAction::Allow;
    }
    // deny
    if rule_matches(&policy.deny, category, url) {
        return UrlAction::Deny;
    }
    // allow
    if rule_matches(&policy.allow, category, url) {
        return UrlAction::Allow;
    }
    // default
    policy.default
}

/// check if any rule in the list matches the given URL.
fn rule_matches(rules: &[UrlRule], category: Option<&UrlCategory>, url: &str) -> bool {
    rules.iter().any(|rule| match rule {
        UrlRule::Preset(cat) => category == Some(cat),
        UrlRule::Pattern(pattern) => glob_match(pattern, url),
    })
}

/// simple glob matching: `*` matches any sequence, `?` matches one char,
/// `\*` matches literal `*`, `\?` matches literal `?`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt)
}

fn glob_match_inner(pat: &[char], txt: &[char]) -> bool {
    match (pat.first(), txt.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            // try matching zero chars, or skip one char in text
            glob_match_inner(&pat[1..], txt)
                || (!txt.is_empty() && glob_match_inner(pat, &txt[1..]))
        }
        (Some('?'), Some(_)) => glob_match_inner(&pat[1..], &txt[1..]),
        (Some('\\'), _) => {
            // escaped: next char in pattern matches literally
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
```

Update `mod.rs` re-exports:

```rust
pub use security::{
    UrlAction, UrlCategory, UrlPolicy, UrlRule, UrlSafety,
    classify_url, evaluate_url_policy, validate_file_path,
};
```

**Step 4: run tests**

Run: `cargo test -p chibi-core -- test_policy`
Expected: all PASS

**Step 5: commit**

```
feat: add UrlPolicy types and evaluation logic (#147)

two-tier allow/deny with override semantics. rules are either
preset categories or URL glob patterns. first match wins.
```

---

### Task 3: add `url_policy` to config structs

**Files:**
- Modify: `crates/chibi-core/src/config.rs:514-747`
- Modify: `crates/chibi-core/src/state/config_resolution.rs:75-110`

**Step 1: write failing test**

Add to config tests (or `security.rs` tests):

```rust
#[test]
fn test_url_policy_toml_roundtrip() {
    let toml_str = r#"
[url_policy]
default = "deny"
allow = ["preset:loopback", "https://api.example.com/*"]
deny_override = ["preset:cloud_metadata"]
"#;
    // parse as Config and check the field is populated
    let config: Config = toml::from_str(toml_str).expect("valid toml");
    let policy = config.url_policy.expect("policy should be present");
    assert_eq!(policy.default, UrlAction::Deny);
    assert_eq!(policy.allow.len(), 2);
    assert_eq!(policy.deny_override.len(), 1);
}
```

**Step 2: run test to verify it fails**

Run: `cargo test -p chibi-core -- test_url_policy_toml`
Expected: FAIL — field doesn't exist on `Config`

**Step 3: add field to config structs**

In `config.rs`, add to `Config` (after `file_tools_allowed_paths`):

```rust
    /// URL security policy for sensitive URL handling
    #[serde(default)]
    pub url_policy: Option<UrlPolicy>,
```

Add to `LocalConfig` (after `file_tools_allowed_paths`):

```rust
    /// URL security policy override
    pub url_policy: Option<UrlPolicy>,
```

Add to `ResolvedConfig` (after `file_tools_allowed_paths`):

```rust
    /// URL security policy (None = use permission handler fallback)
    pub url_policy: Option<UrlPolicy>,
```

In `config_resolution.rs` `resolve_config()`, add to the `ResolvedConfig` struct literal:

```rust
    url_policy: self.config.url_policy.clone(),
```

In `LocalConfig::apply_overrides()`, add `url_policy` to the `apply_option_overrides!` macro invocation. Note: since `url_policy` is `Option<UrlPolicy>` on both sides (not `Option<T>` → `T`), it needs the same special handling as `api_key`. Add after the macro call:

```rust
    // url_policy: whole-object override (not merge)
    if self.url_policy.is_some() {
        resolved.url_policy = self.url_policy.clone();
    }
```

Add to `ResolvedConfig::get_field()` and `ResolvedConfig::list_fields()` as needed (check patterns for other `Option` fields).

Add necessary `use` for the `UrlPolicy` type in `config.rs`.

**Step 4: run tests**

Run: `cargo test -p chibi-core`
Expected: all PASS

**Step 5: commit**

```
feat: add url_policy to Config/LocalConfig/ResolvedConfig (#147)

whole-object override semantics: per-context replaces global,
per-invocation replaces per-context. None = use permission handler.
```

---

### Task 4: add `url_policy` to `JsonInput` and wire it through

**Files:**
- Modify: `crates/chibi-json/src/input.rs:6-28`
- Modify: `crates/chibi-json/src/main.rs:76-80`

**Step 1: add field to `JsonInput`**

In `input.rs`, add after `project_root`:

```rust
    /// URL security policy override (replaces config-level policy)
    #[serde(default)]
    pub url_policy: Option<chibi_core::tools::UrlPolicy>,
```

(check the actual import path — might need `use chibi_core::tools::UrlPolicy;` at top)

**Step 2: wire it into resolved config**

In `main.rs`, after `let resolved = chibi.resolve_config(...)`:

```rust
    // Apply per-invocation URL policy override (highest priority, whole-object)
    let mut resolved = resolved;
    if json_input.url_policy.is_some() {
        resolved.url_policy = json_input.url_policy.clone();
    }
```

**Step 3: verify JSON schema includes the field**

Run: `cargo run -p chibi-json -- --json-schema 2>/dev/null | jq '.properties.url_policy'`
Expected: non-null schema object

**Step 4: run tests**

Run: `cargo test`
Expected: all PASS

**Step 5: commit**

```
feat: add url_policy to JsonInput (#147)

per-invocation override lets parent agents constrain child URL
access via JSON pipe. whole-object replacement, not merge.
```

---

### Task 5: integrate policy evaluation into `send.rs`

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:866-896`

**Step 1: write failing test**

This is an integration-level change. Add a unit test in `security.rs` that
demonstrates the expected flow (policy takes precedence):

```rust
#[test]
fn test_policy_overrides_sensitive_classification() {
    // a sensitive URL with an allow policy should be allowed
    let url = "http://localhost:3000/api";
    let safety = classify_url(url);
    assert!(matches!(safety, UrlSafety::Sensitive(UrlCategory::Loopback)));

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
```

**Step 2: refactor the permission check block**

Replace the existing block at `send.rs:866-896` with:

```rust
        // URL policy / permission check for retrieve_content
        if tool_call.name == tools::RETRIEVE_CONTENT_TOOL_NAME
            && let Some(source) = args.get_str("source")
            && tools::agent_tools::is_url(source)
        {
            let safety = tools::classify_url(source);

            if let Some(ref policy) = resolved_config.url_policy {
                // policy is authoritative — no fallback to permission handler
                if tools::evaluate_url_policy(source, &safety, policy) == tools::UrlAction::Deny {
                    let reason = match &safety {
                        tools::UrlSafety::Sensitive(cat) => cat.to_string(),
                        tools::UrlSafety::Safe => "denied by URL policy".to_string(),
                    };
                    let msg = format!("Permission denied: {}", reason);
                    return Ok(ToolExecutionResult {
                        final_result: msg.clone(),
                        original_result: msg,
                        was_cached: false,
                        diagnostics,
                    });
                }
            } else if let tools::UrlSafety::Sensitive(category) = &safety {
                // no policy — existing behaviour: check permission handler
                let hook_data = json!({
                    "tool_name": tool_call.name,
                    "url": source,
                    "safety": "sensitive",
                    "reason": category.to_string(),
                });
                match check_permission(
                    tools,
                    tools::HookPoint::PreFetchUrl,
                    &hook_data,
                    permission_handler,
                )? {
                    Ok(()) => {}
                    Err(reason) => {
                        let msg = format!("Permission denied: {}", reason);
                        return Ok(ToolExecutionResult {
                            final_result: msg.clone(),
                            original_result: msg,
                            was_cached: false,
                            diagnostics,
                        });
                    }
                }
            }
        }
```

**Step 3: run all tests**

Run: `cargo test`
Expected: all PASS

**Step 4: commit**

```
feat: integrate URL policy evaluation into agentic loop (#147)

policy is authoritative when present. no policy = fall through to
existing permission handler (backwards compatible).
```

---

### Task 6: URL canonicalization

**Files:**
- Modify: `crates/chibi-core/src/tools/security.rs`

**Step 1: write failing tests**

```rust
#[test]
fn test_canonicalize_percent_encoded_host() {
    // %6c%6f%63%61%6c%68%6f%73%74 = "localhost"
    let url = "http://%6c%6f%63%61%6c%68%6f%73%74/path";
    let safety = classify_url(url);
    // after canonicalization, should be classified as loopback
    assert!(matches!(safety, UrlSafety::Sensitive(UrlCategory::Loopback)),
        "percent-encoded localhost should be classified as loopback, got: {:?}", safety);
}

#[test]
fn test_policy_glob_matches_decoded_punycode() {
    // this test verifies glob matching works on normalized hostnames.
    // url::Url::parse already lowercases hosts, so basic domain matching
    // should work. punycode IDN domains are a stretch — add if needed.
    let mut policy = make_policy(UrlAction::Deny);
    policy.allow.push(UrlRule::Pattern("https://example.com/*".to_string()));
    let url = "https://EXAMPLE.COM/path";
    assert_eq!(
        evaluate_url_policy(
            &url.to_lowercase(), // caller should normalize
            &classify_url(url),
            &policy,
        ),
        UrlAction::Allow,
    );
}
```

**Step 2: verify they fail (or pass — `url::Url::parse` may already handle these)**

Run: `cargo test -p chibi-core -- test_canonicalize test_policy_glob_matches`

Check results. `url::Url::parse` may already percent-decode the host. If tests pass, great — document that the `url` crate handles it. If they fail, add canonicalization in `classify_url()`.

**Step 3: add canonicalization if needed**

If `classify_url` doesn't catch percent-encoded hosts, add percent-decoding of the host string before classification. The `url` crate's `Url::host_str()` returns the decoded host, but check the actual behaviour.

Add a `canonicalize_url()` helper that:
1. parses with `url::Url::parse()`
2. lowercases
3. returns the canonical string for glob matching

Call this from `evaluate_url_policy()` for the glob matching path.

**Step 4: run tests**

Run: `cargo test -p chibi-core`
Expected: all PASS

**Step 5: commit**

```
feat: URL canonicalization for policy evaluation (#147)

percent-decode hosts, lowercase for glob matching.
category matching already safe via parsed IP addresses.
```

---

### Task 7: serde round-trip tests and error handling

**Files:**
- Modify: `crates/chibi-core/src/tools/security.rs` (tests)

**Step 1: write tests**

```rust
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
```

**Step 2: run tests**

Run: `cargo test -p chibi-core -- test_url_rule_serde test_url_policy_serde test_url_policy_toml`
Expected: all PASS (these should pass if task 2 was implemented correctly)

**Step 3: commit**

```
test: URL policy serde round-trips and error cases (#147)
```

---

### Task 8: verify full build, all tests, `cargo clippy`

**Step 1: full build**

Run: `cargo build 2>&1`
Expected: no errors

**Step 2: all tests**

Run: `cargo test 2>&1`
Expected: all PASS

**Step 3: clippy**

Run: `cargo clippy -- -D warnings 2>&1`
Expected: no warnings

**Step 4: commit any fixups, then final commit if needed**

If clippy or test issues were found and fixed, commit them.

---

### Task 9: update AGENTS.md and design doc

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/plans/2026-02-15-url-policy-design.md` (mark as implemented)

**Step 1: update AGENTS.md**

Add `url_policy` to the config description in the storage layout or config section if it documents config fields.

Mention URL policy in the security section if one exists, or add a brief note.

**Step 2: commit**

```
docs: update AGENTS.md for URL policy (#147)
```
