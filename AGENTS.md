# AGENTS.md

## Vision

Chibi is a minimal, composable building block for LLM interactions — not an agent framework. It provides persistent context storage, extensible behavior via plugins/hooks, and communication primitives. Everything else (coordination patterns, workflows, domain behaviors) lives in plugins. This separation keeps chibi small and enables unlimited experimentation at the plugin layer.

## Principles

- Establish patterns now that scale well, refactor liberally when beneficial.
- Backwards compatibility not a priority, legacy code unwanted. (Pre-alpha.)
- Focused, secure core. Protect file operations from corruption and race conditions.
- Modular designs, not monolithic. Less code is better code; one pattern is better than two.
- Self-documenting code; keep symbols, comments, and docs consistent.
- Missing or incorrect documentation including code comments are critical bugs.
- Iterate over structures to prevent code duplication.
- Comprehensive tests including edge cases.
- Remind user about `just pre-push` before pushing and `just merge-to-dev` when merging feature branches.

## Build

```bash
cargo build                              # Debug build
cargo test                               # Run tests
cargo install --path .                   # Install to ~/.cargo/bin
```

Git dependencies: [ratatoskr](https://github.com/emesal/ratatoskr) (LLM API client), [streamdown-rs](https://github.com/emesal/streamdown-rs) (markdown renderer).

## Architecture

Cargo workspace with four crates:

```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)

chibi-mcp-bridge (binary, async daemon)
    communicates with chibi-core via JSON-over-TCP
```

LLM communication is delegated to ratatoskr; `gateway.rs` bridges chibi's types to ratatoskr's `ModelGateway` interface. See [docs/architecture.md](docs/architecture.md) for per-file details, storage layout, and data flow.

## Documentation

- [docs/architecture.md](docs/architecture.md) — Crate structure, file listings, storage layout, data flow
- [docs/plugins.md](docs/plugins.md) — Plugin authoring, hooks registration, language plugins
- [docs/hooks.md](docs/hooks.md) — Hook reference with payloads and examples
- [docs/configuration.md](docs/configuration.md) — Config options
- [docs/contexts.md](docs/contexts.md) — Context management
- [docs/agentic.md](docs/agentic.md) — Agentic workflows, sub-agents, tool output caching
- [docs/cli-reference.md](docs/cli-reference.md) — CLI flags and usage
