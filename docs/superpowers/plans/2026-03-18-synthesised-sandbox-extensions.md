# Synthesised Sandbox Extensions Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable synthesised tools to declare categories/summary_params, restrict HTTP access via config, forward env vars, and optionally trust tool-declared HTTP prefixes.

**Architecture:** Extend `ToolsConfig` with `http` and `env` sections using the same longest-prefix-match pattern as `tiers`. Replace `load_tools_from_source_with_tier` with a unified `load_tools_from_source` that takes `&ToolsConfig` and resolves tier/http/env internally. Two-phase context build for trust-declared HTTP prefixes.

**Tech Stack:** Rust, tein (scheme interpreter), serde/toml config, `cfg(feature = "synthesised-tools")`

**Spec:** `docs/superpowers/specs/2026-03-18-synthesised-sandbox-extensions-design.md`

**Chunk numbering note:** The spec defines chunks 1–5. This plan inserts a refactor step (Chunk 3) between spec-chunks 2 and 3, and adds a final verification chunk. Mapping: plan Chunk 1–2 = spec Chunk 1–2, plan Chunk 3 = refactor (not in spec), plan Chunk 4 = spec Chunk 3, plan Chunk 5 = spec Chunk 4, plan Chunk 6 = spec Chunk 5, plan Chunk 7 = verification.

**tein API notes:** `ContextBuilder::http_allow` takes `&[&str]` (not `&[String]`). `ContextBuilder::environment_variables` takes `&[(&str, &str)]` (not `&[(String, String)]`). All plan code converts owned strings to ref slices at call sites.

---

## Chunk 1: Tool-Declared `category` and `summary_params`

### Task 1.1: `ToolCategory::from_category_str`

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

- [ ] **Step 1: Write the test**

In the `#[cfg(test)] mod tests` block at the bottom of `registry.rs`, add:

```rust
#[test]
fn test_from_category_str() {
    assert_eq!(ToolCategory::from_category_str("network"), ToolCategory::Network);
    assert_eq!(ToolCategory::from_category_str("fs_read"), ToolCategory::FsRead);
    assert_eq!(ToolCategory::from_category_str("fs_write"), ToolCategory::FsWrite);
    assert_eq!(ToolCategory::from_category_str("shell"), ToolCategory::Shell);
    assert_eq!(ToolCategory::from_category_str("memory"), ToolCategory::Memory);
    assert_eq!(ToolCategory::from_category_str("flow"), ToolCategory::Flow);
    assert_eq!(ToolCategory::from_category_str("vfs"), ToolCategory::Vfs);
    assert_eq!(ToolCategory::from_category_str("index"), ToolCategory::Index);
    assert_eq!(ToolCategory::from_category_str("eval"), ToolCategory::Eval);
    assert_eq!(ToolCategory::from_category_str("synthesised"), ToolCategory::Synthesised);
    // unknown → Synthesised
    assert_eq!(ToolCategory::from_category_str("bogus"), ToolCategory::Synthesised);
    assert_eq!(ToolCategory::from_category_str(""), ToolCategory::Synthesised);
}
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test --lib -p chibi-core test_from_category_str`
Expected: compilation error — `from_category_str` doesn't exist yet.

- [ ] **Step 3: Implement `from_category_str`**

Add this method to `impl ToolCategory` in `registry.rs`, right after the existing `as_str` method (around line 158):

```rust
/// Parse a category string into a `ToolCategory` variant.
///
/// Unknown strings map to `Synthesised` (graceful fallback).
/// Symmetric with `as_str`.
pub fn from_category_str(s: &str) -> Self {
    match s {
        "memory" => Self::Memory,
        "fs_read" => Self::FsRead,
        "fs_write" => Self::FsWrite,
        "shell" => Self::Shell,
        "network" => Self::Network,
        "index" => Self::Index,
        "flow" => Self::Flow,
        "vfs" => Self::Vfs,
        "plugin" => Self::Plugin,
        "mcp" => Self::Mcp,
        "eval" => Self::Eval,
        _ => Self::Synthesised,
    }
}
```

- [ ] **Step 4: Run test — expect PASS**

Run: `cargo test --lib -p chibi-core test_from_category_str`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/registry.rs
git commit -m "feat(registry): add ToolCategory::from_category_str (#230)"
```

### Task 1.2: Expand `HARNESS_PREAMBLE` `define-tool` syntax-rules

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (the `HARNESS_PREAMBLE` LazyLock, around line 346)

- [ ] **Step 1: Replace the existing `define-tool` syntax-rules**

The current `define-tool` macro (around line 386–393) produces 4-element entries. Replace it with four patterns that produce 6-element entries `(name desc params handler category-or-#f summary-params-or-#f)`:

```scheme
(define-syntax define-tool
  (syntax-rules (description category summary-params parameters execute)
    ;; pattern 1: baseline (no category, no summary-params)
    ((define-tool name
       (description desc)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler #f #f)
             %tool-registry%)))
    ;; pattern 2: category only
    ((define-tool name
       (description desc)
       (category cat)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler cat #f)
             %tool-registry%)))
    ;; pattern 3: summary-params only
    ((define-tool name
       (description desc)
       (summary-params sp)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler #f sp)
             %tool-registry%)))
    ;; pattern 4: category + summary-params
    ((define-tool name
       (description desc)
       (category cat)
       (summary-params sp)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler cat sp)
             %tool-registry%)))))
```

Note: the `syntax-rules` keyword list must include `category` and `summary-params` as literals so they're matched structurally, not as pattern variables.

- [ ] **Step 2: Run existing tests — expect PASS (no regressions)**

Run: `cargo test --lib -p chibi-core synthesised`
Expected: all existing tests pass — pattern 1 matches the same shape as before.

- [ ] **Step 3: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): expand define-tool syntax-rules for category/summary-params (#230)"
```

### Task 1.3: `extract_multi_tools` reads category and summary_params

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (`extract_multi_tools` function, around line 1129)

- [ ] **Step 1: Write test for define-tool with category**

Add to the `#[cfg(test)] mod tests` in `synthesised.rs`:

```rust
#[test]
fn test_define_tool_category() {
    let source = r#"
(import (scheme base))
(define-tool net_fetch
  (description "fetches stuff")
  (category "network")
  (parameters '())
  (execute (lambda (args) "ok")))
"#;
    let path = VfsPath::new("/tools/shared/cat.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].category, ToolCategory::Network);
}
```

- [ ] **Step 2: Write test for define-tool with summary-params**

```rust
#[test]
fn test_define_tool_summary_params() {
    let source = r#"
(import (scheme base))
(define-tool my_action
  (description "does things")
  (summary-params '("ticker" "qty"))
  (parameters '())
  (execute (lambda (args) "ok")))
"#;
    let path = VfsPath::new("/tools/shared/sp.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].summary_params, vec!["ticker", "qty"]);
}
```

- [ ] **Step 3: Write test for define-tool with both**

```rust
#[test]
fn test_define_tool_category_and_summary_params() {
    let source = r#"
(import (scheme base))
(define-tool trade
  (description "places a trade")
  (category "network")
  (summary-params '("ticker" "quantity"))
  (parameters '())
  (execute (lambda (args) "ok")))
"#;
    let path = VfsPath::new("/tools/shared/both.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].category, ToolCategory::Network);
    assert_eq!(tools[0].summary_params, vec!["ticker", "quantity"]);
}
```

- [ ] **Step 4: Write test for unknown category string**

```rust
#[test]
fn test_define_tool_unknown_category() {
    let source = r#"
(import (scheme base))
(define-tool unknown_cat
  (description "unknown category")
  (category "banana")
  (parameters '())
  (execute (lambda (args) "ok")))
"#;
    let path = VfsPath::new("/tools/shared/unk.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools[0].category, ToolCategory::Synthesised);
}
```

- [ ] **Step 5: Write test for multi-tool with different categories**

```rust
#[test]
fn test_define_tool_multi_different_categories() {
    let source = r#"
(import (scheme base))
(define-tool reader
  (description "reads files")
  (category "fs_read")
  (parameters '())
  (execute (lambda (args) "read")))
(define-tool writer
  (description "writes files")
  (category "fs_write")
  (parameters '())
  (execute (lambda (args) "write")))
"#;
    let path = VfsPath::new("/tools/shared/multi.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].category, ToolCategory::FsRead);
    assert_eq!(tools[1].category, ToolCategory::FsWrite);
}
```

- [ ] **Step 6: Run tests — expect FAIL**

Run: `cargo test --lib -p chibi-core test_define_tool_category`
Expected: FAIL — `extract_multi_tools` still reads 4-element entries and hardcodes `ToolCategory::Synthesised`.

- [ ] **Step 7: Update `extract_multi_tools` to read category and summary_params**

In `extract_multi_tools` (around line 1155–1225):

1. Change the length check from `f.len() >= 4` to `f.len() >= 4` (still accept old 4-element for backwards compat, but new entries will be 6-element).

2. After the existing handler validation (`fields[3].is_procedure()` check), add extraction of the two new fields:

```rust
// category (index 4, optional — absent in 4-element legacy entries)
let category = if fields.len() > 4 {
    fields[4]
        .as_string()
        .map(|s| ToolCategory::from_category_str(s))
        .unwrap_or(ToolCategory::Synthesised)
} else {
    ToolCategory::Synthesised
};

// summary_params (index 5, optional)
let summary_params = if fields.len() > 5 {
    match &fields[5] {
        Value::List(items) => items
            .iter()
            .filter_map(|v| v.as_string().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
} else {
    vec![]
};
```

3. In the `Tool` struct literal at the bottom, replace:
   - `summary_params: vec![],` → `summary_params,`
   - `category: ToolCategory::Synthesised,` → `category,`

- [ ] **Step 8: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_define_tool_category test_define_tool_summary_params test_define_tool_category_and_summary_params test_define_tool_unknown_category test_define_tool_multi_different_categories`
Expected: all PASS

- [ ] **Step 9: Run full test suite — check for regressions**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS (existing 4-element entries still work via `fields.len() > 4` guard)

- [ ] **Step 10: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): extract_multi_tools reads category and summary_params (#230)"
```

### Task 1.4: `extract_single_tool` reads category and summary_params

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (`extract_single_tool` function, around line 1070)

- [ ] **Step 1: Write test for convention-format category**

```rust
#[test]
fn test_convention_category() {
    let source = r#"
(import (scheme base))
(define tool-name "net_tool")
(define tool-description "a network tool")
(define tool-category "network")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let path = VfsPath::new("/tools/shared/conv_cat.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools[0].category, ToolCategory::Network);
}
```

- [ ] **Step 2: Write test for convention-format summary_params**

```rust
#[test]
fn test_convention_summary_params() {
    let source = r#"
(import (scheme base))
(define tool-name "summ_tool")
(define tool-description "has summary params")
(define tool-summary-params '("path" "mode"))
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let path = VfsPath::new("/tools/shared/conv_sp.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools[0].summary_params, vec!["path", "mode"]);
}
```

- [ ] **Step 3: Write test for missing category/summary_params defaults**

```rust
#[test]
fn test_convention_defaults() {
    let source = r#"
(import (scheme base))
(define tool-name "plain_tool")
(define tool-description "no extras")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let path = VfsPath::new("/tools/shared/plain.scm").unwrap();
    let registry = make_registry();
    let tools = load_tools_from_source_with_tier(
        source,
        &path,
        &registry,
        crate::config::SandboxTier::Sandboxed,
    )
    .unwrap();
    assert_eq!(tools[0].category, ToolCategory::Synthesised);
    assert!(tools[0].summary_params.is_empty());
}
```

- [ ] **Step 4: Run tests — expect FAIL for category/summary_params tests**

Run: `cargo test --lib -p chibi-core test_convention_category test_convention_summary_params`
Expected: FAIL — `extract_single_tool` still hardcodes `ToolCategory::Synthesised` and `summary_params: vec![]`.

- [ ] **Step 5: Update `extract_single_tool`**

In `extract_single_tool` (around line 1070–1120), after the existing `tool-execute` validation, add:

```rust
// optional: tool-category
let category = session
    .evaluate("tool-category")
    .ok()
    .and_then(|v| v.as_string().map(|s| s.to_string()))
    .map(|s| ToolCategory::from_category_str(&s))
    .unwrap_or(ToolCategory::Synthesised);

// optional: tool-summary-params
let summary_params = session
    .evaluate("tool-summary-params")
    .ok()
    .and_then(|v| match v {
        Value::List(items) => Some(
            items
                .iter()
                .filter_map(|i| i.as_string().map(|s| s.to_string()))
                .collect::<Vec<_>>(),
        ),
        _ => None,
    })
    .unwrap_or_default();
```

Then update the `Tool` struct literal:
- `summary_params: vec![],` → `summary_params,`
- `category: ToolCategory::Synthesised,` → `category,`

- [ ] **Step 6: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_convention_category test_convention_summary_params test_convention_defaults`
Expected: all PASS

- [ ] **Step 7: Run full test suite**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

- [ ] **Step 8: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): extract_single_tool reads category and summary_params (#230)"
```

### Task 1.5: Lint + verify chunk 1

- [ ] **Step 1: Run `just lint`**

Run: `just lint`
Expected: no new warnings/errors

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: all PASS

- [ ] **Step 3: Commit any lint fixes**

```bash
git add -u && git commit -m "fmt"
```

---

## Chunk 2: Network Category No-URL Fallback

### Task 2.1: Update `PreFetchUrl` HOOK_METADATA

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs` (around line 653–692)

- [ ] **Step 1: Update the `PreFetchUrl` HookMeta**

Update the `description` and `payload_fields` to document the `no_url` variant. The `description` should mention that it also fires for network-category tools without a URL. Add `summary` as a payload field. Update the `url` and `reason` field descriptions to note they're absent when `safety == "no_url"`.

```rust
HookMeta {
    point: HookPoint::PreFetchUrl,
    category: "url_security",
    description: "fires before fetching a sensitive URL or invoking a network-category tool without a URL; deny-only",
    can_modify: true,
    payload_fields: &[
        FieldMeta {
            name: "tool_name",
            typ: "string",
            description: "name of the tool making the network call",
        },
        FieldMeta {
            name: "url",
            typ: "string",
            description: "URL being fetched (absent when safety is \"no_url\")",
        },
        FieldMeta {
            name: "safety",
            typ: "string",
            description: "\"sensitive\" for URL-based calls, \"no_url\" for network tools without a URL parameter",
        },
        FieldMeta {
            name: "reason",
            typ: "string",
            description: "classification reason (absent when safety is \"no_url\")",
        },
        FieldMeta {
            name: "summary",
            typ: "string",
            description: "human-readable summary from summary_params (present only when safety is \"no_url\")",
        },
    ],
    // return_fields and notes unchanged
```

- [ ] **Step 2: Run `cargo test --lib -p chibi-core hook_metadata` to check no regressions**

Expected: PASS (the completeness test only checks all HookPoints have metadata, not exact field shapes)

- [ ] **Step 3: Commit**

```bash
git add crates/chibi-core/src/tools/hooks.rs
git commit -m "docs(hooks): update PreFetchUrl metadata for no_url variant (#230)"
```

### Task 2.2: No-URL fallback in `send.rs` Network arm

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (around line 1115–1147)

- [ ] **Step 1: Implement the no-URL fallback**

Replace the `ToolCategory::Network` arm (lines 1115–1147) with:

```rust
ToolCategory::Network => {
    let url = args.get_str("url").unwrap_or("");
    if url.is_empty() {
        // No URL parameter — use summary_params for the permission prompt.
        let summary = tools::tool_call_summary(
            &registry.read().unwrap(),
            &tool_call.name,
            &tool_call.arguments,
        )
        .unwrap_or_default();
        let hook_data = json!({
            "tool_name": tool_call.name,
            "summary": summary,
            "safety": "no_url",
        });
        check_permission(
            plugin_tools,
            tools::HookPoint::PreFetchUrl,
            &hook_data,
            permission_handler,
            tein_ctx,
        )?
        .err()
        .map(|r| format!("Permission denied: {}", r))
    } else {
        let safety = tools::classify_url(url);
        if let Some(ref policy) = resolved_config.url_policy {
            if tools::evaluate_url_policy(url, &safety, policy) == tools::UrlAction::Deny {
                let reason = match &safety {
                    tools::UrlSafety::Sensitive(cat) => cat.to_string(),
                    tools::UrlSafety::Safe => "denied by URL policy".to_string(),
                };
                Some(format!("Permission denied: {}", reason))
            } else {
                None
            }
        } else if let tools::UrlSafety::Sensitive(category) = &safety {
            let hook_data = json!({
                "tool_name": tool_call.name,
                "url": url,
                "safety": "sensitive",
                "reason": category.to_string(),
            });
            check_permission(
                plugin_tools,
                tools::HookPoint::PreFetchUrl,
                &hook_data,
                permission_handler,
                tein_ctx,
            )?
            .err()
            .map(|r| format!("Permission denied: {}", r))
        } else {
            None
        }
    }
}
```

- [ ] **Step 2: Run existing tests — check for regressions**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

Note: The no-URL fallback path is hard to unit-test in isolation because `send.rs` permission logic requires a full `ToolCallContext` with async runtime, VFS, and app state. The path is exercised integration-style when a synthesised tool with `category: "network"` and no `url` parameter is invoked. Verifying correctness via code review + the existing permission test infrastructure is acceptable here. The chunk 1 tests already verify that synthesised tools get `ToolCategory::Network` assigned correctly.

- [ ] **Step 3: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "feat(send): network category no-URL fallback uses summary_params (#230)"
```

### Task 2.3: Lint + verify chunk 2

- [ ] **Step 1: Run `just lint`**
- [ ] **Step 2: Run full test suite: `cargo test -p chibi-core`**
- [ ] **Step 3: Commit any lint fixes**

---

## Chunk 3: Refactor `load_tools_from_source` Signature

### Task 3.1: Rewrite `load_tools_from_source` to take `&ToolsConfig`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Remove `load_tools_from_source_with_tier` and rewrite `load_tools_from_source`**

Remove the current `load_tools_from_source` (around line 999–1022, the convenience wrapper) and `load_tools_from_source_with_tier` (around line 1024–1048). Replace with a single function:

```rust
/// Load one or more synthesised tools from scheme source.
///
/// Resolves sandbox tier, HTTP prefixes, and env vars from `tools_config`
/// using longest-prefix match on `vfs_path`. Evaluates `source` in a tein
/// context configured accordingly.
#[cfg(feature = "synthesised-tools")]
pub fn load_tools_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    tools_config: &crate::config::ToolsConfig,
) -> io::Result<Vec<Tool>> {
    let source_owned = source.to_string();
    let tier = tools_config.resolve_tier(vfs_path.as_str());

    let (session, worker_thread_id) = build_tein_context(source_owned, tier)?;

    let multi = session.evaluate("%tool-registry%").ok();
    let is_multi = matches!(
        &multi,
        Some(Value::List(items)) if !items.is_empty()
    );

    if is_multi {
        extract_multi_tools(session, vfs_path, registry, worker_thread_id)
    } else {
        extract_single_tool(session, vfs_path, registry, worker_thread_id).map(|t| vec![t])
    }
}
```

Note: `http_prefixes` and `env_vars` will be threaded through in chunks 4 and 5. For now this just replaces the tier parameter with config lookup.

- [ ] **Step 2: Update `load_tool_from_source` (singular)**

This should now call `load_tools_from_source` with `&ToolsConfig::default()` since it doesn't take a config param:

```rust
pub fn load_tool_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> io::Result<Tool> {
    let mut tools = load_tools_from_source(source, vfs_path, registry, &crate::config::ToolsConfig::default())?;
    match tools.len() {
        1 => Ok(tools.remove(0)),
        n => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected 1 tool, got {n} (use load_tools_from_source for multi-tool files)"),
        )),
    }
}
```

- [ ] **Step 3: Update `scan_zone`**

In `scan_zone` (around line 1341), remove the `resolve_tier` call and pass `tools_config` directly:

Change:
```rust
let tier = tools_config.resolve_tier(file_path.as_str());
if let Ok(tools) = load_tools_from_source_with_tier(&source_str, &file_path, registry, tier)
```
To:
```rust
if let Ok(tools) = load_tools_from_source(&source_str, &file_path, registry, tools_config)
```

- [ ] **Step 4: Update `reload_tool_from_content`**

Similarly, remove the `resolve_tier` call:

Change:
```rust
let tier = tools_config.resolve_tier(path.as_str());
if let Ok(tools) = load_tools_from_source_with_tier(source_str, path, registry, tier) {
```
To:
```rust
if let Ok(tools) = load_tools_from_source(source_str, path, registry, tools_config) {
```

- [ ] **Step 5: Compile check**

Run: `cargo check -p chibi-core`
Expected: compilation errors in test call sites (still calling `load_tools_from_source_with_tier`). This is expected — we fix those next.

### Task 3.2: Add test helper for `ToolsConfig` with tier overrides

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (test module)

- [ ] **Step 1: Add helper function**

In the `#[cfg(test)] mod tests` block, add a helper to create `ToolsConfig` with a specific tier for a VFS path:

```rust
/// Build a `ToolsConfig` that maps `vfs_path` to the given tier.
fn config_with_tier(vfs_path: &str, tier: u8) -> crate::config::ToolsConfig {
    let mut tiers = std::collections::HashMap::new();
    tiers.insert(vfs_path.to_string(), tier);
    crate::config::ToolsConfig {
        tiers: Some(tiers),
        ..Default::default()
    }
}
```

### Task 3.3: Migrate `synthesised.rs` test call sites

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (test module)

- [ ] **Step 1: Migrate all test call sites**

For each `load_tools_from_source_with_tier(source, &path, &registry, SandboxTier::Sandboxed)`:
→ `load_tools_from_source(source, &path, &registry, &ToolsConfig::default())`

For each `load_tools_from_source_with_tier(source, &path, &registry, SandboxTier::Unsandboxed)`:
→ `load_tools_from_source(source, &path, &registry, &config_with_tier(path.as_str(), 2))`

This is a mechanical find-and-replace. There are ~14 call sites in `synthesised.rs`.

- [ ] **Step 2: Compile check**

Run: `cargo check -p chibi-core --lib`
Expected: may still have errors from `hooks.rs` — that's next task.

### Task 3.4: Migrate `hooks.rs` test call sites

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs` (test module)

- [ ] **Step 1: Add the same helper at the top of the hooks.rs test module**

The hooks.rs tests are in a separate `#[cfg(test)]` block. Add:

```rust
fn config_with_tier(vfs_path: &str, tier: u8) -> crate::config::ToolsConfig {
    let mut tiers = std::collections::HashMap::new();
    tiers.insert(vfs_path.to_string(), tier);
    crate::config::ToolsConfig {
        tiers: Some(tiers),
        ..Default::default()
    }
}
```

- [ ] **Step 2: Migrate all ~32 call sites**

Same mechanical replacement as Task 3.3. `Sandboxed` → `&ToolsConfig::default()`, `Unsandboxed` → `&config_with_tier(path, 2)`.

- [ ] **Step 3: Compile and test**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs crates/chibi-core/src/tools/hooks.rs
git commit -m "refactor(synthesised): load_tools_from_source takes &ToolsConfig (#230)"
```

### Task 3.5: Lint + verify chunk 3 (refactor)

- [ ] **Step 1: Run `just lint`**
- [ ] **Step 2: Run full test suite: `cargo test -p chibi-core`**
- [ ] **Step 3: Commit any lint fixes**

---

## Chunk 4: HTTP-Restricted Sandbox

### Task 4.1: Add `HttpConfig`, `HttpAllow` types and `ToolsConfig` fields

**Files:**
- Modify: `crates/chibi-core/src/config.rs`
- Modify: `crates/chibi-core/Cargo.toml`

- [ ] **Step 1: Enable `http` feature on tein dep**

In `crates/chibi-core/Cargo.toml`, change:
```toml
tein = { ..., features = ["json", "regex"] ... }
```
to:
```toml
tein = { ..., features = ["json", "regex", "http"] ... }
```

- [ ] **Step 2: Add types to `config.rs`**

Add near the `ToolsConfig` struct (around line 430):

```rust
/// HTTP access configuration for synthesised tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct HttpConfig {
    /// Per-path HTTP prefix allowlists. Longest-prefix match on VFS path.
    /// Values are either a list of URL prefixes or the string `"trust-declared"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<std::collections::HashMap<String, HttpAllow>>,
    /// Global toggle for trusting tool-declared HTTP prefixes.
    /// When `true`, tools that declare `tool-http-allow` get those prefixes
    /// even without an explicit `[tools.http.allow]` entry. Default: `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_declared: Option<bool>,
}

/// Per-path HTTP allowlist entry — either explicit prefixes or trust delegation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HttpAllow {
    /// Explicit list of allowed URL prefixes.
    Prefixes(Vec<String>),
    /// The string `"trust-declared"` — trust the tool's own `tool-http-allow` binding.
    TrustDeclared(String),
}

/// Result of resolving HTTP allowlist for a VFS path.
#[cfg(feature = "synthesised-tools")]
#[derive(Debug, Clone, PartialEq)]
pub enum HttpAllowResult {
    /// Explicit config prefixes — use directly.
    Prefixes(Vec<String>),
    /// Trust-declared applies — caller should read tool's declared prefixes.
    NeedDeclared,
    /// No HTTP access configured.
    NoAccess,
}
```

- [ ] **Step 3: Add `http` and `env` fields to `ToolsConfig`**

Add to the `ToolsConfig` struct:

```rust
/// HTTP access configuration for synthesised tools.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub http: Option<HttpConfig>,
/// Environment variable forwarding for synthesised tools.
/// Keys are VFS path prefixes, values are lists of env var names.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub env: Option<std::collections::HashMap<String, Vec<String>>>,
```

- [ ] **Step 4: Update `merge_local`**

In `merge_local` (around line 518), add the two new fields to the returned `ToolsConfig` struct literal:

```rust
ToolsConfig {
    include,
    exclude: merge_option_vecs(&self.exclude, &local.exclude),
    exclude_categories: merge_option_vecs(
        &self.exclude_categories,
        &local.exclude_categories,
    ),
    tiers,
    http: self.http.clone(),
    env: self.env.clone(),
}
```

- [ ] **Step 5: Compile check**

Run: `cargo check -p chibi-core`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/chibi-core/Cargo.toml crates/chibi-core/src/config.rs
git commit -m "feat(config): add HttpConfig, HttpAllow types and ToolsConfig fields (#230)"
```

### Task 4.2: `resolve_http_allow` method

**Files:**
- Modify: `crates/chibi-core/src/config.rs`

- [ ] **Step 1: Write tests**

Add to the `#[cfg(test)]` block in `config.rs`:

```rust
#[test]
fn test_resolve_http_allow_explicit_prefixes() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/t212.scm".to_string(),
        HttpAllow::Prefixes(vec!["https://demo.trading212.com/".to_string()]),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::Prefixes(vec!["https://demo.trading212.com/".to_string()]),
    );
}

#[test]
fn test_resolve_http_allow_longest_prefix() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        HttpAllow::Prefixes(vec!["https://general.com/".to_string()]),
    );
    allow.insert(
        "/tools/shared/t212.scm".to_string(),
        HttpAllow::Prefixes(vec!["https://specific.com/".to_string()]),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::Prefixes(vec!["https://specific.com/".to_string()]),
    );
}

#[test]
fn test_resolve_http_allow_no_config() {
    let config = ToolsConfig::default();
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::NoAccess,
    );
}

#[test]
fn test_resolve_http_allow_trust_declared() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        HttpAllow::TrustDeclared("trust-declared".to_string()),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::NeedDeclared,
    );
}

#[test]
fn test_resolve_http_allow_global_trust_no_path_match() {
    let config = ToolsConfig {
        http: Some(HttpConfig {
            trust_declared: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    // no [tools.http.allow] entries at all, but global trust_declared = true
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::NeedDeclared,
    );
}

#[test]
fn test_resolve_http_allow_unknown_string() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        HttpAllow::TrustDeclared("typo-declared".to_string()),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    // unknown string → treated as None
    assert_eq!(
        config.resolve_http_allow("/tools/shared/t212.scm"),
        HttpAllowResult::NoAccess,
    );
}
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `cargo test --lib -p chibi-core test_resolve_http_allow`
Expected: FAIL — `resolve_http_allow` doesn't exist.

- [ ] **Step 3: Implement `resolve_http_allow`**

Add to `impl ToolsConfig`, after `resolve_tier`:

```rust
/// Resolve HTTP prefix allowlist for a synthesised tool at the given VFS path.
///
/// Uses longest-prefix match on `[tools.http.allow]` entries.
/// Returns `HttpAllowResult::Prefixes` for explicit lists,
/// `HttpAllowResult::NeedDeclared` for `"trust-declared"` entries,
/// `HttpAllowResult::NoAccess` when no entry matches.
#[cfg(feature = "synthesised-tools")]
pub fn resolve_http_allow(&self, vfs_path: &str) -> HttpAllowResult {
    let http = match &self.http {
        Some(h) => h,
        None => return HttpAllowResult::NoAccess,
    };
    let allow_map = match &http.allow {
        Some(m) => m,
        None => {
            // No per-path entries — check global trust_declared
            return if http.trust_declared.unwrap_or(false) {
                HttpAllowResult::NeedDeclared
            } else {
                HttpAllowResult::NoAccess
            };
        }
    };

    // Longest-prefix match
    let mut best: Option<(&str, &HttpAllow)> = None;
    for (pattern, entry) in allow_map {
        if vfs_path.starts_with(pattern.as_str()) {
            match best {
                None => best = Some((pattern, entry)),
                Some((prev, _)) if pattern.len() > prev.len() => {
                    best = Some((pattern, entry));
                }
                _ => {}
            }
        }
    }

    match best {
        Some((_, HttpAllow::Prefixes(prefixes))) => {
            HttpAllowResult::Prefixes(prefixes.clone())
        }
        Some((_, HttpAllow::TrustDeclared(s))) if s == "trust-declared" => {
            HttpAllowResult::NeedDeclared
        }
        Some((pattern, HttpAllow::TrustDeclared(s))) => {
            eprintln!(
                "warning: [tools.http.allow] {pattern:?}: unrecognised value {s:?}, ignoring"
            );
            HttpAllowResult::NoAccess
        }
        None => {
            // Check global trust_declared
            if http.trust_declared.unwrap_or(false) {
                HttpAllowResult::NeedDeclared
            } else {
                HttpAllowResult::NoAccess
            }
        }
    }
}
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_resolve_http_allow`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add crates/chibi-core/src/config.rs
git commit -m "feat(config): resolve_http_allow with longest-prefix match (#230)"
```

### Task 4.3: Thread HTTP prefixes through `build_tein_context`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Add `http_prefixes` param to `build_tein_context`**

Update the signature (around line 866):

```rust
fn build_tein_context(
    source: String,
    tier: crate::config::SandboxTier,
    http_prefixes: Option<Vec<String>>,
) -> io::Result<(TeinSession, std::thread::ThreadId)> {
```

Refactor **both** branches to use intermediate `builder` variables (needed for chunks 4 and 5). In the `Sandboxed` branch, add `.http_allow()` when prefixes are present:

```rust
crate::config::SandboxTier::Sandboxed => {
    let mut builder = Context::builder()
        .standard_env()
        .sandboxed(Modules::Safe)
        .step_limit(10_000_000);
    if let Some(ref prefixes) = http_prefixes {
        let refs: Vec<&str> = prefixes.iter().map(|s| s.as_str()).collect();
        builder = builder.http_allow(&refs);
    }
    builder.build_managed(init)
}
crate::config::SandboxTier::Unsandboxed => {
    let builder = Context::builder()
        .standard_env()
        .with_vfs_shadows();
    builder.build_managed(init)
}
```

Note: `http_allow` takes `&[&str]`, so we convert from `Vec<String>`. The Unsandboxed branch doesn't get `http_allow` (already has full access) but is refactored to builder variable pattern for Task 5.2 (env vars).

- [ ] **Step 2: Update `build_sandboxed_harness_context`**

Change:
```rust
build_tein_context(String::new(), crate::config::SandboxTier::Sandboxed)
```
To:
```rust
build_tein_context(String::new(), crate::config::SandboxTier::Sandboxed, None)
```

- [ ] **Step 3: Update `load_tools_from_source` to resolve and pass HTTP prefixes**

```rust
let http_prefixes = match tools_config.resolve_http_allow(vfs_path.as_str()) {
    crate::config::HttpAllowResult::Prefixes(p) => Some(p),
    _ => None, // NeedDeclared and None handled in chunk 5
};

let (session, worker_thread_id) = build_tein_context(source_owned, tier, http_prefixes)?;
```

- [ ] **Step 4: Fix all other `build_tein_context` call sites**

Search for any direct `build_tein_context` calls in tests and update them to pass `None` for `http_prefixes`.

- [ ] **Step 5: Compile and test**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): thread http_prefixes through build_tein_context (#230)"
```

### Task 4.4: Lint + verify chunk 4

- [ ] **Step 1: Run `just lint`**
- [ ] **Step 2: Run full test suite: `cargo test -p chibi-core`**
- [ ] **Step 3: Commit any lint fixes**

---

## Chunk 5: Env Var Forwarding

### Task 5.1: `resolve_env` method

**Files:**
- Modify: `crates/chibi-core/src/config.rs`

- [ ] **Step 1: Write tests**

Note: `std::env::set_var`/`remove_var` are not thread-safe. Use unique env var names per test to minimise collision risk with parallel test execution.

```rust
#[test]
fn test_resolve_env_present() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "/tools/shared/t212.scm".to_string(),
        vec!["CHIBI_TEST_ENV_KEY_1".to_string(), "CHIBI_TEST_ENV_MISSING_1".to_string()],
    );
    let config = ToolsConfig {
        env: Some(env),
        ..Default::default()
    };
    // SAFETY: using unique env var name to avoid parallel test collision
    unsafe { std::env::set_var("CHIBI_TEST_ENV_KEY_1", "secret123") };
    let result = config.resolve_env("/tools/shared/t212.scm");
    unsafe { std::env::remove_var("CHIBI_TEST_ENV_KEY_1") };

    let vars = result.unwrap();
    assert_eq!(vars, vec![("CHIBI_TEST_ENV_KEY_1".to_string(), "secret123".to_string())]);
    // CHIBI_TEST_ENV_MISSING_1 was not set, so it's skipped
}

#[test]
fn test_resolve_env_no_config() {
    let config = ToolsConfig::default();
    assert!(config.resolve_env("/tools/shared/t212.scm").is_none());
}

#[test]
fn test_resolve_env_longest_prefix() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "/tools/shared/".to_string(),
        vec!["GENERAL_KEY".to_string()],
    );
    env.insert(
        "/tools/shared/t212.scm".to_string(),
        vec!["CHIBI_TEST_ENV_SPECIFIC_1".to_string()],
    );
    let config = ToolsConfig {
        env: Some(env),
        ..Default::default()
    };
    unsafe { std::env::set_var("CHIBI_TEST_ENV_SPECIFIC_1", "val") };
    let result = config.resolve_env("/tools/shared/t212.scm");
    unsafe { std::env::remove_var("CHIBI_TEST_ENV_SPECIFIC_1") };

    let vars = result.unwrap();
    assert_eq!(vars, vec![("CHIBI_TEST_ENV_SPECIFIC_1".to_string(), "val".to_string())]);
}
```

- [ ] **Step 2: Implement `resolve_env`**

Add to `impl ToolsConfig`:

```rust
/// Resolve environment variable forwarding for a synthesised tool.
///
/// Uses longest-prefix match on `[tools.env]` entries. Reads the listed
/// var names from the real process environment, returning name+value pairs.
/// Vars not set in the process are silently skipped.
///
/// Returns `None` when no config entry matches (distinct from `Some(vec![])`
/// which means "config matched but no vars were set").
#[cfg(feature = "synthesised-tools")]
pub fn resolve_env(&self, vfs_path: &str) -> Option<Vec<(String, String)>> {
    let env_map = self.env.as_ref()?;

    let mut best: Option<(&str, &Vec<String>)> = None;
    for (pattern, var_names) in env_map {
        if vfs_path.starts_with(pattern.as_str()) {
            match best {
                None => best = Some((pattern, var_names)),
                Some((prev, _)) if pattern.len() > prev.len() => {
                    best = Some((pattern, var_names));
                }
                _ => {}
            }
        }
    }

    best.map(|(_, var_names)| {
        var_names
            .iter()
            .filter_map(|name| {
                std::env::var(name).ok().map(|val| (name.clone(), val))
            })
            .collect()
    })
}
```

- [ ] **Step 3: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_resolve_env`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/config.rs
git commit -m "feat(config): resolve_env with longest-prefix match (#230)"
```

### Task 5.2: Thread env vars through `build_tein_context`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Add `env_vars` param to `build_tein_context`**

Update signature:

```rust
fn build_tein_context(
    source: String,
    tier: crate::config::SandboxTier,
    http_prefixes: Option<Vec<String>>,
    env_vars: Option<Vec<(String, String)>>,
) -> io::Result<(TeinSession, std::thread::ThreadId)> {
```

In both the `Sandboxed` and `Unsandboxed` builder branches (already refactored to builder variable pattern in Task 4.3), add env var forwarding before `.build_managed(init)`:

```rust
if let Some(ref vars) = env_vars {
    let refs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    builder = builder.environment_variables(&refs);
}
```

Note: `environment_variables` takes `&[(&str, &str)]`, so we convert from the owned `Vec<(String, String)>`. Add this to both the `Sandboxed` and `Unsandboxed` branches (the `Unsandboxed` `builder` binding needs to become `let mut builder` to allow reassignment).

- [ ] **Step 2: Update `build_sandboxed_harness_context`**

Add `None` for the new param:
```rust
build_tein_context(String::new(), crate::config::SandboxTier::Sandboxed, None, None)
```

- [ ] **Step 3: Update `load_tools_from_source`**

Add env resolution:
```rust
let env_vars = tools_config.resolve_env(vfs_path.as_str());

let (session, worker_thread_id) = build_tein_context(source_owned, tier, http_prefixes, env_vars)?;
```

- [ ] **Step 4: Fix all other `build_tein_context` call sites**

Add `None` for `env_vars` to any direct `build_tein_context` calls in tests.

- [ ] **Step 5: Compile and test**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): thread env_vars through build_tein_context (#230)"
```

### Task 5.3: Lint + verify chunk 5

- [ ] **Step 1: Run `just lint`**
- [ ] **Step 2: Run full test suite: `cargo test -p chibi-core`**
- [ ] **Step 3: Commit any lint fixes**

---

## Chunk 6: Trust-Declared HTTP Prefixes

### Task 6.1: `resolve_http_allow_with_declared` method

**Files:**
- Modify: `crates/chibi-core/src/config.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_resolve_http_allow_with_declared_trust_per_path() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        HttpAllow::TrustDeclared("trust-declared".to_string()),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    let declared = vec!["https://api.example.com/".to_string()];
    let result = config.resolve_http_allow_with_declared("/tools/shared/t212.scm", &declared);
    assert_eq!(result, Some(vec!["https://api.example.com/".to_string()]));
}

#[test]
fn test_resolve_http_allow_with_declared_explicit_wins() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/t212.scm".to_string(),
        HttpAllow::Prefixes(vec!["https://explicit.com/".to_string()]),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    let declared = vec!["https://ignored.com/".to_string()];
    let result = config.resolve_http_allow_with_declared("/tools/shared/t212.scm", &declared);
    assert_eq!(result, Some(vec!["https://explicit.com/".to_string()]));
}

#[test]
fn test_resolve_http_allow_with_declared_global_trust() {
    let config = ToolsConfig {
        http: Some(HttpConfig {
            trust_declared: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let declared = vec!["https://api.example.com/".to_string()];
    let result = config.resolve_http_allow_with_declared("/tools/shared/t212.scm", &declared);
    assert_eq!(result, Some(vec!["https://api.example.com/".to_string()]));
}

#[test]
fn test_resolve_http_allow_with_declared_no_trust() {
    let config = ToolsConfig::default();
    let declared = vec!["https://api.example.com/".to_string()];
    let result = config.resolve_http_allow_with_declared("/tools/shared/t212.scm", &declared);
    assert_eq!(result, None);
}

#[test]
fn test_resolve_http_allow_with_declared_empty_declared() {
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        HttpAllow::TrustDeclared("trust-declared".to_string()),
    );
    let config = ToolsConfig {
        http: Some(HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    let declared: Vec<String> = vec![];
    let result = config.resolve_http_allow_with_declared("/tools/shared/t212.scm", &declared);
    // trust-declared but nothing declared → no access
    assert_eq!(result, None);
}
```

- [ ] **Step 2: Implement `resolve_http_allow_with_declared`**

Add to `impl ToolsConfig`:

```rust
/// Resolve HTTP prefixes with tool-declared fallback.
///
/// Called after reading `tool-http-allow` from the tool source.
/// Uses `resolve_http_allow` internally:
/// - `Prefixes(p)` → `Some(p)` (explicit config wins, declared ignored)
/// - `NeedDeclared` + non-empty declared → `Some(declared.to_vec())`
/// - `NeedDeclared` + empty declared → `None`
/// - `None` → `None`
#[cfg(feature = "synthesised-tools")]
pub fn resolve_http_allow_with_declared(
    &self,
    vfs_path: &str,
    declared: &[String],
) -> Option<Vec<String>> {
    match self.resolve_http_allow(vfs_path) {
        HttpAllowResult::Prefixes(p) => Some(p),
        HttpAllowResult::NeedDeclared if !declared.is_empty() => {
            Some(declared.to_vec())
        }
        _ => None,
    }
}
```

- [ ] **Step 3: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_resolve_http_allow_with_declared`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/chibi-core/src/config.rs
git commit -m "feat(config): resolve_http_allow_with_declared for trust delegation (#230)"
```

### Task 6.2: Two-phase load in `load_tools_from_source`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

- [ ] **Step 1: Write test for trust-declared two-phase load**

```rust
#[test]
fn test_trust_declared_reads_tool_http_allow() {
    let source = r#"
(import (scheme base))
(define tool-http-allow '("https://api.example.com/"))
(define tool-name "http_tool")
(define tool-description "uses HTTP")
(define tool-category "network")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
    let path = VfsPath::new("/tools/shared/http.scm").unwrap();
    let registry = make_registry();
    let mut allow = std::collections::HashMap::new();
    allow.insert(
        "/tools/shared/".to_string(),
        crate::config::HttpAllow::TrustDeclared("trust-declared".to_string()),
    );
    let config = crate::config::ToolsConfig {
        http: Some(crate::config::HttpConfig {
            allow: Some(allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Should load successfully — two-phase build trusts the declared prefixes
    let tools = load_tools_from_source(source, &path, &registry, &config).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].category, ToolCategory::Network);
}
```

- [ ] **Step 2: Implement two-phase load**

Update `load_tools_from_source` in `synthesised.rs`. After the initial `build_tein_context` call, check for `NeedDeclared` and do a second pass:

```rust
pub fn load_tools_from_source(
    source: &str,
    vfs_path: &VfsPath,
    registry: &Arc<RwLock<ToolRegistry>>,
    tools_config: &crate::config::ToolsConfig,
) -> io::Result<Vec<Tool>> {
    let source_owned = source.to_string();
    let tier = tools_config.resolve_tier(vfs_path.as_str());
    let env_vars = tools_config.resolve_env(vfs_path.as_str());

    let http_result = tools_config.resolve_http_allow(vfs_path.as_str());
    let http_prefixes = match &http_result {
        crate::config::HttpAllowResult::Prefixes(p) => Some(p.clone()),
        _ => None, // NeedDeclared resolved after phase 1
    };

    let (session, worker_thread_id) =
        build_tein_context(source_owned.clone(), tier, http_prefixes.clone(), env_vars.clone())?;

    // Phase 2: trust-declared HTTP — read tool-http-allow, rebuild if needed
    let (session, worker_thread_id) =
        if matches!(http_result, crate::config::HttpAllowResult::NeedDeclared) {
            // Read tool-http-allow binding from phase 1 session
            let declared = session
                .evaluate("tool-http-allow")
                .ok()
                .and_then(|v| match v {
                    Value::List(items) => Some(
                        items
                            .iter()
                            .filter_map(|i| i.as_string().map(|s| s.to_string()))
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .unwrap_or_default();

            if let Some(trusted_prefixes) =
                tools_config.resolve_http_allow_with_declared(vfs_path.as_str(), &declared)
            {
                // Rebuild with the trusted prefixes
                build_tein_context(source_owned, tier, Some(trusted_prefixes), env_vars)?
            } else {
                (session, worker_thread_id)
            }
        } else {
            (session, worker_thread_id)
        };

    let multi = session.evaluate("%tool-registry%").ok();
    let is_multi = matches!(
        &multi,
        Some(Value::List(items)) if !items.is_empty()
    );

    if is_multi {
        extract_multi_tools(session, vfs_path, registry, worker_thread_id)
    } else {
        extract_single_tool(session, vfs_path, registry, worker_thread_id).map(|t| vec![t])
    }
}
```

- [ ] **Step 3: Run tests — expect PASS**

Run: `cargo test --lib -p chibi-core test_trust_declared`
Expected: PASS

- [ ] **Step 4: Run full test suite**

Run: `cargo test --lib -p chibi-core`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs
git commit -m "feat(synthesised): two-phase load for trust-declared HTTP prefixes (#230)"
```

### Task 6.3: Lint + verify chunk 6

- [ ] **Step 1: Run `just lint`**
- [ ] **Step 2: Run full test suite: `cargo test -p chibi-core`**
- [ ] **Step 3: Commit any lint fixes**

---

## Chunk 7: Final Verification + Docs

### Task 7.1: Update docs/hooks.md (generated)

- [ ] **Step 1: Regenerate hooks docs**

Run: `just generate-docs`
Expected: `docs/hooks.md` updates with the new `PreFetchUrl` payload fields.

- [ ] **Step 2: Commit**

```bash
git add docs/hooks.md
git commit -m "docs: regenerate hooks.md with PreFetchUrl no_url variant (#230)"
```

### Task 7.2: Update AGENTS.md with new quirks

- [ ] **Step 1: Add new quirks**

Add to the `Quirks / Gotchas` section of `AGENTS.md`:

```
- `load_tools_from_source` takes `&ToolsConfig` (not a tier param). Tests use `&ToolsConfig::default()` for sandboxed, or `config_with_tier(path, 2)` for unsandboxed. `load_tool_from_source` (singular) still takes no config param — uses default internally.
- `ToolCategory::from_category_str` maps category strings from scheme tools to variants. Unknown strings → `Synthesised`.
- `HttpAllowResult::NeedDeclared` triggers two-phase context build: phase 1 evaluates source without HTTP to read `tool-http-allow`, phase 2 rebuilds with trusted prefixes.
- `PreFetchUrl` hook fires with `safety: "no_url"` and `summary` field (no `url`/`reason`) for network-category tools without a URL parameter.
```

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: update AGENTS.md with sandbox extension quirks (#230)"
```

### Task 7.3: Full integration test

- [ ] **Step 1: Run the complete test suite**

Run: `cargo test -p chibi-core`
Expected: all PASS

- [ ] **Step 2: Run `just lint`**

Expected: clean

- [ ] **Step 3: Verify build**

Run: `cargo build`
Expected: clean build
