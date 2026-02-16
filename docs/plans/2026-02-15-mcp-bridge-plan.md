# chibi-mcp-bridge Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** bridge MCP servers into chibi's tool system via a standalone async daemon, so chibi can use any MCP-compatible tool provider.

**Architecture:** a new workspace crate `chibi-mcp-bridge` (tokio async binary) manages MCP server lifecycles and proxies tool calls over JSON-over-TCP. chibi-core connects synchronously via a new `tools/mcp.rs` module. MCP tools appear as regular `Tool` structs with virtual `mcp://` paths.

**Tech Stack:** rmcp (MCP client SDK), ratatoskr (LLM summaries), tokio, serde/serde_json

**Design doc:** `docs/plans/2026-02-15-mcp-bridge-design.md`

---

## Phase 1: daemon skeleton ✓

### Task 1: create crate and workspace entry ✓

**Files:**
- Create: `crates/chibi-mcp-bridge/Cargo.toml`
- Create: `crates/chibi-mcp-bridge/src/main.rs`
- Modify: `Cargo.toml` (workspace root)

**Step 1: create `Cargo.toml`**

```toml
[package]
name = "chibi-mcp-bridge"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
rmcp = { version = "0.15", features = ["client", "transport-child-process"] }
ratatoskr.workspace = true
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["full"] }
toml = "0.9"

[lints.rust]
dead_code = "deny"
```

Note: check `rmcp` features at build time -- the exact feature names for stdio
client transport may differ. the goal is: client handler + child process
transport. consult docs.rs/rmcp for the current feature list.

**Step 2: create minimal `main.rs`**

```rust
fn main() {
    eprintln!("chibi-mcp-bridge: not yet implemented");
    std::process::exit(1);
}
```

**Step 3: add to workspace**

In root `Cargo.toml`, add `"crates/chibi-mcp-bridge"` to `workspace.members`.

**Step 4: verify it builds**

Run: `cargo build -p chibi-mcp-bridge`
Expected: compiles successfully

**Step 5: commit**

```
feat(mcp-bridge): scaffold crate and workspace entry (#154)
```

---

### Task 2: protocol types ✓

**Files:**
- Create: `crates/chibi-mcp-bridge/src/protocol.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs` (add `mod protocol;`)

**Step 1: write tests (in `protocol.rs`)**

Test round-trip serialisation of each request and response variant:
- `Request::ListTools` serialises to `{"op": "list_tools"}`
- `Request::CallTool { server, tool, args }` serialises correctly
- `Request::GetSchema { server, tool }` serialises correctly
- `Response::ok_tools(...)` serialises with `"ok": true`
- `Response::ok_result(...)` serialises with `"ok": true`
- `Response::error(...)` serialises with `"ok": false`

**Step 2: run tests, verify they fail**

Run: `cargo test -p chibi-mcp-bridge`

**Step 3: implement types**

```rust
use serde::{Deserialize, Serialize};

/// Tool info returned by list_tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub server: String,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Incoming request from chibi-core
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    ListTools,
    CallTool {
        server: String,
        tool: String,
        args: serde_json::Value,
    },
    GetSchema {
        server: String,
        tool: String,
    },
}

/// Outgoing response to chibi-core
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Tools {
        ok: bool,
        tools: Vec<ToolInfo>,
    },
    Result {
        ok: bool,
        result: String,
    },
    Schema {
        ok: bool,
        schema: serde_json::Value,
    },
    Error {
        ok: bool,
        error: String,
    },
}
```

Add constructors `Response::ok_tools(...)`, `Response::ok_result(...)`,
`Response::ok_schema(...)`, `Response::error(...)`.

**Step 4: run tests, verify they pass**

Run: `cargo test -p chibi-mcp-bridge`

**Step 5: commit**

```
feat(mcp-bridge): protocol types for JSON-over-TCP (#154)
```

---

### Task 3: config parsing ✓

**Files:**
- Create: `crates/chibi-mcp-bridge/src/config.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs` (add `mod config;`)

**Step 1: write tests**

- parse a valid `mcp-bridge.toml` with one server entry
- parse with `idle_timeout_minutes` override
- parse with `[summary]` section
- parse with multiple servers
- missing file returns defaults (no servers, default timeout)

**Step 2: run tests, verify they fail**

**Step 3: implement**

```rust
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// MCP server definition
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
}

/// Summary generation config
#[derive(Debug, Clone, Deserialize)]
pub struct SummaryConfig {
    #[serde(default = "default_summary_model")]
    pub model: String,
}

fn default_summary_model() -> String {
    "ratatoskr:free/text-generation".to_string()
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self { model: default_summary_model() }
    }
}

/// Top-level bridge config from mcp-bridge.toml
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_minutes: u64,
    #[serde(default)]
    pub summary: SummaryConfig,
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}

fn default_idle_timeout() -> u64 { 5 }

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            idle_timeout_minutes: 5,
            summary: SummaryConfig::default(),
            servers: HashMap::new(),
        }
    }
}

impl BridgeConfig {
    pub fn load(home: &Path) -> Self {
        let path = home.join("mcp-bridge.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content)
                .unwrap_or_else(|e| {
                    eprintln!("[mcp-bridge] config parse error: {e}");
                    Self::default()
                }),
            Err(_) => Self::default(),
        }
    }
}
```

**Step 4: run tests, verify they pass**

**Step 5: commit**

```
feat(mcp-bridge): config parsing for mcp-bridge.toml (#154)
```

---

### Task 4: ~~refactor lock.rs~~ SKIPPED

> **Skipped:** the bridge lockfile is fundamentally different from ContextLock
> (JSON with PID/address, no heartbeat, liveness via kill(pid,0)). the bridge
> writes its own lockfile directly in main.rs using the same atomic O_CREAT |
> O_EXCL pattern. refactoring lock.rs would add complexity for marginal reuse.

**Files:**
- Modify: `crates/chibi-core/src/lock.rs`

The existing `ContextLock` uses atomic `O_CREAT | O_EXCL`, heartbeat, and
staleness detection. We need the same mechanics for the bridge lockfile but
with JSON content instead of a bare timestamp.

**Step 1: write tests for the new `Lockfile` API**

Test the generic lockfile primitives:
- `Lockfile::try_create(path, content)` creates file atomically
- `Lockfile::try_create` fails with `AlreadyExists` if file exists
- `Lockfile::read(path)` returns content string
- `Lockfile::is_stale(path, heartbeat_secs)` detects staleness (same logic)
- `Lockfile::remove(path)` removes the file

**Step 2: run tests, verify they fail**

**Step 3: extract primitives**

Extract the static methods `try_create_lock`, `is_stale`, `touch` into a
`Lockfile` struct (or module-level functions) that `ContextLock` delegates to.
`ContextLock`'s API stays identical — this is a pure refactor.

The key insight: `try_create_lock` currently writes a bare timestamp.
Generalize it to accept content as a parameter. `is_stale` reads the file and
parses the timestamp — for `ContextLock` the content IS the timestamp; for
bridge locks, we parse JSON to extract the `started` field.

Approach: keep `Lockfile` content-agnostic. It handles atomic creation (with
arbitrary string content), staleness (by file mtime or explicit callback), and
removal. `ContextLock` continues to use timestamp-as-content. The bridge will
use JSON content and check PID liveness externally.

Simplest: just make `try_create_lock` and `touch` accept a content string
parameter instead of hardcoding the timestamp. `ContextLock::acquire` passes
`timestamp.to_string()`. The bridge will pass its JSON.

**Step 4: run all existing lock tests**

Run: `cargo test -p chibi-core lock`
Expected: all existing tests still pass (pure refactor)

**Step 5: commit**

```
refactor(lock): generalize lockfile primitives for reuse (#154)
```

---

### Task 5: TCP listener + idle timeout ✓

**Files:**
- Modify: `crates/chibi-mcp-bridge/src/main.rs`

**Step 1: write integration test**

Test that the daemon:
- binds to a random port on loopback
- writes a lockfile with `{"pid", "address", "started"}`
- responds to a `list_tools` request with `{"ok": true, "tools": []}`
  (no servers configured)
- shuts down after idle timeout

**Step 2: implement `main.rs`**

```rust
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::time::Instant;

mod config;
mod protocol;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = chibi_home();
    let config = config::BridgeConfig::load(&home);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    eprintln!("[mcp-bridge] listening on {addr}");

    write_lockfile(&home, &addr)?;

    let idle_timeout = Duration::from_secs(config.idle_timeout_minutes * 60);
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    // Spawn idle timeout watchdog
    // ...

    loop {
        tokio::select! {
            Ok((stream, _)) = listener.accept() => {
                *last_activity.lock().unwrap() = Instant::now();
                handle_connection(stream).await;
            }
            // idle timeout check
        }
    }
}
```

The lockfile uses the generalized `try_create_lock` from task 4. Content:
```json
{"pid": <PID>, "address": "127.0.0.1:<PORT>", "started": <UNIX_TIMESTAMP>}
```

On shutdown (idle timeout or signal), remove the lockfile.

`chibi_home()`: check `CHIBI_HOME` env, fall back to `~/.chibi`.

**Step 3: run test, verify it passes**

**Step 4: commit**

```
feat(mcp-bridge): TCP listener with idle timeout and lockfile (#154)
```

---

### Task 6: MCP server management via rmcp ✓

**Files:**
- Create: `crates/chibi-mcp-bridge/src/server.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs`

**Step 1: implement `ServerManager`**

```rust
use rmcp::ServiceExt;
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;
use std::collections::HashMap;

pub struct ManagedServer {
    service: rmcp::Service<rmcp::RoleClient>,
    tools: Vec<rmcp::model::Tool>,
}

pub struct ServerManager {
    servers: HashMap<String, ManagedServer>,
}
```

Key methods:
- `ServerManager::new()` — empty
- `start_server(name, config) -> Result<()>` — spawns child process via
  `TokioChildProcess`, connects rmcp client, calls `list_tools`, stores result
- `list_all_tools() -> Vec<ToolInfo>` — aggregates tools from all servers,
  prefixed by server name
- `call_tool(server, tool, args) -> Result<String>` — routes to correct
  server, calls `service.call_tool(...)`, extracts text result
- `get_schema(server, tool) -> Result<Value>` — returns full tool schema

**Step 2: wire into `main.rs`**

On startup, iterate `config.servers` and call `start_server` for each.
Route incoming TCP requests through `ServerManager`.

**Step 3: manual test**

If you have an MCP server available (e.g. serena), add it to
`~/.chibi/mcp-bridge.toml` and test with a TCP client:

```bash
echo '{"op":"list_tools"}' | nc 127.0.0.1 <PORT>
```

**Step 4: commit**

```
feat(mcp-bridge): MCP server lifecycle management via rmcp (#154)
```

---

### Task 7: request routing (bridge.rs) ✓

**Files:**
- Create: `crates/chibi-mcp-bridge/src/bridge.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs`

**Step 1: implement `Bridge`**

`Bridge` owns the `ServerManager` and handles request dispatch:

```rust
pub struct Bridge {
    server_manager: ServerManager,
}

impl Bridge {
    pub async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::ListTools => {
                let tools = self.server_manager.list_all_tools();
                Response::ok_tools(tools)
            }
            Request::CallTool { server, tool, args } => {
                match self.server_manager.call_tool(&server, &tool, &args).await {
                    Ok(result) => Response::ok_result(result),
                    Err(e) => Response::error(e.to_string()),
                }
            }
            Request::GetSchema { server, tool } => {
                match self.server_manager.get_schema(&server, &tool) {
                    Ok(schema) => Response::ok_schema(schema),
                    Err(e) => Response::error(e.to_string()),
                }
            }
        }
    }
}
```

**Step 2: refactor `main.rs` to use `Bridge`**

The TCP handler deserialises the request, passes it to `bridge.handle_request`,
serialises the response, and writes it back.

**Step 3: commit**

```
feat(mcp-bridge): request routing via Bridge (#154)
```

---

## Phase 2: chibi-core integration

### Task 8: `tools/mcp.rs` — bridge client

**Files:**
- Create: `crates/chibi-core/src/tools/mcp.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs` (add `pub mod mcp;`, re-exports)

**Step 1: write tests**

- `parse_mcp_path("mcp://serena/find_symbol")` returns `("serena", "find_symbol")`
- `parse_mcp_path("mcp://foo/bar_baz")` works with underscores
- `parse_mcp_path("/usr/bin/tool")` returns `None`
- `is_mcp_tool(tool)` returns true for `mcp://` paths, false otherwise
- `mcp_tool_from_info(ToolInfo)` produces a `Tool` with correct `mcp://` path
  and prefixed name

**Step 2: run tests, verify they fail**

**Step 3: implement**

```rust
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Check if a tool has an MCP virtual path
pub fn is_mcp_tool(tool: &super::Tool) -> bool {
    tool.path.to_str().is_some_and(|p| p.starts_with("mcp://"))
}

/// Parse server and tool name from an mcp:// path
pub fn parse_mcp_path(path: &Path) -> Option<(&str, &str)> {
    let s = path.to_str()?;
    let rest = s.strip_prefix("mcp://")?;
    rest.split_once('/')
}

/// Convert bridge ToolInfo into a chibi Tool
pub fn mcp_tool_from_info(
    server: &str,
    name: &str,
    description: &str,
    parameters: serde_json::Value,
) -> super::Tool {
    super::Tool {
        name: format!("{server}_{name}"),
        description: description.to_string(),
        parameters,
        path: PathBuf::from(format!("mcp://{server}/{name}")),
        hooks: vec![],
        metadata: super::ToolMetadata::new(),
        summary_params: vec![],
    }
}
```

TCP client functions:

- `read_bridge_address(home: &Path) -> io::Result<SocketAddr>` — reads and
  parses lockfile JSON, checks PID liveness via `kill(pid, 0)`
- `ensure_bridge_running(home: &Path) -> io::Result<SocketAddr>` — tries
  `read_bridge_address`; if stale/missing, spawns `chibi-mcp-bridge` as a
  detached child, waits for lockfile to appear (poll with short sleep, timeout
  after 10s)
- `send_request(addr: SocketAddr, request: &str) -> io::Result<String>` —
  opens TCP connection, writes request + shutdown write half, reads response
- `load_mcp_tools(home: &Path) -> io::Result<Vec<super::Tool>>` — calls
  `ensure_bridge_running`, sends `list_tools`, deserialises, maps to `Tool`s
- `execute_mcp_tool(tool: &super::Tool, args: &serde_json::Value, home: &Path) -> io::Result<String>`
  — parses `mcp://` path, sends `call_tool`, returns result string

**Step 4: run tests, verify they pass**

**Step 5: commit**

```
feat(mcp): bridge client module in chibi-core (#154)
```

---

### Task 9: integrate MCP tools into Chibi load + dispatch

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs`
- Modify: `crates/chibi-core/src/api/send.rs`

**Step 1: load MCP tools alongside plugins**

In `Chibi::load_with_options()` (`chibi.rs:153-174`), after loading plugin
tools, also load MCP tools and extend the `tools` vec:

```rust
let mut tools = tools::load_tools(&app.plugins_dir, verbose)?;

// Load MCP bridge tools (non-fatal: bridge may not be configured)
match tools::mcp::load_mcp_tools(&app.chibi_dir) {
    Ok(mcp_tools) => {
        if verbose && !mcp_tools.is_empty() {
            eprintln!("[MCP: {} tools loaded]", mcp_tools.len());
        }
        tools.extend(mcp_tools);
    }
    Err(e) => {
        if verbose {
            eprintln!("[MCP: bridge unavailable: {e}]");
        }
    }
}
```

**Step 2: add MCP dispatch branch in `Chibi::execute_tool()`**

In `chibi.rs:302-366`, add a branch before the plugin fallback:

```rust
// Try MCP tools (virtual path mcp://server/tool)
if let Some(tool) = tools::find_tool(&self.tools, name)
    && tools::mcp::is_mcp_tool(tool)
{
    return tools::mcp::execute_mcp_tool(tool, &args, &self.app.chibi_dir);
}
```

**Step 3: add MCP to `ToolType` enum and `classify_tool_type`**

In `send.rs`, add `Mcp` variant to `ToolType` and a classification branch:

```rust
enum ToolType {
    Builtin,
    File,
    Agent,
    Coding,
    Mcp,     // <-- new
    Plugin,
}

impl ToolType {
    fn as_str(&self) -> &'static str {
        match self {
            // ...existing...
            Self::Mcp => "mcp",
        }
    }
}
```

In `classify_tool_type`, add before the plugin check:

```rust
} else if plugin_names.iter().any(|n| {
    tools.iter().find(|t| t.name == *n)
        .is_some_and(|t| mcp::is_mcp_tool(t))
}) {
    ToolType::Mcp
```

Actually, simpler: MCP tools ARE in the plugin `tools` vec, so
`classify_tool_type` receives their names in `plugin_names`. We need a way to
distinguish them. Since we have access to the tools slice, we can check the
path prefix. But `classify_tool_type` only receives `plugin_names: &[&str]`
not the full `Tool` structs.

Cleanest approach: pass `tools: &[Tool]` to `classify_tool_type` instead of
just names, and check `is_mcp_tool()`. Or: maintain a separate `mcp_names`
list. The function signature change is cleaner.

Update `classify_tool_type` signature:
```rust
fn classify_tool_type(name: &str, tools: &[Tool]) -> ToolType
```

And check:
```rust
} else if tools.iter().any(|t| t.name == name && mcp::is_mcp_tool(t)) {
    ToolType::Mcp
} else if tools.iter().any(|t| t.name == name) {
    ToolType::Plugin
} else {
    ToolType::Plugin // unknown defaults to plugin
}
```

Update all call sites (there are two: `build_tool_info_list` and
`filter_tools_by_config`).

**Step 4: add `mod mcp` and re-exports to `tools/mod.rs`**

```rust
pub mod mcp;
pub use mcp::is_mcp_tool;
```

**Step 5: run tests**

Run: `cargo test -p chibi-core`
Expected: all existing tests pass, new classification tests pass

**Step 6: commit**

```
feat(mcp): integrate MCP tools into load and dispatch (#154)
```

---

### Task 10: end-to-end integration test

**Files:**
- Create: `crates/chibi-core/tests/mcp_integration.rs` (or add to existing test module)

**Step 1: write test**

Test the full flow with a mock bridge (a TCP listener in-process):
- spawn a test TCP server that responds to `list_tools` with one fake tool
- write a lockfile pointing to the test server
- call `load_mcp_tools` — verify it returns a `Tool` with `mcp://` path
- call `execute_mcp_tool` — verify it sends `call_tool` and returns the result

**Step 2: run test, verify it passes**

**Step 3: commit**

```
test(mcp): end-to-end integration test with mock bridge (#154)
```

---

## Phase 3: summary generation

### Task 11: summary cache

**Files:**
- Create: `crates/chibi-mcp-bridge/src/cache.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs`

**Step 1: write tests**

- empty cache returns `None` for any tool
- `set(key, summary)` + `get(key)` roundtrip
- cache key is `"server:tool:schema_hash"` — different schema hash = cache miss
- `load` from a JSONL file populates the cache
- `save` writes the cache to JSONL

**Step 2: implement**

```rust
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::path::Path;

pub struct SummaryCache {
    entries: HashMap<String, String>,
    path: PathBuf,
}

impl SummaryCache {
    pub fn load(home: &Path) -> Self { ... }
    pub fn save(&self) -> io::Result<()> { ... }
    pub fn get(&self, server: &str, tool: &str, schema: &Value) -> Option<&str> { ... }
    pub fn set(&mut self, server: &str, tool: &str, schema: &Value, summary: String) { ... }
}

fn cache_key(server: &str, tool: &str, schema: &Value) -> String {
    let hash = Sha256::digest(serde_json::to_string(schema).unwrap_or_default());
    format!("{server}:{tool}:{}", hex::encode(&hash[..8]))
}
```

Cache file: `CHIBI_HOME/mcp-bridge/cache.jsonl`, one JSON object per line:
```json
{"key": "serena:find_symbol:a1b2c3d4", "summary": "find code symbols by name path pattern"}
```

**Step 3: run tests, verify they pass**

**Step 4: commit**

```
feat(mcp-bridge): summary cache with schema-hash invalidation (#154)
```

---

### Task 12: LLM summary generation via ratatoskr

**Files:**
- Modify: `crates/chibi-mcp-bridge/src/cache.rs`
- Modify: `crates/chibi-mcp-bridge/src/server.rs`
- Modify: `crates/chibi-mcp-bridge/src/main.rs`

**Step 1: implement `generate_summary`**

```rust
pub async fn generate_summary(
    model: &str,
    tool_name: &str,
    description: &str,
    schema: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    // Use ratatoskr to call the free text-generation model
    // Prompt: "Compress this MCP tool description into a single concise sentence
    //          suitable for an LLM tool listing. Include key parameters.
    //          Tool: {tool_name}
    //          Description: {description}
    //          Schema: {schema}"
    // Return the generated summary
}
```

**Step 2: integrate into startup flow**

After `ServerManager` loads all tools, iterate tools and fill cache gaps:
- for each tool, check `cache.get(server, tool, schema)`
- if missing, call `generate_summary`
- store result in cache
- save cache to disk

Use the original description as fallback until generation completes. This
means `list_tools` returns immediately with originals, and summaries are
populated in the background.

**Step 3: commit**

```
feat(mcp-bridge): LLM-powered tool summary generation (#154)
```

---

### Task 13: update AGENTS.md

**Files:**
- Modify: `AGENTS.md`

**Step 1: add MCP bridge to architecture section**

Add the new crate to the workspace diagram:
```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)

chibi-mcp-bridge (binary, async daemon)
    communicates with chibi-core via JSON-over-TCP
```

Add `mcp.rs` to the chibi-core file listing.

Add the `mcp-bridge.toml` config file to the storage layout.

Add the `mcp://` virtual path convention to a new section.

**Step 2: commit**

```
docs: add MCP bridge to architecture docs (#154)
```
