# Codebase Review — 2026-02-25

Full codebase review of chibi at commit `bae75691` (dev branch).

## Critical

- [ ] **#1 byte-boundary panics on multi-byte UTF-8**
  `api/compact.rs:134` — `&content[..500]` panics on non-ASCII
  `api/send.rs:2104` — `&continue_prompt[..77]` panics on non-ASCII
  Fix: use `floor_char_boundary` or the `char_indices().nth()` pattern already used in manual compaction at `compact.rs:460-464`.

## Important

- [ ] **#2 flow-control entries: `is_context_entry` vs `entries_to_messages` contradiction**
  `state/mod.rs` — `is_context_entry` returns true for flow-control entries, but `entries_to_messages` silently skips them. Doc comment at `context.rs:178` says they should be suppressed from context.jsonl. Either exclude from `is_context_entry` or handle in `entries_to_messages`.

- [ ] **#3 `save_and_register_context` stale in-memory state**
  `state/context_ops.rs:27-51` — writes new context to disk but never updates `self.state.contexts` in memory. Subsequent calls see stale state until `sync_state_with_filesystem()`.

- [ ] **#4 dual request-building paths**
  `api/request.rs` (`build_request_body`) and `gateway.rs` (`to_chat_options`) both convert config to API params with nearly identical logic. New params must be added in two places. Risk of drift between logged request and actual request.

- [ ] **#5 `compact_context_with_llm` incorrect doc comment**
  `api/compact.rs:351-359` — doc says "full compaction: summarizes all messages and starts fresh" but it delegates to `rolling_compact`. Incorrect per project standards.

- [ ] **#6 `execute_tool_pure` duplication**
  `api/send.rs:795-1307` — 400-line function with ~8x duplicated `match` dispatch pattern (`Some(Ok(r))/Some(Err(e))/None`). Extract a helper.

- [ ] **#7 `docs/plugins.md` vs `docs/hooks.md` disagreement**
  plugins.md says `pre_api_tools` returns `{"remove": [...]}`, hooks.md says `{"exclude": [...]}`. plugins.md says `pre_agentic_loop` returns `{"handoff": ...}`, hooks.md says `{"fallback": ...}`. hooks.md is authoritative; plugins.md needs updating.

- [ ] **#8 `PRESET_DESCRIPTION_PLACEHOLDER` in agent tool def**
  `tools/agent_tools.rs:67` — placeholder description visible if schema accessed directly (bypassing `all_agent_tools_to_api_format`). Replace with a real description.

- [ ] **#9 `builtin_summary_params` missing VFS tools**
  `tools/builtin.rs:238-247` — chains 4 registries but skips `vfs_tools::VFS_TOOL_DEFS`. Tool call summaries produce `None` for VFS tools.

- [ ] **#10 no timeout on hook/plugin execution**
  `tools/hooks.rs:52-118` and `tools/plugins.rs:241-279` — `wait_with_output()` with no timeout. A hung plugin freezes the entire application. Every other process execution in the codebase has timeout protection.

- [ ] **#11 `PostIndexFile` hook receives cumulative stats**
  `index/indexer.rs:252-258` — `stats.symbols_added` and `stats.refs_added` are cumulative across all files, not per-file counts. Hook consumers get inflating numbers.

- [ ] **#12 stale/duplicated doc comment**
  `chibi-cli/src/config.rs:281-288` — `ResolvedConfig::get_field` has a partial TODO that cuts off mid-sentence, followed by a replacement doc comment pasted below instead of replacing the original.

- [ ] **#13 `InspectConfigList` hardcoded items duplicated**
  CLI (`main.rs:423-431`) and JSON (`main.rs:113-121`) both hardcode `["system_prompt", "reflection", "todos", "goals", "home"]`. Should be a constant in core.

## Suggestions

- [ ] **#14** `AppState` missing doc comment (`state/mod.rs:39`)
- [ ] **#15** context names `--`/`---` valid but problematic for CLI
- [ ] **#16** most `Config` fields lack `#[doc]` comments
- [ ] **#17** `set_field("api.stop")` silently stores to `extra` map instead of parsing
- [ ] **#18** `ResolvedConfig` doesn't derive `Serialize`
- [ ] **#19** `has_tool_calls` field possibly redundant with `!tool_calls.is_empty()`
- [ ] **#20** `compact_context_by_name` redundant condition `message_count == 0 || message_count <= 2`
- [ ] **#21** `read_jsonl_file` uses `eprintln!` in a library crate
- [ ] **#22** `execute_tool` re-derives project root from env instead of `self.project_root`
- [ ] **#23** `create_test_script` duplicated in hooks.rs and plugins.rs tests
- [ ] **#24** MCP spawn-mutex can be orphaned on crash
- [ ] **#25** `BoxFuture` type alias duplicated in backend.rs and local.rs
- [ ] **#26** `check_write` exact match vs `is_reserved_caller_name` case-insensitive — intentional but undocumented
- [ ] **#27** `touch_lockfile` and `SummaryCache::save` not atomic, inconsistent with project patterns
