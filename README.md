# Chibi

A CLI tool for having conversations with AI models via OpenRouter. Chibi maintains conversation state across invocations, allowing you to have coherent multi-turn conversations directly from your terminal.

## Features

- **Persistent conversations** - State is saved between sessions
- **Multiple contexts** - Maintain separate conversations for different projects/topics
- **Transcript history** - Full chat history is preserved when contexts are cleared or compacted
- **Streaming responses** - Real-time output as the AI responds
- **Context window management** - Warnings when approaching context limits
- **Context compaction** - Reduce context size while preserving key information
- **TOML configuration** - Config file with comments support

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
# Common options:
# - anthropic/claude-3.5-sonnet
# - anthropic/claude-3.5-haiku
# - openai/gpt-4o
# - openai/gpt-4o-mini
# - meta-llama/llama-3.1-70b-instruct
model = "anthropic/claude-3.5-sonnet"

# Context window limit (tokens)
# This is used for calculating when to warn about approaching limits
context_window_limit = 100000

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

Chibi supports custom prompts to define its personality and behavior. Copy the example prompts from `prompts.example/` to `~/.chibi/prompts/`:

```bash
mkdir -p ~/.chibi/prompts
cp prompts.example/*.md ~/.chibi/prompts/
```

**Available prompts:**

- `chibi.md` - System/personality prompt (prepended as a system message to every conversation)
- `compaction.md` - Instructions for summarizing conversations when compacting
- `continuation.md` - Instructions after compaction/summary to guide the LLM's continuation

**Note:** These are optional. If not provided, default prompts will be used.

## Storage Structure

Chibi stores data in `~/.chibi/`:

```
~/.chibi/
├── config.toml          # Configuration (TOML format)
├── state.json           # Current context tracking
├── prompts/
│   ├── chibi.md         # System/personality prompt
│   ├── compaction.md    # Compaction instructions
│   └── continuation.md  # Post-compaction instructions
└── contexts/
    ├── default/
    │   ├── context.json     # Current conversation state
    │   └── transcript.txt   # Full chat history
    ├── coding/
    │   ├── context.json
    │   └── transcript.txt
    └── my-project/
        ├── context.json
        └── transcript.txt
```

## Command Reference

Commands are specified with the `--` prefix. When a non-dash argument is encountered, all remaining arguments are treated as a prompt.

| Flag | Description |
|------|-------------|
| `--switch <name>` / `-s <name>` | Switch to a different context |
| `--list` / `-l` | List all contexts |
| `--which` / `-w` | Show current context name |
| `--delete <name>` / `-d <name>` | Delete a context |
| `--clear` / `-C` | Clear current context (saves to transcript) |
| `--compact` / `-c` | Compact current context (saves to transcript) |
| `--rename <old> <new>` / `-r <old> <new>` | Rename a context |
| `--help` / `-h` | Show help message |
| `--version` / `-v` | Show version |

**Note:** Full-word commands require `--` prefix. Short flags (`-s`, `-l`, etc.) still work for convenience.

## Examples

### Starting a New Conversation

```bash
# Simple prompt
chibi What are the benefits of using Rust?

# Multi-line prompt
chibi
Explain the following concepts:
- Ownership
- Borrowing
- Lifetimes
.
```

### Continuing a Conversation

```bash
# Switch to a context, then ask follow-up
chibi --switch rust-learning
chibi Can you give me an example of borrowing?

# Or combine in one command
chibi --switch rust-learning give me examples of ownership in action

# Using short flags
chibi -s rust-learning give me examples of ownership
```

### Managing Multiple Projects

```bash
# Work on different projects in separate contexts
chibi --switch project-a explain the architecture

chibi --switch project-b help me debug this error

# List all your contexts
chibi --list

# Using short flags
chibi -l
```

### When Context Gets Too Large

```bash
# Check which context you're in
chibi --which

# Compact to keep only essential messages
chibi --compact

# Or clear entirely and start fresh (history preserved)
chibi --clear

# Using short flags
chibi -w
chibi -c
chibi -C
```

### Auto-Compaction

If `auto_compact = true` in config, chibi will automatically compact the context when it reaches the threshold:

```toml
# In ~/.chibi/config.toml
auto_compact = true
auto_compact_threshold = 80.0  # Compact when 80% of context window is used
```

### Creating a New Context

```bash
# Switching to a non-existent context creates it automatically
chibi --switch my-new-project
# Now working in 'my-new-project'

# Send a prompt (context will be created if it doesn't exist)
chibi --switch another-context explain something
```

### Custom Prompts

After setting up custom prompts in `~/.chibi/prompts/`, chibi will use them automatically:

- `chibi.md` defines the AI's personality
- `compaction.md` guides how conversations are summarized
- `continuation.md` helps the AI continue after compaction

### Prompt Starting with Dash

```bash
# Force prompt interpretation with --
chibi -- -this prompt starts with dash
chibi -- -v is a flag but here it's part of the prompt
```

## Transcript File Format

The `transcript.txt` file stores the full conversation history:

```
=== USER ===
What is Rust?

=== ASSISTANT ===
Rust is a systems programming language...

================================

=== USER ===
Tell me more about ownership.

=== ASSISTANT ===
Ownership is Rust's key feature...
```

## Error Handling

Errors are reported to stderr. Common errors include:

- **Config not found**: Create `~/.chibi/config.toml` with required fields
- **API errors**: Check your API key and network connection
- **Empty prompt**: Prompt cannot be empty (after trimming whitespace)
- **Context doesn't exist**: Check context name with `chibi --list` or `chibi -l`
- **Unknown option**: Make sure full-word commands use `--` prefix (e.g., `--switch` not `switch`)
- **Compaction errors**: If using auto-compaction or manual compaction, ensure the LLM can generate a summary

**Tips:**
- Contexts are created automatically when you switch to them or send a prompt
- The `default` context is created on first use
- Prompts can be changed by editing `~/.chibi/prompts/*.md`

## Building from Source

```bash
# Clone and build
git clone <repository>
cd chibi
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

## Dependencies

- `reqwest` - HTTP client with streaming support
- `tokio` - Async runtime
- `serde` - Serialization
- `toml` - TOML parsing for config files
- `dirs-next` - Cross-platform directory handling

## License

MIT
