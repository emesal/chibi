![chibi~](docs/images/socials-wide-trans.png)

A minimal, composable building block for LLM interactions. Chibi provides persistent context, a plugin/hook system, and communication primitives — everything else lives in plugins.

Think of it as a Lego brick: tiny, light, but infinitely combinable. Multiple chibis with different models, temperatures, and plugins can work together. The plugin system is deliberately permissive, exposing the full lifecycle via hooks to enable experimentation with coordination patterns, workflows, and agentic behaviors.

**Early development — not yet stable.**

## Install

```bash
git clone https://github.com/emesal/chibi.git
cd chibi && just install
```

Requires [just](https://github.com/casey/just) and a Rust toolchain. Cargo fetches all dependencies automatically.

## Use

```bash
chibi What is Rust?                       # Simple prompt
cat error.log | chibi "explain this"      # Pipe content
chibi -c project "Review this function"   # Named context
chibi -v "Read my Cargo.toml"             # Verbose (show tool use)
```

Contexts persist across invocations. Switch with `-c <name>`, list with `-L`.

![chibi explain this girl](docs/images/explain_this.png)

## Configure

For better models or your own API key, create `~/.chibi/config.toml`:

```toml
# API key for OpenRouter (https://openrouter.ai/settings/keys)
api_key = "your-api-key-here"

# Model to use (default: ratatoskr:free/agentic)
model = "anthropic/claude-sonnet-4"
```

All fields are optional. See [Configuration](docs/configuration.md) for the full reference.

## Documentation

- [Getting Started](docs/getting-started.md) — Installation and first steps
- [Configuration](docs/configuration.md) — Full config reference
- [Contexts](docs/contexts.md) — Managing conversations
- [Plugins](docs/plugins.md) — Creating tools for the LLM
- [Hooks](docs/hooks.md) — Lifecycle event system
- [MCP Servers](docs/mcp.md) — Using MCP-compatible tool providers
- [Virtual File System](docs/vfs.md) — Sandboxed shared file space for contexts
- [Agentic Workflows](docs/agentic.md) — Autonomous processing
- [CLI Reference](docs/cli-reference.md) — All flags and commands
- [Images](docs/images.md) — Terminal image rendering
- [Markdown Themes](docs/markdown-themes.md) — Customising colour schemes
- [Transcript Format](docs/transcript-format.md) — JSONL format spec
- [Upgrade Notes](docs/upgrade-notes.md) — Breaking changes requiring user action

Example plugins: [chibi-plugins](https://github.com/emesal/chibi-plugins)

## License

ISC

Make meow, not rawr
