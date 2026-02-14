# Twelve-Factor Audit

> **Issue:** #130 — make chibi a twelve-factor app?
>
> Not every factor applies perfectly to a CLI tool, but the methodology provides useful guidance for keeping chibi clean, portable, and ops-friendly. This audit identifies which factors are already satisfied, which need work, and which don't apply.

## Summary

| Factor | Status | Action |
|--------|--------|--------|
| I. Codebase | satisfied | none |
| II. Dependencies | satisfied | none |
| III. Config | partially satisfied | file issue for env var overrides |
| IV. Backing Services | satisfied | none |
| V. Build/Release/Run | satisfied | none |
| VI. Processes | satisfied | none |
| VII. Port Binding | n/a | CLI tool, no ports |
| VIII. Concurrency | satisfied | none |
| IX. Disposability | satisfied | none |
| X. Dev/Prod Parity | satisfied | none |
| XI. Logs | satisfied | none |
| XII. Admin Processes | satisfied | none |

---

## I. Codebase — One codebase tracked in revision control, many deploys

**Status: satisfied**

Single git repo (`emesal/chibi`). Cargo workspace with two crates (`chibi-core`, `chibi-cli`). Multiple deployment paths from same codebase: `cargo install`, Nix flake, dev builds. No multi-repo issues.

---

## II. Dependencies — Explicitly declare and isolate dependencies

**Status: satisfied**

All dependencies explicit in `Cargo.toml` files with pinned versions. No wildcard versions. System-level deps (`pkg-config`, `openssl`) declared in `flake.nix`. `rusqlite` uses bundled SQLite (no system sqlite assumption). Git deps (`ratatoskr`, `streamdown-rs`) pinned to specific repos.

---

## III. Config — Store config in the environment

**Status: partially satisfied**

This is the main gap. Chibi is heavily file-based for configuration:

**What works:**
- `CHIBI_HOME` env var overrides the home directory
- `CHIBI_PROJECT_ROOT` env var overrides project root detection
- Layered config resolution: global → model → context → CLI flags

**What's missing:**
- **API key** (`api_key`) cannot be set via environment variable — must be in `config.toml` or `local.toml`. This is the primary twelve-factor violation. Secrets should be injectable via env vars for CI/CD, containers, and security best practices.
- **Model** (`model`) has no env var override.
- No general `CHIBI_*` env var convention for overriding arbitrary config fields.

**Current state:** `docs/configuration.md` explicitly documents: "Chibi does not use environment variables for configuration. All settings come from the config files described above." This is an intentional design choice, not an oversight.

**Recommendation:** Add `CHIBI_API_KEY` and `CHIBI_MODEL` env var overrides. These two cover the most important cases: secret injection and model selection in automation. A general env-override scheme for all config fields would be over-engineering for a CLI tool — file-based config is appropriate for the vast majority of settings.

**Follow-up:** File issue for `CHIBI_API_KEY` and `CHIBI_MODEL` env var support.

---

## IV. Backing Services — Treat backing services as attached resources

**Status: satisfied**

- **Plugins:** Executable scripts in `~/.chibi/plugins/`, dynamically loaded by scanning the directory. No hardcoded plugin list. Symlink validation prevents path traversal.
- **SQLite (codebase index):** Per-project database at `<project-root>/.chibi/codebase.db`. WAL mode for concurrent access. Path-based binding — moving the project moves the DB.
- **LLM API:** Delegated to ratatoskr library. API endpoint configured via model selection (OpenRouter). No hardcoded service URLs in chibi itself.
- **Context storage:** File-based (`contexts/<name>/`), fully portable. Moving `~/.chibi` moves all state.

Everything is location-based and reconfigurable.

---

## V. Build, Release, Run — Strictly separate build and run stages

**Status: satisfied**

- **Build:** `cargo build` produces deterministic binary. `flake.nix` provides fully reproducible builds.
- **Release:** `justfile` provides `just release <version>` for squash-merge, tag, and push. Version in `Cargo.toml` workspace.
- **Run:** Binary + config files. No runtime compilation, no environment-dependent code paths.

No `.FREEZE` file or feature flags that change compiled behavior between environments.

---

## VI. Processes — Execute the app as one or more stateless processes

**Status: satisfied**

Each chibi invocation is stateless: load config → read context files → execute → save results → exit. No in-memory state carried between runs. All persistent state lives in the filesystem:

- `context.jsonl` — append-only transcript
- `transcript/` — partitioned archive
- `state.json` — context metadata
- `local.toml` — per-context config

File locking (`lock.rs`) prevents concurrent writes to the same context. Heartbeat mechanism detects stale locks. Atomic writes via `safe_io` module prevent corruption.

The "share-nothing" model holds: each invocation reads state from files, does its work, writes results back.

---

## VII. Port Binding — Export services via port binding

**Status: n/a**

Chibi is a CLI tool. No network ports, no listening sockets. LLM API calls are outbound HTTP via ratatoskr. This factor doesn't apply.

---

## VIII. Concurrency — Scale out via the process model

**Status: satisfied**

- Multiple chibi instances can run simultaneously (different contexts or different projects)
- Per-context file locking prevents concurrent access to same context
- SQLite WAL mode allows concurrent readers
- Append-only JSONL avoids write conflicts
- `spawn_agent` feature runs child chibi processes (each with own context lock)

The process-per-context model is the natural concurrency unit.

---

## IX. Disposability — Maximize robustness with fast startup and graceful shutdown

**Status: satisfied**

- **Fast startup:** Load TOML files (text), open SQLite (fast), done. No daemon, no warm-up.
- **Graceful shutdown:** Rust `Drop` trait on `ContextLock` releases file lock and stops heartbeat thread. Atomic writes ensure no half-written files.
- **Crash recovery:** Stale lock detection (1.5× heartbeat interval, max 3 retries). JSONL append-only means partial writes lose at most one entry. WAL recovery is automatic for SQLite.

No explicit signal handlers needed — the OS + Rust Drop semantics handle cleanup.

---

## X. Dev/Prod Parity — Keep development, staging, and production as similar as possible

**Status: satisfied**

- No feature flags, no `#[cfg(not(test))]` in production code
- Same binary runs everywhere
- Behavior differences controlled via CLI flags (`--trust`, `--verbose`, `--raw`), not environment detection
- `#[cfg(test)]` only used for test helpers (e.g., `AppState::from_dir`)

---

## XI. Logs — Treat logs as event streams

**Status: satisfied**

Already well-designed:

- **stdout:** LLM output only. Pipeable. JSON mode (`--json-output`) emits JSONL `TranscriptEntry` records.
- **stderr:** Diagnostics, warnings, errors. Controlled by `-v` flag.
- **`--raw`:** Disables markdown rendering for clean stdout piping.

No log files written. No log rotation. Output goes to stdout/stderr as event streams — exactly what twelve-factor prescribes.

---

## XII. Admin Processes — Run admin/management tasks as one-off processes

**Status: satisfied**

Admin operations are CLI commands, not separate daemons:

- Context management: create, delete, switch, list
- Cache cleanup: automatic on exit (`auto_cleanup_cache`), manual via clear
- Index management: `index_update`, `index_query`
- Compaction: automatic or manual
- Lock cleanup: automatic stale detection

All admin tasks use the same codebase and config as the main app. No separate admin tooling.

---

## Follow-up Issues

### 1. `CHIBI_API_KEY` and `CHIBI_MODEL` env var overrides

**Priority:** Medium. The main twelve-factor gap.

**Scope:** Add env var override support for `api_key` and `model` in config resolution. Resolution order becomes:
1. CLI flag (if applicable)
2. Context `local.toml`
3. Environment variable (`CHIBI_API_KEY`, `CHIBI_MODEL`)
4. Global `config.toml`
5. Default

This enables:
- Secret injection in CI/CD without config files
- Container deployments with env-based config
- Quick model switching: `CHIBI_MODEL=openai/o3 chibi "solve this"`

**Not recommended:** A general `CHIBI_*` env convention for all config fields. File-based config is appropriate for most settings. Over-generalizing adds complexity without clear benefit for a CLI tool.
