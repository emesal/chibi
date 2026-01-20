# chibi

Prototype CLI tool for having conversations with AI models via OpenRouter. Chibi maintains conversation state across invocations, allowing you to have coherent multi-turn conversations directly from your terminal.

## Features

- **Persistent conversations** - State is saved between sessions
- **Multiple contexts** - Maintain separate conversations for different projects/topics
- **Per-context system prompts** - Each context can have its own personality/instructions
- **Plugin system** - Extend capabilities with custom scripts the LLM can call
- **Transcript history** - Full chat history is preserved when contexts are cleared or compacted
- **Streaming responses** - Real-time output as the AI responds
- **Context window management** - Warnings when approaching context limits
- **Rolling compaction** - Automatically strips oldest messages and integrates them into a summary
- **Agentic workflow** - Built-in tools for todos, goals, and autonomous processing
- **Cross-context access** - Read state from other contexts (for sub-agents)
- **Piped input** - Accept prompts from stdin for scripting
- **Unix philosophy** - Only LLM output goes to stdout, making it pipeable
- **Per-context config** - Override model, API key, username per context (local.toml)
- **Context locking** - Prevents concurrent access to the same context
- **JSONL transcripts** - Machine-readable conversation logs with metadata

## Installation

### From Source

```bash
cargo install --path .
```

### Build Manually

```bash
cargo build --release
# Binary will be at target/release/chibi
```

## Configuration

Create a configuration file at `~/.chibi/config.toml`:

```toml
# OpenRouter API key
# Get one at https://openrouter.ai/settings/keys
api_key = "your-openrouter-api-key-here"

# Model to use
model = "xiaomi/mimo-v2-flash:free"

# Context window limit (tokens)
# This is used for calculating when to warn about approaching limits
context_window_limit = 262144

# Warning threshold percentage (0.0-100.0)
# When context usage exceeds this percentage, a warning is printed to stderr
warn_threshold_percent = 80.0

# Auto-compaction settings
# When enabled, chibi will automatically compact the context when it reaches
# the threshold percentage of the context window
auto_compact = false
auto_compact_threshold = 80.0

# Optional: Custom API base URL
# Default: https://openrouter.ai/api/v1/chat/completions
# base_url = "https://openrouter.ai/api/v1/chat/completions"

# Reflection settings
# When enabled, the LLM has access to a persistent memory that spans all contexts
reflection_enabled = true
reflection_character_limit = 10000
```

**Required fields:**
- `api_key` - Your OpenRouter API key
- `model` - The model to use
- `context_window_limit` - Token limit for context window warnings
- `warn_threshold_percent` - Percentage of context window at which to warn (0-100)

**Optional fields:**
- `auto_compact` - Enable automatic compaction (default: false)
- `auto_compact_threshold` - Percentage at which to auto-compact (default: 80.0)
- `base_url` - Custom API endpoint (default: `https://openrouter.ai/api/v1/chat/completions`)
- `reflection_enabled` - Enable reflection/memory feature (default: true)
- `reflection_character_limit` - Max characters for reflection content (default: 10000)
- `max_recursion_depth` - Limit for `recurse` tool loops (default: 15)
- `username` - Default username shown to LLM (default: "user")
- `lock_heartbeat_seconds` - Interval for context lock heartbeat (default: 30)

### Per-Context Configuration (local.toml)

Each context can override global settings with a `local.toml` file:

```
~/.chibi/contexts/<name>/local.toml
```

```toml
# Override model for this context
model = "anthropic/claude-3-opus"

# Override API key (useful for different providers)
api_key = "sk-different-key"

# Override base URL
base_url = "https://api.anthropic.com/v1/messages"

# Override username
username = "alice"

# Override auto-compact behavior
auto_compact = true

# Override recursion depth
max_recursion_depth = 25
```

You can set the username via CLI flag `-u` which automatically saves to `local.toml`:

```bash
chibi -u alice "Hello"  # Saves username to local.toml
```

### Model Aliases (models.toml)

Define model aliases and metadata in `~/.chibi/models.toml`:

```toml
[models.claude]
context_window = 200000

[models.gpt4]
context_window = 128000

[models.fast]
context_window = 32000
```

When a model name matches a key in `models.toml`, chibi will use the `context_window` value from there instead of `config.toml`. This is useful for:

- Documenting context windows for models you frequently use
- Overriding context window limits per model

## System Prompts

Chibi supports a default system prompt and per-context custom prompts.

### Default Prompt

Copy the example prompts from `examples/prompts/` to `~/.chibi/prompts/`:

```bash
mkdir -p ~/.chibi/prompts
cp prompts.example/*.md ~/.chibi/prompts/
```

**Available prompts:**

- `chibi.md` - Default system/personality prompt (used when no context-specific prompt is set)
- `compaction.md` - Instructions for summarizing conversations when compacting
- `continuation.md` - Instructions after compaction/summary to guide the LLM's continuation

### Per-Context Prompts

Each context can have its own system prompt, overriding the default:

```bash
# View current context's system prompt
chibi -p

# Set a custom prompt for the current context (from text)
chibi -e "You are a helpful coding assistant"

# Set a custom prompt from a file
chibi -e ~/prompts/coder.md
```

When you set a custom prompt, it's stored in `~/.chibi/contexts/<name>/system_prompt.md`. If no custom prompt is set, the default from `~/.chibi/prompts/chibi.md` is used.

This allows different contexts to have completely different personalities:

```bash
chibi -s coding
chibi -e "You are a senior software engineer. Be precise and technical."

chibi -s creative
chibi -e "You are a creative writing assistant. Be imaginative and playful."

chibi -s default  # Uses the default chibi.md prompt
```

## Plugins

Plugins are executable scripts that provide tools for the LLM to call. They enable actions like reading files, fetching URLs, or running commands.

### THIS IS THE DANGER ZONE!

Chibi does not impose any restrictions on plugins. NONE. Each plugin is responsible for its own safety measures. *You are expected to understand the plugins you install.*

See *Plugin Safety* below.

### Setting Up Plugins

You need to do this yourself.

1. Create the plugins directory: `mkdir -p ~/.chibi/plugins`
2. Add executable scripts to the directory
3. Each script must support `--schema` to describe itself

### Plugin Script Requirements

Each plugin script must:

1. **Be executable** (`chmod +x`)
2. **Output JSON schema when called with `--schema`**:
   ```json
   {
     "name": "tool_name",
     "description": "What the tool does",
     "parameters": {
       "type": "object",
       "properties": {
         "param1": {"type": "string", "description": "..."}
       },
       "required": ["param1"]
     }
   }
   ```
3. **Read JSON parameters from `CHIBI_TOOL_ARGS` env var**
4. **Output results to stdout**
5. **Use stderr for prompts** and stdin for user input (both are free since args come via env var)

**Note on Environment Variables:** Tool parameters are passed as a single JSON string in `CHIBI_TOOL_ARGS`. Environment variables have size limits (typically 128KB-2MB depending on OS) and cannot represent certain binary data. For the simple plugins chibi is designed for, this is rarely an issue. If you need to pass large data, consider using file paths instead.

### Example Plugin (Bash)

```bash
#!/bin/bash
# ~/.chibi/plugins/read_file

if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "read_file",
  "description": "Read and return the contents of a file",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {"type": "string", "description": "Path to the file"}
    },
    "required": ["path"]
  }
}
EOF
  exit 0
fi

# Read args from env var
path=$(echo "$CHIBI_TOOL_ARGS" | jq -r '.path')
cat "$path"
```

### Example Plugin (Python)

```python
#!/usr/bin/env python3
# ~/.chibi/plugins/web_search

import sys
import json
import os

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "web_search",
        "description": "Search the web",
        "parameters": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        }
    }))
    sys.exit(0)

# Read args from env var
params = json.loads(os.environ["CHIBI_TOOL_ARGS"])

# ... perform search ...
print(json.dumps(results))
```

### Example Plugins

See the [chibi-plugins](https://github.com/emesal/chibi-plugins) repository for ready-to-use plugins:

**LLM-Callable Tools:**
- `read_file` - Read file contents (bash)
- `fetch_url` - Fetch content from a URL (bash)
- `run_command` - Execute shell commands (bash)
- `web_search` - Search the web via DuckDuckGo (Python, requires `uv`)
- `read_context` - Read another context's state (bash)
- `sub-agent` - Spawn sub-agents in other contexts (bash)
- `recurse` - Continue processing without returning to user (bash)

**Hook Plugins:**
- `hook-inspector` - Detailed hook debugger with JSON data logging (bash)

### MCP Wrapper Plugins

Chibi can connect to MCP (Model Context Protocol) servers through wrapper plugins. See [chibi-plugins](https://github.com/emesal/chibi-plugins) for examples:

- **fetch-mcp** (Bash) - Simple MCP wrapper, no caching
- **github-mcp** (Python) - Full-featured with caching

These serve as templates for connecting to any MCP server.

### Plugin Safety

Plugins are responsible for their own safety guardrails. Chibi passes these environment variables to plugins:
- `CHIBI_TOOL_ARGS` - JSON arguments (always set)
- `CHIBI_VERBOSE=1` - when `-v` is used, allowing plugins to adjust their behavior

Since args come via env var, **stdin is free for user interaction** (confirmations, multi-line input, etc.).

**run_command** always requires user confirmation:
```
┌─────────────────────────────────────────────────────────────
│ Tool: run_command
│ Command: rm -rf /tmp/test
└─────────────────────────────────────────────────────────────
Execute this command? [y/N]
```

**github-mcp** has configurable safety lists:
- `TOOLS_REQUIRE_CONFIRMATION` - always prompt (delete, create, merge, etc.)
- `TOOLS_SAFE` - never prompt (read-only operations)
- Unknown tools prompt unless `CHIBI_VERBOSE=1`

Edit the tool files directly to customize which operations need confirmation.

### Viewing Tool Activity

Use `-v` to see which tools are loaded and when they're called:

```bash
chibi -v "Read my Cargo.toml"
# stderr: [Loaded 1 tool(s): read_file]
# stderr: [Tool: read_file]
# stdout: <LLM response about the file>
```

## Hooks

Chibi supports a hooks system that allows tools to register for lifecycle events. Tools can observe events or modify data as it flows through the system.

### Hook Points

Tools can register for these hook points:

**Session Lifecycle:**
- `on_start` - When chibi starts (before any processing)
- `on_end` - When chibi exits (after all processing)

**Message Lifecycle:**
- `pre_message` - Before sending a prompt to the LLM (can modify prompt)
- `post_message` - After receiving LLM response (observe only)

**Tool Lifecycle:**
- `pre_tool` - Before executing a tool (can modify arguments)
- `post_tool` - After executing a tool (observe only)

**Context Lifecycle:**
- `on_context_switch` - When switching contexts
- `pre_clear` - Before clearing context
- `post_clear` - After clearing context
- `pre_compact` - Before full compaction
- `post_compact` - After full compaction
- `pre_rolling_compact` - Before rolling compaction
- `post_rolling_compact` - After rolling compaction

### Hook Registration

Tools register for hooks via their `--schema` JSON output by adding a `hooks` array:

```json
{
  "name": "my_tool",
  "description": "Tool description",
  "parameters": { ... },
  "hooks": ["on_start", "pre_message", "post_message"]
}
```

### Hook Execution

When a hook fires, registered tools are called with:
- `CHIBI_HOOK` - Hook point name (e.g., "pre_message")
- `CHIBI_HOOK_DATA` - JSON data about the event

Hook data varies by hook type:

**on_start / on_end:**
```json
{"current_context": "default", "verbose": true}
```

**pre_message:**
```json
{"prompt": "user's prompt", "context_name": "default", "summary": "..."}
```

**post_message:**
```json
{"prompt": "...", "response": "...", "context_name": "default"}
```

**pre_tool / post_tool:**
```json
{"tool_name": "read_file", "arguments": {...}, "result": "..."}
```

**on_context_switch:**
```json
{"from_context": "old", "to_context": "new", "is_sub_context": false}
```

**pre_clear / post_clear:**
```json
{"context_name": "default", "message_count": 10, "summary": "..."}
```

**pre_compact / post_compact / pre_rolling_compact / post_rolling_compact:**
```json
{"context_name": "default", "message_count": 20, "summary": "..."}
```

### Hook Output

- **Modifying hooks** (`pre_*`): Can output JSON to modify data (not yet implemented for all hooks)
- **Observing hooks** (`post_*`, `on_*`): Output is ignored

### Example Hook Plugins

See [chibi-plugins](https://github.com/emesal/chibi-plugins) for hook examples like `logger` and `hook-inspector`.

A minimal hook plugin that logs event names:

```bash
#!/bin/bash

if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "logger",
  "description": "Logs lifecycle events",
  "parameters": {"type": "object", "properties": {}},
  "hooks": ["on_start", "on_end", "pre_message", "post_message"]
}
EOF
  exit 0
fi

# Log the event
echo "[$( date '+%Y-%m-%d %H:%M:%S')] $CHIBI_HOOK" >> ~/.chibi/hook.log
```

Example output:
```
[2026-01-19 20:19:45] on_context_switch
  Data:
    {
      "from_context": "default",
      "is_sub_context": false,
      "to_context": "my-context"
    }
```

### Use Cases

- **Logging**: Record all interactions for debugging or auditing
- **Metrics**: Track tool usage, message counts, context switches
- **Integration**: Notify external systems about events
- **Validation**: Pre-check messages or tool arguments before execution
- **Backup**: Save state before destructive operations (clear, compact)

## Reflection (Persistent Memory)

Chibi includes a built-in "reflection" feature that gives the LLM persistent memory across all contexts and sessions. The LLM can store notes, preferences, and insights that it wants to remember.

### How It Works

- The reflection is stored in `~/.chibi/prompts/reflection.md`
- It's automatically appended to the system prompt on every invocation
- The LLM has a built-in `update_reflection` tool to modify its reflection
- The reflection has a configurable character limit (default: 10,000 characters)

### Configuration

In `config.toml`:

```toml
# Enable/disable reflection (default: true)
reflection_enabled = true

# Maximum characters allowed (default: 10000)
reflection_character_limit = 10000
```

### Disabling Reflection

You can disable reflection for a single invocation:

```bash
chibi -x "Your prompt here"
chibi --no-reflection "Your prompt here"
```

Or disable it permanently in `config.toml` by setting `reflection_enabled = false`.

### Use Cases

- The LLM can remember user preferences discovered during conversations
- Store important facts or context that should persist
- Keep notes for future conversations
- Build up knowledge over time

## Agentic Workflow

Chibi includes built-in tools that enable autonomous, multi-step workflows.

### Built-in Tools

The LLM always has access to these tools (no setup required):

| Tool | Description |
|------|-------------|
| `update_todos` | Track tasks for the current conversation (persists in `todos.md`) |
| `update_goals` | Set high-level objectives (persists in `goals.md`) |
| `update_reflection` | Update persistent memory (when reflection is enabled) |

### Optional External Plugins

These are available in [chibi-plugins](https://github.com/emesal/chibi-plugins):

| Plugin | Tool Provided | Description |
|--------|---------------|-------------|
| `recurse` | `recurse` | Continue working without returning to user |
| `read_context` | `read_context` | Read another context's state (read-only) |
| `sub-agent` | `sub-agent` | Spawn sub-agents in another context |

### Todos and Goals

Each context can have its own todos and goals stored in markdown files:

- **Todos** (`~/.chibi/contexts/<name>/todos.md`) - Short-term tasks for the current round
- **Goals** (`~/.chibi/contexts/<name>/goals.md`) - Long-term objectives that persist

These are automatically included in the system prompt, so the LLM always knows what it's working toward.

### Recurse (Autonomous Mode)

The `recurse` plugin (from [chibi-plugins](https://github.com/emesal/chibi-plugins)) lets the LLM work autonomously:

```
LLM: "I need to do more work. Let me continue."
     [calls recurse with note: "Check the test results next"]

LLM: (new round) "Continuing from previous round. Note to self: Check the test results next"
     ... continues working ...
```

The LLM leaves itself a note about what to do next, then the conversation continues automatically. The `max_recursion_depth` config option limits how many times this can happen (default: 15).

### Sub-Agents

Use the `-S` (sub-context) flag to spawn agents without affecting the global context state:

```bash
# Run a task in another context (doesn't change your current context)
chibi -S research "Find information about quantum computing"

# Set system prompt and send task in one command
chibi -S coding -e "You are a code reviewer" "Review this function for bugs"
```

The `sub-agent` plugin provides a convenient wrapper for the LLM:

```
Main: [calls sub-agent with context: "research", task: "Find info about X"]
Main: [calls read_context with context_name: "research"]
Main: "The sub-agent found: ..."
```

The key difference between `-s` and `-S`:
- `-s` (switch): Changes global context permanently
- `-S` (sub-context): Uses context for this invocation only, global state unchanged

### Rolling Compaction

When auto-compaction is enabled and the context exceeds the threshold:

1. The oldest half of messages are stripped
2. The LLM summarizes the stripped content
3. The summary is integrated with the existing conversation summary
4. Goals and todos guide what's important to preserve

This happens automatically, keeping the conversation going indefinitely while preserving key context.

## Storage Structure

Chibi stores data in `~/.chibi/`:

```
~/.chibi/
├── config.toml          # Global configuration (TOML format)
├── models.toml          # Model aliases and metadata (optional)
├── state.json           # Current context tracking
├── prompts/
│   ├── chibi.md         # Default system/personality prompt
│   ├── compaction.md    # Compaction instructions
│   ├── continuation.md  # Post-compaction instructions
│   └── reflection.md    # LLM's persistent memory (auto-created)
├── cache/               # Tool caches (optional, tool-managed)
│   └── github-mcp.json  # Example: cached MCP tool definitions
├── plugins/             # Plugin scripts (provide tools for LLM)
│   ├── read_file
│   ├── fetch_url
│   ├── recurse          # Continue processing tool
│   ├── sub-agent        # Spawn sub-agents tool
│   ├── github-mcp       # MCP wrapper example
│   └── ...
└── contexts/
    ├── default/
    │   ├── context.json      # Current conversation state (messages only)
    │   ├── local.toml        # Per-context config overrides (optional)
    │   ├── summary.md        # Conversation summary (auto-created on compaction)
    │   ├── transcript.txt    # Full chat history (human-readable)
    │   ├── transcript.jsonl  # Full chat history (machine-readable, JSONL)
    │   ├── todos.md          # Current todos (auto-created)
    │   ├── goals.md          # Current goals (auto-created)
    │   └── .lock             # Context lock file (when active)
    ├── coding/
    │   ├── context.json
    │   ├── local.toml        # Example: different model for this context
    │   ├── summary.md
    │   ├── transcript.txt
    │   ├── transcript.jsonl
    │   ├── todos.md
    │   ├── goals.md
    │   └── system_prompt.md  # Custom prompt for this context
    └── my-project/
        ├── context.json
        ├── summary.md
        ├── transcript.txt
        ├── transcript.jsonl
        ├── todos.md
        └── goals.md
```

## Command Reference

| Flag | Description |
|------|-------------|
| `-s, --switch <name>` | Switch to a different context (`new` for auto-name, `new:prefix` for prefixed) |
| `-S, --sub-context <name>` | Run in a context without changing global state (for sub-agents) |
| `-l, --list` | List all contexts (shows `[active]` or `[stale]` lock status) |
| `-w, --which` | Show current context name |
| `-d, --delete <name>` | Delete a context |
| `-C, --clear` | Clear current context (saves to transcript) |
| `-c, --compact` | Compact current context (saves to transcript) |
| `-r, --rename <old> <new>` | Rename a context |
| `-H, --history` | Show recent messages (default: 6) |
| `-n, --num-messages <N>` | Number of messages to show (0 = all, implies -H) |
| `-p, --prompt` | Show system prompt for current context |
| `-e, --set-prompt <arg>` | Set system prompt (can combine with a prompt to send) |
| `-v, --verbose` | Show extra info (tools loaded, warnings, etc.) |
| `-x, --no-reflection` | Disable reflection for this invocation |
| `-u, --username <name>` | Set username (persists to context's local.toml) |
| `-U, --temp-username <name>` | Set username for this invocation only |
| `-h, --help` | Show help message |
| `-V, --version` | Show version |

## Output Philosophy

Chibi follows Unix conventions:

- **stdout**: Only LLM responses (clean, pipeable)
- **stderr**: Diagnostics (only with `-v`)

This means you can pipe chibi's output:

```bash
# Pipe to another command
chibi "Generate a JSON config" | jq .

# Save response to file
chibi "Write a poem" > poem.txt

# Use in scripts
result=$(chibi "What is 2+2")
```

## Examples

### Basic Usage

```bash
# Simple prompt
chibi What are the benefits of using Rust?

# Multi-line prompt (end with . on empty line)
chibi
Explain the following concepts:
- Ownership
- Borrowing
- Lifetimes
.
```

### Managing Contexts

```bash
# Switch to a context (creates if needed)
chibi -s rust-learning

# Create a new auto-named context (e.g., 20240115_143022)
chibi -s new

# Create a prefixed auto-named context (e.g., bugfix_20240115_143022)
chibi -s new:bugfix

# Continue conversation
chibi Can you give me an example?

# List all contexts
chibi -l

# Check current context
chibi -w

# Clear context (preserves transcript)
chibi -C

# Delete a context
chibi -d old-project
```

### Using Custom Prompts

```bash
# Create a coding-focused context
chibi -s coding
chibi -e "You are a senior engineer. Be precise and technical."

# View the prompt
chibi -p

# Create a creative context
chibi -s stories
chibi -e ~/prompts/storyteller.md
```

### Using Tools

```bash
# With verbose mode to see tool calls
chibi -v "Read my package.json and list the dependencies"

# Tools work silently by default
chibi "What's in my Cargo.toml?"
```

### Piping and Scripting

```bash
# Pipe content into chibi
cat error.log | chibi "explain this error"
git diff | chibi "review these changes"

# Combine piped input with prompt argument
cat schema.sql | chibi "add a users table to this schema"

# Generate and save
chibi "Write a haiku about coding" > haiku.txt

# Process output
chibi "List 5 random numbers as JSON" | jq '.[0]'

# Use in shell scripts
version=$(chibi "What version of Python should I use for a new project in 2024? Just the number.")
echo "Using Python $version"
```

### Prompts Starting with Dash

```bash
# Use -- to force prompt interpretation
chibi -- -v is not a flag here, it's part of my prompt
```

## Transcript File Format

### Human-Readable (transcript.txt)

The `transcript.txt` file stores conversation history in a format matching the LLM's context:

```
[USER]: What is Rust?

[ASSISTANT]: Rust is a systems programming language...

[USER]: Tell me more about ownership.

[ASSISTANT]: Ownership is Rust's key feature...
```

### Machine-Readable (transcript.jsonl)

The `transcript.jsonl` file stores the same history in JSON Lines format, with additional metadata:

```json
{"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":1705123456,"from":"alice","to":"default","content":"What is Rust?","entry_type":"message"}
{"id":"550e8400-e29b-41d4-a716-446655440001","timestamp":1705123460,"from":"default","to":"user","content":"Rust is a systems programming language...","entry_type":"message"}
{"id":"550e8400-e29b-41d4-a716-446655440002","timestamp":1705123465,"from":"default","to":"read_file","content":"{\"path\":\"Cargo.toml\"}","entry_type":"tool_call"}
{"id":"550e8400-e29b-41d4-a716-446655440003","timestamp":1705123466,"from":"read_file","to":"default","content":"[package]\nname = \"chibi\"...","entry_type":"tool_result"}
```

Each entry contains:
- `id` - Unique UUID for the entry
- `timestamp` - Unix timestamp
- `from` / `to` - Source and destination (username, context name, or tool name)
- `content` - The message content or tool arguments/results
- `entry_type` - One of: `message`, `tool_call`, `tool_result`, `compaction`

## Context Locking

When chibi is actively using a context, it creates a lock file (`.lock`) containing a Unix timestamp. A background thread updates this timestamp periodically (every `lock_heartbeat_seconds`, default 30).

### Lock Status

The `-l` flag shows lock status for each context:

```bash
$ chibi -l
* default [active]    # Currently in use by a chibi process
  coding [stale]      # Lock exists but process likely crashed
  research            # No lock, not in use
```

- **`[active]`** - Lock file exists and was updated recently (within 1.5x heartbeat interval)
- **`[stale]`** - Lock file exists but is old (process likely crashed)
- No indicator - No lock file, context is free

### Stale Lock Handling

If you try to use a context with a stale lock, chibi will automatically clean it up and acquire a new lock. Active locks will block with an error message.

## Error Handling

Errors are reported to stderr. Common errors include:

- **Config not found**: Create `~/.chibi/config.toml` with required fields
- **API errors**: Check your API key and network connection
- **Empty prompt**: Prompt cannot be empty
- **Tool errors**: Check tool scripts are executable and output valid JSON

**Tips:**
- Contexts are created automatically when you switch to them
- Use `-v` to debug tool and API issues
- The default context is created on first use

## Building from Source

```bash
git clone <repository>
cd chibi
cargo build --release
cargo install --path .
```

## Dependencies

- `reqwest` - HTTP client with streaming support
- `tokio` - Async runtime
- `serde` / `serde_json` - Serialization
- `toml` - TOML parsing for config files
- `dirs-next` - Cross-platform directory handling
- `futures-util` - Stream utilities
- `uuid` - Unique IDs for transcript entries

## License

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.
