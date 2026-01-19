# chibi

Prototype CLI tool for having conversations with AI models via OpenRouter. Chibi maintains conversation state across invocations, allowing you to have coherent multi-turn conversations directly from your terminal.

## Features

- **Persistent conversations** - State is saved between sessions
- **Multiple contexts** - Maintain separate conversations for different projects/topics
- **Per-context system prompts** - Each context can have its own personality/instructions
- **Tools support** - Extend capabilities with custom scripts the LLM can call
- **Transcript history** - Full chat history is preserved when contexts are cleared or compacted
- **Streaming responses** - Real-time output as the AI responds
- **Context window management** - Warnings when approaching context limits
- **Rolling compaction** - Automatically strips oldest messages and integrates them into a summary
- **Agentic workflow** - Built-in tools for todos, goals, and autonomous processing
- **Cross-context access** - Read state from other contexts (for sub-agents)
- **Piped input** - Accept prompts from stdin for scripting
- **Unix philosophy** - Only LLM output goes to stdout, making it pipeable

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
- `max_recursion_depth` - Limit for `continue_processing` loops (default: 15)

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

## Tools

Chibi can call external scripts as tools, allowing the LLM to perform actions like reading files, fetching URLs, or running commands.

### THIS IS THE DANGER ZONE!

Chibi does not impose any restrictions on tools. NONE. Each tool is responsible for its own safety measures. *You are expected to understand the tools you install.*

See *Tool Safety* below.

### Setting Up Tools

You need to do this yourself.

1. Create the tools directory: `mkdir -p ~/.chibi/tools`
2. Add executable scripts to the directory
3. Each script must support `--schema` to describe itself

### Tool Script Requirements

Each tool script must:

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

### Example Tool (Bash)

```bash
#!/bin/bash
# ~/.chibi/tools/read_file

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

### Example Tool (Python)

```python
#!/usr/bin/env python3
# ~/.chibi/tools/web_search

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

### Example Tools

The `examples/tools/` directory contains ready-to-use tools:

- `read_file` - Read file contents (bash)
- `fetch_url` - Fetch content from a URL (bash)
- `run_command` - Execute shell commands (bash)
- `web_search` - Search the web via DuckDuckGo (Python, requires `uv`)

Copy them to use:
```bash
cp examples/tools/* ~/.chibi/tools/
chmod +x ~/.chibi/tools/*
```

### MCP Wrapper Tools

Chibi can connect to MCP (Model Context Protocol) servers through wrapper tools. Two examples are provided:

**fetch-mcp** (Bash) - Simple MCP wrapper, no caching:
```bash
# Requires: curl, jq
# Configure: FETCH_MCP_URL=http://your-mcp-server
cp examples/tools/fetch-mcp ~/.chibi/tools/
```

**github-mcp** (Python) - Full-featured with caching:
```bash
# Requires: uv (https://docs.astral.sh/uv/) - deps managed automatically
# Configure: GITHUB_TOKEN=your-token
cp examples/tools/github-mcp ~/.chibi/tools/

# First, refresh the tool cache:
echo '{"refresh_cache": true}' | ~/.chibi/tools/github-mcp
```

The GitHub MCP wrapper demonstrates caching: it stores discovered tools in `~/.chibi/cache/github-mcp.json` so they're available at startup. The LLM can call `{"refresh_cache": true}` to update the cache if tools change.

These examples serve as templates - copy and modify them to connect to any MCP server.

### Tool Safety

Tools are responsible for their own safety guardrails. Chibi passes these environment variables to tools:
- `CHIBI_TOOL_ARGS` - JSON arguments (always set)
- `CHIBI_VERBOSE=1` - when `-v` is used, allowing tools to adjust their behavior

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
| `read_context` | Read another context's state (read-only, for sub-agents) |
| `continue_processing` | Continue working without returning to user |
| `update_reflection` | Update persistent memory (when reflection is enabled) |

### Todos and Goals

Each context can have its own todos and goals stored in markdown files:

- **Todos** (`~/.chibi/contexts/<name>/todos.md`) - Short-term tasks for the current round
- **Goals** (`~/.chibi/contexts/<name>/goals.md`) - Long-term objectives that persist

These are automatically included in the system prompt, so the LLM always knows what it's working toward.

### Continue Processing (Autonomous Mode)

The `continue_processing` tool lets the LLM work autonomously:

```
LLM: "I need to do more work. Let me continue."
     [calls continue_processing with note: "Check the test results next"]

LLM: (new round) "Continuing from previous round. Note to self: Check the test results next"
     ... continues working ...
```

The LLM leaves itself a note about what to do next, then the conversation continues automatically.

### Sub-Agents

Use the wrapper tool approach for sub-agents. Create a tool that spawns chibi with a different context:

```bash
#!/bin/bash
# ~/.chibi/tools/spawn_agent
context_name=$(echo "$CHIBI_TOOL_ARGS" | jq -r '.context')
task=$(echo "$CHIBI_TOOL_ARGS" | jq -r '.task')
chibi -s "$context_name" "$task"
```

The main agent can then use `read_context` to inspect the sub-agent's results:

```
Main: [calls spawn_agent with context: "research", task: "Find info about X"]
Main: [calls read_context with context_name: "research"]
Main: "The sub-agent found: ..."
```

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
├── config.toml          # Configuration (TOML format)
├── state.json           # Current context tracking
├── prompts/
│   ├── chibi.md         # Default system/personality prompt
│   ├── compaction.md    # Compaction instructions
│   ├── continuation.md  # Post-compaction instructions
│   └── reflection.md    # LLM's persistent memory (auto-created)
├── cache/               # Tool caches (optional, tool-managed)
│   └── github-mcp.json  # Example: cached MCP tool definitions
├── tools/               # Executable tool scripts
│   ├── read_file
│   ├── fetch_url
│   ├── github-mcp       # MCP wrapper example
│   └── ...
└── contexts/
    ├── default/
    │   ├── context.json     # Current conversation state (includes summary)
    │   ├── transcript.txt   # Full chat history
    │   ├── todos.md         # Current todos (auto-created)
    │   └── goals.md         # Current goals (auto-created)
    ├── coding/
    │   ├── context.json
    │   ├── transcript.txt
    │   ├── todos.md
    │   ├── goals.md
    │   └── system_prompt.md # Custom prompt for this context
    └── my-project/
        ├── context.json
        ├── transcript.txt
        ├── todos.md
        └── goals.md
```

## Command Reference

| Flag | Description |
|------|-------------|
| `-s, --switch <name>` | Switch to a different context (`new` for auto-name, `new:prefix` for prefixed) |
| `-l, --list` | List all contexts |
| `-w, --which` | Show current context name |
| `-d, --delete <name>` | Delete a context |
| `-C, --clear` | Clear current context (saves to transcript) |
| `-c, --compact` | Compact current context (saves to transcript) |
| `-r, --rename <old> <new>` | Rename a context |
| `-H, --history` | Show recent messages (default: 6) |
| `-n, --num-messages <N>` | Number of messages to show (0 = all, implies -H) |
| `-p, --prompt` | Show system prompt for current context |
| `-e, --set-prompt <arg>` | Set system prompt (file path or literal text) |
| `-v, --verbose` | Show extra info (tools loaded, warnings, etc.) |
| `-x, --no-reflection` | Disable reflection for this invocation |
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

The `transcript.txt` file stores conversation history in a format matching the LLM's context:

```
[USER]: What is Rust?

[ASSISTANT]: Rust is a systems programming language...

[USER]: Tell me more about ownership.

[ASSISTANT]: Ownership is Rust's key feature...
```

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

## License

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.
