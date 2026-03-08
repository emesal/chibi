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
- [docs/vfs.md](docs/vfs.md) — Virtual file system
- [docs/cli-reference.md](docs/cli-reference.md) — CLI flags and usage

## Quirks / Gotchas

- `ContextEntry.cwd` is `None` for contexts created before this field was added; `config_resolution` falls back to `std::env::current_dir()` in that case.
- `call_agent` is not exposed to the LLM as a callable tool (not in `FLOW_TOOL_DEFS`). Its constant, metadata, and `HandoffTarget::Agent` are retained for the fallback tool mechanism, hook overrides, and future inter-agent control transfer.
- `TranscriptEntry` now has `role: Option<String>` and `flow_control: bool`. Old entries without `role` use the `to == "user"` heuristic in `entries_to_messages` for backwards compat. Prefer builder pattern over struct literals to avoid missing new fields.
- `AppState.state` is `Arc<RwLock<ContextState>>` — use `.read().unwrap()` / `.write().unwrap()` guards. Panics only on lock poison (indicates a prior panic, not normal flow).
- `ContextsBackend` reads the flock registry directly from disk (not via VFS) to avoid a circular dependency — it is itself a VFS backend.
- `PartitionMeta.prompt_count` uses `serde(default)` — pre-existing manifests without this field deserialise to 0; no backfill of old manifests.
- `ContextsBackend` uses `PartitionManager::load_with_config` for `prompt_count` on each `state.json` read (full active-partition scan). If performance becomes a concern, pass a cached `ActiveState` via `load_with_cached_state`.
- Synthesised tools: `(harness tools)` module provides `call-tool` and `define-tool`. `HARNESS_PREAMBLE` defines `%tool-registry%` and `define-tool` at top level (not inside the library) so `set!` can mutate it and rust can read it post-eval.
- `ToolImpl::Synthesised` has `exec_binding` field: `"tool-execute"` for convention format, `"%tool-execute-{name}%"` for `define-tool` multi-tool files.
- `reload_tool_from_content` and `scan_and_register` require `&ToolsConfig` for tier resolution. Pass `&ToolsConfig::default()` when no tier overrides needed.
- `call-tool` bridge uses one global mutex: `BRIDGE_CALL_CTX` (set/cleared per execute via `CallContextGuard`). Registry is embedded in `ToolImpl::Synthesised` and passed through `execute_synthesised` — no longer a separate global. Reason: tein runs scheme on a dedicated worker thread; thread-locals set on the caller thread would be invisible there.
- `ToolImpl::Synthesised` carries `registry: Arc<RwLock<ToolRegistry>>` so `call-tool` can dispatch to any registered tool from the tein worker thread without thread-local state.
- Harness also exposes `%context-name%` (mutable binding, injected per call), `(generate-id)` (4 hex chars, subsecond nanos), and `(current-timestamp)` (`YYYYMMDD-HHMMz` UTC).
- Structured tasks replace `todos.md`. `.task` files under `/home/<ctx>/tasks/` and `/flocks/<name>/tasks/`. Parsed at each prompt by `state::tasks::collect_tasks`, ephemeral table injected before last user message. `tasks.scm` plugin in `plugins/` provides CRUD tools.
- `Modules::Safe` allowlist (tein) includes `(scheme base)`, `(scheme write)`, `(scheme read)`, `(scheme char)`, and other pure modules. Modules with `default_safe: false` (e.g. `(scheme regex)`, `(tein modules)`) are blocked in the sandboxed tier.
