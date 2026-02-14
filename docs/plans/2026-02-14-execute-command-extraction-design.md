# extract shared execute_command() into chibi-core

> **status:** approved design, pending implementation plan
> **issue:** #143
> **branch:** bugfix/pre-0-8-0-review-2602

## problem

both chibi-cli and chibi-json dispatch `Command` variants independently with
significant duplication: ~15 command handlers, the resolve → lock → send
sequence (inlined 4× in chibi-json), and the pre/post-command lifecycle
(init, auto-destroy, context touch, shutdown, cache cleanup).

## solution

extract a single `execute_command()` into `crates/chibi-core/src/execution.rs`
that handles the full lifecycle and all command dispatch. binaries become thin
pipelines: parse input → resolve config → build sinks → call `execute_command()`.

## public API

```rust
// crates/chibi-core/src/execution.rs

pub async fn execute_command(
    chibi: &mut Chibi,
    context: &str,              // already resolved by binary
    command: &Command,
    flags: &ExecutionFlags,
    config: &CoreConfig,        // core config only, no presentation
    username: Option<&str>,     // ephemeral username override
    output: &dyn OutputSink,
    sink: &mut dyn ResponseSink,
) -> io::Result<CommandEffect>
```

### CommandEffect

returned so binaries can update their own state without core knowing about
sessions or other binary-specific concepts:

```rust
pub enum CommandEffect {
    None,
    ContextDestroyed(String),
    ContextRenamed { old: String, new: String },
}
```

CLI inspects this to update session. JSON ignores it.

## OutputSink extension

one new method on the existing trait:

```rust
fn emit_markdown(&self, content: &str) -> io::Result<()>;
```

- CLI's `OutputHandler`: renders via streamdown
- JSON's `JsonOutputSink`: passes through raw (calls `emit_result()`)

covers `ShowLog` message content, `Inspect` todos/goals, and any future
markdown-bearing output.

## lifecycle ownership

`execute_command()` owns the full lifecycle:

**pre-command:**
1. `chibi.init()` (OnStart hooks)
2. auto-destroy expired contexts
3. ensure context dir + `ContextEntry` exist
4. touch context with debug destroy settings

**post-command:**
1. `chibi.shutdown()` (OnEnd hooks)
2. automatic cache cleanup

## send path

private helper inside `execution.rs`:

```rust
async fn send_prompt(
    chibi: &Chibi,
    context: &str,
    prompt: &str,
    config: &CoreConfig,
    flags: &ExecutionFlags,
    fallback: Option<HandoffTarget>,
    sink: &mut dyn ResponseSink,
) -> io::Result<()>
```

handles: apply `no_tool_calls` → acquire `ContextLock` → build `PromptOptions`
→ `send_prompt_streaming()`. used by `SendPrompt`, `CallTool` (with
continuation), `CheckInbox`, `CheckAllInboxes`.

`PromptOptions::force_render` is always `false` in core — rendering is a sink
concern, baked into sink construction by the binary.

## what stays in binaries

**both:**
- input parsing (clap / stdin JSON)
- config resolution (core + presentation layer)
- sink construction
- `ShowHelp` / `ShowVersion` (intercepted before `execute_command()`)

**CLI only:**
- context name resolution (session, `new`, `new:prefix`, `-`)
- context switching + session persistence
- username persistence (`-u` → local.toml)
- session updates from `CommandEffect`
- `force_markdown`, `raw` → applied to sink construction
- image cache cleanup

**JSON only:**
- `--json-schema`
- trust-mode auto-approve (set on chibi before calling)

## what gets removed

- ~200 lines from each binary's command dispatch
- chibi-json's 4× inlined send path
- duplicated lifecycle code in both binaries

## drift prevention

SYNC comments at the two sites where binaries construct their call to
`execute_command()`:

```rust
// chibi-cli/src/main.rs
// SYNC: chibi-json also calls execute_command — check crates/chibi-json/src/main.rs

// chibi-json/src/main.rs
// SYNC: chibi-cli also calls execute_command — check crates/chibi-cli/src/main.rs
```
