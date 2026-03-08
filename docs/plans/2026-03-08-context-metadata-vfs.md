# Context Metadata VFS Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

## Progress

**Tasks 1–5 complete** (committed on branch `feature/context-metadata-vfs-2603`):
- ✅ Task 1: `prompt_count` in `PartitionMeta` + `ActiveState` — commit `c67b2c94`
- ✅ Task 2: `AppState::prompt_count` + `chibi -l` shows "Prompts: N" — commit `ddffaa56`
- ✅ Task 3: `ReadOnlyVfsBackend` trait with blanket impl — commit `3714c687`
- ✅ Task 4: `ToolsBackend` migrated to `ReadOnlyVfsBackend` — commit `36a88c47`
- ✅ Task 5: `AppState.state` → `Arc<RwLock<ContextState>>` — commit `372dac41`

**Remaining: Tasks 6, 7, 8, 9**

### Implementation notes from completed tasks
- `BoxFuture` is `pub(super)` — accessible within the `vfs` module (all backends live there, fine)
- `ToolsBackend` test for `write()` needs `Box<dyn VfsBackend>` cast to disambiguate from `ReadOnlyVfsBackend::write` — see the pattern in `tools_backend.rs`
- `AppState.state` mutation sites ended up being 7 total (not 5) — `context_ops.rs` has additional sites: `destroy_context`, `rename_context`, `list_contexts`, `save_and_register_context`
- `Manifest` is already `pub` and `pub use`'d from `partition` module — `ContextsBackend` can import it directly
- `PartitionManager::load` is `#[cfg(test)]` only — `ContextsBackend` must use `PartitionManager::load_with_config` with `StorageConfig::default()` instead

---

**Goal:** Expose context metadata as read-only virtual files under `/sys/contexts/<name>/` (issue #187)

**Architecture:** `ReadOnlyVfsBackend` sub-trait with blanket `VfsBackend` impl handles write rejection for all virtual backends. `ContextsBackend` implements it, mounted at `/sys/contexts/`. `prompt_count` added to partition tracking for efficient turn counting. `AppState.state` is refactored from `ContextState` to `Arc<RwLock<ContextState>>` (single source of truth — no parallel sync field).

**Tech Stack:** Rust, async traits, serde, JSONL partitions

**Verified architecture facts:**
- `ENTRY_TYPE_MESSAGE` is defined in `context.rs` (value: `"message"`)
- `AppState` constructors are `from_dir` (test) and `load` (production) — not `new`/`new_with_options`
- `AppState.state` mutation sites: `sync_state_with_filesystem` (retain, push, sort), `touch_context_with_destroy_settings`, `auto_destroy_expired_contexts` — exactly 5 call sites
- VFS routing strips mount prefix before passing path to backend (`/sys/contexts/foo` → `/foo`)
- `site_flock_name`, `resolve_flock_vfs_root`, `FlockRegistry::flocks_for` are all public

---

### ✅ Task 1: Add `prompt_count` to `PartitionMeta` and `ActiveState`

**Files:**
- Modify: `crates/chibi-core/src/partition.rs`

**Step 1: Add `prompt_count` field to `PartitionMeta`**

In `PartitionMeta` (around line 227), add after `entry_count`:

```rust
/// Number of user prompt entries in this partition.
#[serde(default)]
pub prompt_count: usize,
```

**Step 2: Add `prompt_count` field to `ActiveState`**

In `ActiveState` (around line 291), add after `entry_count`:

```rust
/// Number of user prompt entries in active partition.
prompt_count: usize,
```

**Step 3: Add public accessor on `ActiveState`**

Add a public getter so `ContextsBackend` can read it without exposing the full struct:

```rust
pub fn prompt_count(&self) -> usize {
    self.prompt_count
}
```

**Step 4: Update `ActiveState::from_file` to count prompts**

In `from_file()` (around line 304), add a `prompt_count` counter. Inside the entry parse loop, after `count += 1;`:

```rust
if entry.entry_type == ENTRY_TYPE_MESSAGE
    && entry.role.as_deref() == Some("user")
{
    prompt_count += 1;
}
```

Include `prompt_count` in the returned `Self`. Add `use crate::context::ENTRY_TYPE_MESSAGE;` at the top of the file if not already imported (it is defined in `context.rs`).

**Step 5: Update `ActiveState::record_append` to track prompts**

In `record_append()` (around line 337), add after `self.entry_count += 1;`:

```rust
if entry.entry_type == ENTRY_TYPE_MESSAGE
    && entry.role.as_deref() == Some("user")
{
    self.prompt_count += 1;
}
```

**Step 6: Update `ActiveState::reset`**

In `reset()` (around line 348), add:

```rust
self.prompt_count = 0;
```

**Step 7: Update `PartitionManager::rotate` to record `prompt_count`**

In `rotate()` (around line 750), where `PartitionMeta` is constructed, add the prompt count from the in-memory `ActiveState` before reset (it reflects the entries being rotated):

```rust
let prompt_count = self.active.prompt_count;
```

Add `prompt_count` to the `PartitionMeta { ... }` struct literal.

**Step 8: Add public accessor `total_prompt_count` on `PartitionManager`**

After `total_entry_count()` (line ~882), add:

```rust
/// Returns the total prompt count across all partitions.
pub fn total_prompt_count(&self) -> usize {
    let archived: usize = self.manifest.partitions.iter().map(|p| p.prompt_count).sum();
    archived + self.active.prompt_count
}
```

**Step 9: Write tests**

Add tests in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn test_prompt_count_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

    // Regular entry (not a user prompt)
    let entry = make_entry("tool output");
    pm.append_entry(&entry).unwrap();
    assert_eq!(pm.total_prompt_count(), 0);

    // User prompt entry
    let prompt = TranscriptEntry::builder()
        .from("user")
        .to("default")
        .content("hello")
        .entry_type(ENTRY_TYPE_MESSAGE)
        .role("user")
        .build();
    pm.append_entry(&prompt).unwrap();
    assert_eq!(pm.total_prompt_count(), 1);

    // Another non-prompt message (assistant)
    let assistant = TranscriptEntry::builder()
        .from("default")
        .to("user")
        .content("hi there")
        .entry_type(ENTRY_TYPE_MESSAGE)
        .role("assistant")
        .build();
    pm.append_entry(&assistant).unwrap();
    assert_eq!(pm.total_prompt_count(), 1);
}

#[test]
fn test_prompt_count_survives_rotation() {
    let temp_dir = TempDir::new().unwrap();
    let config = StorageConfig {
        partition_max_entries: Some(2),
        partition_max_age_seconds: Some(86400),
        partition_max_tokens: None,
        bytes_per_token: None,
        enable_bloom_filters: Some(false),
    };
    let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

    let prompt = TranscriptEntry::builder()
        .from("user")
        .to("default")
        .content("hello")
        .entry_type(ENTRY_TYPE_MESSAGE)
        .role("user")
        .build();
    pm.append_entry(&prompt).unwrap();
    pm.append_entry(&make_entry("response")).unwrap();
    pm.rotate_if_needed().unwrap();

    // Prompt count preserved in archived partition
    assert_eq!(pm.manifest.partitions[0].prompt_count, 1);
    assert_eq!(pm.total_prompt_count(), 1);

    // Add another prompt post-rotation
    pm.append_entry(&prompt).unwrap();
    assert_eq!(pm.total_prompt_count(), 2);
}

#[test]
fn test_prompt_count_serde_default() {
    // Old manifests without prompt_count should default to 0
    let json = r#"{"file":"test.jsonl","start_ts":1000,"end_ts":2000,"entry_count":10}"#;
    let meta: PartitionMeta = serde_json::from_str(json).unwrap();
    assert_eq!(meta.prompt_count, 0);
}
```

Note: verify that `make_entry` (the test helper) does NOT set `role: Some("user")`, otherwise the "tool output" entry would be counted as a prompt. Adjust the test if needed.

**Step 10: Run tests**

Run: `cargo test -p chibi-core partition`
Expected: all tests pass, including the 3 new ones.

**Step 11: Commit**

```
feat(partition): track prompt_count in PartitionMeta and ActiveState

adds per-partition count of user prompt entries (entry_type="message",
role="user") alongside the existing entry_count. serde(default) for
backwards compat with existing manifests.

part of #187
```

---

### ✅ Task 2: Add `AppState::prompt_count` helper and fix `chibi -l`

**Files:**
- Modify: `crates/chibi-core/src/state/mod.rs`
- Modify: `crates/chibi-core/src/execution.rs` (around line 178)

**Step 1: Add `prompt_count` method to `AppState`**

Near `read_transcript_entries()` (around line 622), add:

```rust
/// Returns the total number of user prompts for a context.
///
/// Sums prompt counts across all archived partitions and the active
/// partition. Uses cached active state when available.
pub fn prompt_count(&self, name: &str) -> io::Result<usize> {
    self.migrate_transcript_if_needed(name)?;
    let transcript_dir = self.transcript_dir(name);
    let storage_config = self.resolve_config(name, None)?.storage;
    let cached_state = self.active_state_cache.borrow().get(name).cloned();
    let pm = PartitionManager::load_with_cached_state(
        &transcript_dir,
        storage_config,
        cached_state,
    )?;
    Ok(pm.total_prompt_count())
}
```

Note: `AppState.state` will be `Arc<RwLock<ContextState>>` after Task 6. This method does not access `self.state` directly, so it is unaffected by that refactor.

**Step 2: Update `chibi -l` output**

In `execution.rs` around line 178, change:

```rust
output.emit_result(&format!("Messages: {}", ctx.messages.len()));
```

to:

```rust
let prompt_count = chibi.app.prompt_count(context).unwrap_or(0);
output.emit_result(&format!("Prompts: {}", prompt_count));
```

**Step 3: Run tests**

Run: `cargo test -p chibi-core`
Expected: all pass. Check for any `ListCurrentContext` snapshot or integration test that asserts "Messages:" and update it to "Prompts:".

**Step 4: Commit**

```
feat(cli): show prompt count from transcript in chibi -l

replaces working-memory message count with actual user prompt count
from the transcript. adds AppState::prompt_count() helper.

part of #187
```

---

### ✅ Task 3: Introduce `ReadOnlyVfsBackend` trait with blanket impl

**Files:**
- Modify: `crates/chibi-core/src/vfs/backend.rs`

**Step 1: Write the trait and blanket impl**

Add after the `VfsBackend` trait definition (after line 74):

```rust
/// Sub-trait for read-only virtual VFS backends.
///
/// Implementors provide only the read operations (`read`, `list`, `exists`,
/// `metadata`). The blanket `VfsBackend` impl fills in all write operations
/// (`write`, `append`, `delete`, `mkdir`, `copy`, `rename`) with
/// `PermissionDenied` errors that include `backend_name()` for diagnostics.
pub trait ReadOnlyVfsBackend: Send + Sync {
    /// Human-readable name for error messages (e.g. "virtual tool registry").
    fn backend_name(&self) -> &str;

    /// Read the full contents of a virtual file.
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>>;

    /// List entries in a virtual directory.
    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>>;

    /// Check whether a virtual path exists.
    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>>;

    /// Get metadata for a virtual path.
    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>>;
}

impl<T: ReadOnlyVfsBackend> VfsBackend for T {
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        ReadOnlyVfsBackend::read(self, path)
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        ReadOnlyVfsBackend::list(self, path)
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        ReadOnlyVfsBackend::exists(self, path)
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        ReadOnlyVfsBackend::metadata(self, path)
    }

    fn write<'a>(&'a self, path: &'a VfsPath, _data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path, name),
            ))
        })
    }

    fn append<'a>(&'a self, path: &'a VfsPath, _data: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path, name),
            ))
        })
    }

    fn delete<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path, name),
            ))
        })
    }

    fn mkdir<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", path, name),
            ))
        })
    }

    fn copy<'a>(&'a self, src: &'a VfsPath, _dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", src, name),
            ))
        })
    }

    fn rename<'a>(&'a self, src: &'a VfsPath, _dst: &'a VfsPath) -> BoxFuture<'a, io::Result<()>> {
        let name = self.backend_name();
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("'{}' is read-only ({})", src, name),
            ))
        })
    }
}
```

**Step 2: Export the trait**

In `crates/chibi-core/src/vfs/mod.rs`, add to the `pub use` block:

```rust
pub use backend::ReadOnlyVfsBackend;
```

**Step 3: Write tests**

Add a new `#[cfg(test)] mod tests` block at the bottom of `backend.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::types::{VfsEntry, VfsEntryKind, VfsMetadata};
    use crate::vfs::path::VfsPath;

    /// Minimal read-only backend for testing the blanket impl.
    struct StubBackend;

    impl ReadOnlyVfsBackend for StubBackend {
        fn backend_name(&self) -> &str {
            "test stub"
        }

        fn read<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
            Box::pin(async { Ok(b"hello".to_vec()) })
        }

        fn list<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn exists<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
            Box::pin(async { Ok(true) })
        }

        fn metadata<'a>(&'a self, _path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
            Box::pin(async {
                Ok(VfsMetadata {
                    size: 5,
                    created: None,
                    modified: None,
                    kind: VfsEntryKind::File,
                })
            })
        }
    }

    #[tokio::test]
    async fn test_read_only_backend_read_delegates() {
        let backend: &dyn VfsBackend = &StubBackend;
        let path = VfsPath::new("/test").unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn test_read_only_backend_write_rejected() {
        let backend: &dyn VfsBackend = &StubBackend;
        let path = VfsPath::new("/test").unwrap();
        let err = backend.write(&path, b"data").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("test stub"));
    }

    #[tokio::test]
    async fn test_read_only_backend_all_writes_rejected() {
        let backend: &dyn VfsBackend = &StubBackend;
        let p = VfsPath::new("/x").unwrap();
        let p2 = VfsPath::new("/y").unwrap();

        assert_eq!(backend.append(&p, b"d").await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.delete(&p).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.mkdir(&p).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.copy(&p, &p2).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(backend.rename(&p, &p2).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core backend::tests`
Expected: all pass.

**Step 5: Commit**

```
refactor(vfs): introduce ReadOnlyVfsBackend trait with blanket impl

sub-trait for virtual backends that only need read operations (read,
list, exists, metadata). blanket VfsBackend impl rejects all writes
with PermissionDenied. eliminates boilerplate in virtual backends.

part of #187
```

---

### ✅ Task 4: Refactor `ToolsBackend` to use `ReadOnlyVfsBackend`

**Files:**
- Modify: `crates/chibi-core/src/vfs/tools_backend.rs`

**Step 1: Change the impl block**

Replace `impl VfsBackend for ToolsBackend` with `impl ReadOnlyVfsBackend for ToolsBackend`.

Add `backend_name()`:

```rust
fn backend_name(&self) -> &str {
    "virtual tool registry"
}
```

**Step 2: Remove the 6 write methods**

Delete `write`, `append`, `delete`, `mkdir`, `copy`, `rename` methods — now provided by the blanket impl.

**Step 3: Remove the `write_denied` helper**

Delete `fn write_denied(path: &VfsPath) -> io::Error` — no longer needed.

**Step 4: Update import**

Change `use super::backend::{BoxFuture, VfsBackend};` to `use super::backend::{BoxFuture, ReadOnlyVfsBackend};`

**Step 5: Run tests**

Run: `cargo test -p chibi-core tools_backend`
Expected: all existing tests pass unchanged.

**Step 6: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: all pass.

**Step 7: Commit**

```
refactor(vfs): migrate ToolsBackend to ReadOnlyVfsBackend

removes 6 boilerplate write-rejection methods, now provided by the
blanket impl. existing tests pass unchanged.

part of #187
```

---

### ✅ Task 5: Refactor `AppState.state` to `Arc<RwLock<ContextState>>`

**Goal:** Replace the owned `ContextState` field with a shared reference that can be passed directly to `ContextsBackend` without a second copy or manual sync.

**Files:**
- Modify: `crates/chibi-core/src/state/mod.rs`
- Possibly: `crates/chibi-core/src/chibi.rs` and any other crates that access `app.state`

**Step 1: Change the field type**

In `AppState` struct, change:
```rust
pub state: ContextState,
```
to:
```rust
pub state: Arc<RwLock<ContextState>>,
```

Add required imports at the top of `state/mod.rs`:
```rust
use std::sync::{Arc, RwLock};
```

**Step 2: Update constructors**

In `from_dir` and `load`, wherever `state` is initialised (e.g. `state: loaded_state`), wrap it:
```rust
state: Arc::new(RwLock::new(loaded_state)),
```

**Step 3: Update all read sites**

Find all places that read `self.state` or `app.state`. Replace direct field access with a read guard:
```rust
let state = self.state.read().unwrap();
// use state.contexts, state.save(), etc.
```

**Step 4: Update all mutation sites (exactly 5)**

The known mutation sites — all in `state/mod.rs`:

1. `sync_state_with_filesystem()` — `retain` on `self.state.contexts`
2. `sync_state_with_filesystem()` — `push` to `self.state.contexts`
3. `sync_state_with_filesystem()` — `sort` on `self.state.contexts`
4. `touch_context_with_destroy_settings()` — `iter_mut().find()` on `self.state.contexts`
5. `auto_destroy_expired_contexts()` — `retain` on `self.state.contexts`

For each, acquire a write guard:
```rust
let mut state = self.state.write().unwrap();
state.contexts.retain(...);
```

**Step 5: Update `save()`**

The `save()` method calls `self.state.save(&self.state_path)`. Update:
```rust
pub fn save(&self) {
    let state = self.state.read().unwrap();
    state.save(&self.state_path);
}
```

**Step 6: Search for external access**

Run: `grep -rn "\.state\." crates/ --include="*.rs"` to find any call sites in other crates (e.g. `chibi-cli`, `chibi-mcp-bridge`). Update each to go through the read/write guard.

**Step 7: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: all pass.

**Step 8: Commit**

```
refactor(state): wrap ContextState in Arc<RwLock> for shared access

eliminates the need for a parallel shared_state field when mounting
virtual VFS backends that need live context metadata. single source
of truth for context state.

part of #187
```

---

### Task 6: Implement `ContextsBackend`

**Files:**
- Create: `crates/chibi-core/src/vfs/contexts_backend.rs`
- Modify: `crates/chibi-core/src/vfs/mod.rs` (add module + export)

**Step 1: Create the module file**

Create `crates/chibi-core/src/vfs/contexts_backend.rs`:

```rust
//! Virtual VFS backend for `/sys/contexts/`.
//!
//! Exposes read-only context metadata as virtual files. Each context
//! appears as a directory containing `state.json` (generated on read)
//! and `transcript/` (read-through to on-disk partition files).
//!
//! This backend is mounted at `/sys/contexts/` by `Chibi::load_with_options()`.
//! VFS routing strips the mount prefix, so this backend receives paths
//! relative to `/sys/contexts/` (e.g. `/alice/state.json`, not
//! `/sys/contexts/alice/state.json`).

use std::io::{self, BufReader, ErrorKind};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use super::backend::{BoxFuture, ReadOnlyVfsBackend};
use super::flock::{FlockRegistry, resolve_flock_vfs_root, site_flock_name};
use super::path::VfsPath;
use super::types::{VfsEntry, VfsEntryKind, VfsMetadata};
use crate::context::{ContextEntry, ContextState};
use crate::partition::{Manifest, PartitionManager, StorageConfig};

/// Read-only VFS backend synthesising context metadata.
///
/// Receives stripped paths (the `/sys/contexts` prefix is already removed
/// by `Vfs::resolve_backend`). The path structure is:
///
/// - `/` — lists all contexts as directories
/// - `/<name>/` — lists `state.json` and `transcript/`
/// - `/<name>/state.json` — generated JSON with context metadata
/// - `/<name>/transcript/manifest.json` — read-through from disk
/// - `/<name>/transcript/partitions/<file>` — read-through from disk
/// - `/<name>/transcript/active.jsonl` — read-through from disk
pub struct ContextsBackend {
    /// Shared context state (source of truth for names + metadata).
    /// The same `Arc` that `AppState.state` holds after the Task 5 refactor.
    state: Arc<RwLock<ContextState>>,
    /// Root chibi data directory (e.g. `~/.chibi`). Used to locate
    /// transcript files at `<data_dir>/contexts/<name>/transcript/`.
    data_dir: PathBuf,
    /// Site identifier for flock membership lookups.
    site_id: String,
}

/// JSON structure for `/sys/contexts/<name>/state.json`.
#[derive(Serialize)]
struct ContextStateJson {
    created_at: u64,
    last_activity_at: u64,
    prompt_count: usize,
    auto_destroy_at: Option<u64>,
    auto_destroy_after_inactive_secs: Option<u64>,
    flocks: Vec<String>,
    paths: ContextPaths,
}

/// VFS path references in `state.json`.
#[derive(Serialize)]
struct ContextPaths {
    todos: String,
    goals: Vec<String>,
}

impl ContextsBackend {
    pub fn new(
        state: Arc<RwLock<ContextState>>,
        data_dir: PathBuf,
        site_id: String,
    ) -> Self {
        Self { state, data_dir, site_id }
    }

    /// Look up a context entry by name. Returns `NotFound` if missing.
    fn find_context(&self, name: &str) -> io::Result<ContextEntry> {
        let state = self.state.read().map_err(|_| {
            io::Error::other("ContextsBackend: state lock poisoned")
        })?;
        state
            .contexts
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, format!("no context: {name}"))
            })
    }

    /// Context directory on disk (e.g. `~/.chibi/contexts/<name>`).
    fn context_dir(&self, name: &str) -> PathBuf {
        self.data_dir.join("contexts").join(name)
    }

    /// Transcript directory on disk. Handles legacy layout where transcript
    /// files live directly in the context dir rather than a `transcript/` subdir.
    fn transcript_dir(&self, name: &str) -> PathBuf {
        let ctx_dir = self.context_dir(name);
        let transcript_subdir = ctx_dir.join("transcript");
        if transcript_subdir.is_dir() {
            transcript_subdir
        } else {
            ctx_dir
        }
    }

    /// Compute the prompt count for a context using `PartitionManager`.
    ///
    /// This uses the same path that `AppState::prompt_count` uses, ensuring
    /// consistent counts. Archived partitions use cached `prompt_count` from
    /// the manifest; the active partition uses the in-memory `ActiveState`
    /// built during `PartitionManager::load`. No per-line scanning needed.
    fn prompt_count_for(&self, name: &str) -> io::Result<usize> {
        let transcript_dir = self.transcript_dir(name);
        // NOTE: PartitionManager::load is #[cfg(test)] only — use load_with_config instead
        let pm = PartitionManager::load_with_config(&transcript_dir, StorageConfig::default())?;
        Ok(pm.total_prompt_count())
    }

    /// Build `state.json` content for a context.
    fn build_state_json(&self, name: &str) -> io::Result<Vec<u8>> {
        let entry = self.find_context(name)?;

        // Load flock registry from disk.
        // Note: we read this directly from disk rather than via VFS to avoid
        // a circular dependency (ContextsBackend is itself a VFS backend).
        let vfs_root = self.data_dir.join("vfs");
        let registry_path = vfs_root.join("flocks").join("registry.json");
        let registry: FlockRegistry = if registry_path.exists() {
            let data = std::fs::read_to_string(&registry_path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            FlockRegistry::default()
        };

        // Flocks: site flock + explicit memberships.
        let site_flock = site_flock_name(&self.site_id);
        let explicit = registry.flocks_for(name, &site_flock);
        let mut flocks = vec![site_flock.clone()];
        flocks.extend(explicit.clone());

        // Goal paths: one per flock (site + explicit).
        let mut goal_paths = Vec::new();
        if let Ok(root) = resolve_flock_vfs_root(&site_flock, &self.site_id) {
            goal_paths.push(format!("{}/goals.md", root.as_str()));
        }
        for flock_name in &explicit {
            if let Ok(root) = resolve_flock_vfs_root(flock_name, &self.site_id) {
                goal_paths.push(format!("{}/goals.md", root.as_str()));
            }
        }

        // Prompt count via PartitionManager (no line scanning).
        let prompt_count = self.prompt_count_for(name).unwrap_or(0);

        let state = ContextStateJson {
            created_at: entry.created_at,
            last_activity_at: entry.last_activity_at,
            prompt_count,
            auto_destroy_at: if entry.destroy_at == 0 { None } else { Some(entry.destroy_at) },
            auto_destroy_after_inactive_secs: if entry.destroy_after_seconds_inactive == 0 {
                None
            } else {
                Some(entry.destroy_after_seconds_inactive)
            },
            flocks,
            paths: ContextPaths {
                todos: format!("/home/{}/todos.md", name),
                goals: goal_paths,
            },
        };

        serde_json::to_vec_pretty(&state).map_err(|e| io::Error::other(e.to_string()))
    }

    /// Load the partition manifest for a context.
    fn load_manifest(&self, name: &str) -> io::Result<Manifest> {
        let dir = self.transcript_dir(name);
        let manifest_path = dir.join("manifest.json");
        if manifest_path.exists() {
            let data = std::fs::read_to_string(&manifest_path)?;
            serde_json::from_str(&data).map_err(|e| {
                io::Error::new(ErrorKind::InvalidData, format!("bad manifest: {e}"))
            })
        } else {
            Ok(Manifest::default())
        }
    }

    /// Parse a stripped path into `(context_name, remainder)`.
    /// E.g. `/foo/state.json` → `("foo", "state.json")`.
    /// Returns `None` for root (`/` or empty).
    fn parse_path(path: &VfsPath) -> Option<(&str, &str)> {
        let p = path.as_str().trim_start_matches('/');
        if p.is_empty() {
            return None;
        }
        let (name, rest) = match p.find('/') {
            Some(i) => (&p[..i], &p[i + 1..]),
            None => (p, ""),
        };
        Some((name, rest))
    }
}

impl ReadOnlyVfsBackend for ContextsBackend {
    fn backend_name(&self) -> &str {
        "virtual context metadata"
    }

    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            let (name, rest) = Self::parse_path(path).ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "cannot read directory; use list()")
            })?;

            match rest {
                "" => Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "cannot read directory; use list()",
                )),
                "state.json" => self.build_state_json(name),
                "transcript/manifest.json" => {
                    let path = self.transcript_dir(name).join("manifest.json");
                    std::fs::read(&path).map_err(|e| {
                        if e.kind() == ErrorKind::NotFound {
                            io::Error::new(
                                ErrorKind::NotFound,
                                format!("no transcript manifest for context '{name}'"),
                            )
                        } else {
                            e
                        }
                    })
                }
                rest if rest.starts_with("transcript/partitions/") => {
                    let file = rest.strip_prefix("transcript/partitions/").unwrap();
                    let _ = self.find_context(name)?; // verify context exists
                    let path = self.transcript_dir(name).join("partitions").join(file);
                    std::fs::read(&path)
                }
                "transcript/active.jsonl" => {
                    let _ = self.find_context(name)?;
                    let manifest = self.load_manifest(name)?;
                    let path = self.transcript_dir(name).join(&manifest.active_partition);
                    std::fs::read(&path)
                }
                _ => Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("no virtual file: {}", path),
                )),
            }
        })
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/').trim_end_matches('/');

            if p.is_empty() {
                // Root: list all contexts as directories.
                let state = self.state.read().map_err(|_| {
                    io::Error::other("ContextsBackend: state lock poisoned")
                })?;
                return Ok(state
                    .contexts
                    .iter()
                    .map(|c| VfsEntry { name: c.name.clone(), kind: VfsEntryKind::Directory })
                    .collect());
            }

            let (name, rest) = match p.find('/') {
                Some(i) => (&p[..i], &p[i + 1..]),
                None => (p, ""),
            };

            let _ = self.find_context(name)?; // verify context exists

            match rest {
                "" => Ok(vec![
                    VfsEntry { name: "state.json".into(), kind: VfsEntryKind::File },
                    VfsEntry { name: "transcript".into(), kind: VfsEntryKind::Directory },
                ]),
                "transcript" => Ok(vec![
                    VfsEntry { name: "manifest.json".into(), kind: VfsEntryKind::File },
                    VfsEntry { name: "active.jsonl".into(), kind: VfsEntryKind::File },
                    VfsEntry { name: "partitions".into(), kind: VfsEntryKind::Directory },
                ]),
                "transcript/partitions" => {
                    let manifest = self.load_manifest(name)?;
                    Ok(manifest
                        .partitions
                        .iter()
                        .filter_map(|p| {
                            let file_name = p.file.rsplit('/').next()?;
                            Some(VfsEntry {
                                name: file_name.to_string(),
                                kind: VfsEntryKind::File,
                            })
                        })
                        .collect())
                }
                _ => Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("no virtual directory: {}", path),
                )),
            }
        })
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/');
            if p.is_empty() {
                return Ok(true); // root always exists
            }

            let Some((name, rest)) = Self::parse_path(path) else {
                return Ok(true);
            };

            if self.find_context(name).is_err() {
                return Ok(false);
            }

            match rest {
                "" | "state.json" | "transcript" | "transcript/partitions" => Ok(true),
                "transcript/manifest.json" => {
                    Ok(self.transcript_dir(name).join("manifest.json").exists())
                }
                "transcript/active.jsonl" => {
                    let manifest = self.load_manifest(name).unwrap_or_default();
                    Ok(self.transcript_dir(name).join(&manifest.active_partition).exists())
                }
                rest if rest.starts_with("transcript/partitions/") => {
                    let file = rest.strip_prefix("transcript/partitions/").unwrap();
                    Ok(self.transcript_dir(name).join("partitions").join(file).exists())
                }
                _ => Ok(false),
            }
        })
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/');
            let is_dir = p.is_empty()
                || Self::parse_path(path)
                    .map(|(_, rest)| matches!(rest, "" | "transcript" | "transcript/partitions"))
                    .unwrap_or(true);

            Ok(VfsMetadata {
                size: 0,
                created: None,
                modified: None,
                kind: if is_dir { VfsEntryKind::Directory } else { VfsEntryKind::File },
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::backend::VfsBackend;
    use tempfile::TempDir;

    fn mock_state(names: &[&str]) -> Arc<RwLock<ContextState>> {
        let contexts = names
            .iter()
            .map(|n| ContextEntry {
                name: n.to_string(),
                created_at: 1000,
                last_activity_at: 2000,
                destroy_after_seconds_inactive: 0,
                destroy_at: 0,
                cwd: None,
            })
            .collect();
        Arc::new(RwLock::new(ContextState { contexts }))
    }

    fn make_backend(state: Arc<RwLock<ContextState>>, data_dir: &std::path::Path) -> ContextsBackend {
        std::fs::create_dir_all(data_dir.join("contexts")).unwrap();
        std::fs::create_dir_all(data_dir.join("vfs").join("flocks")).unwrap();
        ContextsBackend::new(state, data_dir.to_path_buf(), "test-site".into())
    }

    #[tokio::test]
    async fn test_list_root_returns_contexts() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice", "bob"]);
        let backend = make_backend(state, tmp.path());

        let entries = backend.list(&VfsPath::new("/").unwrap()).await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"bob"));
        assert!(entries.iter().all(|e| e.kind == VfsEntryKind::Directory));
    }

    #[tokio::test]
    async fn test_list_context_dir() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend = make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        let entries = backend.list(&VfsPath::new("/alice").unwrap()).await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"state.json"));
        assert!(names.contains(&"transcript"));
    }

    #[tokio::test]
    async fn test_read_state_json() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend = make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        let data = backend.read(&VfsPath::new("/alice/state.json").unwrap()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(json["created_at"], 1000);
        assert_eq!(json["last_activity_at"], 2000);
        assert_eq!(json["prompt_count"], 0);
        assert!(json["auto_destroy_at"].is_null());
        assert!(json["flocks"].is_array());
        assert_eq!(json["paths"]["todos"], "/home/alice/todos.md");
    }

    #[tokio::test]
    async fn test_read_nonexistent_context() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&[]);
        let backend = make_backend(state, tmp.path());

        let err = backend.read(&VfsPath::new("/ghost/state.json").unwrap()).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_exists() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend = make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        assert!(backend.exists(&VfsPath::new("/").unwrap()).await.unwrap());
        assert!(backend.exists(&VfsPath::new("/alice").unwrap()).await.unwrap());
        assert!(backend.exists(&VfsPath::new("/alice/state.json").unwrap()).await.unwrap());
        assert!(!backend.exists(&VfsPath::new("/ghost").unwrap()).await.unwrap());
        assert!(!backend.exists(&VfsPath::new("/alice/nope.txt").unwrap()).await.unwrap());
    }

    #[tokio::test]
    async fn test_write_rejected() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let path = VfsPath::new("/alice/state.json").unwrap();
        let err = backend.write(&path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_transcript_manifest_read_through() {
        use serde_json::json;

        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend = make_backend(state, tmp.path());

        let transcript_dir = tmp.path().join("contexts/alice/transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let manifest = json!({
            "version": 1,
            "active_partition": "active.jsonl",
            "partitions": [{
                "file": "partitions/1000-2000.jsonl",
                "start_ts": 1000,
                "end_ts": 2000,
                "entry_count": 10,
                "prompt_count": 3
            }],
            "rotation_policy": {
                "max_entries": 1000,
                "max_age_seconds": 2592000
            }
        });
        std::fs::write(
            transcript_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ).unwrap();

        let data = backend
            .read(&VfsPath::new("/alice/transcript/manifest.json").unwrap())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(parsed["version"], 1);
    }

    #[tokio::test]
    async fn test_list_transcript_partitions() {
        use serde_json::json;

        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend = make_backend(state, tmp.path());

        let transcript_dir = tmp.path().join("contexts/alice/transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let manifest = json!({
            "version": 1,
            "active_partition": "active.jsonl",
            "partitions": [
                {"file": "partitions/1000-2000.jsonl", "start_ts": 1000, "end_ts": 2000, "entry_count": 10},
                {"file": "partitions/2001-3000.jsonl", "start_ts": 2001, "end_ts": 3000, "entry_count": 5}
            ],
            "rotation_policy": {"max_entries": 1000, "max_age_seconds": 2592000}
        });
        std::fs::write(
            transcript_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ).unwrap();

        let entries = backend
            .list(&VfsPath::new("/alice/transcript/partitions").unwrap())
            .await
            .unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["1000-2000.jsonl", "2001-3000.jsonl"]);
    }
}
```

**Step 2: Register the module**

In `crates/chibi-core/src/vfs/mod.rs`, add:

```rust
pub mod contexts_backend;
pub use contexts_backend::ContextsBackend;
```

**Step 3: Run tests**

Run: `cargo test -p chibi-core contexts_backend`
Expected: all pass.

**Step 4: Commit**

```
feat(vfs): implement ContextsBackend for /sys/contexts/

virtual read-only backend exposing context metadata (state.json with
timestamps, prompt count, flocks, path references) and transcript
read-through. implements ReadOnlyVfsBackend. uses PartitionManager
for prompt_count — no per-line scanning.

part of #187
```

---

### Task 7: Mount `ContextsBackend` in `chibi.rs`

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs` (around line 230-238)

**Step 1: Mount the backend**

After the `AppState.state` refactor (Task 5), `app.state` is already `Arc<RwLock<ContextState>>`. Clone the `Arc` to pass to `ContextsBackend`:

```rust
let contexts_backend = crate::vfs::ContextsBackend::new(
    Arc::clone(&app.state),
    app.chibi_dir.clone(),
    app.site_id.clone(),
);
app.vfs = crate::vfs::Vfs::builder(&site_id)
    .mount("/", Box::new(local_backend))
    .mount("/tools/sys", Box::new(tools_backend))
    .mount("/sys/contexts", Box::new(contexts_backend))
    .build();
```

Add `use std::sync::Arc;` if not already imported.

**Step 2: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: all pass.

**Step 3: Commit**

```
feat(vfs): mount ContextsBackend at /sys/contexts/

clones Arc<RwLock<ContextState>> from app.state and passes it to
the virtual context metadata backend. contexts are now discoverable
and introspectable via VFS.

closes #187
```

---

### Task 8: Update documentation

**Files:**
- Modify: `docs/vfs.md` — add `/sys/contexts/` to the VFS layout and document the virtual file structure
- Modify: `docs/architecture.md` — add `contexts_backend.rs` to the VFS module file listing
- Modify: `docs/cli-reference.md` — update `chibi -l` description if it mentions "Messages"

**Step 1: Update `docs/vfs.md`**

Add a section describing `/sys/contexts/<name>/` with the virtual file tree and `state.json` schema.

**Step 2: Update `docs/architecture.md`**

Add `contexts_backend.rs` to the VFS module file listing.

**Step 3: Update CLI docs if needed**

Search for "Messages" in `docs/cli-reference.md` and update to "Prompts" if present.

**Step 4: Run `just lint`**

Run: `just lint`
Expected: no warnings.

**Step 5: Commit**

```
docs: document /sys/contexts/ virtual files and cli -l prompt count

updates VFS docs, architecture file listing, and CLI reference.

part of #187
```

---

### Task 9: Final verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: all pass across all crates.

**Step 2: Run `just pre-push`**

Run: `just pre-push`
Expected: clean.

**Step 3: Review all commits on the branch**

Run: `git log --oneline main..HEAD`
Verify commit messages reference #187.

**Step 4: Collect AGENTS.md notes**

Gotchas discovered during implementation to add to `AGENTS.md`:
- `ContextsBackend` reads flock registry directly from disk (not via VFS) to avoid circular dependency
- `prompt_count` in pre-existing `PartitionMeta` defaults to 0 via `serde(default)` — no backfill of old manifests
- `AppState.state` is `Arc<RwLock<ContextState>>` — use `.read().unwrap()` / `.write().unwrap()` guards; panics only on lock poison (indicates a prior panic, not normal operation)
- `ContextsBackend` uses `PartitionManager::load` for prompt count — this does a full active-partition scan on each `state.json` read. If performance becomes a concern, pass a cached `ActiveState` via `load_with_cached_state`.
