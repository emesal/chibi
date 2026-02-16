# CLAUDE.md

## Vision

Chibi is a minimal, composable building block for LLM interactions — not an agent framework. It provides persistent context storage, extensible behavior via plugins/hooks, and communication primitives. Everything else (coordination patterns, workflows, domain behaviors) lives in plugins. This separation keeps chibi small and enables unlimited experimentation at the plugin layer.

## Principles

- Establish patterns now that scale well, refactor liberally when beneficial.
- Backwards compatibility not a priority, legacy code unwanted. (Pre-alpha.)
- Focused, secure core. Protect file operations from corruption and race conditions.
- Self-documenting code; keep symbols, comments, and docs consistent.
- Missing or incorrect documentation including code comments are critical bugs.
- Comprehensive tests including edge cases.
- Remind user about `just pre-push` before pushing and `just merge-to-dev` when merging feature branches.

## Build

```bash
cargo build                              # Debug build
cargo test                               # Run tests
cargo install --path .                   # Install to ~/.cargo/bin
```

Git dependencies: [ratatoskr](https://github.com/emesal/ratatoskr) (LLM API client), [streamdown-rs](https://github.com/emesal/streamdown-rs) (markdown renderer).

## Architecture

Cargo workspace with four crates:

```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)

chibi-mcp-bridge (binary, async daemon)
    communicates with chibi-core via JSON-over-TCP
```

**`crates/chibi-core/`** — Library crate (reusable logic)
- `chibi.rs` — Main `Chibi` struct, tool execution
- `context.rs`, `state/` — Context management, file I/O, config resolution
- `api/` — Request building, streaming, agentic loop (`send.rs`), compaction
- `gateway.rs` — Type conversions between chibi and ratatoskr; context window auto-resolution
- `model_info.rs` — Model metadata retrieval and formatting
- `tools/` — Plugins, hooks, built-in tools (builtin, coding, file, agent categories), URL security policy, MCP bridge client (`mcp.rs`)
- `partition.rs` — Partitioned transcript storage with bloom filters
- `config.rs` — Core configuration types (`Config`, `LocalConfig`, `ResolvedConfig`)
- `agents_md.rs` — AGENTS.md discovery and loading (VCS-aware hierarchy)
- `vcs.rs` — VCS root detection (`.git`, `.hg`, etc.)
- `index/` — Codebase indexing (SQLite WAL, symbol extraction, language plugin interface)
- `execution.rs` — Shared command execution (`execute_command`, `CommandEffect`)
- `input.rs` — Core input types (`Command`, `ExecutionFlags`, `Inspectable`)
- `output.rs` — `OutputSink` trait (abstraction over CLI text / JSON output)

**LLM Communication:** Delegated to the [ratatoskr](https://github.com/emesal/ratatoskr) crate, which handles HTTP requests, SSE streaming, and response parsing. Chibi's `gateway.rs` converts between internal types and ratatoskr's `ModelGateway` interface. This abstraction keeps HTTP/networking concerns out of chibi's core logic.

**`crates/chibi-cli/`** — Binary crate (CLI-specific)
- `main.rs` — Entry point, command dispatch
- `cli.rs` — Argument parsing (clap)
- `input.rs` — Input types (`ChibiInput`, `ContextSelection`, `UsernameOverride`)
- `session.rs` — CLI session state (implied context)
- `config.rs` — CLI-specific config (markdown, images)
- `output.rs` — `OutputHandler` (`OutputSink` impl for terminal)
- `sink.rs` — `CliResponseSink` (`ResponseSink` impl, markdown streaming)
- `markdown.rs` — Markdown rendering pipeline (streamdown-rs integration)
- `image_cache.rs` — Image caching for terminal output

**`crates/chibi-json/`** — Binary crate (JSON-mode, programmatic)
- `main.rs` — Entry point, command dispatch
- `input.rs` — `JsonInput` (stdin JSON, stateless per invocation)
- `output.rs` — `JsonOutputSink` (JSONL `OutputSink` impl)
- `sink.rs` — `JsonResponseSink` (JSONL `ResponseSink` impl)

**`crates/chibi-mcp-bridge/`** — Binary crate (async daemon)
- `main.rs` — Entry point, TCP listener, idle timeout, lockfile management
- `bridge.rs` — Request dispatch (`Bridge` struct)
- `server.rs` — MCP server lifecycle (`ServerManager`, rmcp client)
- `protocol.rs` — JSON-over-TCP protocol types (`Request`, `Response`, `ToolInfo`)
- `config.rs` — `BridgeConfig` from `mcp-bridge.toml`
- `cache.rs` — Summary cache with schema-hash invalidation (JSONL persistence)
- `summary.rs` — LLM-powered tool summary generation via ratatoskr

**MCP tools:** MCP tools use virtual `mcp://server/tool` paths and appear as regular `Tool` structs. chibi-core's `tools/mcp.rs` discovers the bridge via its lockfile, auto-spawns it if needed, and proxies tool calls over TCP. Tool names are prefixed with the server name (e.g. `serena_find_symbol`).

**Data flow:**
- CLI: args → `parse()` → `ChibiInput` → `execute_from_input()` → core APIs
- JSON: stdin → `JsonInput` → `execute_json_command()` → core APIs

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
└── contexts/<name>/
    ├── context.jsonl          # LLM window (compaction-bounded)
    ├── transcript/            # Authoritative log (partitioned)
    ├── local.toml, todos.md, goals.md, inbox.jsonl, summary.md
    └── tool_cache/
```

Home directory: `--home` flag > `CHIBI_HOME` env > `~/.chibi`

## Plugins

Executable scripts in `~/.chibi/plugins/`. Available plugins live in the separate [chibi-plugins](https://github.com/emesal/chibi-plugins) repo — install individually by symlinking or copying. Several former plugins are now built-in tools: `fetch_url` (coding tool), `read_context` (builtin), `shell_exec` (replaces `run_command`), `file_head`/`file_lines` (replaces `read_file`), `call_agent` (replaces `recurse`).

Schema via `--schema`, args via stdin (JSON).

```python
#!/usr/bin/env -S uv run --quiet --script
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "tool_name",
        "description": "...",
        "parameters": {"type": "object", "properties": {}, "required": []},
        "hooks": ["on_start"],  # optional
        "summary_params": ["param_name"]  # optional: params to show in tool-call notices
    }))
    sys.exit(0)

if os.environ.get("CHIBI_HOOK"):
    data = json.load(sys.stdin)  # Hook data via stdin
    # process hook...
    print("{}")
    sys.exit(0)

params = json.load(sys.stdin)  # Tool args via stdin
print("result")
```

## Hooks

31 hook points in `tools/hooks.rs`. Plugins register via `"hooks": [...]` in schema.

`on_start`, `on_end`, `pre_message`, `post_message`, `pre_tool`, `post_tool`, `pre_tool_output`, `post_tool_output`, `pre_system_prompt`, `post_system_prompt`, `pre_send_message`, `post_send_message`, `pre_clear`, `post_clear`, `pre_compact`, `post_compact`, `pre_rolling_compact`, `post_rolling_compact`, `pre_cache_output`, `post_cache_output`, `pre_api_tools`, `pre_api_request`, `pre_agentic_loop`, `post_tool_batch`, `pre_file_read`, `pre_file_write`, `pre_shell_exec`, `pre_fetch_url`, `pre_spawn_agent`, `post_spawn_agent`, `post_index_file`

Hook data: `CHIBI_HOOK` env var + stdin (JSON).

## Language Plugins

Language plugins provide symbol extraction for the codebase index. Core handles all database writes.

**Convention:** plugins named `lang_<language>` (e.g. `lang_rust`, `lang_python`).

**Input** (stdin, JSON):
```json
{"files": [{"path": "src/foo.rs", "content": "..."}]}
```

**Output** (stdout, JSON):
```json
{
  "symbols": [
    {"name": "parse", "kind": "function", "parent": "Parser",
     "line_start": 42, "line_end": 67, "signature": "fn parse(&self) -> Result<AST>", "visibility": "public"}
  ],
  "refs": [
    {"from_line": 55, "to_name": "TokenStream::new", "kind": "call"}
  ]
}
```

Fields: `name` (required), `kind` (required), `line_start`/`line_end` (optional), `parent` (optional, for nesting), `signature`/`visibility` (optional). Refs: `from_line`, `to_name`, `kind` (all optional but recommended).

The `post_index_file` hook fires after each file is indexed with `{"path", "lang", "symbol_count", "ref_count"}`.

## CLI Conventions

- stdout: LLM output only (pipeable); markdown-rendered when TTY
- stderr: Diagnostics (with `-v`)
- `--raw` disables markdown rendering
