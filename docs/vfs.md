# Virtual File System

sandboxed, shared file space for contexts. contexts can read and write without engaging the OS-level permission system or `PreFileWrite` hooks. enables multi-context workflows: swarming, blackboards, coordination patterns.

## namespace layout

```
/shared/                          all contexts: read + write
/home/<context>/                  owner: read + write; others: read only
/sys/                             read only (SYSTEM-populated)
/sys/tool_cache/<context>/        cached tool outputs (SYSTEM-written, world-readable)
/sys/contexts/<name>/             read-only context metadata (virtual, generated on-demand)
/site/                            site-wide flock data (world-writable)
/flocks/<name>/                   per-flock data (members only)
/tools/shared/                    synthesised tools: visible to all contexts
/tools/home/<context>/            synthesised tools: visible to owner context only
/tools/flocks/<flock>/            synthesised tools: visible to flock members only
/tools/sys/                       read-only virtual: tool schema JSON (generated on demand)
```

## permission model

zone-based. path *is* policy:

- **read** — always allowed for all zones
- **write** — allowed in `/shared/`, `/home/<own_name>/`, and `/site/`; `/flocks/<name>/` for members only; denied elsewhere
- **SYSTEM** — reserved caller with unrestricted write access (including `/sys/`, `/flocks/registry.json`). context names reject "system" (case-insensitive) to prevent impersonation

no chmod, no ACLs, no ownership metadata.

### VfsCaller enum

all VFS operations are attributed to a typed `VfsCaller`:

| Variant | Access | Usage |
|---------|--------|-------|
| `VfsCaller::System` | unrestricted write to all zones | startup bootstrap, flock management, goal writes |
| `VfsCaller::Context(&str)` | zone-restricted (own `/home/` + `/shared/` only) | context tool calls |

## flock registry

flocks are named groups of contexts that share goals and prompts. membership and goals live entirely in the VFS:

```
/site/                    site-wide flock (implicit, all contexts belong)
  goals.md                site-wide goals
  prompt.md               site-wide injected prompt (optional)
/flocks/<name>/           named flock
  goals.md                flock goals
  prompt.md               flock injected prompt (optional)
/flocks/registry.json     centralised membership registry (SYSTEM only)
```

membership is stored centrally in `/flocks/registry.json`, not per-flock. the site flock is identified as `site:<site_id>`. `/site/` and `/flocks/` directories are bootstrapped on startup.

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
│   │   │   └── tasks/      # per-context task files (see task system)
│   │   │       └── <id>.task
│   │   └── coder/
│   ├── sys/
│   ├── site/               # site flock data
│   │   ├── goals.md
│   │   └── prompt.md
│   └── flocks/             # named flock data
│       ├── registry.json    # centralised flock membership (SYSTEM only)
│       └── <name>/
│           ├── goals.md
│           ├── prompt.md
│           └── tasks/      # flock-scoped task files
│               └── <id>.task
├── config.toml
└── contexts/
```

`vfs/` sits alongside `contexts/` in CHIBI_HOME.

## multi-backend mounting

`Vfs` supports multiple backends mounted at different path prefixes, resolved by longest-prefix match. Use `Vfs::builder(site_id)` to compose them:

```rust
let vfs = Vfs::builder(site_id)
    .mount("/", Box::new(LocalBackend::new(vfs_root)))
    .mount("/tools/sys", Box::new(ToolsBackend::new(registry)))
    .mount("/sys/contexts", Box::new(ContextsBackend::new(state, data_dir, site_id)))
    .build();
```

`ToolsBackend` is a read-only virtual backend mounted at `/tools/sys/` that synthesises tool schema JSON on demand from the registry. Reads enumerate tools; writes are rejected.

`ContextsBackend` is a read-only virtual backend mounted at `/sys/contexts/` that synthesises context metadata from the shared `Arc<RwLock<ContextState>>`. It does not hold its own copy of context data — it reads from the same source of truth as `AppState`.

### `/sys/contexts/` virtual file structure

```
/sys/contexts/
└── <name>/
    ├── state.json        # generated: timestamps, prompt_count, flocks, path refs
    ├── task-dirs         # generated: scheme list datum of all task dirs visible to context
    └── transcript/
        ├── manifest.json       # read-through from disk
        ├── active.jsonl        # read-through from disk (active partition)
        └── partitions/
            └── <file>.jsonl    # read-through from disk (archived partitions)
```

`task-dirs` contains a Scheme list datum of all task directories visible to the context — context-local first, followed by each flock's task directory. Example:

```scheme
("/home/alice/tasks" "/flocks/site:abc123/tasks" "/flocks/frontend/tasks")
```

`state.json` schema:

```json
{
  "created_at": 1700000000,
  "last_activity_at": 1700001234,
  "prompt_count": 42,
  "auto_destroy_at": null,
  "auto_destroy_after_inactive_secs": null,
  "flocks": ["site:my-machine-abc123", "frontend"],
  "paths": {
    "tasks": "/home/alice/tasks",
    "goals": ["/site/goals.md", "/flocks/frontend/goals.md"]
  }
}
```

`prompt_count` counts user prompt entries (`entry_type="message"`, `role="user"`) across all archived and active partitions. Uses `PartitionManager`'s cached partition metadata — no per-line scanning of archived files.

## synthesised tools zone

Scheme (`.scm`) files placed under `/tools/` are automatically loaded as synthesised tools. Three zones are scanned at startup and on hot-reload:

| Zone | VFS Path | Visibility |
|------|----------|------------|
| shared | `/tools/shared/` | all contexts |
| home | `/tools/home/<context>/` | owner context only |
| flocks | `/tools/flocks/<flock>/` | flock members only |

Files are scanned recursively. Non-`.scm` files and files with invalid Scheme source are silently skipped. Valid tools are registered and appear alongside regular plugin tools.

**Hot-reload:** writing a `.scm` file via the VFS triggers immediate re-registration. If the new source is invalid, the previous version of the tool remains registered. Deleting a file unregisters all tools defined in it (multi-tool files supported).

**Sandbox tiers:** each zone uses the `sandboxed` tier by default (safe R7RS subset). Override per path prefix with `[tools.tiers]` in `config.toml`. See [configuration.md](configuration.md) and [plugins.md](plugins.md) for details.

## task system

Structured tasks replace the old flat `todos.md`. Each task is a `.task` file containing two Scheme datums: a metadata alist and a body string.

### file format

```scheme
((id . "a3f2")
 (status . pending)           ; pending | in-progress | done
 (priority . high)            ; low | medium | high
 (assigned-to . "worker-1")  ; optional: context name
 (depends-on "b1c4" "e7d0")  ; optional: blocking task IDs
 (created . "20260308-1423z")
 (updated . "20260308-1445z"))

"implement the auth flow.

acceptance criteria:
- JWT tokens"
```

### storage layout

| Scope | VFS path | Access |
|-------|----------|--------|
| context | `/home/<ctx>/tasks/<path>.task` | owner context |
| flock | `/flocks/<name>/tasks/<path>.task` | flock members |

Task paths are arbitrary: `auth/login.task`, `deploy.task`, etc. Subdirectories are created automatically. Use `flock:<name>/path` as the `path` argument to route to a flock's task directory.

### ephemeral injection

At each prompt, chibi reads all accessible task files (context + flocks) via `state::tasks::collect_tasks`, parses metadata with tein-sexp, and injects a compact table summary as a system message immediately before the current user turn. The summary is never persisted to the transcript.

```
--- tasks ---
id     status      priority  path              summary
a3f2   in-progress high      epic/login        implement the auth flow
--- 1 active, 0 done ---
```

### crud tools

The `tasks.scm` plugin (installed to `/tools/shared/tasks.scm`) exposes five tools:

| Tool | Description |
|------|-------------|
| `task_create` | Create a new `.task` file; returns id and path |
| `task_update` | Update status, priority, body, or assigned-to by task ID |
| `task_view` | Read full task content by ID |
| `task_list` | List tasks with optional status/priority/assigned-to filters (substring match on raw file content — body text mentioning a filter value will also match) |
| `task_delete` | Remove a task file by ID |

## future evolution

- **middleware layers** — composable tower-style layers (logging, caching) wrapping backends (approach C). refactor from approach A when needed.
