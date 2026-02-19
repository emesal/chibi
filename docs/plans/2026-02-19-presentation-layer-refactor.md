# Presentation Layer Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove `verbose`, `hide_tool_calls`, and `show_thinking` from
`chibi-core`'s config layer entirely, replacing stringly-typed/flag-gated
diagnostic emission with semantically typed events that clients filter
themselves.

**Architecture:** Core emits all events unconditionally, classified by semantic
variant. Both `ResponseSink` (streaming path) and `OutputSink` (command path)
gain typed event enums; the `verbose: bool` parameter disappears from both.
`CliConfig` (cli.toml layer) gains the three presentation fields; core config
loses them.

**Tech Stack:** Rust, serde, schemars. No new dependencies.

**Design doc:** `docs/plans/2026-02-19-presentation-layer-refactor-design.md`

---

## Task 1: Replace `ResponseEvent::Diagnostic` with typed variants

**Files:**
- Modify: `crates/chibi-core/src/api/sink.rs`

This is the core event type change. All consumers (`send.rs`, `CliResponseSink`,
`JsonResponseSink`, `CollectingSink`) will break at compile time after this —
that's intentional. Fix them in subsequent tasks.

**Step 1: Add `FuelEvent` enum and replace `Diagnostic` in `ResponseEvent`**

In `crates/chibi-core/src/api/sink.rs`, replace the `Diagnostic` variant and
add the new enum. The full replacement for the `ResponseEvent` enum:

```rust
/// Describes which fuel-related moment triggered a FuelStatus event.
#[derive(Debug, Clone)]
pub enum FuelEvent {
    EnteringTurn,
    AfterToolBatch,
    AfterContinuation { prompt_preview: String },
    EmptyResponse,
}

pub enum ResponseEvent<'a> {
    TextChunk(&'a str),
    Reasoning(&'a str),
    TranscriptEntry(TranscriptEntry),
    ToolStart { name: String, summary: Option<String> },
    ToolResult { name: String, result: String, cached: bool },
    Finished,
    Newline,
    StartResponse,
    // --- replaces Diagnostic ---
    /// Hook filter/modification/override debug info (verbose-tier in CLI)
    HookDebug { hook: String, message: String },
    /// Fuel budget status update (verbose-tier in CLI)
    FuelStatus { remaining: usize, total: usize, event: FuelEvent },
    /// Fuel budget exhausted — always shown in CLI
    FuelExhausted { total: usize },
    /// Context window nearing limit (verbose-tier in CLI)
    ContextWarning { tokens_remaining: usize },
    /// Per-tool diagnostic message (verbose-tier in CLI)
    ToolDiagnostic { tool: String, message: String },
    /// Inbox messages injected into prompt (verbose-tier in CLI)
    InboxInjected { count: usize },
}
```

Also remove the `CollectingSink` match arm for `Diagnostic` and add arms for
each new variant (most are no-ops for the collecting sink — only `TextChunk`
and `Reasoning` and `TranscriptEntry` need content). The existing
`CollectingSink` test for `Diagnostic` should be updated to use e.g.
`FuelExhausted`.

**Step 2: Verify compile errors (don't fix yet)**

```bash
cargo build 2>&1 | grep "^error" | head -30
```

Expected: many errors referencing `ResponseEvent::Diagnostic`. That's correct.

**Step 3: Commit the type definition**

```bash
git add crates/chibi-core/src/api/sink.rs
git commit -m "refactor(sink): replace Diagnostic with typed ResponseEvent variants"
```

---

## Task 2: Update `send.rs` emit sites

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs`

24 `ResponseEvent::Diagnostic` sites, all in this file. Also remove all `if
verbose {` guards around sink emissions — emit unconditionally.

The `verbose` local variable (derived from `options.verbose`) is used as a
guard in many places. After removing those guards, `verbose` will be unused in
`send.rs`. Remove it.

**Mapping of old sites to new variants:**

| Old message pattern | New variant |
|---|---|
| `[Hook pre_api_tools: {} include/exclude filter: ...]` | `HookDebug { hook, message }` |
| `[Hook pre_api_request: {} modifying request ...]` | `HookDebug { hook, message }` |
| `[Hook {} set fallback to {}]` | `HookDebug { hook, message }` |
| `[Hook {} set fuel to {}]` | `HookDebug { hook, message }` |
| `[Hook {} adjusted fuel by {}]` | `HookDebug { hook, message }` |
| `[fuel: {}/{} entering turn]` | `FuelStatus { remaining, total, event: FuelEvent::EnteringTurn }` |
| `[fuel: {}/{} after tool batch]` | `FuelStatus { remaining, total, event: FuelEvent::AfterToolBatch }` |
| `[continuing (fuel: {}/{}): {}]` | `FuelStatus { remaining, total, event: FuelEvent::AfterContinuation { prompt_preview } }` |
| `[empty response, fuel: {}/{}]` | `FuelStatus { remaining, total, event: FuelEvent::EmptyResponse }` |
| `[fuel exhausted (0/{}), returning control to user]` | `FuelExhausted { total }` |
| `[Context window warning: {} tokens remaining]` | `ContextWarning { tokens_remaining }` |
| `[Inbox: {} message(s) injected]` | `InboxInjected { count }` |
| `[Tool: {}]` (verbose tool name echo) | `ToolDiagnostic { tool, message }` |
| Tool `result.diagnostics` items | `ToolDiagnostic { tool, message }` |
| `[{}]\n{}` (todo/goals content) | `ToolDiagnostic { tool, message }` |

The `[Hook pre_message: {} modified prompt]` at ~line 1855 goes directly to
`eprintln!` instead of the sink — move it to `HookDebug` via the sink.

Note the two `if verbose` / `if !verbose` blocks around tool diagnostics
(~lines 1543–1567): these emit diagnostics either before or after `ToolStart`
depending on verbose mode. After removing the flag, always emit them before
`ToolStart` (consistent ordering).

Remove `verbose` from the signatures of `apply_request_modifications` and
`apply_hook_overrides` (they currently take `verbose: bool`). Update their
call sites.

Also remove `options.verbose` usage in `send_prompt` — `PromptOptions.verbose`
will be removed in Task 5.

**Step 1: Update `apply_tool_filters` (hook pre_api_tools)**

Replace the two `if verbose { sink.handle(Diagnostic { ... }) }` blocks with
unconditional `sink.handle(HookDebug { ... })`. Remove `verbose` param.

**Step 2: Update `apply_request_modifications`**

Same pattern. Remove `verbose` param.

**Step 3: Update `apply_hook_overrides`**

Replace all five conditional diagnostic emits with unconditional `HookDebug`.
Remove `verbose` param.

**Step 4: Update `send_prompt` outer loop**

Replace `FuelStatus`/`FuelExhausted`/`ContextWarning`/`InboxInjected` sites.
Remove `let verbose = options.verbose;`. Remove `if verbose` guards.

**Step 5: Update tool execution section**

Replace tool diagnostic sites. Fix the `if verbose` / `if !verbose` split —
always emit before `ToolStart`.

**Step 6: Run tests**

```bash
cargo test -p chibi-core 2>&1 | tail -20
```

Expected: compile errors in sink consumers only (Tasks 3–4). Core tests pass
once `send.rs` compiles.

**Step 7: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "refactor(send): emit typed ResponseEvent variants, remove verbose gating"
```

---

## Task 3: Update `CliResponseSink`

**Files:**
- Modify: `crates/chibi-cli/src/sink.rs`

Update the `match event` arms to handle the new variants. The sink already
holds `verbose: bool`, `show_tool_calls: bool`, `show_thinking: bool` — use
them for filtering here.

**Filtering policy (CLI):**

```
HookDebug          → show if verbose
FuelStatus         → show if verbose
FuelExhausted      → always show
ContextWarning     → show if verbose
ToolDiagnostic     → show if verbose
InboxInjected      → show if verbose
```

**String formatting** lives here now, not in core. Example:

```rust
ResponseEvent::FuelStatus { remaining, total, event } => {
    if self.verbose {
        let msg = match event {
            FuelEvent::EnteringTurn =>
                format!("[fuel: {}/{} entering turn]", remaining, total),
            FuelEvent::AfterToolBatch =>
                format!("[fuel: {}/{} after tool batch]", remaining, total),
            FuelEvent::AfterContinuation { prompt_preview } =>
                format!("[continuing (fuel: {}/{}): {}]", remaining, total, prompt_preview),
            FuelEvent::EmptyResponse =>
                format!("[empty response, fuel: {}/{}]", remaining, total),
        };
        self.output.diagnostic_always(&msg);
    }
}
ResponseEvent::FuelExhausted { total } => {
    self.output.diagnostic_always(
        &format!("[fuel exhausted (0/{}), returning control to user]", total)
    );
}
ResponseEvent::HookDebug { hook: _, message } => {
    if self.verbose {
        self.output.diagnostic_always(&message);
    }
}
ResponseEvent::ContextWarning { tokens_remaining } => {
    if self.verbose {
        self.output.diagnostic_always(
            &format!("[Context window warning: {} tokens remaining]", tokens_remaining)
        );
    }
}
ResponseEvent::ToolDiagnostic { tool: _, message } => {
    if self.verbose {
        self.output.diagnostic_always(&message);
    }
}
ResponseEvent::InboxInjected { count } => {
    if self.verbose {
        self.output.diagnostic_always(
            &format!("[Inbox: {} message(s) injected]", count)
        );
    }
}
```

Note: `ToolStart` and `ToolResult` handling stays the same — they use
`self.output.diagnostic(&msg, self.show_tool_calls)` which gates on
`show_tool_calls`. Keep that as-is for now (it routes through `OutputSink`
which still has `diagnostic(msg, verbose)` — that gets fixed in Task 7).

Also remove `Diagnostic { message, verbose_only }` match arm entirely.

**Step 1: Update match arms in `handle`**

**Step 2: Update existing sink tests**

The tests in `sink.rs` that reference `ResponseEvent::Diagnostic` need
updating. Replace with appropriate typed variants. Add new tests for
`FuelExhausted` (always shown) and `FuelStatus` (verbose-gated).

**Step 3: Build**

```bash
cargo build -p chibi-cli 2>&1 | grep "^error"
```

**Step 4: Commit**

```bash
git add crates/chibi-cli/src/sink.rs
git commit -m "refactor(cli/sink): handle typed ResponseEvent variants with local filtering"
```

---

## Task 4: Update `JsonResponseSink`

**Files:**
- Modify: `crates/chibi-json/src/sink.rs`

Emit all new variants unconditionally as structured JSONL. No filtering.

```rust
ResponseEvent::HookDebug { hook, message } => {
    let json = serde_json::json!({
        "type": "hook_debug",
        "hook": hook,
        "message": message,
    });
    eprintln!("{}", json);
}
ResponseEvent::FuelStatus { remaining, total, event } => {
    let event_str = match &event {
        FuelEvent::EnteringTurn => "entering_turn",
        FuelEvent::AfterToolBatch => "after_tool_batch",
        FuelEvent::AfterContinuation { .. } => "after_continuation",
        FuelEvent::EmptyResponse => "empty_response",
    };
    let mut j = serde_json::json!({
        "type": "fuel_status",
        "remaining": remaining,
        "total": total,
        "event": event_str,
    });
    if let FuelEvent::AfterContinuation { prompt_preview } = event {
        j["prompt_preview"] = serde_json::json!(prompt_preview);
    }
    eprintln!("{}", j);
}
ResponseEvent::FuelExhausted { total } => {
    eprintln!("{}", serde_json::json!({
        "type": "fuel_exhausted",
        "total": total,
    }));
}
ResponseEvent::ContextWarning { tokens_remaining } => {
    eprintln!("{}", serde_json::json!({
        "type": "context_warning",
        "tokens_remaining": tokens_remaining,
    }));
}
ResponseEvent::ToolDiagnostic { tool, message } => {
    eprintln!("{}", serde_json::json!({
        "type": "tool_diagnostic",
        "tool": tool,
        "message": message,
    }));
}
ResponseEvent::InboxInjected { count } => {
    eprintln!("{}", serde_json::json!({
        "type": "inbox_injected",
        "count": count,
    }));
}
```

Remove the old `Diagnostic { message, verbose_only }` arm.

**Step 1: Update match arms**

**Step 2: Build and test**

```bash
cargo build -p chibi-json && cargo test -p chibi-json
```

**Step 3: Commit**

```bash
git add crates/chibi-json/src/sink.rs
git commit -m "refactor(json/sink): emit all typed ResponseEvent variants as structured JSONL"
```

---

## Task 5: Remove `verbose` from `PromptOptions`

**Files:**
- Modify: `crates/chibi-core/src/api/request.rs`
- Modify: `crates/chibi-core/src/execution.rs`
- Modify: `crates/chibi-core/src/chibi.rs`
- Modify: `crates/chibi-core/src/tools/agent_tools.rs`
- Modify: `crates/chibi-core/src/tools/file_tools.rs`
- Modify: `crates/chibi-core/src/tools/security.rs`
- Modify: `crates/chibi-core/src/gateway.rs`

`PromptOptions.verbose` was the mechanism for passing `verbose` into
`send_prompt`. Since `send.rs` no longer uses it (Task 2), remove the field.

**Step 1: Remove `verbose` field from `PromptOptions`**

In `request.rs`, remove `pub verbose: bool` from the struct and `verbose` from
`new(...)`. Update the `new` signature:

```rust
pub fn new(use_reflection: bool, debug: &'a [DebugKey], force_render: bool) -> Self {
```

**Step 2: Fix all `PromptOptions::new(...)` call sites**

Search: `PromptOptions::new(`

Sites to update (remove first `verbose` argument):
- `crates/chibi-core/src/execution.rs` (~line 453): `PromptOptions::new(config.verbose, ...)` → `PromptOptions::new(...)`
- `crates/chibi-core/src/chibi.rs` (~line 22, 288 — doc examples)
- `crates/chibi-core/src/lib.rs` (~line 19 — doc example)

**Step 3: Remove `verbose` from tool sub-agent `ResolvedConfig` literals**

In `agent_tools.rs`, `file_tools.rs`, `security.rs`, `gateway.rs`: remove
`verbose: false` from the `ResolvedConfig { ... }` struct literals. These will
become compile errors once `verbose` is removed from `ResolvedConfig` in Task
6.

**Step 4: Build**

```bash
cargo build -p chibi-core 2>&1 | grep "^error"
```

Expected: errors about `verbose` in `ResolvedConfig` — those are fixed in Task 6.

**Step 5: Commit**

```bash
git add crates/chibi-core/src/api/request.rs crates/chibi-core/src/execution.rs \
        crates/chibi-core/src/chibi.rs crates/chibi-core/src/tools/agent_tools.rs \
        crates/chibi-core/src/tools/file_tools.rs crates/chibi-core/src/tools/security.rs \
        crates/chibi-core/src/gateway.rs crates/chibi-core/src/lib.rs
git commit -m "refactor(core): remove verbose from PromptOptions"
```

---

## Task 6: Remove `verbose`, `hide_tool_calls`, `show_thinking` from core config

**Files:**
- Modify: `crates/chibi-core/src/config.rs`
- Modify: `crates/chibi-core/src/state/config_resolution.rs`
- Modify: `crates/chibi-core/src/state/tests.rs`
- Modify: `crates/chibi-core/src/chibi.rs` (ResolvedConfig literal in tests)
- Modify: `crates/chibi-core/src/gateway.rs` (ResolvedConfig literal in tests)

Remove the three fields from `Config`, `LocalConfig`, `ResolvedConfig`, and
all associated machinery.

**Step 1: Remove from `Config` struct (~line 615)**

Remove:
```rust
pub verbose: bool,
pub hide_tool_calls: bool,
pub show_thinking: bool,
```
And the three `default_*` wrapper functions and `ConfigDefaults::VERBOSE`,
`ConfigDefaults::HIDE_TOOL_CALLS`, `ConfigDefaults::SHOW_THINKING`.

**Step 2: Remove from `LocalConfig` struct (~line 695)**

Remove:
```rust
pub verbose: Option<bool>,
pub hide_tool_calls: Option<bool>,
pub show_thinking: Option<bool>,
```

**Step 3: Remove from `ResolvedConfig` struct (~line 818)**

Remove:
```rust
pub verbose: bool,
pub hide_tool_calls: bool,
pub show_thinking: bool,
```

**Step 4: Remove from `config_resolution.rs`**

Remove the three lines:
```rust
verbose: self.config.verbose,
hide_tool_calls: self.config.hide_tool_calls,
show_thinking: self.config.show_thinking,
```

**Step 5: Remove from `get_field`, `list_fields`, `set_field` in `config.rs`**

- `get_field`: remove `verbose, hide_tool_calls, show_thinking` from the
  `display:` list in the `config_get_field!` macro call
- `list_fields`: remove `"verbose"`, `"hide_tool_calls"`, `"show_thinking"`
  from the static array
- `set_field`: remove `verbose, hide_tool_calls, show_thinking` from the
  `bool:` list in `config_set_field!` macro call

**Step 6: Fix `ResolvedConfig` struct literals in tests**

In `state/tests.rs` (~lines 18–21, 546–549, 609–612): remove `verbose: false`,
`hide_tool_calls: false`, `show_thinking: false`.

In `LocalConfig` literal (~line 682–685): remove `verbose: None`,
`hide_tool_calls: None`, `show_thinking: None`.

In `chibi.rs` test literal (~line 493–497): remove the three fields.

In `gateway.rs` test literal (~line 362–364): remove the three fields.

**Step 7: Build and run core tests**

```bash
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all pass.

**Step 8: Commit**

```bash
git add crates/chibi-core/src/config.rs \
        crates/chibi-core/src/state/config_resolution.rs \
        crates/chibi-core/src/state/tests.rs \
        crates/chibi-core/src/chibi.rs \
        crates/chibi-core/src/gateway.rs
git commit -m "refactor(config): remove verbose, hide_tool_calls, show_thinking from core"
```

---

## Task 7: Type the `OutputSink` command-path events

**Files:**
- Modify: `crates/chibi-core/src/output.rs`
- Modify: `crates/chibi-core/src/execution.rs`
- Modify: `crates/chibi-cli/src/output.rs`
- Modify: `crates/chibi-json/src/output.rs`

Replace `diagnostic(&self, msg: &str, verbose: bool)` and
`diagnostic_always(&self, msg: &str)` with a single typed method
`emit_event(&self, event: CommandEvent)`.

**Step 1: Define `CommandEvent` and update `OutputSink` in `output.rs`**

```rust
/// Semantic events emitted on the command path (non-streaming).
///
/// Clients decide which events to display and how to format them.
#[derive(Debug, Clone)]
pub enum CommandEvent {
    /// Expired contexts auto-destroyed on startup (verbose-tier)
    AutoDestroyed { count: usize },
    /// Old cache entries removed on startup (verbose-tier)
    CacheCleanup { removed: usize, max_age_days: u64 },
    /// System prompt saved for a context (verbose-tier)
    SystemPromptSet { context: String },
    /// Username saved for a context (verbose-tier)
    UsernameSaved { username: String, context: String },
    /// No inbox messages for context (verbose-tier)
    InboxEmpty { context: String },
    /// Inbox messages being processed (verbose-tier)
    InboxProcessing { count: usize, context: String },
    /// All inboxes empty (verbose-tier)
    AllInboxesEmpty,
    /// Processed N context inboxes (verbose-tier)
    InboxesProcessed { count: usize },
}

pub trait OutputSink {
    fn emit_result(&self, content: &str);
    fn emit_event(&self, event: CommandEvent);
    fn newline(&self);
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;
    fn confirm(&self, prompt: &str) -> bool;
    fn emit_markdown(&self, content: &str) -> io::Result<()> {
        self.emit_result(content);
        Ok(())
    }
}
```

**Step 2: Update `execution.rs`**

Replace all `output.diagnostic(msg, verbose)` and `output.diagnostic_always(msg)`
calls with `output.emit_event(CommandEvent::...)`. Remove `let verbose =
config.verbose;` (now unused). Also remove the `verbose` parameter from
`execute_command`'s signature if it threads through — check the full signature.

Mapping:

| Old call | New call |
|---|---|
| `output.diagnostic("[Auto-destroyed {} ...]", verbose)` | `output.emit_event(CommandEvent::AutoDestroyed { count })` |
| `output.diagnostic("[Auto-cleanup: removed {} ...]", verbose)` | `output.emit_event(CommandEvent::CacheCleanup { removed, max_age_days })` |
| `output.diagnostic("[System prompt set for context '{}']", verbose)` | `output.emit_event(CommandEvent::SystemPromptSet { context })` |
| `output.diagnostic("[Username '{}' saved to context '{}']", verbose)` (in `main.rs`) | `output.emit_event(CommandEvent::UsernameSaved { username, context })` |
| `output.diagnostic("[No messages in inbox for '{}']", verbose)` | `output.emit_event(CommandEvent::InboxEmpty { context })` |
| `output.diagnostic("[Processing {} message(s) from inbox for '{}']", verbose)` | `output.emit_event(CommandEvent::InboxProcessing { count, context })` |
| `output.diagnostic("[No messages in any inbox.]", verbose)` | `output.emit_event(CommandEvent::AllInboxesEmpty)` |
| `output.diagnostic("[Processed inboxes for {} context(s).]", verbose)` | `output.emit_event(CommandEvent::InboxesProcessed { count })` |

**Step 3: Update `OutputHandler` in `chibi-cli/src/output.rs`**

Replace `diagnostic` / `diagnostic_always` impl with `emit_event`. The handler
holds `verbose: bool`; verbose-tier events show only when `self.verbose`.

```rust
fn emit_event(&self, event: CommandEvent) {
    let msg = match &event {
        CommandEvent::AutoDestroyed { count } =>
            Some((format!("[Auto-destroyed {} expired context(s)]", count), true)),
        CommandEvent::CacheCleanup { removed, max_age_days } =>
            Some((format!("[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                removed, max_age_days + 1), true)),
        CommandEvent::SystemPromptSet { context } =>
            Some((format!("[System prompt set for context '{}']", context), true)),
        CommandEvent::UsernameSaved { username, context } =>
            Some((format!("[Username '{}' saved to context '{}']", username, context), true)),
        CommandEvent::InboxEmpty { context } =>
            Some((format!("[No messages in inbox for '{}']", context), true)),
        CommandEvent::InboxProcessing { count, context } =>
            Some((format!("[Processing {} message(s) from inbox for '{}']", count, context), true)),
        CommandEvent::AllInboxesEmpty =>
            Some(("[No messages in any inbox.]".to_string(), true)),
        CommandEvent::InboxesProcessed { count } =>
            Some((format!("[Processed inboxes for {} context(s).]", count), true)),
    };
    if let Some((text, verbose_only)) = msg {
        if !verbose_only || self.verbose {
            eprintln!("{}", text);
        }
    }
}
```

Also remove `verbose: bool` from `emit_entry` calls that used it (those are
the tool_call/tool_result display — `emit_entry` already holds `self.verbose`).

**Step 4: Update `JsonOutputSink` in `chibi-json/src/output.rs`**

Emit all `CommandEvent` variants unconditionally as structured JSONL:

```rust
fn emit_event(&self, event: CommandEvent) {
    let json = match event {
        CommandEvent::AutoDestroyed { count } =>
            serde_json::json!({ "type": "auto_destroyed", "count": count }),
        CommandEvent::CacheCleanup { removed, max_age_days } =>
            serde_json::json!({ "type": "cache_cleanup", "removed": removed,
                                "max_age_days": max_age_days }),
        CommandEvent::SystemPromptSet { context } =>
            serde_json::json!({ "type": "system_prompt_set", "context": context }),
        CommandEvent::UsernameSaved { username, context } =>
            serde_json::json!({ "type": "username_saved", "username": username,
                                "context": context }),
        CommandEvent::InboxEmpty { context } =>
            serde_json::json!({ "type": "inbox_empty", "context": context }),
        CommandEvent::InboxProcessing { count, context } =>
            serde_json::json!({ "type": "inbox_processing", "count": count,
                                "context": context }),
        CommandEvent::AllInboxesEmpty =>
            serde_json::json!({ "type": "all_inboxes_empty" }),
        CommandEvent::InboxesProcessed { count } =>
            serde_json::json!({ "type": "inboxes_processed", "count": count }),
    };
    eprintln!("{}", json);
}
```

**Step 5: Fix `chibi-cli/src/main.rs`**

The `UsernameSaved` event currently fires in `main.rs` (not `execution.rs`)
with `output.diagnostic(...)`. Update to `output.emit_event(CommandEvent::UsernameSaved { ... })`.

Also remove `early_verbose` variable if it was only used for that diagnostic.

**Step 6: Build all**

```bash
cargo build 2>&1 | grep "^error"
```

**Step 7: Run all tests**

```bash
cargo test 2>&1 | tail -30
```

**Step 8: Commit**

```bash
git add crates/chibi-core/src/output.rs crates/chibi-core/src/execution.rs \
        crates/chibi-cli/src/output.rs crates/chibi-json/src/output.rs \
        crates/chibi-cli/src/main.rs
git commit -m "refactor(output): replace diagnostic(msg, verbose) with typed CommandEvent"
```

---

## Task 8: Add `verbose`, `hide_tool_calls`, `show_thinking` to `CliConfig`

**Files:**
- Modify: `crates/chibi-cli/src/config.rs`
- Modify: `crates/chibi-cli/src/main.rs`
- Modify: `crates/chibi-cli/src/cli.rs`
- Modify: `crates/chibi-cli/src/output.rs`

Move the three presentation fields into the CLI config layer.

**Step 1: Add to `RawCliConfig`, `CliConfig`, `CliConfigOverride` in `config.rs`**

```rust
// in RawCliConfig:
#[serde(default)]
pub verbose: bool,
#[serde(default)]
pub hide_tool_calls: bool,
#[serde(default)]
pub show_thinking: bool,

// in CliConfig:
pub verbose: bool,
pub hide_tool_calls: bool,
pub show_thinking: bool,

// in CliConfigOverride:
pub verbose: Option<bool>,
pub hide_tool_calls: Option<bool>,
pub show_thinking: Option<bool>,
```

Update `CliConfig::default()` (verbose: false, hide_tool_calls: false,
show_thinking: false) and `CliConfig::merge_with` to apply the three overrides.

**Step 2: Update `main.rs` to read from `CliConfig`**

The three lines that currently read from `cli_config.core`:
```rust
// BEFORE:
let verbose = cli_config.core.verbose;
let show_tool_calls = !cli_config.core.hide_tool_calls || verbose;
let show_thinking = cli_config.core.show_thinking || verbose;

// AFTER:
let verbose = cli_config.verbose;
let show_tool_calls = !cli_config.hide_tool_calls || verbose;
let show_thinking = cli_config.show_thinking || verbose;
```

Also update the `OutputHandler::new(verbose)` call to use `cli_config.verbose`.

**Step 3: Update `cli.rs` — remove from config_overrides, apply directly**

Currently `--verbose`, `--hide-tool-calls`, `--show-thinking` push to
`config_overrides` (which patched `ResolvedConfig`). Since these fields no
longer exist in core config, they need a different path.

The simplest approach: pass them directly to the `CliConfig` after it's
resolved. In `main.rs`, after `resolve_cli_config`:

```rust
// CLI flags override cli.toml values
if input.verbose_flag { cli_config.verbose = true; }
if input.hide_tool_calls_flag { cli_config.hide_tool_calls = true; }
if input.show_thinking_flag { cli_config.show_thinking = true; }
```

Add `verbose_flag`, `hide_tool_calls_flag`, `show_thinking_flag` booleans to
`ChibiInput` (in `crates/chibi-cli/src/input.rs`), populated from the `Cli`
struct in `cli.rs` instead of pushing to `config_overrides`.

Remove the three `config_overrides.push(...)` lines from `cli.rs`.

**Step 4: Update `OutputHandler`**

`OutputHandler::new(verbose)` already takes verbose. Ensure it's now sourced
from `cli_config.verbose` (not `cli_config.core.verbose`).

**Step 5: Build and run all tests**

```bash
cargo test 2>&1 | tail -30
```

**Step 6: Commit**

```bash
git add crates/chibi-cli/src/config.rs crates/chibi-cli/src/main.rs \
        crates/chibi-cli/src/cli.rs crates/chibi-cli/src/input.rs \
        crates/chibi-cli/src/output.rs
git commit -m "feat(cli/config): add verbose, hide_tool_calls, show_thinking to CliConfig"
```

---

## Task 9: Remove `verbose` from `chibi-json` input handling

**Files:**
- Modify: `crates/chibi-json/src/main.rs`

`chibi-json` currently reads `verbose` from `overrides`/`config` to control
load-time diagnostic emission. Since `JsonOutputSink` now emits all events
unconditionally and `verbose` no longer exists in `LocalConfig` or
`ResolvedConfig`, remove this logic.

**Step 1: Remove `load_verbose` extraction and its uses**

In `main.rs`, remove:
```rust
let load_verbose = json_input
    .overrides.as_ref()...
    .or_else(|| json_input.config.as_ref().and_then(|c| c.verbose))
    .unwrap_or(false);
```

Update `Chibi::load_with_options(LoadOptions { verbose: load_verbose, ... })` —
check what `LoadOptions.verbose` does. If it only fed into `OutputSink`
diagnostics, remove it too (see below).

**Step 2: Remove `verbose` from `LoadOptions` if unused**

Search for `LoadOptions` definition in `chibi-core`. If `verbose` only
controlled emission of "loaded N tools" diagnostic, remove it — `JsonOutputSink`
now always emits `CommandEvent::ContextLoaded` (add this variant to
`CommandEvent` if not already there from Task 7).

**Step 3: Update `output.diagnostic(...)` → `output.emit_event(...)`**

The one remaining `output.diagnostic(...)` in `chibi-json/src/main.rs`
(~line 74: "loaded N tools") becomes:
```rust
output.emit_event(CommandEvent::ContextLoaded { tool_count: chibi.tool_count() });
```

Add `ContextLoaded { tool_count: usize }` to `CommandEvent` in Task 7 (or add
it here if Task 7 is already committed).

**Step 4: Build and test**

```bash
cargo test -p chibi-json 2>&1 | tail -20
```

**Step 5: Commit**

```bash
git add crates/chibi-json/src/main.rs
git commit -m "refactor(json): remove verbose load-time handling, emit all events unconditionally"
```

---

## Task 10: Update documentation

**Files:**
- Modify: `docs/configuration.md`
- Modify: `docs/cli-reference.md`

**Step 1: Update `configuration.md`**

Remove `verbose`, `hide_tool_calls`, `show_thinking` from:
- The `config.toml` example block
- The `local.toml` per-context override section
- Any table listing these as core config fields

Add a note that these are now CLI presentation settings (cli.toml).

**Step 2: Add to cli.toml section in `configuration.md`**

Document the three new `cli.toml` fields alongside `render_markdown`:

```toml
# Show verbose diagnostics (default: false)
verbose = false

# Hide tool call display (default: false, verbose overrides)
hide_tool_calls = false

# Show thinking/reasoning content (default: false, verbose overrides)
show_thinking = false
```

**Step 3: Update `cli-reference.md`**

The line listing config overrides: `"verbose"`, `"hide_tool_calls"`,
`"show_thinking"` are no longer settable via `-s`/`--set`. Remove them from
that list. Note that `--verbose`, `--hide-tool-calls`, `--show-thinking` flags
still work.

**Step 4: Commit**

```bash
git add docs/configuration.md docs/cli-reference.md
git commit -m "docs: update config docs for presentation layer refactor"
```

---

## Task 11: Full test pass and pre-push checks

**Step 1: Run full test suite**

```bash
cargo test 2>&1 | tail -40
```

Expected: all pass.

**Step 2: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | grep "^error"
```

Fix any warnings.

**Step 3: Run `just pre-push`**

```bash
just pre-push
```

**Step 4: Final commit if needed**

```bash
git add -p  # stage any clippy fixes
git commit -m "style: clippy fixes for presentation layer refactor"
```
