![chibi~](docs/images/socials-wide-trans.png)

A minimal, composable building block for LLM interactions. Chibi provides persistent context, a plugin/hook system, and communication primitives — everything else lives in plugins.

Think of it as a Lego brick: tiny, light, but infinitely combinable. Multiple chibis with different models, temperatures, and plugins can work together. The plugin system is deliberately permissive, exposing the full lifecycle via hooks to enable experimentation with coordination patterns, workflows, and agentic behaviors.

**Early development — not yet stable.**

## Install

```bash
git clone --recurse-submodules https://github.com/emesal/chibi.git
cd chibi && cargo install --path .
```

## Configure

Create `~/.chibi/config.toml`:

```toml
api_key = "your-openrouter-api-key"
model = "anthropic/claude-sonnet-4"
context_window_limit = 200000
warn_threshold_percent = 80.0
```

(This step will be automated in a future release.)

Copy the example prompts:

```bash
mkdir -p ~/.chibi/prompts && cp examples/prompts/*.md ~/.chibi/prompts/
```

## Use

```bash
chibi What is Rust?                       # Simple prompt
cat error.log | chibi "explain this"      # Pipe content
chibi -c project "Review this function"   # Named context
chibi -v "Read my Cargo.toml"             # Verbose (show tool use)
```

Contexts persist across invocations. Switch with `-c <name>`, list with `-L`.

![chibi explain this girl](docs/images/explain_this.png)

## Documentation

- [Getting Started](docs/getting-started.md) — Installation and first steps
- [Configuration](docs/configuration.md) — Full config reference
- [Contexts](docs/contexts.md) — Managing conversations
- [Plugins](docs/plugins.md) — Creating tools for the LLM
- [Hooks](docs/hooks.md) — Lifecycle event system
- [Agentic Workflows](docs/agentic.md) — Autonomous processing
- [CLI Reference](docs/cli-reference.md) — All flags and commands
- [Images](docs/images.md) — Terminal image rendering
- [Transcript Format](docs/transcript-format.md) — JSONL format spec

Example plugins: [chibi-plugins](https://github.com/emesal/chibi-plugins)

## License

ISC

Make meow, not rawr
