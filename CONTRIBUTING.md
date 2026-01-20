# Contributing

## Design Goals

Chibi aims to be a minimal, secure core with extensibility through plugins.

- **Core binary**: Small, stable, few dependencies. Changes here need strong justification.
- **Plugins**: Where features live. Most contributions should be plugins, not core changes.

See [PHILOSOPHY.md](PHILOSOPHY.md) for the full picture.

## Plugins vs Core

Before adding code to the Rust binary, ask: can this be a plugin?

Plugins can:
- Add new capabilities for the LLM (tool calls)
- Hook into lifecycle events (pre_message, post_tool, etc.)
- Inject content into system prompts
- Intercept and modify behavior

The core handles: API communication, context management, plugin loading, streaming.

## CLI Conventions

Commands use flags (`-l`, `-s`, `-C`). Bare words are prompts.

```bash
chibi -l              # command: list contexts
chibi list            # prompt: sends "list" to LLM
```

Tests in `src/cli.rs` codify this behavior.

## Testing

```bash
cargo test            # all tests
cargo test cli        # CLI parsing tests
```

## Plugins Repo

For plugin contributions, see [chibi-plugins](https://github.com/emesal/chibi-plugins).
