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
- Synthesised tools: `(harness tools)` module provides `call-tool` and `define-tool`. `(harness hooks)` module provides `register-hook`. `HARNESS_PREAMBLE` defines `%tool-registry%`, `%hook-registry%`, `define-tool`, and `register-hook` at top level (not inside the library) so `set!` can mutate them and rust can read them post-eval.
- `ToolImpl::Synthesised` has `exec_binding` field: `"tool-execute"` for convention format, `"%tool-execute-{name}%"` for `define-tool` multi-tool files.
- `reload_tool_from_content` and `scan_and_register` require `&ToolsConfig` for tier resolution. Pass `&ToolsConfig::default()` when no tier overrides needed.
- `call-tool` bridge uses one global mutex: `BRIDGE_CALL_CTX` (set/cleared per execute via `CallContextGuard`). Registry is embedded in `ToolImpl::Synthesised` and passed through `execute_synthesised` — no longer a separate global. Reason: tein runs scheme on a dedicated worker thread; thread-locals set on the caller thread would be invisible there.
- `ToolImpl::Synthesised` carries `registry: Arc<RwLock<ToolRegistry>>` so `call-tool` can dispatch to any registered tool from the tein worker thread without thread-local state.
- Harness also exposes `%context-name%` (mutable binding, injected per call), `(generate-id)` (8 hex chars, uuid v4), and `(current-timestamp)` (`YYYYMMDD-HHMMz` UTC).
- Structured tasks replace `todos.md`. `.task` files under `/home/<ctx>/tasks/` and `/flocks/<name>/tasks/`. Parsed at each prompt by `state::tasks::collect_tasks`, ephemeral table injected before last user message. `tasks.scm` plugin in `plugins/` provides CRUD tools.
- `/sys/contexts/<name>/task-dirs` is a virtual file returning a Scheme list datum (via `tein_sexp::Sexp`) of all task directories visible to the context. It is the single source of truth for task directory enumeration — `tasks.scm` reads it via `file_head` + `read`; Rust-side `collect_tasks` uses `task_dirs_for()` on the same `ContextsBackend`.
- `Modules::Safe` allowlist (tein) includes `(scheme base)`, `(scheme write)`, `(scheme read)`, `(scheme char)`, and other pure modules. Modules with `default_safe: false` (e.g. `(scheme regex)`, `(tein modules)`) are blocked in the sandboxed tier.
- `execute_hook` dispatches to both subprocess plugins and synthesised tein callbacks. Tein dispatch uses a thread-local `TEIN_HOOK_GUARD` (`HashSet<HookPoint>`) for re-entrancy prevention — if a tein hook callback triggers the same hook point, tein callbacks are skipped on the recursive call. `ToolImpl::Synthesised` carries `hook_bindings: HashMap<HookPoint, String>` mapping hook points to named scheme bindings that `execute_hook` calls.
- `execute_hook` accepts an optional `TeinHookContext` (4th parameter, always present regardless of feature flags). When `Some`, sets `CallContextGuard` per tein tool during dispatch, enabling `call-tool` and `(harness io)` from hook callbacks. Call sites without full async context (sync lifecycle hooks, indexer, compact) pass `None`.
- `(harness io)` is only available at `SandboxTier::Unsandboxed`. Uses `BRIDGE_CALL_CTX` (same mechanism as `call-tool`) for runtime context — IO functions only work during active tool execution or hook dispatch with `TeinHookContext`. VFS operations use `VfsCaller::System` (bypasses zone permissions). Path dispatch: `"vfs://..."` → VFS, bare absolute path → `tokio::fs`. IO bypasses the hook layer entirely — no hook callbacks fire from `io-write`/`io-read`. Exports: `io-read`, `io-write`, `io-append`, `io-list`, `io-exists?`, `io-delete`.
- `execute_hook` deduplicates tein dispatch by `(worker_thread_id, binding)` pair. Multi-tool plugins share bindings across all their tools; without dedup, each hook event would fire N times (once per tool). The dedup set is local to each `execute_hook` call.
- `with_vfs_shadows()` is required on the unsandboxed `Context::builder()` in `build_tein_context`. Without it, `(scheme process-context)` is missing — blocking `(chibi term ansi)` → `(chibi diff)`. Added in `synthesised.rs`.
- `BUILTIN_UNSANDBOXED` in `config.rs`: compile-time list of VFS paths that `resolve_tier` defaults to `Unsandboxed` without user config. Currently: `["/tools/shared/history.scm"]`. User `[tools.tiers]` overrides take precedence.
- `PreVfsWrite` / `PostVfsWrite` hooks fire only for context-initiated writes going through `send.rs` tool dispatch. Writes via `VfsCaller::System` / `(harness io)` bypass the hook dispatch layer entirely.
- `history.scm` stores snapshots under `<file-dir>/.chibi/history/<filename>/<N>`. The `.chibi/` prefix hides them from `vfs_list` (dotfile filter). `io-list` and direct addressing still reach them. Prunes to 10 revisions by default.
- `scheme_eval` tool (`eval.rs`): persistent sandboxed tein environments keyed by context name in `EVAL_CONTEXTS` (process-global `LazyLock<Mutex<HashMap>>`). Evicted on context clear/destroy/rename via `evict_eval_context()` (called from `context_ops.rs`); next eval lazily recreates. `parallel: false` — concurrent calls for the same context would collide on `BRIDGE_CALL_CTX`. Registered via `register_eval_tools(&Arc<RwLock<ToolRegistry>>)` after the registry Arc is created (not with other `register_*_tools(&mut reg)` calls).
- `(tein json)` exports `json-parse` and `json-stringify` (not `json-read-string`). `(tein safe-regexp)` exports `regexp`, `regexp-search`, `regexp-matches?`, `regexp-replace`, `regexp-replace-all`, `regexp-split`, `regexp-extract`, `regexp-fold`, `regexp-match-submatch`, `regexp-match->list`. Requires `regex` cargo feature on tein dep.
- `build_sandboxed_harness_context()` in `synthesised.rs` is the `pub(crate)` bridge for `eval.rs` — wraps `build_tein_context("", Sandboxed)` so eval can add its own prelude without duplicating FFI setup.
- `scheme_eval` and `execute_synthesised` return structured output: `"result: <value>\nstdout: <output>\nstderr: <output>"`. The `result` field contains the expression's return value (or `"error: ..."`). stdout/stderr show `"(empty)"` when nothing was captured. `format_eval()` stringifies via `to_string()`; `format_tool()` unwraps scheme strings via `as_string()`. `TeinSession::with_capture` uses flush-then-drain-run-flush to isolate each call's output (flush-output-port, R7RS, works in sandboxed contexts). Test helpers: use `extract_result_field(output)` (splits on `"\nstdout: "`) to isolate the result value.
- `(harness docs)` is the canonical import for harness API discovery: `(import (harness docs))` then `(describe hooks-docs)` to list all hook points with payload/return contracts, `(module-doc hooks-docs 'pre_message)` for a specific hook, or `(describe harness-tools-docs)` for the harness tool API (`define-tool`, `call-tool`, `register-hook`, etc.). Both `hooks-docs` and `harness-tools-docs` are also available as top-level bindings (pre-imported in `EVAL_PRELUDE`) but `(harness docs)` is the documented access path. `describe` takes an alist directly — NOT a symbol.
- `hooks-docs` is generated at startup from `HOOK_METADATA` (`hooks.rs`) — the single source of truth for all hook contracts. `docs/hooks.md` hook reference is also generated from it via `just generate-docs`. Adding a `HookPoint` variant without a `HOOK_METADATA` entry fails `test_hook_metadata_completeness`.
- `(module-exports '(harness docs))` (and `'(harness tools)`, `'(harness hooks)`) errors — runtime-registered modules are absent from tein's build-time `MODULE_EXPORTS` table. Use `harness-tools-docs` and `hooks-docs` for API discovery instead.
- `insert_symbols` (`indexer.rs`) now does a two-pass insert for parent resolution: first pass inserts all symbols with `parent_id = NULL`, second pass resolves `parent` names via line-range containment (smallest enclosing range wins). Plugins that don't emit `parent` are unaffected.
- Language plugins (e.g. `lang_rust`): `tree-sitter-rust` exposes visibility as a `visibility_modifier` child kind, not a named field — `child_by_field_name("visibility")` returns `None`. Use `node.children().find(|n| n.kind() == "visibility_modifier")` instead. Also, `use_wildcard` nodes contain the full path text (e.g. `"std::collections::*"`), not just `"*"` — take the full node text rather than constructing `prefix + "::*"`.
