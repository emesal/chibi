# ToolRegistry abstraction + VFS tool namespace

**issue:** #190
**date:** 2026-03-07

## motivation

tools in chibi currently have no unified registry. builtins are dispatched by name
in ad-hoc match arms, plugin tools are OS paths, MCP tools are fake `mcp://` URIs
stored in `Tool::path`. adding a new tool source requires touching the dispatcher.
there's also no way for an LLM to browse or introspect available tools without
injecting all schemas into context.

this design establishes a `ToolRegistry` abstraction and a VFS namespace for tools,
enabling lazy discovery, synthesised tool authoring via tein, and a clean extension
point for future backends.

## core types

### ToolImpl — dispatch discriminant

replaces the polymorphic `Tool.path` field with a typed enum:

```rust
pub enum ToolImpl {
    /// built-in rust handler. closure captures its own dependencies
    /// (Arc<Vfs>, Arc<ContextStore>, etc.) at registration time.
    Builtin(ToolHandler),

    /// OS-path plugin executable (spawned as subprocess)
    Plugin(PathBuf),

    /// MCP bridge tool (JSON-over-TCP to mcp-bridge daemon)
    Mcp { server: String, tool_name: String },

    /// tein scheme source in a writable VFS zone.
    /// gated behind `#[cfg(feature = "synthesised-tools")]`.
    Synthesised {
        vfs_path: VfsPath,
        context: Arc<ThreadLocalContext>,
    },
}
```

### ToolHandler — uniform async handler

closures capture their own state at registration time. no god-object.

```rust
pub type ToolHandler = Arc<
    dyn Fn(ToolCall<'_>) -> BoxFuture<'_, io::Result<String>> + Send + Sync
>;
```

### ToolCall — extensible input

```rust
/// Runtime context passed per-call. Carries values not known at registration time.
pub struct ToolCallContext<'a> {
    pub app: &'a AppState,
    pub context_name: &'a str,
    pub config: &'a ResolvedConfig,
    pub project_root: &'a Path,
    pub vfs: &'a Vfs,
    pub vfs_caller: VfsCaller<'a>,
}

pub struct ToolCall<'a> {
    pub name: &'a str,
    pub args: &'a serde_json::Value,
    pub context: &'a ToolCallContext<'a>,
}
```

`ToolCallContext` is per-call, not captured. most handlers capture nothing at
registration and just use `call.context.*`. can add `call_id`, trace fields,
etc. to `ToolCallContext` later without changing handler signatures.

### ToolCategory — replaces all is_*_tool() predicates

```rust
pub enum ToolCategory {
    Memory, FsRead, FsWrite, Shell, Network, Index, Flow, Vfs,
    Plugin, Mcp, Synthesised,
}
```

set once at registration time. replaces `ToolType` in `send.rs` and all eight
`is_*_tool()` predicate functions.

### Tool struct — updated

```rust
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(skip)]
    pub r#impl: ToolImpl,
    pub category: ToolCategory,
    pub hooks: Vec<HookPoint>,
    pub metadata: ToolMetadata,
    pub summary_params: Vec<String>,
}
```

`Tool.path` is gone. `ToolImpl` carries typed dispatch info. `ToolCategory` enables
filtering and permission routing.

## the registry

```rust
pub struct ToolRegistry {
    tools: IndexMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Tool);
    pub fn unregister(&mut self, name: &str) -> Option<Tool>;
    pub fn get(&self, name: &str) -> Option<&Tool>;
    pub fn all(&self) -> impl Iterator<Item = &Tool>;
    pub fn filter(&self, pred: impl Fn(&Tool) -> bool) -> Vec<&Tool>;
    pub async fn dispatch_with_context(&self, name: &str, args: &Value, ctx: &ToolCallContext<'_>) -> io::Result<String>;
}
```

- **`IndexMap`** — O(1) lookup, preserves insertion order for deterministic
  tool lists sent to the LLM. `shift_remove` (not `swap_remove`) is used for
  unregistration to keep order stable.
- **pure dispatch** — `dispatch()` finds the tool and calls its handler. hooks,
  permissions, and caching stay in `send.rs` as middleware wrapping this call
- **concrete struct, not a trait** — there's only one registry; a trait adds
  indirection for no benefit

dispatch implementation:

```rust
async fn dispatch_with_context(
    &self,
    name: &str,
    args: &Value,
    ctx: &ToolCallContext<'_>,
) -> io::Result<String> {
    let tool = self.get(name)
        .ok_or_else(|| io::Error::new(NotFound, format!("unknown tool: {name}")))?;
    let call = ToolCall { name, args, context: ctx };
    match &tool.r#impl {
        ToolImpl::Builtin(handler)          => handler(call).await,
        ToolImpl::Plugin(path)              => execute_plugin(path, &call).await,
        ToolImpl::Mcp { server, tool_name } => execute_mcp(server, tool_name, &call).await,
        ToolImpl::Synthesised { context, .. } => execute_synthesised(context, &call).await,
    }
}
```

## registration — how tools enter the registry

### builtins

each tool module provides a `register_*_tools()` function:

```rust
pub fn register_fs_read_tools(
    registry: &mut ToolRegistry,
    vfs: Arc<Vfs>,
    config: Arc<Config>,
) {
    let handler: ToolHandler = Arc::new(move |call| {
        let vfs = vfs.clone();
        let config = config.clone();
        Box::pin(async move {
            execute_fs_read_tool(&call, &vfs, &config).await
        })
    });

    for def in FS_READ_TOOL_DEFS {
        registry.register(Tool {
            name: def.name.to_string(),
            // ... schema from def ...
            r#impl: ToolImpl::Builtin(handler.clone()),
            category: ToolCategory::FsRead,
            // ...
        });
    }
}
```

one handler closure shared across all tools in a group (they dispatch internally
by name). each group captures only its own dependencies.

### plugins

`load_tools()` calls `registry.register()` for each discovered plugin with
`ToolImpl::Plugin(path)` and `ToolCategory::Plugin`.

### MCP

`load_mcp_tools()` registers with `ToolImpl::Mcp { server, tool_name }` and
`ToolCategory::Mcp`. the `mcp://` path convention is gone.

### synthesised

loaded on startup by scanning writable VFS zones, registered on VFS writes
via hot-reload. see "synthesised tool lifecycle" below.

## VFS tool namespace

```
/tools/
├── sys/              read-only, virtual (builtins + plugins + MCP)
├── shared/           world-readable + writable (shared synthesised tools)
├── flocks/<name>/    flock-scoped synthesised tools
└── home/<ctx>/       context-owned synthesised tools
```

### /tools/sys/ — lazy discovery

virtual namespace backed by `ToolsBackend` (implements `VfsBackend`). holds an
`Arc<RwLock<ToolRegistry>>` and synthesises responses:

- `vfs_list vfs:///tools/sys/` → lists all registered tool names
- `file_head vfs:///tools/sys/word_count` → schema JSON:

```json
{
  "name": "word_count",
  "description": "count words in text",
  "category": "synthesised",
  "parameters": { ... }
}
```

writes rejected (read-only zone).

### writable zones

real VFS zones backed by `LocalBackend`, same permission model as existing zones:

| zone | read | write |
|------|------|-------|
| `/tools/sys/` | all | none (virtual) |
| `/tools/shared/` | all | all |
| `/tools/home/<ctx>/` | all | owner only |
| `/tools/flocks/<name>/` | flock members | flock members |

storage: `CHIBI_HOME/vfs/tools/{shared,home,flocks}/`

## synthesised tool lifecycle

### authoring

convention-based scheme source:

```scheme
;; /tools/shared/word-count.scm
(define tool-name "word_count")
(define tool-description "count words in text")
(define tool-parameters
  '((text . ((type . "string") (description . "the text to count words in")))))

(define (tool-execute args)
  (let ((text (cdr (assoc "text" args))))
    (number->string (length (string-split text #\space)))))
```

conventions:
- `tool-name` — string, registered tool name (must be unique)
- `tool-description` — string, shown to the LLM
- `tool-parameters` — alist of `(name . schema-alist)`, converted to JSON schema
- `tool-execute` — `(alist) -> string`, the handler

### loading

on startup, scan writable zones for `.scm` files. for each:

1. read source from VFS
2. create sandboxed tein context:
   - `Modules::Safe` (no eval, no filesystem escape)
   - `step_limit` (configurable, default 100,000)
   - `file_read` restricted to the tool's own VFS zone
   - no `file_write`
3. evaluate source → bindings land in the context
4. extract `tool-name`, `tool-description`, `tool-parameters`
5. validate (name unique, parameters well-formed, `tool-execute` is a procedure)
6. register as `ToolImpl::Synthesised` with `Arc<ThreadLocalContext>` (persistent
   mode — holds evaluated bindings, reused across invocations)

### dispatch

1. convert JSON args → scheme alist
2. call `tool-execute` in the held `ThreadLocalContext`
3. convert scheme string result → tool output string
4. step limit exceeded or error → informative error to LLM

### hot-reload

on VFS write to a `.scm` file in a writable zone:

1. unregister old tool if it existed, drop old tein context
2. load new source → create fresh tein context → register
3. load failure → log error, old tool stays unregistered

### deletion

VFS delete of a `.scm` file → unregister the tool.

### visibility scoping

synthesised tools are registered globally in the registry. visibility filtering
happens in `send.rs` based on the active context and zone permissions — the
registry stays dumb, visibility is policy.

**deferred:** the scoping filter in `send.rs` (checking `ToolImpl::Synthesised { vfs_path }`
against the active context's accessible VFS zones) is not implemented in Phase 4.
until it is, all synthesised tools are visible to all contexts. tracked as a follow-up
before merging Phase 4 to main.

## tein dependency

feature-gated, default on:

```toml
[features]
default = ["synthesised-tools"]
synthesised-tools = ["tein"]

[dependencies]
tein = { git = "https://github.com/emesal/tein", branch = "main", optional = true }
```

when off: `ToolImpl::Synthesised` variant exists but construction is gated,
loader is compiled out, `.scm` files in writable zones are ignored, dispatch
to `Synthesised` returns an error.

## migration — what changes

### removed

| current | replaced by |
|---------|------------|
| `Tool.path: PathBuf` | `Tool.r#impl: ToolImpl` |
| `is_memory_tool()` ... (8 predicates) | `Tool.category: ToolCategory` |
| `ToolType` enum in `send.rs` | `ToolCategory` in `tools/mod.rs` |
| `classify_tool_type()` in `send.rs` | gone — category set at registration |
| `is_mcp_tool(tool)` path prefix check | `matches!(tool.r#impl, ToolImpl::Mcp { .. })` |
| if/else chain in `chibi.rs::execute_tool` | `registry.dispatch()` |
| if/else chain in `send.rs::execute_tool_pure` | middleware + `registry.dispatch()` |
| `find_tool(&plugin_tools, name)` | `registry.get(name)` |
| `plugin_tools: Vec<Tool>` on `Chibi` | gone — tools live in registry |

### retained

| what | why |
|------|-----|
| `BuiltinToolDef` + static `*_TOOL_DEFS` | declaration source for builtin registration |
| `execute_*_tool()` per module | implementation logic, wrapped in `ToolHandler` closures |
| hook/permission middleware in `send.rs` | stays as policy layer wrapping `registry.dispatch()` |
| `ToolMetadata` | unchanged |

### new files

| file | contents |
|------|----------|
| `tools/registry.rs` | `ToolRegistry` struct |
| `tools/synthesised.rs` | synthesised tool loader + tein integration |
| `vfs/tools_backend.rs` | `ToolsBackend` implementing `VfsBackend` |

### Chibi struct

```rust
// before
pub struct Chibi {
    pub tools: Vec<Tool>,  // plugin + MCP tools
    // ...
}

// after
pub struct Chibi {
    pub registry: Arc<RwLock<ToolRegistry>>,  // single source of truth; shared with ToolsBackend
    // ...
}
```

## migration order

1. introduce `ToolImpl`, `ToolCategory`, `ToolCall`, `ToolRegistry` types
2. add `register_*_tools()` to each builtin module
3. build registry at `Chibi` init, register builtins + plugins + MCP
4. replace dispatch in `chibi.rs::execute_tool` with `registry.dispatch()`
5. replace dispatch in `send.rs::execute_tool_pure` with middleware + `registry.dispatch()`
6. remove `is_*_tool()`, `classify_tool_type()`, `Tool.path`, `find_tool()`
7. add `ToolsBackend` and mount `/tools/sys/`
8. add writable `/tools/` zones
9. add synthesised tool loader with tein
10. add hot-reload via VFS write hooks
