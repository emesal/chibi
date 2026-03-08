# Tein Integration — Remaining Items

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete the synthesised tool system so scheme tools can call other tools, all VFS zones are scanned, visibility is scoped per-context, multi-tool files are supported, and sandbox tiers are configurable.

**Issue:** #193

---

## Progress Summary

### ✅ DONE (committed: `7cd92df6`)

Tasks 1–8 complete in one commit. Covers:

- `HARNESS_PREAMBLE` const: defines `%tool-registry%` and `define-tool` macro at top level
- `HARNESS_TOOLS_MODULE` const: `(harness tools)` library exporting `call-tool`
- `build_tein_context(source, tier)` helper: creates sandboxed/unsandboxed tein context
- `call_tool_fn` (`#[tein_fn(name = "call-tool")]`): reads `BRIDGE_REGISTRY` + `BRIDGE_CALL_CTX` thread-locals, converts scheme alist → JSON, dispatches via `ToolRegistry::dispatch_impl`, bridges sync→async via `Handle::current().block_on()`
- `CallContextGuard`: RAII sets/clears `BRIDGE_CALL_CTX` per `execute_synthesised` call
- `load_tools_from_source(source, vfs_path, registry)`: delegates to `load_tools_from_source_with_tier(..., Sandboxed)`
- `load_tools_from_source_with_tier(source, vfs_path, registry, tier)`: builds context, detects multi vs single via `%tool-registry%`, calls `extract_multi_tools` or `extract_single_tool`
- `extract_single_tool`: convention format (`tool-name`, `tool-description`, etc.)
- `extract_multi_tools`: reads `%tool-registry%` LIFO list, binds per-tool exec handlers as `%tool-execute-{name}%`
- `ToolImpl::Synthesised` gained `exec_binding: String` field
- `execute_synthesised(context, exec_binding, call)`: sets guard, looks up binding, calls it
- `find_all_by_vfs_path` replaces `find_by_vfs_path` in registry (multi-tool aware)
- `scan_and_register(vfs, registry, tools_config)`: discovers home+flock zones, calls `scan_zone`
- `scan_zone`: calls `load_tools_from_source_with_tier` with tier from config
- `reload_tool_from_content(registry, path, content, tools_config)`: unregisters old tools, registers new ones
- `unregister_tool_at_path`: uses `find_all_by_vfs_path`
- `scheme_value_to_json`: converts scheme alist/list/atoms → `serde_json::Value`
- `SandboxTier` enum in `config.rs` (`Sandboxed`, `Unsandboxed`)
- `ToolsConfig.tiers: Option<HashMap<String, u8>>` field added
- `ToolsConfig::resolve_tier(vfs_path)` added
- `ToolsConfig::merge_local` updated to merge `tiers`
- `send.rs`: visibility filter added after `filter_tools_by_config` (uses `is_tool_visible`)
- `ToolRegistry::is_tool_visible(name, context_name, flock_memberships)` added
- All existing tests updated for new signatures; 27 synthesised tests pass, 824 total

### ✅ DONE (committed: `683069f6`, `843e6645`, `8dc7be8e`)

Tasks A–E complete. All 30 synthesised tests pass. Workspace tests pass. Lint clean.

---

## Remaining Work

### ~~Task A: Fix test callsites for `reload_tool_from_content`~~ ✅ DONE

**Files:** `crates/chibi-core/src/tools/synthesised.rs`

All 9 failing call sites in the `#[cfg(all(test, feature = "synthesised-tools"))]` block need `&crate::config::ToolsConfig::default()` as 4th argument.

Lines with 3-arg `reload_tool_from_content(...)` (all in test module):
- 1055, 1069, 1082, 1101, 1127, 1131 — use `(&registry, &path, ...bytes...)`
- 1149 — inside VFS callback closure: `reload_tool_from_content(&reg, path, bytes)` → add `, &crate::config::ToolsConfig::default()`
- 1436, 1448 — multi-tool hot-reload test

**Fix:** add `, &crate::config::ToolsConfig::default()` to each call.

After fix, run: `cargo test -p chibi-core synthesised --features synthesised-tools`
Expected: 27 tests pass.

**Commit:**
```
fix(tein): pass ToolsConfig to reload_tool_from_content in tests
```

### Task B: Add tier tests to synthesised.rs (from plan task 9)

Add tests at the bottom of the test module:

```rust
#[test]
fn test_tier1_rejects_unsafe_imports() {
    let source = r#"
(import (scheme base))
(import (scheme file))  ; should fail in sandboxed
(define tool-name "bad_tool")
(define tool-description "tries unsafe import")
(define tool-parameters '())
(define (tool-execute args) "should not load")
"#;
    let vfs_path = VfsPath::new("/tools/shared/bad.scm").unwrap();
    let registry = make_registry();
    let result = load_tools_from_source_with_tier(source, &vfs_path, &registry, crate::config::SandboxTier::Sandboxed);
    assert!(result.is_err(), "sandboxed tier should reject (scheme file)");
}

#[test]
fn test_tier2_allows_full_scheme() {
    let source = r#"
(import (scheme base))
(define tool-name "tier2_tool")
(define tool-description "uses full scheme")
(define tool-parameters '())
(define (tool-execute args) "full scheme works")
"#;
    let vfs_path = VfsPath::new("/tools/shared/full.scm").unwrap();
    let registry = make_registry();
    // tier 2 — no sandboxing; just verify it loads without error
    let result = load_tools_from_source_with_tier(source, &vfs_path, &registry, crate::config::SandboxTier::Unsandboxed);
    assert!(result.is_ok(), "unsandboxed tier should allow loading");
}
```

Note: `(scheme file)` may or may not be blocked by sandbox depending on tein version. If `test_tier1_rejects_unsafe_imports` passes with a module that IS blocked, great. If tein's `Modules::Safe` doesn't block `(scheme file)` specifically, adjust to use another unsafe module or just verify the context builds differently.

Also `load_tools_from_source_with_tier` needs to be `pub` in synthesised.rs (check — it may already be).

Run: `cargo test -p chibi-core tier --features synthesised-tools`

**Commit:**
```
test(tein): tier 1/2 sandbox boundary tests
```

### Task C: Integration test (plan task 11)

Add to synthesised.rs test module:

```rust
#[tokio::test]
async fn test_integration_harness_import_works() {
    // verify (import (harness tools)) succeeds and call-tool is available
    let registry = make_registry();
    let source = r#"
(import (scheme base))
(import (harness tools))
(define tool-name "harness_test")
(define tool-description "tests harness import")
(define tool-parameters '())
(define (tool-execute args) "harness ok")
"#;
    let vfs_path = VfsPath::new("/tools/shared/harness_test.scm").unwrap();
    let tool = load_tool_from_source(source, &vfs_path, &registry).unwrap();
    assert_eq!(tool.name, "harness_test");
}
```

Run: `cargo test -p chibi-core test_integration --features synthesised-tools`

**Commit:**
```
test(tein): integration test for harness import and call-tool availability (#193)
```

### Task D: Full workspace test + lint (plan task 12)

Run: `cargo test --workspace`
Expected: all pass.

Run: `just lint`
Fix any warnings.

**Commit:**
```
chore: lint fixes and AGENTS.md updates for tein integration (#193)
```

### Task E: Update documentation (plan task 10)

**Files to update:**
- `docs/plugins.md` — document `(harness tools)` module, `call-tool`, `define-tool`
- `docs/configuration.md` — document `[tools.tiers]` config
- `docs/vfs.md` — document `/tools/home/` and `/tools/flocks/` scanning
- `AGENTS.md` — add quirks below

**AGENTS.md quirks to add:**
```
- synthesised tools: `(harness tools)` module provides `call-tool` and `define-tool`.
  `HARNESS_PREAMBLE` defines `%tool-registry%` and `define-tool` at top-level (not
  inside the library) so `set!` can mutate it and rust can read it post-eval.
- `ToolImpl::Synthesised` has `exec_binding` field: `"tool-execute"` for convention
  format, `"%tool-execute-{name}%"` for `define-tool` multi-tool files.
- `reload_tool_from_content` and `scan_and_register` require `&ToolsConfig` for tier
  resolution. Pass `&ToolsConfig::default()` when no tier overrides needed.
- `call-tool` bridge uses two thread-locals: `BRIDGE_REGISTRY` (set at load time,
  retained) and `BRIDGE_CALL_CTX` (set/cleared per execute via `CallContextGuard`).
```

**Commit:**
```
docs: document harness tools module, tiers, and visibility scoping (#193)
```

### Task F: Close issue + finishing-a-development-branch

After all tasks complete and full test suite passes:
- Use `superpowers:finishing-a-development-branch` skill
- Closes #193

---

## Key Architecture Notes for Next Session

**Branch:** `feature/tein-integration-2603`

**Critical files changed (uncommitted):**
- `crates/chibi-core/src/tools/synthesised.rs` — main implementation
- `crates/chibi-core/src/tools/registry.rs` — `is_tool_visible`, `find_all_by_vfs_path`, `exec_binding` field
- `crates/chibi-core/src/api/send.rs` — visibility filter, `ToolsConfig` literal fixes
- `crates/chibi-core/src/config.rs` — `SandboxTier`, `ToolsConfig.tiers`, `resolve_tier`
- `crates/chibi-core/src/chibi.rs` — passes `tools_config` to scan/reload

**Design decisions made:**
- `%tool-registry%` and `define-tool` defined at TOP LEVEL (not inside the library) via `HARNESS_PREAMBLE`, because `set!` inside a library's `begin` cannot mutate top-level bindings
- `exec_binding` stored in `ToolImpl::Synthesised` so multi-tool files can share one context but dispatch to per-tool handlers
- `reload_tool_from_content` now unregisters all tools at path before registering new ones (multi-tool aware)
- `scan_zone` passes tier from `tools_config.resolve_tier(file_path.as_str())` per file
- `send.rs` visibility filter uses `vfs_block_on(app.vfs.flock_list_for(&context.name))` to get flock memberships
