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

## Quick Start

Chibi uses [OpenRouter](https://openrouter.ai) as its API backend. Get a free key at
[openrouter.ai/settings/keys](https://openrouter.ai/settings/keys) (no credit card needed), then:

```bash
CHIBI_API_KEY=your-key chibi "hello, what can you do?"
```

## Customisation

Persist your key and any other settings in `~/.chibi/config.toml`:

```toml
# API key for OpenRouter (https://openrouter.ai/settings/keys)
api_key = "your-api-key-here"

# Model to use (default: ratatoskr:free/agentic)
model = "anthropic/claude-sonnet-4"
```

All fields are optional — only set what you want to change. See [configuration.md](configuration.md) for the full reference.

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
# stderr: [Tool: file_head(path: package.json)]
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
