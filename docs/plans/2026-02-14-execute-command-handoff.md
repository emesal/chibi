# execute_command() extraction — handoff

> **Branch:** `bugfix/pre-0-8-0-review-2602`
> **Plan:** `docs/plans/2026-02-14-execute-command-extraction-plan.md`
> **Design:** `docs/plans/2026-02-14-execute-command-extraction-design.md`
> **Issue:** #143

## completed (tasks 1–4)

all four core implementation tasks build cleanly with `cargo build --workspace`.

### task 1: `emit_markdown()` on `OutputSink`
- added `emit_markdown(&self, content: &str) -> io::Result<()>` with default impl to trait in `chibi-core/src/output.rs`
- explicit impls in `chibi-cli/src/output.rs` and `chibi-json/src/output.rs` (both fall back to `emit_result` for now)
- CLI's impl will be upgraded in task 5 to do real streamdown rendering

### task 2: `execution.rs` skeleton
- created `chibi-core/src/execution.rs` with `CommandEffect` enum and module registration in `lib.rs`
- re-exports: `pub use execution::{CommandEffect, execute_command}`

### task 3: lifecycle + non-send commands
- `execute_command()` handles full lifecycle: init → auto-destroy → ensure context → touch → dispatch → shutdown → cache cleanup
- `dispatch_command()` handles all non-send `Command` variants
- `show_log()` helper uses `emit_markdown()` for message content, `emit_entry()` for JSON mode
- `inspect_context()` helper uses `emit_markdown()` for todos/goals

### task 4: send-path commands
- `send_prompt_inner()` helper: config clone, context lock, `PromptOptions` construction, delegates to `send_prompt_streaming`
- `SendPrompt`, `CallTool` (with optional agent continuation via `force_call_agent`), `CheckInbox`, `CheckAllInboxes`

**key design decision:** `execute_command` and helpers are generic `<S: ResponseSink>` (not `dyn`), because `send_prompt_streaming` and 9 internal functions in `send.rs` all use `<S: ResponseSink>` and adding `?Sized` everywhere would be noisy. the generic approach is zero-impact on existing code.

## remaining (tasks 5–7)

### task 5: wire chibi-cli — the big one

**what to replace in `crates/chibi-cli/src/main.rs`:**

the `execute_from_input()` function (line 443) currently does:
1. build `CliSendOptions` (lines 454–462) — **remove**
2. `chibi.init()` (line 465) — **remove** (core does this)
3. auto-destroy + session fallback (lines 468–481) — **remove lifecycle part** (core does this), but **keep session fallback** as a post-execute check
4. context selection (lines 488–520) — **keep** (CLI-specific)
5. context touch (lines 523–551) — **remove** (core does this)
6. username handling (lines 553–572) — **keep** (CLI-specific)
7. `match &input.command` block (lines 574–927) — **replace with core call**
8. shutdown + cache cleanup (lines 929–948) — **remove** (core does this)
9. image cache cleanup (lines 950–972) — **keep** (CLI-specific)
10. no-action check (lines 974–980) — **keep**

**new flow for `execute_from_input()`:**
```
context resolution (CLI) → username handling (CLI) → intercept ShowHelp/ShowVersion →
resolve core config → build CliResponseSink → execute_command() →
handle CommandEffect (session updates) → image cache cleanup → no-action check
```

**session concerns post-auto-destroy:**
core's `execute_command` runs `auto_destroy_expired_contexts` but doesn't return the destroyed list. CLI needs to know if its session context was destroyed. two approaches:
- (a) after `execute_command` returns, check if `session.implied_context` dir still exists; if not, reset to "default"
- (b) add destroyed-list info to `CommandEffect` (new variant or extend existing)

recommend (a) — it's simpler and robust against any destruction cause.

**session concerns post-`ContextDestroyed`:**
CLI currently does `session.handle_context_destroyed()` inline in the destroy handler. with core returning `CommandEffect::ContextDestroyed`, CLI handles it after the call. the plan's pseudocode shows this correctly.

**`CliResponseSink` construction:**
currently `send_with_cli_sink` builds the sink. after extraction, CLI needs to build the sink *before* calling `execute_command`. this means CLI builds the `CliResponseSink` with markdown config upfront, then passes it in. the sink is constructed from `resolve_cli_config` + presentation flags.

**functions to remove:**
- `CliSendOptions` struct (line 191)
- `send_with_cli_sink()` (line 206)
- `inspect_context()` (line 262)
- `show_log()` (line 338)
- `set_prompt_for_context()` (line 424)

**functions to keep:**
- `build_interactive_permission_handler()`, `build_trust_permission_handler()`, `select_permission_handler()`
- `render_markdown_output()`, `md_config_from_resolved()`, `md_config_defaults()`
- `generate_new_context_name()`, `resolve_context_name()`
- `resolve_cli_config()`
- `extract_home_override()`, `extract_project_root_override()`
- `main()`

**import cleanup needed:**
- remove: `ENTRY_TYPE_MESSAGE`, `ENTRY_TYPE_TOOL_CALL`, `ENTRY_TYPE_TOOL_RESULT`, `now_timestamp`, `InspectableExt`
- remove: `PromptOptions`, `api`, `tools` (no longer used directly)
- add: `chibi_core::CommandEffect`

### task 6: wire chibi-json

simpler than CLI — `chibi-json/src/main.rs` currently:
1. `main()` does lifecycle (init, auto-destroy, touch) then calls `execute_json_command()`
2. `execute_json_command()` does all dispatch (~380 lines)

**new flow:**
- `main()` keeps: arg parsing, chibi load, config flag overrides
- replace lifecycle + `execute_json_command()` call with single `execute_command()` call
- delete entire `execute_json_command()` function
- no session concerns (JSON mode is stateless)

**import cleanup:**
- remove: `Context`, `ContextEntry`, `now_timestamp`, `DebugKey`, `Inspectable`
- remove: `PromptOptions`, `api`, `tools`

### task 7: verify and clean up

- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- verify dead code removal (grep for removed function names)
- update AGENTS.md architecture section: add `execution.rs` to chibi-core listing
- `just pre-push` before pushing

## files summary

| file | status | notes |
|------|--------|-------|
| `chibi-core/src/output.rs` | modified | `emit_markdown()` added to trait |
| `chibi-core/src/execution.rs` | **new** | ~628 lines, complete |
| `chibi-core/src/lib.rs` | modified | module + re-exports added |
| `chibi-cli/src/output.rs` | modified | explicit `emit_markdown` impl |
| `chibi-json/src/output.rs` | modified | explicit `emit_markdown` impl |
| `chibi-cli/src/main.rs` | **pending** | task 5 — replace dispatch |
| `chibi-json/src/main.rs` | **pending** | task 6 — replace dispatch |
| `AGENTS.md` | **pending** | task 7 — add execution.rs |
