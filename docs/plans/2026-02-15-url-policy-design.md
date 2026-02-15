# chibi-json: configurable URL security policy (#147) ✅ implemented

## context

`classify_url()` classifies URLs as safe or sensitive (loopback, private network,
link-local, cloud metadata, unparseable). chibi-cli prompts interactively for
sensitive URLs; chibi-json auto-approves everything (trust mode).

with containerisation, a composer configures chibi-json in an isolated container
with plugins/config for a specific purpose. spawned sub-agents can be further
constrained via per-invocation JSON overrides. both config surfaces are equally
important.

## design

### data model

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UrlCategory {
    Loopback,
    PrivateNetwork,
    LinkLocal,
    CloudMetadata,
    Unparseable,
}

/// a rule entry — either a preset category or a URL glob pattern.
/// preset syntax: `"preset:category_name"`
/// glob syntax: standard `*`/`?` wildcards, `\*` for literal asterisk.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum UrlRule {
    Preset { preset: UrlCategory },
    Pattern(String),
}

/// URL security policy with two-tier override semantics.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct UrlPolicy {
    #[serde(default = "default_allow")]
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
```

### evaluation order

first match wins, highest priority first:

1. `deny_override` — if any rule matches → **deny** (unconditional)
2. `allow_override` — if any rule matches → **allow** (unconditional, except deny_override)
3. `deny` — if any rule matches → **deny**
4. `allow` — if any rule matches → **allow**
5. `default` — fallback action

### rule matching

- **preset rules** (`preset:loopback` etc.) — compare against `UrlCategory` derived
  from `classify_url()`. operates on parsed IP addresses, so all IP representation
  tricks (octal, hex, decimal, `127.1`, IPv4-mapped IPv6, etc.) are caught.
- **glob patterns** — match against the URL string as-written. `*` matches any
  sequence, `?` matches one character, `\*` matches literal asterisk. intended for
  domain-name URLs where representation is stable.

**important:** use preset categories to control IP-based access. URL glob patterns
match against the URL as-written and do not normalize IP representations.

### URL canonicalization

before policy evaluation:

1. parse with `url::Url::parse()` (normalizes scheme/host to lowercase, resolves
   `..` path segments)
2. percent-decode the host (catches `%31%32%37.%30.%30.%31` → `127.0.0.1`)
3. resolve punycode IDN domains to unicode (via `idna`, already a transitive dep
   of `url`) — glob matching operates on the decoded form

`classify_url()` operates on parsed `Ipv4Addr`/`Ipv6Addr` which are numeric — all
IP representation variants resolve to the same parsed address. this is the primary
defence against IP-based SSRF bypasses.

### refactoring `UrlSafety`

```rust
pub enum UrlSafety {
    Safe,
    Sensitive(UrlCategory),  // was: Sensitive(String)
}
```

`UrlCategory` becomes the single source of truth. human-readable reason strings
become a `Display` impl on `UrlCategory`.

### config resolution

the policy lives at three levels:

- **`Config`** (`~/.chibi/config.toml`): `url_policy: Option<UrlPolicy>`
- **`LocalConfig`** (per-context `local.toml`): `url_policy: Option<UrlPolicy>`
- **`JsonInput`** (per-invocation): `url_policy: Option<UrlPolicy>`

resolution: `JsonInput` > `LocalConfig` > `Config` > `None`

**whole-object override, not merge.** if a higher layer provides `url_policy`, it
completely replaces the lower layer's policy. merging rule lists across layers
would be surprising and hard to debug.

`None` means "no policy configured" → fall through to existing permission handler
(interactive prompt in CLI, trust-mode in JSON). this preserves full backwards
compatibility.

### integration with send.rs

```
if tool is retrieve_content && url is_url:
    let safety = classify_url(canonical_url)
    if let Some(policy) = &resolved_config.url_policy:
        match evaluate_url_policy(canonical_url, safety, policy):
            Allow → proceed
            Deny → return permission denied
    else:
        // no policy — existing behaviour
        if safety is Sensitive:
            check_permission(permission_handler)
```

policy is authoritative when present. permission handler is the fallback.

### config examples

TOML (container baseline — deny everything, allow public internet):
```toml
[url_policy]
default = "deny"
allow = ["https://*", "http://*"]
deny_override = ["preset:cloud_metadata"]
```

TOML (allow private network but never cloud metadata):
```toml
[url_policy]
default = "deny"
allow = ["preset:private_network", "preset:loopback"]
deny_override = ["preset:cloud_metadata"]
```

JSON input (parent agent constraining child):
```json
{
  "url_policy": {
    "default": "deny",
    "allow": ["preset:loopback"],
    "deny_override": ["preset:cloud_metadata"]
  }
}
```

## changes

| file | change |
|------|--------|
| `chibi-core/src/tools/security.rs` | `UrlCategory`, `UrlPolicy`, `UrlRule`, `UrlAction` types. refactor `UrlSafety::Sensitive(String)` → `Sensitive(UrlCategory)`. `evaluate_url_policy()`, canonicalization, glob matching. `Display` for `UrlCategory`. |
| `chibi-core/src/config.rs` | `url_policy: Option<UrlPolicy>` on `Config`, `LocalConfig`, `ResolvedConfig`. whole-object resolution. |
| `chibi-core/src/api/send.rs` | policy evaluation before permission handler fallback. update `Sensitive` destructuring. |
| `chibi-json/src/input.rs` | `url_policy: Option<UrlPolicy>` on `JsonInput`. |
| `chibi-json/src/main.rs` | pass `JsonInput.url_policy` into resolved config. |

## testing

- **evaluation order:** deny_override > allow_override > deny > allow > default
- **category matching:** all `UrlCategory` variants against `classify_url()` output
- **glob matching:** wildcards, literal `\*`, prefix, suffix patterns
- **canonicalization:** percent-encoded hostnames, punycode domains
- **IP tricks:** verify categories catch all representations (mostly existing tests)
- **config resolution:** per-invocation overrides config, whole-object replacement
- **TOML/JSON round-trip:** serialize/deserialize, invalid preset → clear error
- **`None` policy:** falls through to permission handler (backwards compat)

## out of scope

- network-interface-level filtering
- DNS rebinding protection
- full HTTP client replacement (agent-fetch territory)

## new dependencies

none — `url` and `idna` already in the dependency tree.
