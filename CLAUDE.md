# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo install --path .   # Install to ~/.cargo/bin
cargo run -- <args>      # Run with arguments (e.g., cargo run -- -v "hello")
```

## Architecture Overview

Chibi is a CLI tool for conversing with LLMs via OpenRouter. It maintains persistent conversation state and supports tools/hooks for extensibility.

### Source Files (`src/`)

- **main.rs** - CLI entry point, argument parsing, prompt input handling (interactive/piped)
- **cli.rs** - Argument definitions and parsing logic
- **config.rs** - Config structs (`Config`, `LocalConfig`, `ModelsConfig`, `ResolvedConfig`) and TOML loading
- **context.rs** - Data structures for conversations (`Context`, `Message`, `ContextState`, `TranscriptEntry`, `InboxEntry`)
- **state.rs** - `AppState` manages all file I/O: contexts, prompts, todos, goals, transcripts, inbox
- **api.rs** - LLM API communication, streaming responses, tool execution loop, compaction
- **tools.rs** - Tool loading, schema parsing, hook system (`HookPoint` enum), built-in tool definitions
- **lock.rs** - Context locking with heartbeat to prevent concurrent access

### Data Flow

1. `main.rs` parses CLI args and reads prompt (interactive or piped)
2. `AppState::load()` initializes from `~/.chibi/` directory structure
3. `send_prompt()` in `api.rs` handles the conversation:
   - Executes lifecycle hooks (`on_start`, `pre_message`, `pre_system_prompt`, etc.)
   - Loads/injects inbox messages from other contexts
   - Builds system prompt with todos, goals, summary, reflection
   - Streams response from OpenRouter API
   - Executes tool calls in a loop until final text response
   - Logs to transcript files (txt and jsonl)

### Tool System

External tools are executable scripts in `~/.chibi/tools/` that:
- Output JSON schema when called with `--schema`
- Receive arguments via `CHIBI_TOOL_ARGS` environment variable
- Can register for hooks via `"hooks": [...]` in schema

Built-in tools (defined in `tools.rs`, executed in `api.rs`):
- `update_todos`, `update_goals` - Per-context task tracking
- `update_reflection` - Global persistent memory
- `send_message` - Inter-context messaging

### Hook System

Hooks fire at lifecycle points. Tools register via schema. Hook data passed via `CHIBI_HOOK` and `CHIBI_HOOK_DATA` env vars.

Key hooks: `on_start`, `on_end`, `pre_message`, `post_message`, `pre_tool`, `post_tool`, `pre_system_prompt`, `post_system_prompt`, `pre_send_message`, `post_send_message`, `on_context_switch`, `pre_clear`, `post_clear`, `pre_compact`, `post_compact`, `pre_rolling_compact`, `post_rolling_compact`

### Storage Layout

```
~/.chibi/
├── config.toml              # Global config
├── models.toml              # Model aliases (optional)
├── state.json               # Current context name
├── prompts/                 # System prompts
│   ├── chibi.md            # Default prompt
│   ├── reflection.md       # LLM's persistent memory
│   ├── compaction.md       # Compaction instructions
│   └── continuation.md     # Post-compaction instructions
├── tools/                   # Executable tool scripts
└── contexts/<name>/
    ├── context.json        # Messages array
    ├── local.toml          # Per-context config overrides
    ├── summary.md          # Conversation summary
    ├── transcript.txt      # Human-readable history
    ├── transcript.jsonl    # Machine-readable history
    ├── todos.md            # Current todos
    ├── goals.md            # Current goals
    ├── inbox.jsonl         # Messages from other contexts
    └── prompt.md           # Context-specific system prompt
```

### Example Tool Pattern (Python with uv)

```python
#!/usr/bin/env -S uv run --quiet --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["some-package"]
# ///

import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "tool_name",
        "description": "...",
        "parameters": {"type": "object", "properties": {...}, "required": [...]},
        "hooks": ["on_start", "post_system_prompt"]  # optional
    }))
    sys.exit(0)

# Handle hooks
hook = os.environ.get("CHIBI_HOOK", "")
if hook == "on_start":
    print("{}")
    sys.exit(0)

# Handle tool call
params = json.loads(os.environ["CHIBI_TOOL_ARGS"])
# ... do work ...
print("result")
```

### Key Conventions

- stdout: Only LLM output (pipeable)
- stderr: Diagnostics (with `-v`)
- Tools read args from `CHIBI_TOOL_ARGS` env var (not stdin)
- Tools can use stdin for user interaction (confirmations)
- Hooks receive data via `CHIBI_HOOK` and `CHIBI_HOOK_DATA` env vars

### See Also
- PHILOSOPHY.md for architectural decisions etc
