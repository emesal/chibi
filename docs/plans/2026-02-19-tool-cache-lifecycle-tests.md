# test: tool cache lifecycle (#172)

## context

tool output caching (`vfs_cache.rs` helpers + `AppState` methods) has one monolithic test in `send.rs`. the pure helpers are well-covered, but the **lifecycle** — write → read hit, cache miss, per-context isolation, TTL expiry, clear — lacks granular coverage.

## approach: extract + test (same pattern as #168)

### extract `is_cache_entry_expired` helper

`cleanup_tool_cache` inlines its age comparison. extract it so TTL logic is unit-testable without filesystem timestamp manipulation:

```rust
/// check whether a cache entry's creation timestamp is older than `max_age_days`.
/// the `+1` offset means `max_age_days=0` tolerates entries less than 1 day old.
pub(crate) fn is_cache_entry_expired(
    created: chrono::DateTime<chrono::Utc>,
    max_age_days: u64,
) -> bool {
    let max_age = chrono::Duration::days((max_age_days + 1) as i64);
    let cutoff = chrono::Utc::now() - max_age;
    created < cutoff
}
```

then `cleanup_tool_cache` calls this instead of inline logic.

### tests in `state/mod.rs` (near cache methods)

**`is_cache_entry_expired` tests:**
- fresh timestamp (now) with `max_age_days=0` → not expired
- timestamp 3 days ago with `max_age_days=7` → not expired
- timestamp 10 days ago with `max_age_days=7` → expired
- `max_age_days=0` boundary: 2 days ago → expired, 0 days ago → not expired

**cache lifecycle tests (async, use `AppState` + VFS):**
- write + read hit: write entry via VFS, read back, verify content matches
- cache miss: read non-existent path → `NotFound` error
- per-context isolation: write to context A, verify not visible in context B listing
- `clear_tool_cache`: write entries, clear, verify gone
- `cleanup_tool_cache` fresh entries: write entry, cleanup with `max_age_days=0` → 0 removed
- `cleanup_all_tool_caches`: write to two contexts, cleanup → 0 removed for fresh entries

### files modified

- `crates/chibi-core/src/state/mod.rs` — extract `is_cache_entry_expired`, add tests

### deferred

- `execute_command` auto-cleanup integration (needs full `Chibi` instance — defer to sandbox harness)
- TTL expiry with real old files (filesystem birth-time manipulation isn't portable)

### verification

```bash
cargo test -p chibi-core cache
cargo test -p chibi-core
```
