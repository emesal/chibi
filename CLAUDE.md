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

api/
  mod.rs         send_prompt(), tool execution loop, streaming
  compact.rs     Compaction operations (rolling, manual)
  logging.rs     Debug request/response logging
  request.rs     Request body building, PromptOptions

state/
  mod.rs         AppState: all file I/O, config resolution, entry creation
  jsonl.rs       JSONL file reading utilities

partition.rs     Partitioned storage, manifest, bloom filters

tools/
  mod.rs         Tool struct, public exports
  plugins.rs     Plugin loading, execution, schema parsing
  builtin.rs     Built-in tools (reflection, todos, goals, send_message)
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

### Hooks

17 hook points in `tools/hooks.rs`. Plugins register via `"hooks": [...]` in schema.

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
    ├── context.jsonl       # LLM window (bounded by compaction)
    ├── transcript/         # Authoritative log (partitioned, never truncated)
    │   ├── manifest.json   # Partition metadata, timestamp ranges
    │   ├── active.jsonl    # Current write partition
    │   └── partitions/     # Archived read-only partitions
    │       ├── <ts>-<ts>.jsonl
    │       └── <ts>-<ts>.bloom
    ├── transcript.md       # Human-readable archive
    ├── context_meta.json   # Metadata (created_at)
    ├── local.toml          # Per-context config overrides
    ├── summary.md          # Conversation summary
    ├── todos.md            # Current todos
    ├── goals.md            # Current goals
    ├── inbox.jsonl         # Messages from other contexts
    ├── system_prompt.md    # Context-specific system prompt
    └── .dirty              # Marker: prefix needs rebuild
```

#### Partitioned Transcript Storage

The transcript (authoritative log) uses time-partitioned JSONL with manifest tracking.
Context files remain single JSONL files since compaction keeps them bounded.

Partitions rotate when thresholds are reached (configurable in `[storage]` section):
- Entry count exceeds `partition_max_entries` (default: 1000)
- Age exceeds `partition_max_age_seconds` (default: 30 days)

Legacy `transcript.jsonl` files are migrated automatically on first access.

### Environment Variables

- **CHIBI_HOME**: Override the default `~/.chibi` directory. Useful for testing or running multiple isolated instances.

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

### Storage Configuration

Global `config.toml`:
```toml
[storage]
partition_max_entries = 1000        # Rotate after N entries
partition_max_age_seconds = 2592000 # Or after 30 days
enable_bloom_filters = true         # Build bloom indexes for fast lookups
```

Per-context override in `local.toml`:
```toml
[storage]
partition_max_entries = 500         # More aggressive for busy contexts
```
