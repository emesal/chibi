# Presentation Layer Refactor Design

**Date:** 2026-02-19
**Status:** Approved

## Problem

Three display-only config values live in `ResolvedConfig` (core): `verbose`,
`hide_tool_calls`, and `show_thinking`. Core neither needs nor should hold
these — they are purely client-side filtering decisions.

Additionally, core currently acts as a gatekeeper for its own output: it checks
`verbose` before deciding whether to emit certain events. This inverts the
correct relationship: core should emit all semantically meaningful events;
clients decide what to show.

Two related smells:

1. `ResponseEvent::Diagnostic { message: String, verbose_only: bool }` —
   `verbose_only` is a presentation hint baked into core. The correct model is
   semantic classification: clients map categories to display tiers.

2. `OutputSink::diagnostic(&self, message: &str, verbose: bool)` — same issue
   on the command path. Core passes `verbose` to the sink as a filtering
   instruction rather than the sink holding its own policy.

## Goals

- Remove `verbose`, `hide_tool_calls`, `show_thinking` from `ResolvedConfig`,
  `Config`, `LocalConfig`, and all associated config machinery in core.
- Core emits all events unconditionally, classified semantically.
- Clients (`chibi-cli`, `chibi-json`) hold display policy and filter accordingly.
- Zero behaviour change for users of `-cli` or `-json`.
- `no_tool_calls` stays in core (it affects API requests, not display).

## Architecture

### Typed diagnostic events

Replace `ResponseEvent::Diagnostic { message: String, verbose_only: bool }` with
structured variants. The full set of semantic categories found in `send.rs`:

```rust
pub enum ResponseEvent<'a> {
    // --- existing ---
    TextChunk(&'a str),
    Reasoning(&'a str),
    TranscriptEntry(TranscriptEntry),
    ToolStart { name: String, summary: Option<String> },
    ToolResult { name: String, result: String, cached: bool },
    Finished,
    Newline,
    StartResponse,

    // --- replaces Diagnostic ---
    HookDebug { hook: String, message: String },
    FuelStatus { remaining: usize, total: usize, event: FuelEvent },
    FuelExhausted { total: usize },
    ContextWarning { tokens_remaining: usize },
    ToolDiagnostic { tool: String, message: String },
    AutoDestroyed { count: usize },
    InboxInjected { count: usize },
}

pub enum FuelEvent {
    EnteringTurn,
    AfterToolBatch,
    AfterContinuation { prompt_preview: String },
    EmptyResponse,
}
```

Clients map these to display policy. In `chibi-cli`:
- `HookDebug`, `FuelStatus`, `ContextWarning`, `ToolDiagnostic`,
  `AutoDestroyed`, `InboxInjected` → show only when verbose
- `FuelExhausted` → always show

In `chibi-json`:
- emit all variants unconditionally as structured JSONL; consumers handle
  or ignore variants as they please. No filtering flag.

### Typed command events (OutputSink)

Replace `OutputSink::diagnostic(&self, msg: &str, verbose: bool)` and
`diagnostic_always(&self, msg: &str)` with a single typed method:

```rust
pub trait OutputSink {
    fn emit(&self, event: CommandEvent);
    fn emit_result(&self, content: &str);        // primary command output
    fn emit_markdown(&self, content: &str) -> io::Result<()>;
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;
    fn confirm(&self, prompt: &str) -> bool;
    fn newline(&self);
}

pub enum CommandEvent {
    AutoDestroyed { count: usize },
    ContextLoaded { tool_count: usize },
    InboxMessage { from: String, content: String },
    InboxEmpty { context: String },
    InboxAllEmpty,
    ConfigField { key: String, value: String },   // for -n / get_field output
    // ... other command-path diagnostics as needed
}
```

`CommandEvent` variants are the semantic categories found in `execution.rs`. The
CLI's `OutputHandler` shows verbose-tier events only when its internal `verbose`
flag is set; `chibi-json`'s `JsonOutputSink` emits all as structured JSONL.

### Config changes

Remove from `Config`, `LocalConfig`, `ResolvedConfig`, `ConfigDefaults`, and all
associated `get_field`/`set_field`/`list_fields`/`apply_overrides` machinery:

- `verbose`
- `hide_tool_calls`
- `show_thinking`

Remove from `PromptOptions`:

- `verbose`

Add to `chibi-cli` presentation config (`cli.toml` / per-context `cli.toml`):

```toml
# cli.toml
verbose = false
hide_tool_calls = false
show_thinking = false
```

These join `render_markdown`, `image`, and `markdown_style` in `RawCliConfig` /
`CliConfig` / `CliConfigOverride`.

`chibi-json` emits every event unconditionally — programmatic consumers receive
all typed variants and handle or ignore them as they please. `verbose` in the
`overrides` map / `config` object is removed with no replacement: the concept
of "verbose filtering" is meaningless when output is fully typed structured
events. Future filtering (e.g. a `filter` field in `JsonInput`) is a clean
extension point enabled by typed variants, but is out of scope here.

### Data flow (after)

```
chibi-cli                          chibi-core
─────────────────────              ──────────────────────────────
CliConfig {                        ResolvedConfig {
  verbose: bool,         ──┐         model, api_key,
  hide_tool_calls: bool,   │         no_tool_calls,
  show_thinking: bool,     │         fuel, auto_compact,
  render_markdown: bool,   │         reflection_enabled,
  ...                      │         ...  (no display fields)
}                          │       }
                           │
                           ▼
              CliResponseSink / OutputHandler
              (holds display policy, filters events)
                           │
                           ▼
              ResponseEvent variants / CommandEvent variants
              (emitted unconditionally by core, semantically typed)
```

## Non-goals

- Unifying `OutputSink` and `ResponseSink` into a single trait — they serve
  different interaction models (synchronous command vs async streaming) and
  `confirm` has no streaming equivalent.
- Changing user-visible behaviour in any way.
- Touching `no_tool_calls` (legitimately core).

## Testing

- All existing `CliResponseSink` and `OutputHandler` tests continue to pass.
- New unit tests on each sink verifying correct filtering per event variant.
- Existing integration tests exercise the full stack and confirm output is
  unchanged.
- `chibi-json` output shape tests for the new JSONL diagnostic format.
