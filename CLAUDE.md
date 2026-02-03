# CLAUDE.md

## Vision

Chibi is a minimal, composable building block for LLM interactions — not an agent framework. It provides persistent context storage, extensible behavior via plugins/hooks, and communication primitives. Everything else (coordination patterns, workflows, domain behaviors) lives in plugins. This separation keeps chibi small and enables unlimited experimentation at the plugin layer.

## Principles

- Establish patterns now that scale well, refactor liberally when beneficial.
- Backwards compatibility not a priority, legacy code unwanted. (Pre-alpha.)
- Focused, secure core. Protect file operations from corruption and race conditions.
- Self-documenting code; keep symbols, comments, and docs consistent.
- Missing or incorrent documentation including code comments are critical bugs.
- Comprehensive tests including edge cases.
- Remind user about `just pre-push` before pushing and `just merge-to-dev` when merging feature branches.

## Build

```bash
git submodule update --init --recursive  # First time setup
cargo build                              # Debug build
cargo test                               # Run tests
cargo install --path .                   # Install to ~/.cargo/bin
```

Vendored dependency: `vendor/streamdown-rs/` (forked markdown renderer).

## Architecture

Cargo workspace with two crates:

**`crates/chibi-core/`** — Library crate (reusable logic)
- `chibi.rs` — Main `Chibi` struct, tool execution
- `context.rs`, `state/` — Context management, file I/O
- `api/` — Request building, streaming, tool execution loop
- `tools/` — Plugins, hooks, built-in tools
- `partition.rs` — Partitioned transcript storage with bloom filters
- `config.rs` — Core configuration types

**`crates/chibi-cli/`** — Binary crate (CLI-specific)
- `main.rs` — Entry point, command dispatch
- `cli.rs` — Argument parsing (clap)
- `input.rs` — Input types (`ChibiInput`, `Command`, `Flags`)
- `session.rs` — CLI session state (implied context)
- `config.rs` — CLI-specific config (markdown, images)

**Data flow:** CLI args → `parse()` → `ChibiInput` → `execute_from_input()` → core APIs

## Storage Layout

```
~/.chibi/
├── config.toml, models.toml
├── state.json               # Context metadata (core)
├── session.json             # Navigation state (CLI)
├── prompts/{chibi,reflection,compaction,continuation}.md
├── plugins/
└── contexts/<name>/
    ├── context.jsonl          # LLM window (compaction-bounded)
    ├── transcript/            # Authoritative log (partitioned)
    ├── local.toml, todos.md, goals.md, inbox.jsonl, summary.md
    └── tool_cache/
```

Home directory: `--home` flag > `CHIBI_HOME` env > `~/.chibi`

## Plugins

Executable scripts in `~/.chibi/plugins/`. Schema via `--schema`, args via `CHIBI_TOOL_ARGS` env.

```python
#!/usr/bin/env -S uv run --quiet --script
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "tool_name",
        "description": "...",
        "parameters": {"type": "object", "properties": {}, "required": []},
        "hooks": ["on_start"]  # optional
    }))
    sys.exit(0)

if os.environ.get("CHIBI_HOOK"):
    print("{}")
    sys.exit(0)

params = json.loads(os.environ["CHIBI_TOOL_ARGS"])
print("result")
```

## Hooks

25 hook points in `tools/hooks.rs`. Plugins register via `"hooks": [...]` in schema.

`on_start`, `on_end`, `pre_message`, `post_message`, `pre_tool`, `post_tool`, `pre_tool_output`, `post_tool_output`, `pre_system_prompt`, `post_system_prompt`, `pre_send_message`, `post_send_message`, `pre_clear`, `post_clear`, `pre_compact`, `post_compact`, `pre_rolling_compact`, `post_rolling_compact`, `pre_cache_output`, `post_cache_output`, `pre_api_tools`, `pre_api_request`, `pre_agentic_loop`, `post_tool_batch`, `pre_file_write`

Hook data: `CHIBI_HOOK` + `CHIBI_HOOK_DATA` env vars.

## CLI Conventions

- stdout: LLM output only (pipeable); markdown-rendered when TTY
- stderr: Diagnostics (with `-v`)
- `--raw` disables markdown rendering
