# Virtual File System

sandboxed, shared file space for contexts. contexts can read and write without engaging the OS-level permission system or `PreFileWrite` hooks. enables multi-context workflows: swarming, blackboards, coordination patterns.

## namespace layout

```
/shared/                          all contexts: read + write
/home/<context>/                  owner: read + write; others: read only
/sys/                             read only (SYSTEM-populated)
/sys/tool_cache/<context>/        cached tool outputs (SYSTEM-written, world-readable)
```

## permission model

zone-based. path *is* policy:

- **read** — always allowed for all zones
- **write** — allowed in `/shared/` and `/home/<own_name>/`; denied elsewhere
- **SYSTEM** — reserved caller with unrestricted write access (including `/sys/`). context names reject "system" (case-insensitive) to prevent impersonation

no chmod, no ACLs, no ownership metadata.

## using the VFS

### `vfs://` URI scheme

existing tools detect `vfs://` in path arguments. three slashes required (scheme + empty authority + absolute path):

```
vfs:///shared/tasks.md     →  VfsPath("/shared/tasks.md")
vfs:///home/planner/notes  →  VfsPath("/home/planner/notes")
```

works with: `write_file`, `file_head`, `file_tail`, `file_lines`, `file_grep`, `file_edit`.

VFS paths bypass `PreFileRead`/`PreFileWrite` hooks — the VFS's own permission model is sufficient.

### dedicated VFS tools

for operations not covered by existing tools:

| tool | description |
|------|-------------|
| `vfs_list` | list directory entries |
| `vfs_info` | metadata (size, kind, timestamps) |
| `vfs_copy` | copy a file within the VFS |
| `vfs_move` | move/rename a file within the VFS |
| `vfs_mkdir` | create a directory |
| `vfs_delete` | delete a file or directory |

all dedicated tools also bypass file hooks.

### examples

```json
{"tool": "write_file", "args": {"path": "vfs:///shared/tasks.md", "content": "# tasks\n- review PR"}}
{"tool": "file_head", "args": {"path": "vfs:///shared/tasks.md", "lines": 10}}
{"tool": "vfs_list", "args": {"path": "vfs:///shared"}}
{"tool": "vfs_copy", "args": {"src": "vfs:///shared/tasks.md", "dst": "vfs:///shared/backup.md"}}
{"tool": "file_grep", "args": {"path": "vfs:///sys/tool_cache/myctx/web_fetch_abc123", "pattern": "error"}}
```

### tool output caching

large tool outputs are automatically cached under `/sys/tool_cache/<context>/<id>`. the LLM receives a truncated stub with the `vfs:///` URI and can examine the content using:

```
file_head(path="vfs:///sys/tool_cache/<ctx>/<id>", lines=50)
file_tail(path="vfs:///sys/tool_cache/<ctx>/<id>", lines=50)
file_lines(path="vfs:///sys/tool_cache/<ctx>/<id>", start=100, end=150)
file_grep(path="vfs:///sys/tool_cache/<ctx>/<id>", pattern="error")
```

cache entries are written by SYSTEM and world-readable. they are cleaned up automatically based on `tool_cache_max_age_days`.

## configuration

```toml
[vfs]
backend = "local"   # default, only built-in option
```

unknown backends are rejected at startup.

## architecture

```
tool code  →  Vfs (permissions + path validation)  →  VfsBackend (dumb storage)
```

**approach A — thin trait, fat router.** the `VfsBackend` trait is intentionally simple — just storage, no permission logic. the `Vfs` struct wraps any backend, enforcing permissions and path validation. tools and plugins never touch the backend directly.

### VfsBackend trait

```rust
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

backends translate `VfsPath` values to their native addressing internally. async from day one.

### LocalBackend

maps `VfsPath("/shared/foo.txt")` → `<chibi_home>/vfs/shared/foo.txt`. uses `safe_io::atomic_write` for writes.

### storage layout

```
~/.chibi/
├── vfs/
│   ├── shared/
│   ├── home/
│   │   ├── planner/
│   │   └── coder/
│   └── sys/
├── config.toml
└── contexts/
```

`vfs/` sits alongside `contexts/` in CHIBI_HOME.

## future evolution

- **multi-backend mounting** — `Vfs` maps path prefixes to different backends (e.g. `/shared/` on disk, `/remote/` on XMPP). longest-prefix match.
- **middleware layers** — composable tower-style layers (logging, caching) wrapping backends (approach C). refactor from approach A when needed.
