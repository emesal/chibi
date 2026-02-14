# chibi-json Extraction Design

> **Status:** approved design, pending implementation plan
> **Issue:** #133
> **Branch:** feature/M1.5-basic-composable-agent

## Goal

Split chibi into two binaries with a shared core:

- **chibi-cli** — human-facing: TTY, markdown rendering, interactive prompts, session state
- **chibi-json** — programmatic-facing: JSON in / JSONL out, stateless, lean dependencies

## Crate Topology

```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)
```

**chibi-core** — unchanged role, gains: `OutputSink` trait, `ExecutionRequest`/`ExecutionFlags` types, `execute_command()` function.

**chibi-cli** — depends on chibi-core + clap, streamdown-\*, image libs. Owns CLI parsing, Session, markdown rendering, interactive permission prompts.

**chibi-json** — depends on chibi-core + serde_json, schemars. No clap, no streamdown, no image libs. Lean binary for automation.

## Core Execution Contract

### ExecutionRequest

Replaces the current split between `ChibiInput` (CLI-specific) and `Flags` (mixed concerns). This is what core needs to execute any command:

```rust
pub struct ExecutionRequest {
    pub command: Command,
    pub context: String,                // always explicit, already resolved
    pub flags: ExecutionFlags,          // only flags core cares about
    pub username: Option<String>,       // runtime username override
}

pub struct ExecutionFlags {
    pub verbose: bool,
    pub no_tool_calls: bool,
    pub show_thinking: bool,
    pub hide_tool_calls: bool,
    pub force_call_agent: bool,
    pub force_call_user: bool,
    pub debug: Vec<DebugKey>,
}
```

Each binary maps its own input type → `ExecutionRequest`. Core never sees CLI concepts (sessions, persistent switches) or JSON concepts (always-on JSON mode).

### OutputSink Trait

Abstraction over how command results and diagnostics are presented:

```rust
pub trait OutputSink {
    fn emit_result(&self, content: &str);
    fn diagnostic(&self, message: &str, verbose: bool);
    fn diagnostic_always(&self, message: &str);
    fn newline(&self);
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;
    fn is_json_mode(&self) -> bool;
    fn confirm(&self, prompt: &str) -> bool;
}
```

- chibi-cli implements this with `OutputHandler` (plain text to stdout/stderr, interactive TTY confirmation)
- chibi-json implements this with `JsonOutputSink` (JSONL to stdout/stderr, auto-approve confirmation)

### execute_command()

Shared command dispatch function in chibi-core:

```rust
pub async fn execute_command(
    chibi: &mut Chibi,
    request: &ExecutionRequest,
    config: &ResolvedConfig,
    output: &dyn OutputSink,
    sink: &mut dyn ResponseSink,
) -> io::Result<()>
```

Handles all `Command` variants. Callers provide:
- Resolved context name (CLI resolves via Session, JSON takes from input)
- Config (CLI adds presentation layer, JSON uses core config)
- Output and response sink implementations

## Flags Cleanup

The current `Flags` type in chibi-core mixes execution and presentation concerns:
- `json_output` — presentation only (dead in core, only read by sink)
- `raw` — presentation only (disable markdown)

**Action:** replace `Flags` with `ExecutionFlags`. Remove `json_output` and `raw` from core entirely. `PromptOptions::json_output` also removed (core never reads it; the sink's `is_json_mode()` is the source of truth).

## chibi-json Binary Design

**Interface:** stdin JSON → stdout JSONL, stderr diagnostics (JSONL)

**Input type** (chibi-json's own, not shared):
```rust
pub struct JsonInput {
    pub command: Command,
    pub context: String,                 // required, no "current" concept
    pub flags: ExecutionFlags,           // direct, no translation needed
    pub username: Option<String>,
    pub home: Option<PathBuf>,           // chibi home override
    pub project_root: Option<PathBuf>,   // project root override
}
```

No `ContextSelection`, no `UsernameOverride`, no `Session`. Context is always explicit. Stateless per invocation.

**CLI args** (minimal):
- `--json-schema` — print input schema and exit
- `--version` — print version and exit

Everything else comes via JSON stdin, including `home` and `project_root`.

**Permissions:** always auto-approve (trust mode). Programmatic callers that send `shell_exec` have already decided. Gating belongs in the orchestrator, not the tool.

**ResponseSink:** `JsonResponseSink` — emits transcript entries as JSONL to stdout. Text chunks and reasoning are accumulated and emitted as complete entries (no streaming partial text to stdout in JSON mode).

## What Moves Where

| component | from | to | notes |
|-----------|------|----|-------|
| `Command`, `DebugKey`, `Inspectable` | chibi-core | stays | unchanged |
| `Flags` | chibi-core | removed | replaced by `ExecutionFlags` |
| `ExecutionRequest`, `ExecutionFlags` | — | chibi-core (new) | core's execution contract |
| `OutputSink` trait | — | chibi-core (new) | ~7 methods |
| `execute_command()` | — | chibi-core (new) | command dispatch, ~460 lines |
| `ChibiInput`, `ContextSelection`, `UsernameOverride` | chibi-cli | stays | CLI's input contract |
| `Cli` (clap), `Session` | chibi-cli | stays | unchanged |
| `CliResponseSink`, `MarkdownStream` | chibi-cli | stays | unchanged |
| `OutputHandler` | chibi-cli | stays | becomes `OutputSink` impl, JSON paths removed |
| `json_input.rs` | chibi-cli | chibi-json | adapted for `JsonInput` |
| `--json-config`, `--json-output`, `--json-schema` | chibi-cli | removed | functionality moves to chibi-json |
| `JsonInput` | — | chibi-json (new) | own JSON schema |
| `JsonOutputSink` | — | chibi-json (new) | JSONL `OutputSink` impl |
| `JsonResponseSink` | — | chibi-json (new) | JSONL `ResponseSink` impl |

## chibi-cli Changes

After the split, chibi-cli:
- Loses `--json-config`, `--json-output`, `--json-schema`
- Loses `json_input.rs` module
- `OutputHandler` simplifies: removes all JSON-mode branches
- `main.rs` simplifies: removes JSON config path, maps `ChibiInput` → `ExecutionRequest`, calls `execute_command()`
- No longer needs `schemars` dependency

## Drift Prevention

Two SYNC comments, one in each binary, at the point where `ExecutionRequest` is constructed:

```rust
// chibi-cli/src/main.rs
// SYNC: chibi-json also builds ExecutionRequest — check crates/chibi-json/src/main.rs

// chibi-json/src/main.rs
// SYNC: chibi-cli also builds ExecutionRequest — check crates/chibi-cli/src/main.rs
```

These are the only two sites that map binary-specific input → core contract. If the mapping diverges, it should be intentional.

## Future Considerations

- **Multi-context orchestration:** chibi-json is designed for single-context-per-invocation now, but the stateless design means an orchestrator can invoke it in parallel across many contexts. If we later want batch operations, we extend `JsonInput` (or add a batch wrapper) without touching core.
- **Shared extraction:** if chibi-cli and chibi-json develop significant shared logic beyond what core provides, extract a `chibi-shared` crate. Not expected for v1.
