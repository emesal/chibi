# unwired features audit

**date:** 2026-02-11
**branch:** `bugfix/unwired-features-2602`
**status:** in progress (issues 2, 4, 5, 6 done)

## overview

systematic audit found features that aren't fully wired through the stack. this was prompted by the discovery that reasoning/thinking content wasn't being passed through properly (fixed in `cde1ef9`). the issues fall into three patterns:

1. **dead or incomplete config fields** -- parsed, stored, but never reach the API or have no effect
2. **config bypass** -- code reads `app.config` (global) instead of `resolved_config`, silently ignoring per-context overrides
3. **lossy pipeline** -- data exists in storage but is dropped during conversion

---

## issue 1: `api.prompt_caching` is a dead config field

**severity:** critical (config has zero effect)
**files:** `config.rs:207`, `gateway.rs:70-101`, `request.rs:48-155`

`ApiParams.prompt_caching` is defined with default `Some(true)`, participates in merging, and is inspectable via `-n api.prompt_caching`. but it is never read by `build_request_body()` or `to_chat_options()`. the field has zero effect on actual API requests.

### fix

decide: wire it through to ratatoskr (if ratatoskr supports a prompt caching option), or remove it entirely. if it's openrouter-specific and ratatoskr handles it internally, remove from chibi's config.

---

## issue 2: `entries_to_messages` drops tool call/result history on reload ✓

**severity:** critical (data loss on restart)
**files:** `state/mod.rs`, `context.rs`, `entries.rs`, `send.rs`, `compact.rs`, `context_ops.rs`, `lib.rs`
**status:** done (516b8a0)

`entries_to_messages()` filtered to `entry_type == "message"` only. tool_call and tool_result entries were stored in context.jsonl but silently discarded when rebuilding `context.messages`.

### fix

- replaced `Context.messages: Vec<Message>` with `Vec<serde_json::Value>` so tool interactions flow through natively
- rewrote `entries_to_messages()` to group consecutive tool entries into assistant+tool_calls JSON with tool result messages
- added `tool_call_id: Option<String>` to `TranscriptEntry` for correlating tool_call/tool_result pairs
- fixed write order in `process_tool_calls` (all tool_call entries before tool_result entries, matching API format)
- updated compaction to handle tool exchanges atomically (assistant + tool results dropped together)
- backward compatible: old entries without `tool_call_id` get synthetic IDs on reconstruction

---

## issue 3: `pre_cache_output` / `post_cache_output` hooks never fired

**severity:** critical (documented feature doesn't work)
**files:** `hooks.rs:33-34`, `send.rs:976-1009`

these hook points are defined in the `HookPoint` enum, documented in AGENTS.md, but never invoked. the caching code calls `cache::cache_output()` directly without firing either hook.

### fix

add `execute_hook(tools, HookPoint::PreCacheOutput, &hook_data)` and `PostCacheOutput` calls around the caching logic in `execute_tool_pure()`. pass tool name, result size, cache_id, and the truncated preview as hook data.

---

## issue 4: reasoning content silently dropped in `--json-output` mode ✓

**severity:** critical (silent data loss for programmatic consumers)
**files:** `send.rs:597-601`
**status:** done (91317bf)

when `json_mode` is true, reasoning chunks are silently discarded -- neither forwarded to the sink nor accumulated anywhere. JSON consumers have no access to reasoning content.

### fix

remove the `if !json_mode` guard. always forward reasoning to the sink. the sink implementation (JSON or terminal) can decide how to handle it.

---

## issue 5: `reflection_enabled` reads global config, not resolved config ✓

**severity:** medium (per-context overrides silently ignored)
**files:** `main.rs:639,727,807,882`
**status:** done (803cea6)

```rust
let use_reflection = chibi.app.config.reflection_enabled;
```

should be:

```rust
let use_reflection = resolved.core.reflection_enabled;
```

### fix

four instances in main.rs. replace `chibi.app.config.reflection_enabled` with the resolved config value. the resolved config already properly merges local.toml overrides.

---

## issue 6: fuel model refactor (was `max_recursion_depth` + `max_empty_responses`) ✓

**severity:** medium (per-context overrides silently ignored + conceptual misfit)
**files:** `send.rs`, `config.rs`, `config_resolution.rs`, `gateway.rs`, `agent_tools.rs`, `tests.rs`, `cli/config.rs`, `docs/configuration.md`, `docs/hooks.md`
**status:** done

### current state (prior to fix)

`max_recursion_depth` (default: 30) counts recursive `send_prompt_with_depth` calls. `max_empty_responses` (default: 2) counts consecutive empty LLM responses. both read from `app.config` instead of `resolved_config`, silently ignoring per-context overrides. the "recursion depth" metaphor doesn't match how the agentic loop actually works -- it's not recursion, it's a fuel budget.

the current `recursion_depth` counter increments when:
- the handoff target is `Agent` (i.e., the LLM called `call_agent` or the fallback tool triggers continuation)

but it does NOT account for:
- number of tool calls executed (a single "depth" level can process unlimited tool calls)
- sub-agent spawns (`spawn_agent`, `retrieve_content`)
- consecutive empty responses (tracked separately with its own counter and config)

### structural problems found

1. **inner tool-call loop is unbounded** -- the LLM can do infinite tool-call rounds within a single depth level as long as it keeps calling tools. `max_recursion_depth` only checks when the LLM produces a final text response and wants to re-engage via `call_agent`. a runaway tool-calling LLM is invisible to the depth limit.

2. **depth check happens too late** -- the check is in `handle_final_response` (after the LLM has already responded with text and the handoff says Agent). all tool calls at depth N execute without any depth check. the LLM always gets one more full turn than the limit suggests.

3. **two separate limiting mechanisms** -- `max_recursion_depth` limits recursive calls, `max_empty_responses` limits consecutive empty responses, and the inner tool loop has no limit. three different axes with inconsistent coverage.

4. **`consecutive_empty_responses` resets per depth** -- it's a local variable in `send_prompt_with_depth`, so each recursive call gets a fresh counter. empty responses can't accumulate across depth levels.

5. **`app.config` instead of `resolved_config`** -- both `max_recursion_depth` and `max_empty_responses` read from global config, bypassing per-context overrides that are properly resolved in `config_resolution.rs`.

### proposed design: fuel model

rename `max_recursion_depth` to `fuel` throughout config, code, comments, and docs. the agent starts with a configured amount of fuel, and different actions consume fuel. this unifies the three separate limiting mechanisms into one budget.

```toml
# config.toml / local.toml
fuel = 30                      # total fuel budget for agentic execution
fuel_empty_response_cost = 15  # cost of an empty response (strong "something is wrong" signal)
```

fuel consumption:
- **tool-call round** (LLM responds with tool calls, they execute, loop continues): 1 fuel per round
- **agent continuation** (call_agent / fallback → agent): 1 fuel
- **empty response**: `fuel_empty_response_cost` fuel (default 15 -- an empty response is a strong signal something is wrong, so it costs a lot)
- **sub-agent spawn**: 0 fuel (sub-agents don't consume the parent's fuel budget -- they're self-contained non-streaming calls with no tool loop)

fuel is **not** consumed by:
- `call_user` (this ends the loop, not continues it)
- the initial API call (the first turn is "free" -- you asked for it)

### architectural change: flatten the loop

currently the code has two nesting levels: an inner `loop` for tool-call rounds, and outer recursion via `send_prompt_with_depth(depth+1)` for agent continuations. this makes the fuel model awkward to implement since the counter would need to be threaded through both layers.

**proposed:** replace the recursive `send_prompt_with_depth` call with iteration. the function becomes a single loop that handles both tool-call rounds and agent continuations, with fuel as a mutable counter. this eliminates the recursion entirely and makes fuel tracking trivial.

```
fuel = resolved_config.fuel
loop {
    send to LLM (streaming)
    |
    |-- tool_calls? -> process_tool_calls()
    |       fuel -= 1
    |       if fuel <= 0: return to user
    |       continue loop
    |
    |-- empty text? -> fuel -= fuel_empty_response_cost
    |       if fuel <= 0: return to user
    |       continue loop
    |
    |-- text response -> handle_final_response()
            save assistant msg, post_message hook
            handoff.take()
            |-- User? -> return to user
            |-- Agent? -> fuel -= 1
                          if fuel <= 0: return to user
                          rebuild prompt, continue loop
}
```

this is simpler, more predictable, and makes the fuel budget visible at every decision point.

hooks can modify fuel:
- `pre_agentic_loop` hook result can include `"fuel": <number>` to set initial fuel
- `post_tool_batch` hook result can include `"fuel_delta": <number>` to add or subtract fuel

when fuel reaches 0, control is handed back to the user with a diagnostic message.

### diagnostic messages

```
[fuel exhausted (0/30), returning control to user]
```

during agent continuations:

```
[continuing (fuel: 27/30): working on next step]
```

### migration

this is a pre-alpha breaking change. `max_recursion_depth` in config.toml becomes `fuel`. `max_empty_responses` is removed as a separate config field and replaced by `fuel_empty_response_cost`.

### files to change

- `config.rs` -- rename fields, update `ConfigDefaults`, update `ResolvedConfig`, update `get_field()`/`list_fields()`
- `config_resolution.rs` -- update merge logic
- `send.rs` -- flatten the loop (remove recursion), implement fuel tracking, use `resolved_config` instead of `app.config`
- `request.rs` -- update `PromptOptions` if it carries depth info; remove `recursion_depth` parameter
- `main.rs` -- remove any direct `app.config.max_recursion_depth` references
- `gateway.rs` -- update `ResolvedConfig` in test helpers
- `hooks.rs` / hook data -- replace `recursion_depth` with `fuel` / `fuel_remaining` in hook payloads
- `docs/agentic.md` -- replace "Reasonable Recursion Limits" with fuel model docs
- `docs/configuration.md` -- update config reference
- `AGENTS.md` -- update if mentioned
- tests throughout

---

## issue 7: `frequency_penalty`, `presence_penalty`, `response_format` missing from gateway

**severity:** medium (config options don't reach the actual API)
**files:** `gateway.rs:70-101`

these are in `build_request_body()` (used only for logging/hooks) but NOT in `to_chat_options()` (the actual API path via ratatoskr). the actual API call may not include these parameters.

also: `reasoning.enabled` is defined in `ReasoningConfig` but not forwarded through `to_ratatoskr_reasoning()`.

### fix

check what `ratatoskr::ChatOptions` supports. for each field:
- if ratatoskr has a builder method: wire it through in `to_chat_options()`
- if ratatoskr doesn't support it: either add support in ratatoskr or document as unsupported

for `reasoning.enabled`: check if `RatatoskrReasoningConfig` has an `enabled` field. if not, the semantics should be: `enabled = true` without explicit effort implies `effort = Medium`.

---

## issue 8: config fields bypassing `ResolvedConfig`

**severity:** low-medium (not per-context overridable, not inspectable)
**files:** `config.rs`, various consumers

these fields exist in `Config` but not in `ResolvedConfig`:

| field | consumer | per-context useful? |
|-------|----------|-------------------|
| `reflection_character_limit` | `builtin.rs:343` | yes |
| `rolling_compact_drop_percentage` | `compact.rs:70` | maybe |
| `lock_heartbeat_seconds` | `main.rs`, `lock.rs` | no (global is fine) |
| `storage` (StorageConfig) | `state/mod.rs:569-593` | yes (already in LocalConfig but never resolved) |

### fix

for fields where per-context overrides make sense (`reflection_character_limit`, `storage`): add to `ResolvedConfig` and wire through config resolution.

for global-only fields (`lock_heartbeat_seconds`): leave as-is but document the design decision.

add `fallback_tool` to `get_field()` / `list_fields()` (it's in ResolvedConfig but not inspectable).

---

## issue 9: AGENTS.md says 29 hooks, actual count is 28

**severity:** low (documentation bug)
**files:** `AGENTS.md`, `hooks.rs`

### fix

count the actual hooks and update AGENTS.md.

---

## issue 10: `on_start` / `on_end` hooks fire with empty `{}` data

**severity:** low (limits plugin utility)
**files:** `chibi.rs:183-184,192-193`

### fix

populate hook data with useful context: context name (if available), project root, chibi home, tool count, etc.

---

## issue 11: `json_config` field on `Cli` struct is dead code

**severity:** low (cosmetic)
**files:** `cli.rs:293-295`

the field is parsed by clap but never read from the struct. it's consumed via raw arg scanning before clap runs. the clap definition exists only for `--help` and to prevent "unexpected argument" errors.

### fix

add a comment explaining why the field exists but is never read. or restructure to not need it.

---

## execution order

1. **issue 5** -- `reflection_enabled` resolved config fix ✓ (803cea6)
2. **issue 4** -- reasoning in JSON mode ✓ (91317bf)
3. **issue 6** -- fuel model refactor ✓
4. **issue 2** -- `entries_to_messages` reconstruction ✓ (516b8a0)
5. **issue 1** -- dead `prompt_caching` field (decide and act)
6. **issue 7** -- gateway param gaps (depends on ratatoskr audit)
7. **issue 3** -- unfired cache hooks
8. **issues 8-11** -- remaining cleanup

## out of scope

- image/multi-modal content support (future feature, not a wiring bug)
- text-only tool results (architectural, not a bug)
- non-streaming `chat()` dropping metadata (by design for compaction)
- reasoning content persistence in transcripts (intentionally ephemeral per current design, revisit separately)
