# Getting Started

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

## Basic Configuration

Create a configuration file at `~/.chibi/config.toml`:

```toml
# API key for your LLM provider
# Currently chibi uses OpenRouter (https://openrouter.ai/settings/keys)
api_key = "your-api-key-here"

# Model to use (see https://openrouter.ai/models)
model = "anthropic/claude-sonnet-4"

# Context window limit (tokens)
context_window_limit = 200000

# Warning threshold percentage (0-100)
warn_threshold_percent = 80.0
```

See [configuration.md](configuration.md) for the full configuration reference.

## System Prompts

Copy the example prompts to set up the default personality:

```bash
mkdir -p ~/.chibi/prompts
cp examples/prompts/*.md ~/.chibi/prompts/
```

**Available prompts:**

- `chibi.md` - Default system/personality prompt
- `compaction.md` - Instructions for summarizing conversations during compaction
- `continuation.md` - Instructions after compaction to guide continuation

## First Conversation

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

## Managing Contexts

Contexts are separate conversations. Each context maintains its own message history, todos, goals, and optionally its own system prompt and configuration.

```bash
# Switch to a context (creates if needed)
chibi -c rust-learning

# Continue conversation
chibi Can you give me an example?

# List all contexts
chibi -L

# Show current context info
chibi -l
```

See [contexts.md](contexts.md) for more details.

## Using Tools

Plugins provide tools that the LLM can call. With verbose mode you can see tool activity:

```bash
chibi -v "Read my package.json and list the dependencies"
# stderr: [Loaded 1 tool(s): read_file]
# stderr: [Tool: read_file]
# stdout: <LLM response about the file>
```

See [plugins.md](plugins.md) for how to set up and create plugins.

## Piping and Scripting

Chibi follows Unix conventions - only LLM output goes to stdout:

```bash
# Pipe content into chibi
cat error.log | chibi "explain this error"
git diff | chibi "review these changes"

# Pipe output to other commands
chibi "Generate a JSON config" | jq .

# Save response to file
chibi "Write a poem" > poem.txt

# Use in scripts
result=$(chibi "What is 2+2? Just the number.")
```

## Next Steps

- [Configuration Reference](configuration.md) - All config options including API parameters
- [Context Management](contexts.md) - Multiple conversations, locking, ephemeral contexts
- [Plugins](plugins.md) - Extending chibi with custom tools
- [Hooks](hooks.md) - Lifecycle event system
- [Agentic Workflows](agentic.md) - Todos, goals, reflection, autonomous processing
- [CLI Reference](cli-reference.md) - Complete command reference
