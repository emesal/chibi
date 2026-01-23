# chibi

CLI tool for conversing with AI models via OpenRouter. Maintains conversation state across invocations for coherent multi-turn conversations directly from your terminal.

## Features

- **Persistent conversations** - State saved between sessions
- **Multiple contexts** - Separate conversations for different projects/topics
- **Plugin system** - Extend capabilities with custom tools
- **Streaming responses** - Real-time output as the AI responds
- **Rolling compaction** - Automatic context management with intelligent summarization
- **Agentic workflows** - Built-in tools for todos, goals, and autonomous processing
- **Cross-context messaging** - Contexts can communicate with each other
- **Unix philosophy** - Only LLM output goes to stdout (pipeable)

## Quick Start

### Install

```bash
cargo install --path .
```

### Configure

Create `~/.chibi/config.toml`:

```toml
api_key = "your-openrouter-api-key"
model = "anthropic/claude-sonnet-4"
context_window_limit = 200000
warn_threshold_percent = 80.0
```

Copy example prompts:

```bash
mkdir -p ~/.chibi/prompts
cp prompts.example/*.md ~/.chibi/prompts/
```

### Use

```bash
# Simple prompt
chibi What is Rust?

# Pipe content
cat error.log | chibi "explain this error"

# Different contexts
chibi -c coding "Review this function"
chibi -c research "Find info about X"

# See tool activity
chibi -v "Read my Cargo.toml"
```

## Documentation

- **[Getting Started](docs/getting-started.md)** - Installation and first steps
- **[Configuration](docs/configuration.md)** - Full config reference including API parameters
- **[Contexts](docs/contexts.md)** - Managing multiple conversations
- **[Plugins](docs/plugins.md)** - Creating tools for the LLM
- **[Hooks](docs/hooks.md)** - Lifecycle event system
- **[Agentic Workflows](docs/agentic.md)** - Autonomous multi-step processing
- **[CLI Reference](docs/cli-reference.md)** - All command flags
- **[Transcript Format](docs/transcript-format.md)** - JSONL format specification

## Command Overview

```bash
# Contexts
chibi -c <name>       # Switch context
chibi -C <name>       # Transient context (one-off)
chibi -L              # List contexts
chibi -l              # Current context info

# History
chibi -a              # Archive current context
chibi -z              # Compact current context
chibi -g 10           # Show last 10 log entries

# System prompts
chibi -y "prompt"     # Set current context's prompt
chibi -n system_prompt # View current prompt

# Tools
chibi -v              # Verbose (see tool calls)
chibi -p plugin args  # Run plugin directly
```

See [CLI Reference](docs/cli-reference.md) for the complete list.

## Example Plugins

See [chibi-plugins](https://github.com/emesal/chibi-plugins) for ready-to-use plugins:

- `read_file` - Read file contents
- `fetch_url` - Fetch web content
- `run_command` - Execute shell commands (with confirmation)
- `web_search` - Search via DuckDuckGo
- `recurse` - Continue processing autonomously
- `sub-agent` - Spawn sub-agents in other contexts
- `github-mcp` - GitHub integration via MCP

## Storage

```
~/.chibi/
├── config.toml           # Global configuration
├── models.toml           # Model metadata (optional)
├── prompts/              # System prompts
│   ├── chibi.md          # Default prompt
│   └── reflection.md     # LLM's persistent memory
├── plugins/              # Plugin scripts
└── contexts/<name>/
    ├── context.jsonl     # Conversation history
    ├── local.toml        # Per-context config
    ├── todos.md          # Current todos
    ├── goals.md          # Current goals
    └── system_prompt.md  # Custom prompt (optional)
```
