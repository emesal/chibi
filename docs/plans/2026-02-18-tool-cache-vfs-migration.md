# Tool Cache → VFS Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the standalone tool output cache system with VFS storage under `/sys/tool_cache/<context>/<cache_id>`, eliminating `cache.rs` and all duplicate path/cleanup machinery.

**Architecture:** Cache entries are written via `SYSTEM_CALLER` to `vfs:///sys/tool_cache/<ctx>/<id>` and are world-readable. The LLM receives a `vfs:///` URI in the truncated message and accesses content via existing `path=` param on file tools. Cleanup iterates the VFS listing and uses `VfsMetadata::created` for age checks.

**Tech Stack:** Rust, `crates/chibi-core`, async VFS (`app.vfs`), `SYSTEM_CALLER` from `crate::vfs::SYSTEM_CALLER`

**Design doc:** `docs/plans/2026-02-18-tool-cache-vfs-migration-design.md`

---

## Task 1: Add VFS-backed cache write helper in `send.rs`

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs`

This task replaces the `cache_output(...)` + `generate_truncated_message(...)` block with direct VFS writes. We keep `cache::should_cache` temporarily (deleted in Task 6).

The new logic:
1. Generates cache ID (same format: `{tool}_{timestamp_hex}_{args_hash}`)
2. Writes content to `vfs:///sys/tool_cache/<ctx>/<id>` as `SYSTEM_CALLER`
3. Computes stats (char_count, token_estimate, line_count) inline
4. Generates truncated message referencing the `vfs:///` URI

**Step 1: Locate the caching block in `send.rs`**

Search for `// Check if output should be cached` around line 1192. Read the full block through `Ok(ToolExecutionResult {`.

**Step 2: Write the failing test**

In the `#[cfg(test)]` section of `send.rs` (around line 2540), add:

```rust
#[tokio::test]
async fn test_cache_write_goes_to_vfs() {
    // Set up AppState with a tiny threshold so caching triggers
    let (app, ctx_name) = test_helpers::make_app_with_context("cache-test");
    let large_output = "x".repeat(100); // threshold will be set to 50 in config

    // Simulate what execute_tool_pure does when caching
    let cache_id = vfs_cache::generate_cache_id("test_tool", &serde_json::json!({}));
    let vfs_path = VfsPath::new(&format!("/sys/tool_cache/{}/{}", ctx_name, cache_id)).unwrap();

    app.vfs
        .write(crate::vfs::SYSTEM_CALLER, &vfs_path, large_output.as_bytes())
        .await
        .unwrap();

    let exists = app.vfs
        .exists(crate::vfs::SYSTEM_CALLER, &vfs_path)
        .await
        .unwrap();
    assert!(exists, "cache entry should exist in VFS");

    let content = app.vfs
        .read("cache-test", &vfs_path)
        .await
        .unwrap();
    assert_eq!(content, large_output.as_bytes());
}
```

> Note: `vfs_cache` module and `test_helpers` will be created in later tasks. This test is intentionally written to compile only after Task 2 is done — run it then.

**Step 3: Extract cache ID generation into a new module `vfs_cache.rs`**

Create `crates/chibi-core/src/vfs_cache.rs`:

```rust
//! VFS-backed tool output cache.
//!
//! Stores large tool outputs under `vfs:///sys/tool_cache/<context>/<id>`,
//! written as SYSTEM_CALLER and world-readable. Replaces the old `cache.rs`
//! flat-file system.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a unique cache ID.
/// Format: `{tool}_{timestamp_hex}_{args_hash}` — globally unique, no collision risk.
pub fn generate_cache_id(tool_name: &str, args: &serde_json::Value) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut hasher = DefaultHasher::new();
    args.to_string().hash(&mut hasher);
    let hash = hasher.finish();

    format!("{}_{:x}_{:08x}", tool_name, timestamp, hash as u32)
}

/// Return the VFS path for a cache entry.
pub fn vfs_path_for(context_name: &str, cache_id: &str) -> String {
    format!("/sys/tool_cache/{}/{}", context_name, cache_id)
}

/// Check if content should be cached based on size threshold.
/// Does not cache empty or whitespace-only content.
pub fn should_cache(content: &str, threshold: usize) -> bool {
    if content.trim().is_empty() {
        return false;
    }
    content.len() > threshold
}

/// Generate the truncated stub message shown to the LLM instead of the full output.
pub fn truncated_message(
    vfs_uri: &str,
    tool_name: &str,
    content: &str,
    preview_chars: usize,
) -> String {
    let char_count = content.len();
    let token_estimate = char_count / 4;
    let line_count = content.lines().count();

    let preview: String = content.chars().take(preview_chars).collect();
    let preview = if let Some(pos) = preview.rfind('\n') {
        &preview[..pos]
    } else {
        &preview
    };

    format!(
        "[Output cached: {vfs_uri}]\n\
         Tool: {tool_name} | Size: {char_count} chars, ~{token_estimate} tokens | Lines: {line_count}\n\
         Preview:\n\
         ---\n\
         {preview}\n\
         ---\n\
         Use file_head, file_tail, file_lines, file_grep with path=\"{vfs_uri}\" to examine."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_cache_threshold() {
        assert!(!should_cache("short", 100));
        assert!(should_cache(&"x".repeat(101), 100));
    }

    #[test]
    fn test_should_cache_empty() {
        assert!(!should_cache("", 10));
        assert!(!should_cache("   ", 10));
        assert!(!should_cache("\n\n", 10));
    }

    #[test]
    fn test_generate_cache_id_format() {
        let id = generate_cache_id("web_fetch", &serde_json::json!({"url": "x"}));
        // Should be: web_fetch_{hex}_{hex}
        let parts: Vec<&str> = id.splitn(3, '_').collect();
        assert_eq!(parts[0], "web");   // "web_fetch" splits differently — adjust
        // Actually tool names with underscores are fine; just check prefix
        assert!(id.starts_with("web_fetch_"));
    }

    #[test]
    fn test_vfs_path_for() {
        let p = vfs_path_for("myctx", "tool_abc_123");
        assert_eq!(p, "/sys/tool_cache/myctx/tool_abc_123");
    }

    #[test]
    fn test_truncated_message_contains_uri() {
        let uri = "vfs:///sys/tool_cache/ctx/web_fetch_1_2";
        let msg = truncated_message(uri, "web_fetch", "line1\nline2\nline3", 200);
        assert!(msg.contains(uri));
        assert!(msg.contains("web_fetch"));
        assert!(msg.contains("file_head"));
    }

    #[test]
    fn test_truncated_message_preview_truncates_at_line() {
        let uri = "vfs:///sys/tool_cache/ctx/x";
        // Preview of 6 chars from "abc\ndef" should stop at newline → "abc"
        let msg = truncated_message(uri, "t", "abc\ndef\nghi", 6);
        assert!(msg.contains("abc"));
        assert!(!msg.contains("def"));
    }
}
```

**Step 4: Register `vfs_cache` in `lib.rs`**

In `crates/chibi-core/src/lib.rs`, add alongside `pub mod cache;`:

```rust
pub mod vfs_cache;
```

**Step 5: Run `vfs_cache` unit tests**

```bash
cargo test -p chibi-core vfs_cache
```

Expected: all tests pass (PASS).

**Step 6: Replace the caching block in `send.rs`**

Find the block starting at `// Check if output should be cached` (around line 1192). Replace the entire block:

```rust
// Check if output should be cached
let (final_result, was_cached) = if !tool_result.starts_with("Error:")
    && crate::vfs_cache::should_cache(&tool_result, resolved_config.tool_output_cache_threshold)
{
    // Fire pre_cache_output hook (can block caching)
    let pre_cache_data = serde_json::json!({
        "tool_name": tool_call.name,
        "output_size": tool_result.len(),
        "arguments": args,
    });
    let pre_cache_results =
        tools::execute_hook(tools, tools::HookPoint::PreCacheOutput, &pre_cache_data)?;
    let cache_blocked = pre_cache_results
        .iter()
        .any(|(_, r)| r.get_bool_or("block", false));

    if cache_blocked {
        if verbose {
            diagnostics.push(format!(
                "[Caching blocked by pre_cache_output hook for {}]",
                tool_call.name
            ));
        }
        (tool_result.clone(), false)
    } else {
        let cache_id = crate::vfs_cache::generate_cache_id(&tool_call.name, &args);
        let vfs_path_str = crate::vfs_cache::vfs_path_for(context_name, &cache_id);
        let vfs_path = crate::vfs::VfsPath::new(&vfs_path_str).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
        })?;

        match app
            .vfs
            .write(crate::vfs::SYSTEM_CALLER, &vfs_path, tool_result.as_bytes())
            .await
        {
            Ok(()) => {
                let vfs_uri = format!("vfs:/{}", vfs_path_str);
                let truncated = crate::vfs_cache::truncated_message(
                    &vfs_uri,
                    &tool_call.name,
                    &tool_result,
                    resolved_config.tool_cache_preview_chars,
                );

                if verbose {
                    diagnostics.push(format!(
                        "[Cached {} chars from {} at {}]",
                        tool_result.len(),
                        tool_call.name,
                        vfs_uri,
                    ));
                }

                // Fire post_cache_output hook (notification only)
                let post_cache_data = serde_json::json!({
                    "tool_name": tool_call.name,
                    "cache_id": cache_id,
                    "output_size": tool_result.len(),
                    "preview_size": truncated.len(),
                });
                let _ = tools::execute_hook(
                    tools,
                    tools::HookPoint::PostCacheOutput,
                    &post_cache_data,
                );

                (truncated, true)
            }
            Err(e) => {
                if verbose {
                    diagnostics.push(format!("[Failed to cache output: {}]", e));
                }
                (tool_result.clone(), false)
            }
        }
    }
} else {
    (tool_result.clone(), false)
};
```

> Note: `vfs:///` uses three slashes — scheme + empty authority + absolute path. Double-check the `vfs_uri` format matches what `VfsPath::is_vfs_uri` and `VfsPath::from_uri` expect in `file_tools.rs`.

**Step 7: Remove the old `use crate::cache;` import from `send.rs`**

Find and delete the line:
```rust
use crate::cache;
```

**Step 8: Build to check for compile errors**

```bash
cargo build -p chibi-core 2>&1 | head -50
```

Expected: compiles (may have unused import warnings from `cache.rs` still existing — that's fine).

**Step 9: Commit**

```bash
git add crates/chibi-core/src/vfs_cache.rs crates/chibi-core/src/lib.rs crates/chibi-core/src/api/send.rs
git commit -m "feat(cache): replace file-based cache write with VFS at /sys/tool_cache"
```

---

## Task 2: Replace cleanup and clear on `AppState` with VFS-backed versions

**Files:**
- Modify: `crates/chibi-core/src/state/mod.rs`
- Modify: `crates/chibi-core/src/execution.rs`

**Step 1: Read the current cleanup methods**

Read `crates/chibi-core/src/state/mod.rs` lines 327–360 to see `ensure_tool_cache_dir`, `clear_tool_cache`, `cleanup_tool_cache`, `cleanup_all_tool_caches`.

**Step 2: Write failing tests**

Add to the `#[cfg(test)]` section at the bottom of `state/mod.rs`:

```rust
#[tokio::test]
async fn test_clear_tool_cache_via_vfs() {
    let app = make_test_app();
    let ctx = "cleanup-ctx";

    // Write a fake cache entry
    let path = crate::vfs::VfsPath::new(&format!("/sys/tool_cache/{}/entry1", ctx)).unwrap();
    app.vfs
        .write(crate::vfs::SYSTEM_CALLER, &path, b"data")
        .await
        .unwrap();

    app.clear_tool_cache(ctx).await.unwrap();

    let exists = app.vfs
        .exists(crate::vfs::SYSTEM_CALLER, &path)
        .await
        .unwrap();
    assert!(!exists, "cache entry should be deleted after clear");
}

#[tokio::test]
async fn test_cleanup_old_tool_caches_removes_expired() {
    let app = make_test_app();
    let ctx = "cleanup-ctx2";

    // Write a cache entry
    let path = crate::vfs::VfsPath::new(&format!("/sys/tool_cache/{}/entry1", ctx)).unwrap();
    app.vfs
        .write(crate::vfs::SYSTEM_CALLER, &path, b"old data")
        .await
        .unwrap();

    // max_age_days=0 means delete entries older than 1 day
    // Since this entry was just created, it should NOT be deleted
    let removed = app.cleanup_all_tool_caches(0).await.unwrap();
    assert_eq!(removed, 0, "fresh entry should not be cleaned up");
}
```

**Step 3: Run to verify tests fail**

```bash
cargo test -p chibi-core test_clear_tool_cache_via_vfs 2>&1 | tail -20
```

Expected: compile error — `clear_tool_cache` is currently sync and doesn't use VFS.

**Step 4: Replace cleanup methods in `state/mod.rs`**

Find and replace the four methods (lines ~327–357):

```rust
/// Clear the tool cache for a context (deletes all entries from VFS).
pub async fn clear_tool_cache(&self, name: &str) -> std::io::Result<()> {
    let path_str = crate::vfs_cache::vfs_path_for(name, "");
    // vfs_path_for returns "/sys/tool_cache/<ctx>/", drop trailing "/"
    let dir_str = path_str.trim_end_matches('/');
    let dir = crate::vfs::VfsPath::new(dir_str).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
    })?;
    // delete is a no-op if the directory doesn't exist; backend returns NotFound
    match self.vfs.delete(crate::vfs::SYSTEM_CALLER, &dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Clean up tool cache entries older than `max_age_days` for a single context.
/// Returns the number of entries removed.
pub async fn cleanup_tool_cache(
    &self,
    context_name: &str,
    max_age_days: u64,
) -> std::io::Result<usize> {
    use chrono::Utc;

    let dir_str = format!("/sys/tool_cache/{}", context_name);
    let dir = match crate::vfs::VfsPath::new(&dir_str) {
        Ok(p) => p,
        Err(_) => return Ok(0),
    };

    let entries = match self.vfs.list(crate::vfs::SYSTEM_CALLER, &dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };

    // +1 so max_age_days=0 means <1 day (not "delete immediately")
    let max_age = chrono::Duration::days((max_age_days + 1) as i64);
    let cutoff = Utc::now() - max_age;

    let mut removed = 0;
    for entry in entries {
        if entry.kind != crate::vfs::VfsEntryKind::File {
            continue;
        }
        let file_path_str = format!("{}/{}", dir_str, entry.name);
        let file_path = match crate::vfs::VfsPath::new(&file_path_str) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let meta = match self.vfs.metadata(crate::vfs::SYSTEM_CALLER, &file_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if let Some(created) = meta.created {
            if created < cutoff {
                let _ = self.vfs.delete(crate::vfs::SYSTEM_CALLER, &file_path).await;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Clean up tool cache entries for all contexts.
/// Returns total number of entries removed.
pub async fn cleanup_all_tool_caches(&self, max_age_days: u64) -> std::io::Result<usize> {
    let root = crate::vfs::VfsPath::new("/sys/tool_cache").map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
    })?;

    let ctx_dirs = match self.vfs.list(crate::vfs::SYSTEM_CALLER, &root).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };

    let mut total = 0;
    for dir in ctx_dirs {
        if dir.kind == crate::vfs::VfsEntryKind::Directory {
            total += self.cleanup_tool_cache(&dir.name, max_age_days).await?;
        }
    }
    Ok(total)
}
```

Also delete `ensure_tool_cache_dir` entirely (no longer needed).

**Step 5: Update callers of the now-async methods**

In `execution.rs` (around line 116), the cleanup call is sync. Change it to `.await`:

```rust
if cleanup_config.auto_cleanup_cache {
    let removed = chibi
        .app
        .cleanup_all_tool_caches(cleanup_config.tool_cache_max_age_days)
        .await?;
```

But `run_command` in `execution.rs` may not be async — check the function signature. If it's sync, wrap with `tokio::runtime::Handle::current().block_on(...)`:

```rust
let removed = tokio::runtime::Handle::current()
    .block_on(chibi.app.cleanup_all_tool_caches(cleanup_config.tool_cache_max_age_days))?;
```

Similarly update `clear_tool_cache` call at line ~286 in `execution.rs`.

**Step 6: Build**

```bash
cargo build -p chibi-core 2>&1 | head -60
```

Fix any remaining compile errors. Common issues: `VfsEntryKind` not in scope (import `crate::vfs::VfsEntryKind`), chrono `Duration` ambiguity.

**Step 7: Run the new tests**

```bash
cargo test -p chibi-core test_clear_tool_cache_via_vfs test_cleanup_old_tool_caches
```

Expected: PASS.

**Step 8: Commit**

```bash
git add crates/chibi-core/src/state/mod.rs crates/chibi-core/src/execution.rs
git commit -m "feat(cache): replace AppState cache cleanup with async VFS-backed versions"
```

---

## Task 3: Remove `cache_id` param from file tools and simplify `resolve_file_path`

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs`

**Step 1: Read the current tool definitions and `resolve_file_path`**

Read `file_tools.rs` lines 1–260 (already loaded above). The four affected tools are `file_head`, `file_tail`, `file_lines`, `file_grep`. Each has a `cache_id` property and the `path` description mentions `cache_id`.

**Step 2: Write failing tests for the simplified resolver**

In the `#[cfg(test)]` section of `file_tools.rs`, add:

```rust
#[test]
fn test_resolve_file_path_requires_path() {
    // After removing cache_id, passing no args should return an error
    let app = make_test_app_for_file_tools();
    let config = test_resolved_config();
    let args = serde_json::json!({});
    let result = resolve_file_path(&app, "ctx", &args, &config, Path::new("/tmp"));
    assert!(result.is_err());
}

#[test]
fn test_resolve_file_path_vfs_uri() {
    let app = make_test_app_for_file_tools();
    let config = test_resolved_config();
    let args = serde_json::json!({"path": "vfs:///sys/tool_cache/ctx/entry1"});
    let result = resolve_file_path(&app, "ctx", &args, &config, Path::new("/tmp")).unwrap();
    assert!(matches!(result, ResolvedPath::Vfs(_)));
}
```

**Step 3: Update `FILE_TOOL_DEFS` — remove `cache_id` property from all four tools**

For each of `file_head`, `file_tail`, `file_lines`, `file_grep`: delete the `ToolPropertyDef` block for `cache_id`. Update the `path` description to:

```rust
description: "Absolute or relative path to a file, or a vfs:/// URI for VFS storage",
```

Remove `summary_params: &["path"]` — it should stay, but verify `cache_id` is not referenced there.

**Step 4: Update `CACHE_LIST_TOOL_NAME` — remove it**

Delete:
```rust
pub const CACHE_LIST_TOOL_NAME: &str = "cache_list";
```

And delete the entire `BuiltinToolDef` entry for `cache_list` from `FILE_TOOL_DEFS`.

**Step 5: Simplify `resolve_file_path`**

Replace the function body to remove the `cache_id` branch entirely:

```rust
fn resolve_file_path(
    app: &AppState,
    _context_name: &str,
    args: &serde_json::Value,
    config: &ResolvedConfig,
    project_root: &Path,
) -> io::Result<ResolvedPath> {
    let path = args.get_str("path").ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidInput, "Must provide path")
    })?;

    if VfsPath::is_vfs_uri(path) {
        let vfs_path = VfsPath::from_uri(path)?;
        return Ok(ResolvedPath::Vfs(vfs_path));
    }

    let resolved_path = if Path::new(path).is_relative() {
        project_root.join(path).to_string_lossy().to_string()
    } else {
        path.to_string()
    };
    let resolved = super::security::validate_file_path(&resolved_path, config)?;
    Ok(ResolvedPath::Os(resolved))
}
```

**Step 6: Remove `execute_cache_list` and `use crate::cache;`**

Delete the entire `execute_cache_list` function and the `use crate::cache;` import at the top of the file.

**Step 7: Remove `cache_list` from tool dispatch in `send.rs`**

Find the match arm for `CACHE_LIST_TOOL_NAME` in `send.rs` (search for `cache_list`). Delete it.

Also remove `use crate::tools::file_tools::CACHE_LIST_TOOL_NAME;` if imported.

**Step 8: Build**

```bash
cargo build -p chibi-core 2>&1 | head -60
```

**Step 9: Run file tools tests**

```bash
cargo test -p chibi-core file_tools
```

Expected: all pass. Remove any tests that specifically tested `cache_id` param (they were testing the old path — grep for `cache_id` in test blocks).

**Step 10: Commit**

```bash
git add crates/chibi-core/src/tools/file_tools.rs crates/chibi-core/src/api/send.rs
git commit -m "refactor(file-tools): remove cache_id param and cache_list tool, path= is the only resolver"
```

---

## Task 4: Remove cache path helpers from `AppState` and `state/paths.rs`

**Files:**
- Modify: `crates/chibi-core/src/state/paths.rs`
- Modify: `crates/chibi-core/src/state/mod.rs`

**Step 1: Delete from `paths.rs`**

Remove the three methods from the `StatePaths` trait:
- `tool_cache_dir`
- `cache_file`
- `cache_meta_file`

**Step 2: Build to find all callers**

```bash
cargo build -p chibi-core 2>&1 | grep "error"
```

Fix each compile error. Expected remaining callers: none (Task 2 already removed the methods from `mod.rs`, Task 3 removed the use in `file_tools.rs`).

**Step 3: Check for references to `StatePaths` in tests**

```bash
cargo test -p chibi-core 2>&1 | grep "FAILED\|error"
```

Fix any test referencing `tool_cache_dir`, `cache_file`, or `cache_meta_file`.

**Step 4: Commit**

```bash
git add crates/chibi-core/src/state/paths.rs crates/chibi-core/src/state/mod.rs
git commit -m "refactor(state): remove tool_cache_dir, cache_file, cache_meta_file path helpers"
```

---

## Task 5: Delete `cache.rs`

**Files:**
- Delete: `crates/chibi-core/src/cache.rs`
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: Verify no remaining references**

```bash
grep -r "crate::cache\|use crate::cache\|mod cache" crates/chibi-core/src/ --include="*.rs"
```

Expected: zero results (all removed in Tasks 1–4).

**Step 2: Remove `pub mod cache;` from `lib.rs`**

Find and delete:
```rust
pub mod cache;
```

**Step 3: Delete the file**

```bash
rm crates/chibi-core/src/cache.rs
```

**Step 4: Full build + test**

```bash
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor(cache): delete cache.rs — tool cache now lives in VFS"
```

---

## Task 6: Update docs and tool descriptions

**Files:**
- Modify: `docs/vfs.md`
- Modify: `docs/hooks.md` (if it mentions `cache_id`)
- Modify: `crates/chibi-core/src/tools/file_tools.rs` (tool descriptions)

**Step 1: Search for stale cache_id references in docs**

```bash
grep -r "cache_id\|cache_list\|tool_cache_dir\|\.cache\b" docs/ --include="*.md"
```

**Step 2: Update `docs/vfs.md`**

Add a section under `## namespace layout`:

```markdown
/sys/tool_cache/<context>/   read only (SYSTEM-populated, world-readable)
```

Update the namespace table to include this. Mention that tool outputs cached by the system appear here and can be accessed via `file_head`, `file_tail`, `file_lines`, `file_grep` with `path="vfs:///sys/tool_cache/<ctx>/<id>"`.

**Step 3: Update `docs/hooks.md` if needed**

If `pre_cache_output`/`post_cache_output` hook docs reference `cache_id` as an OS path, update to note it's now a VFS path string.

**Step 4: Update file tool descriptions**

In `file_tools.rs`, update the top-level module doc comment:

```rust
//! File access tools for reading files and cached tool outputs via VFS.
//!
//! Large tool outputs are cached automatically by the system under
//! `vfs:///sys/tool_cache/<context>/<id>`. Use these tools with
//! `path="vfs:///sys/tool_cache/..."` to examine cached content.
```

**Step 5: Build + test**

```bash
cargo test -p chibi-core
```

Expected: all pass.

**Step 6: Commit**

```bash
git add docs/ crates/chibi-core/src/tools/file_tools.rs
git commit -m "docs: update vfs.md and file_tools for VFS-backed tool cache"
```

---

## Task 7: Integration test — end-to-end cache flow via VFS

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (test section)

Add a comprehensive integration test that exercises the full cache flow:

**Step 1: Write the test**

In `send.rs` test section:

```rust
#[tokio::test]
async fn test_full_cache_flow_via_vfs() {
    // Build a minimal Chibi app + context
    let (app, ctx_name) = test_helpers::make_app_with_context("vfs-cache-flow");
    let resolved = ResolvedConfig {
        tool_output_cache_threshold: 10, // tiny threshold for testing
        tool_cache_preview_chars: 5,
        ..ResolvedConfig::test_default()
    };

    // Simulate what happens when a tool returns large output
    let large = "abcdefghijklmnop"; // 16 chars > threshold of 10
    let cache_id = crate::vfs_cache::generate_cache_id("test_tool", &serde_json::json!({}));
    let vfs_path_str = crate::vfs_cache::vfs_path_for(&ctx_name, &cache_id);
    let vfs_uri = format!("vfs:/{}", vfs_path_str);
    let vfs_path = crate::vfs::VfsPath::new(&vfs_path_str).unwrap();

    app.vfs
        .write(crate::vfs::SYSTEM_CALLER, &vfs_path, large.as_bytes())
        .await
        .unwrap();

    let stub = crate::vfs_cache::truncated_message(&vfs_uri, "test_tool", large, 5);

    // Stub references the VFS URI
    assert!(stub.contains(&vfs_uri));
    assert!(stub.contains("test_tool"));

    // LLM can access the content via vfs:/// path
    let content = app.vfs.read(&ctx_name, &vfs_path).await.unwrap();
    assert_eq!(content, large.as_bytes());

    // Cleanup removes the entry when expired (max_age_days=0, entry just created → not removed)
    let removed = app.cleanup_all_tool_caches(0).await.unwrap();
    assert_eq!(removed, 0);

    // Clear removes the context directory
    app.clear_tool_cache(&ctx_name).await.unwrap();
    assert!(!app.vfs.exists(crate::vfs::SYSTEM_CALLER, &vfs_path).await.unwrap());
}
```

**Step 2: Run**

```bash
cargo test -p chibi-core test_full_cache_flow_via_vfs -- --nocapture
```

Expected: PASS.

**Step 3: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "test(cache): add end-to-end VFS cache flow integration test"
```

---

## Final Check

```bash
cargo test -p chibi-core
just pre-push
```

All tests pass. No references to old `cache.rs` symbols remain. Done!
