# MCP Server Integration

Chibi can use tools from any [MCP](https://modelcontextprotocol.io/)-compatible server — code intelligence, databases, APIs, or anything else with an MCP interface. MCP tools appear alongside built-in tools and plugins; the LLM uses them the same way.

## How It Works

A standalone daemon (`chibi-mcp-bridge`) manages MCP server lifecycles and proxies tool calls over TCP. Chibi starts the daemon automatically when MCP servers are configured.

```
chibi-core ──TCP──▶ chibi-mcp-bridge ──stdio──▶ local MCP server(s)
                                     ──HTTP──▶  remote MCP server(s)
```

The bridge:
- spawns local MCP servers as child processes (stdio transport)
- connects to remote MCP servers over HTTP (streamable HTTP transport)
- discovers their tools via the MCP protocol
- proxies tool calls from chibi to the correct server
- shuts down automatically after 5 minutes of inactivity

## Setup

### 1. Install the bridge binary

The bridge is built alongside chibi:

```bash
cargo install --path crates/chibi-mcp-bridge
```

Or if you installed chibi from the workspace root, it's already built.

### 2. Configure MCP servers

Create `~/.chibi/mcp-bridge.toml`:

```toml
[servers.serena]
command = "uvx"
args = ["serena"]

[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/projects"]
```

Each server entry is either a **local** stdio server or a **remote** HTTP server — determined by which field is present.

**Local server** (spawns a child process):

| Field | Type | Description |
|-------|------|-------------|
| `command` | string | Executable to run |
| `args` | string[] | Command-line arguments (optional) |

**Remote server** (connects via HTTP):

| Field | Type | Description |
|-------|------|-------------|
| `url` | string | Full URL of the MCP endpoint |
| `headers` | table | HTTP headers to send with every request (optional) |

`command` and `url` are mutually exclusive — a server entry must have one or the other.

### Remote servers

To connect to a remote MCP server, specify a `url` instead of a `command`:

```toml
[servers.remote-tools]
url = "https://mcp.example.com/mcp"
```

For servers that require authentication, add a `[servers.<name>.headers]` table:

```toml
[servers.remote-tools]
url = "https://mcp.example.com/mcp"

[servers.remote-tools.headers]
Authorization = "Bearer sk-..."
```

You can mix local and remote servers freely:

```toml
[servers.serena]
command = "uvx"
args = ["serena"]

[servers.remote-tools]
url = "https://mcp.example.com/mcp"

[servers.remote-tools.headers]
Authorization = "Bearer sk-..."
```

### 3. Use it

That's it. On the next chibi invocation, MCP tools are loaded automatically:

```bash
chibi -v "Find the parse function in my codebase"
# [MCP: 42 tools loaded]
```

Tools are named `<server>_<tool>` (e.g. `serena_find_symbol`) and the LLM can call them directly.

## Configuration Reference

The full `mcp-bridge.toml` format:

```toml
# How long the bridge stays alive without requests (default: 5)
idle_timeout_minutes = 5

# LLM-powered tool summary generation (optional)
[summary]
enabled = true                             # set to false to disable
model = "ratatoskr:free/text-generation"   # default

# Local MCP server (stdio transport)
[servers.local-name]
command = "path/to/server"
args = ["--flag", "value"]

# Remote MCP server (streamable HTTP transport)
[servers.remote-name]
url = "https://mcp.example.com/mcp"

[servers.remote-name.headers]
Authorization = "Bearer sk-..."
```

### Tool summaries

The bridge can generate concise one-sentence summaries of MCP tool descriptions using an LLM. This runs in the background on first startup and caches results in `~/.chibi/mcp-bridge/cache.jsonl`. Summaries are regenerated automatically when a tool's schema changes.

To disable summary generation entirely, set `enabled = false` in the `[summary]` section. When disabled, no summaries are generated and existing cached summaries are ignored. Re-enabling picks up where it left off — the cache remains intact on disk.

## Architecture

MCP tools use virtual `mcp://server/tool` paths internally. From the LLM's perspective, they're indistinguishable from regular tools.

### Daemon lifecycle

1. chibi checks for `~/.chibi/mcp-bridge.toml` — if absent, MCP is skipped entirely
2. chibi reads `~/.chibi/mcp-bridge.lock` to find a running bridge
3. if no bridge is running, chibi spawns one as a detached process
4. the bridge binds to a random localhost port, writes its address to the lockfile
5. chibi sends `list_tools` over TCP, receives tool definitions
6. tool calls are proxied via `call_tool` requests
7. the bridge shuts down after `idle_timeout_minutes` of inactivity

### Files

| Path | Purpose |
|------|---------|
| `~/.chibi/mcp-bridge.toml` | server definitions and bridge config |
| `~/.chibi/mcp-bridge.lock` | daemon lockfile (pid, address, timestamp) |
| `~/.chibi/mcp-bridge/cache.jsonl` | cached tool summaries |

### Protocol

The bridge speaks JSON-over-TCP (one JSON object per connection, newline-delimited):

```json
{"op": "list_tools"}
{"op": "call_tool", "server": "serena", "tool": "find_symbol", "args": {...}}
{"op": "get_schema", "server": "serena", "tool": "find_symbol"}
```

## Examples

- [Using the MCP bridge with Serena](mcp-bridge-serena.md) — complete walkthrough using a semantic code intelligence server

## Troubleshooting

**"MCP: bridge unavailable"** — the bridge binary isn't in PATH or next to the chibi binary. Run `cargo install --path crates/chibi-mcp-bridge`.

**Tools not appearing** — check that `mcp-bridge.toml` exists and is valid TOML. Run `chibi -v` to see diagnostic output.

**Stale lockfile** — if the bridge crashes, its lockfile may persist. Chibi detects stale lockfiles (via PID liveness check) and cleans them up automatically, but you can also delete `~/.chibi/mcp-bridge.lock` manually.
