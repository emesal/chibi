# chibi-mcp-bridge design

issue: #154

## summary

standalone daemon binary bridging MCP servers into chibi's tool system.
gives chibi access to any MCP-compatible tool provider while keeping token
overhead minimal through LLM-generated tool description summaries.

## architecture

```
chibi-core (sync)
    |
    |  JSON-over-TCP (loopback)
    |
chibi-mcp-bridge (async, tokio — long-running daemon)
    |
    |  MCP stdio transport (via rmcp)
    |
    +-- serena
    +-- other-mcp-server
    +-- ...
```

## tool integration — virtual path pattern

MCP tools are regular `Tool` structs with `path` set to `mcp://<server>/<tool>`.
the existing tool pipeline handles them transparently:

- `tools_to_api_format()` — works as-is
- `ToolsConfig` filtering (include/exclude) — works as-is
- `tool_call_summary()` — works as-is
- `execute_tool()` — new branch before the plugin fallback: parse `mcp://`
  prefix, route to bridge client

this establishes a **virtual tool source** pattern. `path` becomes a URI, not
strictly a filesystem path. future extension languages (e.g. `ext://lua/tool`)
would follow the same convention.

## crate structure

```
crates/chibi-mcp-bridge/
+-- Cargo.toml          # tokio, rmcp, ratatoskr, serde, serde_json
+-- src/
    +-- main.rs         # daemon entry, TCP listener, idle timeout
    +-- config.rs       # mcp-bridge.toml parsing
    +-- server.rs       # spawn/manage MCP server processes via rmcp
    +-- cache.rs        # summary cache + LLM summarisation
    +-- protocol.rs     # JSON-over-TCP request/response types
    +-- bridge.rs       # route requests to appropriate MCP server
```

chibi-core side:

```
crates/chibi-core/src/tools/mcp.rs   # bridge client, tool registration
```

## chibi-core integration (`tools/mcp.rs`)

three public functions:

- `load_mcp_tools(home: &Path) -> io::Result<Vec<Tool>>` — connects to
  bridge daemon, calls `list_tools`, returns `Tool` structs with `mcp://`
  paths. tools are prefixed by server name (e.g. `serena_find_symbol`).
- `execute_mcp_tool(tool: &Tool, args: &Value) -> io::Result<String>` —
  parses server/tool from the `mcp://` path, sends `call_tool` to bridge.
- `ensure_bridge_running(home: &Path) -> io::Result<SocketAddr>` — checks
  lockfile, verifies PID liveness, spawns daemon if needed.

dispatch in `Chibi::execute_tool()` gains a new branch:

```rust
// Try MCP tools (virtual path mcp://server/tool)
if tool.path.to_str().is_some_and(|p| p.starts_with("mcp://")) {
    return mcp::execute_mcp_tool(tool, &args).await;
}
```

## daemon lifecycle

- spawned lazily by chibi-core on first MCP tool interaction
- listens on `127.0.0.1:<random-port>` (loopback only)
- lockfile at `CHIBI_HOME/mcp-bridge.lock`
- idle timeout (configurable, default 5 minutes), exits after no requests

### lockfile

reuses primitives from `lock.rs` (atomic `O_CREAT | O_EXCL`, staleness
detection, heartbeat). the lock content is JSON rather than a bare timestamp:

```json
{"pid": 12345, "address": "127.0.0.1:49152", "started": 1739600000}
```

approach: extract shared lock mechanics from `ContextLock` into a lower-level
`Lockfile` type. both `ContextLock` and the bridge lock build on it.
staleness is checked via PID liveness (`kill(pid, 0)`) in addition to the
timestamp-based heartbeat check.

## protocol (JSON-over-TCP)

requests:
```json
{"op": "list_tools"}
{"op": "call_tool", "server": "serena", "tool": "find_symbol", "args": {...}}
{"op": "get_schema", "server": "serena", "tool": "find_symbol"}
```

responses:
```json
{"ok": true, "tools": [{"server": "serena", "name": "find_symbol", "description": "...", "parameters": {...}}]}
{"ok": true, "result": "..."}
{"ok": false, "error": "..."}
```

each TCP connection handles one request-response pair (no multiplexing needed;
chibi-core is synchronous and single-threaded).

## summary generation (phase 3)

MCP tool schemas are verbose — a server like serena registers 25+ tools with
detailed descriptions, costing thousands of tokens per turn even when unused.

- on startup, bridge checks `CHIBI_HOME/mcp-bridge/cache.jsonl`
- missing summaries generated via ratatoskr `ratatoskr:free/text-generation`
- cache keyed by `server:tool:schema_hash` — stale entries naturally
  invalidated when schemas change
- originals serve as fallback until summaries exist — immediately functional,
  progressively more efficient

## config

`CHIBI_HOME/mcp-bridge.toml`:

```toml
idle_timeout_minutes = 5

[summary]
model = "ratatoskr:free/text-generation"

[servers.serena]
command = "uvx"
args = ["--from", "git+https://github.com/oraios/serena", "serena", "start-mcp-server"]

[servers.another]
command = "npx"
args = ["-y", "some-mcp-server"]
```

## implementation phases

### phase 1: daemon skeleton + protocol
- TCP listener, lockfile (with `lock.rs` refactor), idle timeout
- config parsing (`mcp-bridge.toml`)
- spawn/manage MCP servers via rmcp
- proxy `list_tools` and `call_tool`

### phase 2: chibi-core integration
- `tools/mcp.rs` — bridge client, `ensure_bridge_running`, tool loading
- virtual `mcp://` path convention
- dispatch branch in `Chibi::execute_tool()`
- `load_mcp_tools()` called alongside `load_tools()` in `Chibi::load()`

### phase 3: summary generation
- ratatoskr integration for compressing tool descriptions
- cache management (eager fill on discovery, JSONL storage)
- fallback to originals when cache is cold

## dependencies

- [rmcp](https://github.com/modelcontextprotocol/rust-sdk) — official rust
  MCP SDK (client + stdio transport)
- [ratatoskr](https://github.com/emesal/ratatoskr) — LLM API client (for
  summary generation, already a workspace dependency)
- tokio — async runtime for the daemon
