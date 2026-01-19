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
- **Context compaction** - Reduce context size while preserving key information
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

**Note:** TOML format is used instead of JSON, allowing you to add helpful comments directly in the config file.

## System Prompts

Chibi supports a default system prompt and per-context custom prompts.

### Default Prompt

Copy the example prompts from `prompts.example/` to `~/.chibi/prompts/`:

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

### Setting Up Tools

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
3. **Accept JSON parameters on stdin** when called normally
4. **Output results to stdout**

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

# Read JSON from stdin and extract path
path=$(jq -r '.path')
cat "$path"
```

### Example Tool (Python)

```python
#!/usr/bin/env python3
# ~/.chibi/tools/web_search

import sys
import json

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

params = json.load(sys.stdin)
# ... perform search ...
print(json.dumps(results))
```

### Example Tools

The `examples/tools/` directory contains ready-to-use tools:

- `read_file` - Read file contents
- `fetch_url` - Fetch content from a URL
- `run_command` - Execute shell commands
- `web_search` - Search the web (requires `duckduckgo-search` Python package)

Copy them to use:
```bash
cp examples/tools/* ~/.chibi/tools/
chmod +x ~/.chibi/tools/*
```

### Viewing Tool Activity

Use `-v` to see which tools are loaded and when they're called:

```bash
chibi -v "Read my Cargo.toml"
# stderr: [Loaded 1 tool(s): read_file]
# stderr: [Tool: read_file]
# stdout: <LLM response about the file>
```

## Storage Structure

Chibi stores data in `~/.chibi/`:

```
~/.chibi/
├── config.toml          # Configuration (TOML format)
├── state.json           # Current context tracking
├── prompts/
│   ├── chibi.md         # Default system/personality prompt
│   ├── compaction.md    # Compaction instructions
│   └── continuation.md  # Post-compaction instructions
├── tools/               # Executable tool scripts
│   ├── read_file
│   ├── fetch_url
│   └── ...
└── contexts/
    ├── default/
    │   ├── context.json     # Current conversation state
    │   └── transcript.txt   # Full chat history
    ├── coding/
    │   ├── context.json
    │   ├── transcript.txt
    │   └── system_prompt.md # Custom prompt for this context
    └── my-project/
        ├── context.json
        └── transcript.txt
```

## Command Reference

| Flag | Description |
|------|-------------|
| `-s, --switch <name>` | Switch to a different context |
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
