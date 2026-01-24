# CLAUDE.md

## General

Early development, no backwards compatibility. Prioritize lean, secure, well-structured core. Remove legacy code freely. Self-documenting code preferred.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo install --path .   # Install to ~/.cargo/bin
cargo run -- <args>      # Run with arguments
```

## Architecture

CLI tool for LLM conversations via OpenRouter. Persistent conversation state with plugin/hook extensibility.

### Source Structure (`src/`)

```
main.rs          Entry point, prompt input (interactive/piped)
cli.rs           Clap argument definitions, ChibiInput conversion
config.rs        Config structs (Config, LocalConfig, ModelsConfig, ResolvedConfig)
context.rs       Context, Message, TranscriptEntry, InboxEntry
input.rs         ChibiInput, Command enum, Flags
output.rs        OutputHandler (stdout/JSON modes)
lock.rs          Context locking with heartbeat
inbox.rs         Inter-context messaging
llm.rs           Low-level API request helpers
json_input.rs    JSON config input parsing
cache.rs         Tool output caching (large outputs cached to disk)

api/
  mod.rs         send_prompt(), tool execution loop, streaming, cache integration
  compact.rs     Compaction operations (rolling, manual)
  logging.rs     Debug request/response logging
  request.rs     Request body building, PromptOptions

state/
  mod.rs         AppState: all file I/O, config resolution, cache management
  paths.rs       Path construction helpers
  entries.rs     TranscriptEntry creation (builder pattern)
  jsonl.rs       JSONL file reading utilities

tools/
  mod.rs         Tool struct, public exports
  plugins.rs     Plugin loading, execution, schema parsing
  builtin.rs     Built-in tools (reflection, todos, goals, send_message)
  file_tools.rs  File access tools (file_head, file_tail, file_lines, file_grep, cache_list)
  hooks.rs       HookPoint enum, hook execution
```

### Data Flow

1. `cli.rs` parses args into `ChibiInput`
2. `AppState::load()` initializes from `~/.chibi/`
3. `send_prompt()` handles conversation:
   - Executes lifecycle hooks
   - Injects inbox messages
   - Builds system prompt (todos, goals, summary, reflection)
   - Streams response from OpenRouter
   - Executes tool calls in loop until text response
   - Logs to transcript (JSONL + markdown)

### Built-in Tools

Defined in `tools/builtin.rs`:
- `update_todos`, `update_goals` - Per-context task tracking
- `update_reflection` - Global persistent memory
- `send_message` - Inter-context messaging
- `recurse` - Signal to continue processing (external plugin)

Defined in `tools/file_tools.rs`:
- `file_head`, `file_tail`, `file_lines`, `file_grep` - Surgical file/cache access
- `cache_list` - List cached tool outputs

### Hooks

19 hook points in `tools/hooks.rs`. Plugins register via `"hooks": [...]` in schema.

```
on_start, on_end
pre_message, post_message
pre_tool, post_tool
pre_system_prompt, post_system_prompt
pre_send_message, post_send_message
on_context_switch
pre_clear, post_clear
pre_compact, post_compact
pre_rolling_compact, post_rolling_compact
pre_cache_output, post_cache_output
```

Hook data passed via `CHIBI_HOOK` and `CHIBI_HOOK_DATA` env vars.

### Storage Layout

```
~/.chibi/
├── config.toml              # Required: api_key, model, context_window_limit, warn_threshold_percent
├── models.toml              # Model aliases, context windows, API params
├── state.json               # Current context name
├── prompts/
│   ├── chibi.md            # Default system prompt
│   ├── reflection.md       # LLM's persistent memory
│   ├── compaction.md       # Compaction instructions
│   └── continuation.md     # Post-compaction instructions
├── plugins/                 # Executable scripts (provide tools)
└── contexts/<name>/
    ├── context.jsonl       # LLM window (anchor + system_prompt + entries)
    ├── transcript.jsonl    # Authoritative log (never truncated)
    ├── transcript.md       # Human-readable archive
    ├── context_meta.json   # Metadata (created_at)
    ├── local.toml          # Per-context config overrides
    ├── summary.md          # Conversation summary
    ├── todos.md            # Current todos
    ├── goals.md            # Current goals
    ├── inbox.jsonl         # Messages from other contexts
    ├── system_prompt.md    # Context-specific system prompt
    ├── .dirty              # Marker: prefix needs rebuild
    └── tool_cache/         # Cached large tool outputs
```

### Entry Format (JSONL)

```json
{"id":"uuid","timestamp":1234567890,"from":"alice","to":"default","content":"Hello","entry_type":"message"}
```

Entry types: `message`, `tool_call`, `tool_result`, `compaction`, `archival`, `context_created`, `system_prompt`, `system_prompt_changed`

### Plugin Pattern

```python
#!/usr/bin/env -S uv run --quiet --script
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///

import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "tool_name",
        "description": "...",
        "parameters": {"type": "object", "properties": {}, "required": []},
        "hooks": ["on_start"]  # optional
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook:
    print("{}")  # Hook response
    sys.exit(0)

params = json.loads(os.environ["CHIBI_TOOL_ARGS"])
print("result")
```

### CLI Conventions

- stdout: LLM output only (pipeable)
- stderr: Diagnostics (with `-v`)
- Plugins: args via `CHIBI_TOOL_ARGS` env var, stdin free for user interaction
- Hooks: `CHIBI_HOOK` + `CHIBI_HOOK_DATA` env vars

### Config Resolution Order

1. CLI flags (highest priority)
2. Context local.toml
3. Global config.toml
4. models.toml (for model-specific params)
5. Defaults
