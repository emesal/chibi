# Tool Cache → VFS Migration

**Date:** 2026-02-18
**Status:** approved

## Problem

The tool output cache is a parallel storage system living alongside the VFS in `~/.chibi/contexts/<ctx>/tool_cache/`. It has its own path resolution logic (`cache_id` param), its own read helpers (`cache.rs`), its own cleanup (`cleanup_old_cache`, `cleanup_all_tool_caches`), its own metadata sidecar files (`.meta.json`), and a separate `cache_id` shorthand parameter on file tools — all duplicating structure already provided by the VFS.

## Decision

Move tool cache storage entirely into the VFS at `/sys/tool_cache/<context>/<cache_id>`. Delete `cache.rs` and all cache-specific path machinery. Use the VFS as the single storage abstraction.

## Design

### Storage Layout

```
vfs:///sys/tool_cache/<context>/<cache_id>
```

- Written by `SYSTEM_CALLER` — only system code can create/delete cache entries
- World-readable — all contexts can examine any context's cache via `file_head`, `file_grep`, etc.
- Context-scoped subdirectory — cleanup and listing remain per-context
- Mount-friendly — `/sys/tool_cache/` can be given its own backend later (longest-prefix match)

No sidecar `.meta.json` files. Metadata for cleanup (age) comes from `VfsMetadata::created` via the backend. All other metadata (tool name, size, token estimate, line count) is embedded in the truncated message at write time and never needs to be re-parsed.

### Cache ID Format

Unchanged: `{tool}_{timestamp}_{hash}` — already globally unique, no collision risk.

### Truncated Message

The stub shown to the LLM changes from referencing a `cache_id` to a `vfs:///` URI:

```
[Output cached: vfs:///sys/tool_cache/myctx/web_fetch_1234_abc]

Tool: web_fetch | Size: 52000 chars, ~13000 tokens | Lines: 847

Preview:

---
<first N chars>
---

Use file_head, file_tail, file_lines, file_grep with path="vfs:///sys/tool_cache/myctx/web_fetch_1234_abc" to examine.
```

The LLM accesses cached content via the existing `path` param on file tools — no special `cache_id` shorthand needed.

### Removed: `cache_id` Parameter

The `cache_id` parameter is removed from `file_head`, `file_tail`, `file_lines`, `file_grep`. The `path` parameter already accepts `vfs:///` URIs. `resolve_file_path` in `file_tools.rs` loses the `cache_id` branch and becomes a simple path-or-vfs resolver.

### Removed: `cache_list` Tool

Replaced by `vfs_list` on `vfs:///sys/tool_cache/<context>/`. No dedicated tool needed.

### Cleanup

`cleanup_old_cache` and `cleanup_all_tool_caches` on `AppState` are replaced by a new helper that iterates `vfs.list(SYSTEM_CALLER, "/sys/tool_cache/<ctx>")`, calls `vfs.metadata(...)` for `created`, and `vfs.delete(SYSTEM_CALLER, ...)` for entries older than `tool_cache_max_age_days`. Lives in `state/mod.rs` or a small `cache_cleanup` helper module — not in `cache.rs` (deleted).

`clear_tool_cache` similarly calls `vfs.delete(SYSTEM_CALLER, "/sys/tool_cache/<ctx>")`.

### Write Path (`api/send.rs`)

`cache_output(...)` in `send.rs` is replaced by direct VFS calls:

```rust
let vfs_path = VfsPath::new(&format!("/sys/tool_cache/{}/{}", context_name, cache_id))?;
app.vfs.write(SYSTEM_CALLER, &vfs_path, tool_result.as_bytes()).await?;
```

`send.rs` is already fully async, so no blocking needed.

### Deleted

- `crates/chibi-core/src/cache.rs` — entirely
- `AppState::tool_cache_dir`, `cache_file`, `cache_meta_file` (in `state/paths.rs`)
- `AppState::ensure_tool_cache_dir`, `clear_tool_cache`, `cleanup_tool_cache`, `cleanup_all_tool_caches` (in `state/mod.rs`)
- `cache_id` param from `file_head`, `file_tail`, `file_lines`, `file_grep` tool schemas
- `CACHE_LIST_TOOL_NAME` and `execute_cache_list` from `file_tools.rs`
- `cache_list` from tool dispatch in `send.rs`

### Storage Layout Change

```
before:  ~/.chibi/contexts/<ctx>/tool_cache/<id>.cache
         ~/.chibi/contexts/<ctx>/tool_cache/<id>.meta.json

after:   ~/.chibi/vfs/sys/tool_cache/<ctx>/<id>
```

On-disk location changes but the VFS backend (`LocalBackend`) handles this transparently.

## Migration

Pre-alpha — no migration of existing cache files needed. Old `.cache`/`.meta.json` files in `~/.chibi/contexts/*/tool_cache/` are simply orphaned and can be deleted manually or via a one-time cleanup note in the changelog.

## Testing

- Existing `cache.rs` tests → deleted
- New integration tests in `send.rs` covering: cache write lands at correct VFS path, truncated message contains correct `vfs:///` URI, file tools read from VFS path, cleanup removes entries older than threshold, `clear_tool_cache` deletes context subtree
- `file_tools.rs` tests updated to remove all `cache_id` param variants
- `AppState` tests updated to remove cache path helpers
