![chibi~](docs/images/socials-wide-trans.png)

CLI tool for conversing with AI models via OpenRouter. Maintains conversation state across invocations for coherent multi-turn conversations directly from your terminal.

*This tool is still in early development and not ready for general use.*

## Features

- **Persistent conversations** - State saved between sessions
- **Multiple contexts** - Separate conversations for different projects/topics
- **Plugin system** - Extend capabilities with custom tools
- **Streaming responses** - Real-time output as the AI responds
- **Rolling compaction** - Automatic context management with intelligent summarization
- **Agentic workflows** - Built-in tools for todos, goals, and autonomous processing
- **Cross-context messaging** - Contexts can communicate with each other
- **Large output caching** - Tool outputs automatically cached with surgical access tools
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
cp examples/prompts/*.md ~/.chibi/prompts/
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
- **[Images](docs/images.md)** - Image rendering in the terminal
- **[Contexts](docs/contexts.md)** - Managing multiple conversations
- **[Plugins](docs/plugins.md)** - Creating tools for the LLM
- **[Hooks](docs/hooks.md)** - Lifecycle event system
- **[Agentic Workflows](docs/agentic.md)** - Autonomous multi-step processing
- **[CLI Reference](docs/cli-reference.md)** - All command flags
- **[Transcript Format](docs/transcript-format.md)** - JSONL format specification

## Command Overview

```bash
# Contexts
chibi -c <name>           # Switch to context (persistent)
chibi -C <name>           # Use context for this invocation only
chibi -L                  # List all contexts
chibi -l                  # Current context info

# History
chibi -a                  # Archive current context
chibi -z                  # Compact current context
chibi -g 10               # Show last 10 log entries

# System prompts
chibi -y "prompt"         # Set current context's prompt
chibi -n system_prompt    # View current prompt

# Tools
chibi -v                  # Verbose mode
chibi -x                  # Force-disable the LLM
chibi -X                  # Force-enable the LLM
```

See [CLI Reference](docs/cli-reference.md) for the complete list.

<div align="center">
    <br>
    <img src="docs/images/explain_this.png" width="62%">
    <br>
</div>

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

The .toml and .md files are intended to be modified by humans. Editing other
files might break things in unexpected and unpleasant ways.

```
~/.chibi/
├── config.toml             # Required: api_key, model, context_window_limit, warn_threshold_percent
├── models.toml             # Model aliases, context windows, API params
├── state.json              # Application state
├── prompts/
│   ├── chibi.md            # Default system prompt
│   ├── reflection.md       # LLM's persistent memory
│   ├── compaction.md       # Compaction instructions
│   └── continuation.md     # Post-compaction instructions
├── plugins/                # Executable scripts (provide tools)
└── contexts/<name>/
    ├── context.jsonl       # LLM window (bounded by compaction)
    ├── transcript/         # Authoritative log (partitioned, never truncated)
    │   ├── manifest.json   # Partition metadata, timestamp ranges
    │   ├── active.jsonl    # Current write partition
    │   └── partitions/     # Archived read-only partitions
    ├── transcript.md       # Human-readable archive
    ├── context_meta.json   # Metadata (system_prompt_md_mtime, last_combined_prompt)
    ├── local.toml          # Per-context config overrides
    ├── summary.md          # Conversation summary
    ├── todos.md            # Current todos
    ├── goals.md            # Current goals
    ├── inbox.jsonl         # Messages from other contexts
    ├── system_prompt.md    # Context-specific system prompt
    └── tool_cache/         # Cached large tool outputs
```

## License

ISC

Make meow, not rawr
