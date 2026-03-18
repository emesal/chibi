# Synthesised Tools: HTTP-Restricted Sandbox, Categories, Env Forwarding

**Issue:** #230
**Date:** 2026-03-18
**Branch:** one branch, logical commits per chunk
**Scope:** chunks 1–5 (all five from the issue, including trust-declared HTTP)

## Motivation

The gersemi Trading 212 plugin needs to run as a sandboxed synthesised tool with constrained HTTP access. tein now supports `ContextBuilder::http_allow()` for URL-prefix-restricted HTTP in sandboxed contexts (tein#145). chibi needs plumbing to connect this to synthesised tool loading, plus several related capabilities for tools that make network calls with permission gating.

## Cross-Cutting Decision: `load_tools_from_source_with_tier` Signature

The current signature is `(source, vfs_path, registry, tier)` with ~30+ call sites (mostly tests). Chunks 3, 4, and 5 all need additional data flowing into `build_tein_context`.

**Approach:** replace the `tier: SandboxTier` parameter with `tools_config: &ToolsConfig`. The function resolves tier, HTTP prefixes, and env vars internally via the vfs_path it already has. Test call sites pass `&ToolsConfig::default()` (already the convention for `scan_and_register`/`reload_tool_from_content`).

This means:
- `load_tools_from_source_with_tier` is removed. The existing `load_tools_from_source` (currently a convenience wrapper that passes `Sandboxed`) is replaced with a new version that takes `&ToolsConfig` and resolves tier + http + env internally.
- `load_tool_from_source` (singular, convenience wrapper expecting exactly one tool) delegates to the new `load_tools_from_source`. Its signature stays the same — it already doesn't take a tier param.
- `scan_zone` and `reload_tool_from_content` simplify: both now call `load_tools_from_source(source, path, registry, tools_config)` directly, which resolves tier, HTTP prefixes, and env internally. They no longer call `resolve_tier` themselves
- `build_tein_context` gains `http_prefixes: Option<Vec<String>>` and `env_vars: Option<Vec<(String, String)>>` parameters (resolved values, not config)
- `build_sandboxed_harness_context` (used by `eval.rs`) passes `None`/`None` for the new parameters — eval contexts don't get HTTP or env forwarding

## Chunk 1: Tool-Declared `category` and `summary_params`

### Problem

Synthesised tools get `ToolCategory::Synthesised` hardcoded at registration. Tools cannot declare their category or summary parameters.

### Design

**Convention format** — new optional bindings read post-eval:
```scheme
(define tool-category "network")
(define tool-summary-params '("ticker" "quantity"))
```

**`define-tool` format** — extend `syntax-rules` in `HARNESS_PREAMBLE` with additional patterns supporting optional `(category ...)` and `(summary-params ...)` clauses:

```scheme
;; pattern 1: baseline (existing)
(define-tool name (description d) (parameters p) (execute h))

;; pattern 2: category only
(define-tool name (description d) (category c) (parameters p) (execute h))

;; pattern 3: summary-params only
(define-tool name (description d) (summary-params sp) (parameters p) (execute h))

;; pattern 4: both
(define-tool name (description d) (category c) (summary-params sp) (parameters p) (execute h))
```

**`%tool-registry%` entry** grows from 4 to 6 elements:
```
(name desc params handler category-or-#f summary-params-or-#f)
```

### Changes

- **`HARNESS_PREAMBLE`** (`synthesised.rs`): four `syntax-rules` patterns. Patterns 1/3 store `#f` for category. Patterns 1/2 store `#f` for summary-params.
- **`extract_multi_tools`** (`synthesised.rs`): update the `f.len() >= 4` check to `f.len() >= 6` (all patterns now produce 6-element entries). Read fields at index 4 (category) and 5 (summary-params) from each entry. Map category string → `ToolCategory` variant; `#f` or unknown → `ToolCategory::Synthesised`.
- **`extract_single_tool`** (`synthesised.rs`): check for `tool-category` and `tool-summary-params` bindings post-eval. Same mapping logic.
- **Category string mapping**: `"network"` → `Network`, `"fs_read"` → `FsRead`, `"fs_write"` → `FsWrite`, `"shell"` → `Shell`, `"memory"` → `Memory`, `"flow"` → `Flow`, `"vfs"` → `Vfs`, `"index"` → `Index`, `"eval"` → `Eval`. Unknown → `Synthesised`.

### Tests

- Convention format: tool with `(define tool-category "network")` → `ToolCategory::Network`.
- Convention format: tool with `(define tool-summary-params '("a" "b"))` → `summary_params == vec!["a", "b"]`.
- `define-tool` with `(category "network")` → `ToolCategory::Network`.
- `define-tool` with `(summary-params '("x"))` → correct summary_params.
- `define-tool` with both category and summary-params.
- Unknown category string → `ToolCategory::Synthesised`.
- Missing category/summary-params → defaults (`Synthesised`, empty vec).
- Multi-tool file where different tools have different categories.

## Chunk 2: Network Category No-URL Fallback in Permission Prompt

### Problem

`ToolCategory::Network` tools without a `url` parameter produce a confusing permission prompt. `classify_url("")` returns `Sensitive(Unparseable)`.

### Design

In the `ToolCategory::Network` match arm in `send.rs`: when `url` arg is missing or empty, fall back to `tool_call_summary(registry, tool_name, args_json)` (existing function in `mod.rs`) to construct a human-readable display text, then fire `check_permission` with `PreFetchUrl` hook data carrying the tool name + summary instead of URL + safety.

The prompt becomes:
```
[t212_place_market_order] market buy 10x AAPL_US_EQ [Y/n]
```

### Changes

- **`send.rs`** (`ToolCategory::Network` arm): check if `url` is empty/missing. If so, build summary via `tool_call_summary` (existing function in `mod.rs`), construct hook data with `tool_name` and `summary` fields, fire `check_permission` with `PreFetchUrl`.
- The hook data shape for no-URL network tools:
  ```json
  {
    "tool_name": "t212_place_market_order",
    "summary": "market buy 10x AAPL_US_EQ",
    "safety": "no_url"
  }
  ```
  Note: `"safety": "no_url"` is a new classification value. Existing `PreFetchUrl` hook consumers expect `"sensitive"` with a `"reason"` field. Consumers should treat unknown safety values as requiring permission (fail-safe). The `HOOK_METADATA` entry for `PreFetchUrl` should document this new variant.

### Tests

- Network-categorised tool with no `url` param → permission prompt fires with summary text.
- Network-categorised tool with `url` param → existing behaviour unchanged.

## Chunk 3: HTTP-Restricted Sandbox for Synthesised Tools

### Problem

Sandboxed synthesised tools cannot make HTTP requests. tein supports `ContextBuilder::http_allow()` but chibi has no plumbing to connect it.

### Design

**tein dep:** enable `http` feature in `crates/chibi-core/Cargo.toml`:
```toml
tein = { ..., features = ["json", "regex", "http"] }
```

**Config:**
```toml
[tools.http.allow]
"/tools/shared/trading212.scm" = [
    "https://demo.trading212.com/",
    "https://live.trading212.com/",
]
```

Longest-prefix match, same pattern as `[tools.tiers]`.

### Changes

- **`ToolsConfig`** (`config.rs`): add `http` field:
  ```rust
  pub http: Option<HttpConfig>,
  ```
  where `HttpConfig` is:
  ```rust
  #[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
  pub struct HttpConfig {
      /// Per-path HTTP prefix allowlists. Longest-prefix match on VFS path.
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub allow: Option<HashMap<String, HttpAllow>>,
      /// Global toggle for trusting tool-declared HTTP prefixes.
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub trust_declared: Option<bool>,
  }
  ```
  `HttpAllow` is an enum to support both explicit lists and the `"trust-declared"` string (chunk 5):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
  #[serde(untagged)]
  pub enum HttpAllow {
      Prefixes(Vec<String>),
      TrustDeclared(String),  // "trust-declared"
  }
  ```
  Note: `#[serde(untagged)]` means any string deserialises as `TrustDeclared`. `resolve_http_allow` must validate the string value — only `"trust-declared"` is recognised; unknown strings log a warning and are treated as no HTTP access.
- **`ToolsConfig::resolve_http_allow`** (`config.rs`): longest-prefix match on vfs_path, returns `Option<Vec<String>>`. For `HttpAllow::TrustDeclared`, returns `None` here (chunk 5 handles it).
- **`load_tools_from_source`** (renamed from `load_tools_from_source_with_tier`): resolves tier via `tools_config.resolve_tier(vfs_path)` and http prefixes via `tools_config.resolve_http_allow(vfs_path)`, passes both to `build_tein_context`.
- **`build_tein_context`** (`synthesised.rs`): new `http_prefixes: Option<Vec<String>>` param. When `Some` and tier is `Sandboxed`, calls `.http_allow(&prefixes)` on the tein context builder.
- **`scan_zone`** / **`reload_tool_from_content`**: simplified — no longer call `resolve_tier` themselves, just pass `tools_config` through.

### Tests

- Tool with `[tools.http.allow]` config → `build_tein_context` receives correct prefixes.
- Longest-prefix match works (more specific path wins).
- No config entry → `None` (no HTTP access).
- Unsandboxed tier with HTTP config → prefixes ignored (unsandboxed already has full access).

## Chunk 4: Env Var Forwarding Into Sandboxed Contexts

### Problem

Sandboxed tein contexts cannot access process environment variables. Tools needing API keys have no way to receive them.

### Design

**Config:**
```toml
[tools.env]
"/tools/shared/trading212.scm" = [
    "T212_KEY", "T212_SECRET", "T212_ENV",
]
```

### Changes

- **`ToolsConfig`** (`config.rs`): add field:
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub env: Option<HashMap<String, Vec<String>>>,
  ```
- **`ToolsConfig::resolve_env`** (`config.rs`): longest-prefix match on vfs_path. Reads `std::env::var` for each listed name, returns `Option<Vec<(String, String)>>` (name+value pairs, skipping unset vars). `None` means no config entry for this path.
- **`load_tools_from_source`**: resolves env vars via `tools_config.resolve_env(vfs_path)`, passes to `build_tein_context`.
- **`build_tein_context`** (`synthesised.rs`): new `env_vars: Option<Vec<(String, String)>>` param. When `Some`, calls `.environment_variables()` on the tein context builder. Applied to both sandboxed and unsandboxed tiers (explicit forwarding is cleaner even when unsandboxed has ambient access).

### Tests

- Tool with `[tools.env]` config + env vars set → tein context receives them.
- Env var not set in process → silently skipped.
- No config entry → `None`, `.environment_variables()` not called.
- Longest-prefix match works.

## Chunk 5: Trust-Declared HTTP Prefixes

### Problem

Chunk 3 requires operators to manually list every HTTP prefix in config. When the operator trusts the tool author, this is redundant — the tool already knows what URLs it needs.

### Design

Tools declare their HTTP needs:
```scheme
(define tool-http-allow '("https://demo.trading212.com/" "https://live.trading212.com/"))
```

Config controls trust:
```toml
[tools.http]
trust-declared = false  # global default

[tools.http.allow]
"/tools/home/admin/" = "trust-declared"  # per-path trust
```

### Ordering: Two-Phase Load

There is a chicken-and-egg problem: `tool-http-allow` is a binding defined in the tool source, only available after `build_tein_context` evaluates it. But `build_tein_context` needs the HTTP prefixes to configure the sandbox.

**Solution: two-phase load.** When a `"trust-declared"` config entry (or global `trust_declared: true`) applies to a VFS path:

1. **Phase 1:** build the tein context *without* HTTP prefixes (same as a tool with no HTTP config). Evaluate the source. Read `tool-http-allow` binding.
2. **Phase 2:** if declared prefixes are found and trust applies, rebuild the tein context *with* the declared prefixes and re-evaluate the source.

This costs a double-eval for trust-declared tools only. Tools with explicit config prefixes (chunk 3) are unaffected — they resolve before context construction. The two-phase path only triggers when `resolve_http_allow` returns a sentinel indicating "need declared prefixes" rather than `None` or `Some(prefixes)`.

To avoid the cost when no `tool-http-allow` is declared, phase 1 checks for the binding first. If absent, no phase 2.

### Changes

- **`load_tools_from_source`** (`synthesised.rs`): after building the tein context (phase 1), reads `tool-http-allow` binding from the session. If found and `resolve_http_allow_with_declared(vfs_path, &declared)` returns `Some(prefixes)`, rebuilds the context with those prefixes (phase 2) and re-extracts tools.
- **`ToolsConfig::resolve_http_allow`** (`config.rs`): returns `HttpAllowResult` enum:
  ```rust
  pub enum HttpAllowResult {
      Prefixes(Vec<String>),   // explicit config prefixes — use directly
      NeedDeclared,            // trust-declared applies — need tool's declared prefixes
      NoAccess,                // no HTTP access
  }
  ```
- **`ToolsConfig::resolve_http_allow_with_declared`** (`config.rs`): takes `(vfs_path, declared: &[String])` → `Option<Vec<String>>`. Called in phase 2 when `NeedDeclared` and tool declares prefixes.
- **`tool-http-allow`** is per-file (not per-tool). Read as a top-level binding post-eval — works for both convention and `define-tool` formats without touching `syntax-rules`.

Resolution priority:
1. Explicit `HttpAllow::Prefixes(vec)` in config → use those (tool declarations ignored)
2. `HttpAllow::TrustDeclared` per-path → use tool-declared prefixes
3. No config entry + `trust_declared: true` globally → use tool-declared prefixes
4. No config entry + `trust_declared: false` (or absent) → no HTTP access

### Tests

- Tool declares `tool-http-allow`, config has `"trust-declared"` for path → declared prefixes used.
- Tool declares `tool-http-allow`, config has explicit prefixes → config wins.
- Tool declares `tool-http-allow`, no config, `trust_declared: false` → no HTTP access.
- Tool declares `tool-http-allow`, no config, `trust_declared: true` → declared prefixes used.
- Tool doesn't declare `tool-http-allow`, config has `"trust-declared"` → no HTTP access (nothing to trust).

## `ToolsConfig::merge_local` Semantics

`merge_local` explicitly constructs `ToolsConfig` with named fields. Adding `http` and `env` will break compilation unless updated.

**Design:** `http` and `env` are global-only — they control sandbox plumbing at tool load time, not per-context runtime behaviour. `merge_local` passes through the global value (ignores local):

```rust
ToolsConfig {
    // ... existing fields ...
    http: self.http.clone(),
    env: self.env.clone(),
}
```

If per-context overrides are ever needed, merge logic can be added later.

## File Change Summary

| File | Changes |
|------|---------|
| `crates/chibi-core/Cargo.toml` | enable `http` feature on tein dep |
| `crates/chibi-core/src/config.rs` | `HttpConfig`, `HttpAllow`, `HttpAllowResult` types; `http` and `env` fields on `ToolsConfig`; `resolve_http_allow`, `resolve_http_allow_with_declared`, `resolve_env` methods |
| `crates/chibi-core/src/tools/synthesised.rs` | `HARNESS_PREAMBLE` syntax-rules expansion; `extract_single_tool`/`extract_multi_tools` read category, summary_params, tool-http-allow; `build_tein_context` gains http_prefixes + env_vars params; `build_sandboxed_harness_context` passes `None`/`None`; `load_tools_from_source_with_tier` removed, `load_tools_from_source` rewritten (takes `&ToolsConfig`); `load_tool_from_source` unchanged (delegates to new `load_tools_from_source`); two-phase load for trust-declared HTTP; `scan_zone`/`reload_tool_from_content` simplified |
| `crates/chibi-core/src/api/send.rs` | `ToolCategory::Network` arm: no-URL fallback using `tool_call_summary` |
| `crates/chibi-core/src/tools/registry.rs` | `ToolCategory::from_category_str(s: &str) -> ToolCategory` — maps category strings to variants, unknown → `Synthesised` (symmetric with existing `as_str()`) |
| `crates/chibi-core/src/tools/hooks.rs` | ~15 test call sites: `load_tools_from_source_with_tier` → `load_tools_from_source` with `&ToolsConfig::default()`; update `PreFetchUrl` `HOOK_METADATA` payload_fields to document the `no_url` variant (add `summary` as optional field, note that `url`/`reason` are absent when `safety == "no_url"`) |

## Commit Plan

1. **chunk 1** — tool-declared category and summary_params
2. **chunk 2** — network category no-URL fallback (depends on chunk 1 for category)
3. **refactor** — `load_tools_from_source_with_tier` → `load_tools_from_source` (takes `&ToolsConfig`)
4. **chunk 3** — HTTP-restricted sandbox
5. **chunk 4** — env var forwarding
6. **chunk 5** — trust-declared HTTP prefixes
