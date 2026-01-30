# CLAUDE.md

## Vision

Chibi is a minimal, composable building block for LLM interactions — not an agent framework. It provides persistent context storage, extensible behavior via plugins/hooks, and communication primitives. Everything else (coordination patterns, workflows, domain behaviors) lives in plugins. This separation keeps chibi small and enables unlimited experimentation at the plugin layer.

## Principles

- Early development; backwards compatibility not a priority. Refactor liberally.
- Focused, secure core. Protect file operations from corruption and race conditions.
- Self-documenting code; keep symbols, comments, and docs consistent.
- Missing docs/* for user-facing features are critical bugs.
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

CLI for LLM conversations via OpenRouter. Persistent context with plugin/hook extensibility.

**Key modules:**
- `cli.rs`, `input.rs` — Argument parsing, command handling
- `context.rs`, `state/` — Context management, file I/O
- `api/` — Request building, streaming, tool execution loop
- `tools/` — Plugins, hooks, built-in tools
- `partition.rs` — Partitioned transcript storage with bloom filters

**Data flow:** CLI args → `AppState::load()` → `send_prompt()` (hooks → inbox → system prompt → stream → tool loop → transcript)

## Storage Layout

```
~/.chibi/
├── config.toml, models.toml, state.json
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

23 hook points in `tools/hooks.rs`. Plugins register via `"hooks": [...]` in schema.

`on_start`, `on_end`, `pre_message`, `post_message`, `pre_tool`, `post_tool`, `pre_tool_output`, `post_tool_output`, `pre_system_prompt`, `post_system_prompt`, `pre_send_message`, `post_send_message`, `on_context_switch`, `pre_clear`, `post_clear`, `pre_compact`, `post_compact`, `pre_rolling_compact`, `post_rolling_compact`, `pre_cache_output`, `post_cache_output`, `pre_api_tools`, `pre_api_request`

Hook data: `CHIBI_HOOK` + `CHIBI_HOOK_DATA` env vars.

## CLI Conventions

- stdout: LLM output only (pipeable); markdown-rendered when TTY
- stderr: Diagnostics (with `-v`)
- `--raw` disables markdown rendering
