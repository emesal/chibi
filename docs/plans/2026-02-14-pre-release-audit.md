# Pre-0.8.0 Release Audit

> codebase review covering all 3 crates, focused on the chibi-json extraction and composable agent milestone. docs excluded (reviewed separately in #145).

---

## Critical — fix before release

### 1. [DONE] `retrieve_content` bypasses `file_tools_allowed_paths` security boundary

`agent_tools.rs:154-169` — `read_file()` does raw `std::fs::read_to_string` with only tilde expansion, no path validation. the file tools have `resolve_and_validate_path` checking against `file_tools_allowed_paths`. an LLM can use `retrieve_content` with `source: "/etc/shadow"` to circumvent the allowlist entirely. same issue for URL fetching (no SSRF protection against localhost/metadata endpoints).

**fix:** reuse `resolve_and_validate_path` from `file_tools.rs` (extract to shared utility), add URL validation rejecting private/link-local/localhost ranges.

### 2. [DONE] Duplicate tool name arrays in `send.rs` are out of sync

`send.rs:58-87` — four `const` arrays (`BUILTIN_TOOL_NAMES`, `FILE_TOOL_NAMES`, `CODING_TOOL_NAMES`, `AGENT_TOOL_NAMES`) duplicate what `is_*_tool()` functions already know. they're already wrong:
- `fetch_url` missing from `CODING_TOOL_NAMES`
- `write_file` missing from `FILE_TOOL_NAMES`
- several builtins missing from `BUILTIN_TOOL_NAMES`

this breaks `classify_tool_type()` for hook data and `exclude_categories` filtering.

**fix:** replace the arrays with calls to the authoritative `is_*_tool()` functions (single source of truth).

### 3. [DONE] chibi-json missing `context_window_limit` resolution

chibi-cli resolves `context_window_limit` from ratatoskr's model registry when unset (0). chibi-json doesn't do this at all. any command going through the agentic loop operates with `context_window_limit = 0`, causing incorrect compaction behaviour.

**fix:** add `resolve_context_window` after `resolve_config` in chibi-json, ideally via shared helper to avoid repeating it in every code path.

---

## Important — should fix for release

### 4. [DONE] No concurrency cap on agent spawning

`agent_tools.rs` — sub-agents run with `parallel: true`, so a single agentic round can fire up to 100 concurrent `spawn_agent` calls via `join_all`. no depth limit or concurrency cap exists.

**options:** (a) mark agent tools as non-parallel in `ToolMetadata`, (b) add per-round concurrency cap for agent tools, (c) add `max_concurrent_agents` config field. option (a) is simplest for pre-alpha.

### 5. [DONE] `ArchiveHistory` skips hooks in chibi-json

`chibi-json/main.rs:190` calls `chibi.app.clear_context()` (raw), bypassing `pre_clear`/`post_clear` hooks. chibi-cli correctly calls `chibi.clear_context()`.

**fix:** use `chibi.clear_context(ctx_name)` in chibi-json.

### 6. [DONE] `Chibi::execute_tool()` missing agent tools and coding tools

`chibi.rs:317-349` — tries builtin, file, plugins but skips agent tools and coding tools entirely. library consumers calling `chibi.execute_tool("shell_exec", ...)` get "Tool not found".

**fix:** add agent tool and coding tool dispatch branches.

### 7. [DONE] Stale json-mode references in CLI sink docs

`chibi-cli/sink.rs:3-4` — module doc says it handles "JSON mode", line 18 says it "emits transcript entries in JSON mode". both stale post-split.

### 8. [DONE — fixed in #145] Stale hook count comment

`hooks.rs:122` — says "All 26 hook points" but there are 29. stale since `PreSpawnAgent`, `PostSpawnAgent`, `PostIndexFile` were added.

### 9. [DONE] Stale function name reference in `send.rs`

`send.rs:476` — comment references `send_prompt_with_depth` which no longer exists (now `send_prompt_loop`).

### 10. [DONE] `warn_threshold_percent: 0.8` in agent_tools test helper

`agent_tools.rs:541` — should be `80.0` (percentage scale). latent bug if any future test relies on it.

### 11. [DONE] Verbose diagnostics silently dropped in `JsonResponseSink`

`chibi-json/sink.rs:41` — `verbose_only` events are discarded; the sink has no access to the verbose flag.

**fix:** pass verbose flag into `JsonResponseSink::new()`, or always emit verbose diagnostics in JSON mode (programmatic consumers can filter).

### 12. [DONE] Duplicated "resolve config + build sink + send" pattern in CLI

`chibi-cli/main.rs` — ~20 lines repeated 4 times across `SendPrompt`, `CallTool` (agent), `CheckInbox`, `CheckAllInboxes`. extract into a helper.

---

## Dead code — cleanup

| # | what | where |
|---|------|-------|
| 13 | `ExecutionRequest` — entirely unused, never adopted by either binary | `execution.rs` (whole file) |
| 14 | `Flags` type alias — backward-compat shim in pre-alpha | `input.rs:176-181` |
| 15 | `Chibi::list_context_entries()` — zero callers | `chibi.rs:364-367` |
| 16 | 13 unused accessor methods on CLI `ResolvedConfig` | `chibi-cli/config.rs:328-375` |
| 17 | `pub use` re-exports in CLI `main.rs` — binary crate, unreachable | `chibi-cli/main.rs:14-22` |
| 18 | over-exported internals from `api/mod.rs` | `api/mod.rs:16-18` |
| 19 | `accumulated_text` in `JsonResponseSink` — accumulated then cleared, never read | `chibi-json/sink.rs:8-9` |
| 20 | `send_prompt()` — trivial passthrough wrapper to `send_prompt_loop()` | `send.rs:448-473` |
| 21 | stale refactor note in `inbox.rs:112` | `inbox.rs:112` |
| 22 | `Session::is_implied()` / `is_previous()` — only used in own tests | `chibi-cli/session.rs:84-87, 103-106` |

---

## Design notes (non-blocking, future consideration)

- `is_json_mode()` on `OutputSink` trait is architecturally questionable post-split — CLI always returns false, JSON always returns true. consider removing and using the type system instead.
- `DebugKey::Md` and `DebugKey::ForceMarkdown` are CLI-only concerns living in core's `input.rs`.
- `project_root` is re-derived from env var 3 times in `send.rs` instead of being threaded through.
- `reqwest::Client` created per `fetch_url` call in agent_tools — fine for now, wasteful if batched.
- error output from chibi-json is plain text, not structured JSON — programmatic consumers can't parse failures.
- `off-by-one in auto-cleanup diagnostic` — `chibi-cli/main.rs:1029` adds 1 to `tool_cache_max_age_days` in the diagnostic message but the `CleanupCache` handler at line 783 doesn't. either a bug or undocumented `>` vs `>=` semantics.
- `confirm_action` is a free function only called from `OutputHandler::confirm` — could be inlined as a method.
- test coverage gaps in chibi-json (only 5 integration tests, no JSONL format validation).
- test coverage thin for `Chibi` struct methods (`execute_tool`, `clear_context`, `init`, `shutdown`).
