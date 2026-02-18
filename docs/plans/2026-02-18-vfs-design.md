# Virtual File System Design

## motivation

chibi needs a sandboxed, shared file space where contexts can read and write without engaging the OS-level permission system. two primary drivers:

1. **sandbox** — contexts get write access within well-defined zones without needing `PreFileWrite` approval. files can be "presented" (artifacts-style) via CLI parameters.
2. **shared state for multi-context workflows** — swarming, blackboards, coordination patterns all benefit from a common namespace. the VFS is a more general primitive than a dedicated blackboard; a blackboard can be built on top of it.

## path model

### VfsPath

opaque newtype wrapping `String`. validated on construction:
- must start with `/`
- no `..` components, no `//`, no trailing `/` (except root `/`)
- no null bytes
- this is the *only* way to address VFS content — no `PathBuf` leaks

### namespace layout

```
/shared/              all contexts: read + write
/home/<context>/      owner: read + write; others: read only
/sys/                 read only (SYSTEM-populated)
```

### permission model

lives entirely in the `Vfs` struct. path *is* policy:
- **read** — always allowed for all zones
- **write** — allowed in `/shared/` and `/home/<own_name>/`; denied elsewhere
- caller identifies themselves by context name
- **SYSTEM** — reserved caller with unrestricted write access (including `/sys/`)
- context name validation rejects "system" (case-insensitive) to prevent impersonation

no chmod, no ACLs, no ownership metadata.

## architecture

### approach: thin trait, fat router (approach A)

```
tool code  ->  Vfs (permissions + VfsPath validation)  ->  VfsBackend trait (dumb storage)
```

the `VfsBackend` trait has minimal, low-level operations. the `Vfs` struct wraps any backend, enforcing permissions and path validation. tools and plugins never touch the backend directly.

documented future evolution paths:
- **multi-backend mounting** — `Vfs` maps path prefixes to different backends (e.g. `/shared/` on disk, `/remote/` on XMPP). longest-prefix match, strip mount prefix.
- **middleware layers** — composable tower-style layers (logging, caching, etc.) wrapping backends (approach C). refactor from approach A when needed.

### VfsBackend trait

```rust
#[async_trait]
pub trait VfsBackend: Send + Sync {
    async fn read(&self, path: &VfsPath) -> io::Result<Vec<u8>>;
    async fn write(&self, path: &VfsPath, data: &[u8]) -> io::Result<()>;
    async fn append(&self, path: &VfsPath, data: &[u8]) -> io::Result<()>;
    async fn delete(&self, path: &VfsPath) -> io::Result<()>;
    async fn list(&self, path: &VfsPath) -> io::Result<Vec<VfsEntry>>;
    async fn exists(&self, path: &VfsPath) -> io::Result<bool>;
    async fn mkdir(&self, path: &VfsPath) -> io::Result<()>;
    async fn copy(&self, src: &VfsPath, dst: &VfsPath) -> io::Result<()>;
    async fn rename(&self, src: &VfsPath, dst: &VfsPath) -> io::Result<()>;
    async fn metadata(&self, path: &VfsPath) -> io::Result<VfsMetadata>;
}
```

no permission logic. just storage.

### core types

```rust
pub struct VfsMetadata {
    pub size: u64,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub kind: VfsEntryKind,  // File or Directory
}

pub struct VfsEntry {
    pub name: String,
    pub kind: VfsEntryKind,
}
```

timestamps are `Option` because not every backend can provide all of them. `list()` returns entries without metadata to keep it cheap; callers follow up with `metadata()` if needed.

### Vfs struct

```rust
/// core VFS router and permission enforcer.
///
/// currently wraps a single backend. designed to evolve toward:
/// - multiple backends mounted at different path prefixes
/// - composable middleware layers (logging, caching, etc.) a la tower
///   (approach C in the design doc)
pub struct Vfs {
    backend: Box<dyn VfsBackend>,
}
```

every public method takes `caller: &str` + `VfsPath`, checks permissions, delegates to backend. for two-path operations (copy, rename), both paths get permission checks.

## tool integration

### `vfs://` prefix

existing tools detect `vfs://` in path arguments. `vfs:///shared/tasks.md` -> `VfsPath("/shared/tasks.md")`.

interception points:
- `file_tools::resolve_file_path` — detects prefix, does VFS permission check, routes through `Vfs`
- `coding_tools::resolve_path` — same

every existing tool that goes through path resolution gains VFS support with no API changes. the LLM just uses `vfs:///shared/notes.md` as the path argument.

for read operations: VFS reads content into memory, existing logic operates on it.
for write operations: read via VFS, apply edit, write back via VFS.

### dedicated VFS tools

thin wrappers around `Vfs` methods for operations not covered by existing tools:

- **vfs_list** — list entries at a VFS path (doubles as exists check; returns empty for nonexistent)
- **vfs_info** — metadata (size, timestamps, kind)
- **vfs_copy** / **vfs_move** — copy/rename within VFS
- **vfs_mkdir** — create directory
- **vfs_delete** — remove file

### plugin/hook access

the `Vfs` handle is threaded into hook execution context alongside `AppState`. plugins call VFS methods through the same permission model — a hook running on behalf of context `"planner"` can write to `/shared/` and `/home/planner/` but not `/home/coder/`.

## storage layout (LocalBackend)

```
~/.chibi/
+-- vfs/                    VFS root on disk
|   +-- shared/
|   +-- home/
|   |   +-- planner/
|   |   +-- coder/
|   +-- sys/
+-- config.toml             [vfs] section
+-- contexts/               existing, unchanged
+-- ...
```

`vfs/` sits alongside `contexts/` in CHIBI_HOME. completely separate tree.

### LocalBackend

maps `VfsPath("/shared/foo.txt")` -> `<chibi_home>/vfs/shared/foo.txt`. uses `safe_io::atomic_write` for writes. async methods are thin wrappers around `tokio::fs` or `spawn_blocking`.

## config

```toml
[vfs]
backend = "local"           # default

# future:
# backend = "fossil"
# [vfs.options]
# db_path = "/path/to/repo.fossil"
```

backend selection is config-level. plugins can register backends, but which one is active is determined by config.

## async boundary

`VfsBackend` trait is async from day one. chibi-core is currently sync; tool execution uses `block_on()` at the VFS call boundary. only VFS call sites deal with async. when chibi eventually goes async, the `block_on` calls just disappear.

## reserved names

`is_valid_context_name` extended to reject `"system"` (case-insensitive) to prevent contexts from impersonating the SYSTEM caller.
