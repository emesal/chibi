# Codebase Review — 2026-02-25

Full codebase review of chibi at commit `bae75691` (dev branch).

## Critical

- [x] **#1 byte-boundary panics on multi-byte UTF-8** *(fixed: `floor_char_boundary`)*
  `api/compact.rs:134` — `&content[..500]` panics on non-ASCII
  `api/send.rs:2104` — `&continue_prompt[..77]` panics on non-ASCII

## Important

- [x] **#2 flow-control entries: `is_context_entry` vs `entries_to_messages` contradiction** *(fixed: updated stale doc comments)*
  The code was correct — flow-control entries are stored in context.jsonl but excluded from API messages by `entries_to_messages()`. The doc comment at `context.rs:176` was stale.

- [x] **#3 `save_and_register_context` stale in-memory state** *(documented: by-design)*
  `AppState` takes `&self` — in-memory mutation is impossible without interior mutability. Disk is the source of truth. Added doc comment explaining the design.

- [ ] **#4 dual request-building paths** *(deferred — architectural refactor, needs own session)*
  `api/request.rs` (`build_request_body`) and `gateway.rs` (`to_chat_options`) both convert config to API params with nearly identical logic. New params must be added in two places. Risk of drift between logged request and actual request.

- [x] **#5 `compact_context_with_llm` incorrect doc comment** *(fixed)*
  Doc now correctly describes rolling compaction delegation.

- [x] **#6 `execute_tool_pure` duplication** *(fixed: `unwrap_tool_dispatch` helper)*
  Extracted helper for `Option<io::Result<String>>` → `String` dispatch pattern. Eliminated ~8 repetitions.

- [x] **#7 `docs/plugins.md` vs `docs/hooks.md` disagreement** *(fixed both)*
  Updated plugins.md to use correct field names. Also fixed hooks.md — `on_start` does receive payload (`chibi_home`, `project_root`, `tool_count`), not empty `{}`.

- [x] **#8 `PRESET_DESCRIPTION_PLACEHOLDER` in agent tool def** *(fixed)*
  Replaced with real description.

- [x] **#9 `builtin_summary_params` missing VFS tools** *(fixed)*
  Added `vfs_tools::VFS_TOOL_DEFS` to the chain.

- [x] **#10 no timeout on hook/plugin execution** *(fixed: 30s timeout)*
  Added `wait_with_timeout` utility in tools/mod.rs. Both hooks.rs and plugins.rs now use it. Timeout kills the child process via PID.

- [x] **#11 `PostIndexFile` hook receives cumulative stats** *(fixed: per-file deltas)*
  Snapshot cumulative counts before each file, compute delta for the hook payload.

- [x] **#12 stale/duplicated doc comment** *(fixed)*
  Removed partial TODO and duplicate doc comment.

- [x] **#13 `InspectConfigList` hardcoded items duplicated** *(fixed: `INSPECTABLE_ITEMS` constant)*
  Moved to `execution.rs`, exported via `chibi_core::INSPECTABLE_ITEMS`. Both binaries reference the constant.

## Suggestions

- [x] **#14** `AppState` missing doc comment (`state/mod.rs:39`) *(fixed)*
- [x] **#15** context names `--`/`---` valid but problematic for CLI *(fixed: reject names starting with `-`; updated tests)*
- [x] **#16** most `Config` fields lack `#[doc]` comments *(fixed: all fields in `Config` and `LocalConfig` documented)*
- [x] **#17** `set_field("api.stop")` silently stores to `extra` map instead of parsing *(fixed: added match arm, parses comma-separated tokens)*
- [x] **#18** `ResolvedConfig` doesn't derive `Serialize` *(fixed)*
- [x] **#19** `has_tool_calls` field possibly redundant with `!tool_calls.is_empty()` *(fixed: removed field)*
- [x] **#20** `compact_context_by_name` redundant condition `message_count == 0 || message_count <= 2` *(fixed: simplified to `<= 2`)*
- [x] **#21** `read_jsonl_file` uses `eprintln!` in a library crate *(documented: project-wide pattern, migrate when logging crate adopted)*
- [x] **#22** `execute_tool` re-derives project root from env instead of `self.project_root` *(fixed)*
- [x] **#23** `create_test_script` duplicated in hooks.rs and plugins.rs tests *(fixed: shared `test_helpers` module in `tools/mod.rs`)*
- [x] **#24** MCP spawn-mutex can be orphaned on crash *(fixed: stale check on mutex file age > 60s)*
- [x] **#25** `BoxFuture` type alias duplicated in backend.rs and local.rs *(fixed: `pub(super)` in backend.rs, import in local.rs)*
- [x] **#26** `check_write` exact match vs `is_reserved_caller_name` case-insensitive — intentional but undocumented *(fixed: doc comment explains the asymmetry and security rationale)*
- [x] **#27** `touch_lockfile` and `SummaryCache::save` not atomic, inconsistent with project patterns *(fixed: `atomic_write` helper in mcp-bridge)*
