# Architecture

Cargo workspace with four crates:

```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)

chibi-mcp-bridge (binary, async daemon)
    communicates with chibi-core via JSON-over-TCP
```

## Crates

### chibi-core — Library crate (reusable logic)

- `chibi.rs` — Main `Chibi` struct, tool execution
- `context.rs`, `state/` — Context management, file I/O, config resolution
- `api/` — Request building, streaming, agentic loop (`send.rs`), compaction
- `gateway.rs` — Type conversions between chibi and ratatoskr; context window auto-resolution
- `model_info.rs` — Model metadata retrieval and formatting
- `tools/` — Plugins, hooks, built-in tools (builtin, coding, file, agent, vfs categories), URL security policy, MCP bridge client (`mcp.rs`)
- `vfs/` — Virtual file system: path validation, backend trait, permission model, local backend, router
- `partition.rs` — Partitioned transcript storage with bloom filters
- `config.rs` — Core configuration types (`Config`, `LocalConfig`, `ResolvedConfig`)
- `agents_md.rs` — AGENTS.md discovery and loading (VCS-aware hierarchy)
- `vcs.rs` — VCS root detection (`.git`, `.hg`, etc.)
- `index/` — Codebase indexing (SQLite WAL, symbol extraction, language plugin interface)
- `execution.rs` — Shared command execution (`execute_command`, `CommandEffect`)
- `input.rs` — Core input types (`Command`, `ExecutionFlags`, `Inspectable`)
- `output.rs` — `OutputSink` trait (abstraction over CLI text / JSON output)

### chibi-cli — Binary crate (CLI-specific)

- `main.rs` — Entry point, command dispatch
- `cli.rs` — Argument parsing (clap)
- `input.rs` — Input types (`ChibiInput`, `ContextSelection`, `UsernameOverride`)
- `session.rs` — CLI session state (implied context)
- `config.rs` — CLI-specific config (markdown, images)
- `output.rs` — `OutputHandler` (`OutputSink` impl for terminal)
- `sink.rs` — `CliResponseSink` (`ResponseSink` impl, markdown streaming)
- `markdown.rs` — Markdown rendering pipeline (streamdown-rs integration)
- `image_cache.rs` — Image caching for terminal output

### chibi-json — Binary crate (JSON-mode, programmatic)

- `main.rs` — Entry point, command dispatch
- `input.rs` — `JsonInput` (stdin JSON, stateless per invocation)
- `output.rs` — `JsonOutputSink` (JSONL `OutputSink` impl)
- `sink.rs` — `JsonResponseSink` (JSONL `ResponseSink` impl)

### chibi-mcp-bridge — Binary crate (async daemon)

- `main.rs` — Entry point, TCP listener, idle timeout, lockfile management
- `bridge.rs` — Request dispatch (`Bridge` struct)
- `server.rs` — MCP server lifecycle (`ServerManager`, rmcp client)
- `protocol.rs` — JSON-over-TCP protocol types (`Request`, `Response`, `ToolInfo`)
- `config.rs` — `BridgeConfig` from `mcp-bridge.toml`
- `cache.rs` — Summary cache with schema-hash invalidation (JSONL persistence)
- `summary.rs` — LLM-powered tool summary generation via ratatoskr

## LLM Communication

Delegated to the [ratatoskr](https://github.com/emesal/ratatoskr) crate, which handles HTTP requests, SSE streaming, and response parsing. Chibi's `gateway.rs` converts between internal types and ratatoskr's `ModelGateway` interface. This abstraction keeps HTTP/networking concerns out of chibi's core logic.

## MCP Tools

MCP tools use virtual `mcp://server/tool` paths and appear as regular `Tool` structs. chibi-core's `tools/mcp.rs` discovers the bridge via its lockfile, auto-spawns it if needed, and proxies tool calls over TCP. Tool names are prefixed with the server name (e.g. `serena_find_symbol`).

## Data Flow

- **CLI:** args → `parse()` → `ChibiInput` → `execute_from_input()` → core APIs
- **JSON:** stdin → `JsonInput` → `execute_json_command()` → core APIs

## Storage Layout

```
~/.chibi/
├── config.toml, models.toml
├── state.json               # Context metadata (core)
├── session.json             # Navigation state (CLI)
├── prompts/{chibi,reflection,compaction,continuation}.md
├── plugins/
├── mcp-bridge.toml           # MCP server definitions
├── mcp-bridge.lock            # Bridge daemon lockfile (pid, address)
├── mcp-bridge/cache.jsonl     # LLM-generated tool summaries
├── vfs/                       # Virtual file system (shared storage)
│   ├── shared/                # World-writable zone
│   ├── home/<context>/        # Per-context home directories
│   └── sys/                   # System-only zone
└── contexts/<name>/
    ├── context.jsonl          # LLM window (compaction-bounded)
    ├── transcript/            # Authoritative log (partitioned)
    ├── local.toml, todos.md, goals.md, inbox.jsonl, summary.md
    └── tool_cache/
```

Home directory: `--home` flag > `CHIBI_HOME` env > `~/.chibi`

## CLI Conventions

- stdout: LLM output only (pipeable); markdown-rendered when TTY
- stderr: Diagnostics (with `-v`)
- `--raw` disables markdown rendering
