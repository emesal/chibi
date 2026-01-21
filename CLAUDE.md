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

Chibi is a CLI tool for conversing with LLMs via OpenRouter. It maintains persistent conversation state and supports plugins/hooks for extensibility.

### Source Files (`src/`)

- **main.rs** - CLI entry point, argument parsing, prompt input handling (interactive/piped)
- **cli.rs** - Argument definitions and parsing logic
- **config.rs** - Config structs (`Config`, `LocalConfig`, `ModelsConfig`, `ResolvedConfig`) and TOML loading
- **context.rs** - Data structures for conversations (`Context`, `Message`, `ContextState`, `TranscriptEntry`, `InboxEntry`)
- **state.rs** - `AppState` manages all file I/O: contexts, prompts, todos, goals, transcripts, inbox
- **api.rs** - LLM API communication, streaming responses, tool execution loop, compaction
- **tools.rs** - Plugin loading, schema parsing, hook system (`HookPoint` enum), built-in tool definitions
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

### Plugin System

Plugins are executable scripts in `~/.chibi/plugins/` that provide tools for the LLM:
- Output JSON schema when called with `--schema`
- Receive arguments via `CHIBI_TOOL_ARGS` environment variable
- Can register for hooks via `"hooks": [...]` in schema

Built-in tools (defined in `tools.rs`, executed in `api.rs`):
- `update_todos`, `update_goals` - Per-context task tracking
- `update_reflection` - Global persistent memory
- `send_message` - Inter-context messaging

### Hook System

Hooks fire at lifecycle points. Plugins register via schema. Hook data passed via `CHIBI_HOOK` and `CHIBI_HOOK_DATA` env vars.

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
├── plugins/                 # Plugin scripts (provide tools for LLM)
└── contexts/<name>/
    ├── context.jsonl       # Conversation entries (JSONL, append-only)
    ├── context_meta.json   # Context metadata (created_at timestamp)
    ├── local.toml          # Per-context config overrides
    ├── summary.md          # Conversation summary
    ├── transcript.md       # Human-readable history (for archiving)
    ├── transcript_archive.jsonl  # Archived entries (from compaction)
    ├── todos.md            # Current todos
    ├── goals.md            # Current goals
    ├── inbox.jsonl         # Messages from other contexts
    └── system_prompt.md    # Context-specific system prompt
```

### Context Entry Format (context.jsonl)

Each line is a JSON object with these fields:
- `id`: Unique identifier (UUID)
- `timestamp`: Unix timestamp
- `from`: Source (username, context name, or tool name)
- `to`: Destination (context name, "user", or tool name)
- `content`: Message content or tool arguments/results
- `entry_type`: One of "message", "tool_call", "tool_result", "compaction"

Example:
```jsonl
{"id":"uuid1","timestamp":1234567890,"from":"alice","to":"default","content":"Hello","entry_type":"message"}
{"id":"uuid2","timestamp":1234567891,"from":"default","to":"read_file","content":"{\"path\":\"Cargo.toml\"}","entry_type":"tool_call"}
{"id":"uuid3","timestamp":1234567892,"from":"read_file","to":"default","content":"[package]...","entry_type":"tool_result"}
{"id":"uuid4","timestamp":1234567893,"from":"default","to":"user","content":"Here's the file...","entry_type":"message"}
```

### Example Plugin Pattern (Python with uv)

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
- Plugins read args from `CHIBI_TOOL_ARGS` env var (not stdin)
- Plugins can use stdin for user interaction (confirmations)
- Hooks receive data via `CHIBI_HOOK` and `CHIBI_HOOK_DATA` env vars

### See Also
- PHILOSOPHY.md for architectural decisions etc
