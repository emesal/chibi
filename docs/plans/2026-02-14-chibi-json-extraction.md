# chibi-json Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split chibi into two binaries (chibi-cli + chibi-json) with shared core, following the approved design in `docs/plans/2026-02-14-chibi-json-extraction-design.md`.

**Architecture:** Bottom-up extraction in 5 phases: (1) introduce `OutputSink` trait + `ExecutionFlags` in core, (2) introduce `ExecutionRequest` + refactor CLI to use `OutputSink`, (3) create chibi-json crate, (4) clean up chibi-cli, (5) clean up core + docs. Each phase builds on the previous — no phase can start before its predecessor compiles and passes tests.

**Tech Stack:** Rust, serde_json, schemars, tokio.

**Branch:** `feature/M1.5-basic-composable-agent`

---

## Progress

| Task | Description | Status |
|------|-------------|--------|
| 1 | Add `OutputSink` trait to chibi-core | **done** |
| 2 | Add `ExecutionFlags` to chibi-core | **done** |
| 3 | Add `ExecutionRequest` to chibi-core | **done** |
| 4 | Refactor `OutputHandler` to implement `OutputSink` | **done** |
| 5 | Remove `PromptOptions::json_output` | **done** |
| 6 | Scaffold chibi-json crate | **done** |
| 7 | Implement command execution in chibi-json | **done** |
| 8 | Add integration tests for chibi-json | **done** |
| 9 | Remove JSON paths from chibi-cli | **done** |
| 10 | Add SYNC comments | **done** |
| 11 | Remove `Flags::json_output` and `Flags::raw` from core | **done** |
| 12 | Update docs and file GH issue | pending |

### Session Notes (for next session)

- **all 701 tests pass** after tasks 1–11 (196 lib + 43 cli-integration + 447 core + 5 chibi-json + 10 doctests).
- **no commits yet** — all changes are unstaged. commit when ready.
- task 12 is the only remaining task (docs + GH issue).
- **task 9 deviations:** also removed 17 integration tests for `--json-config`/`--json-schema` (moved to chibi-json). removed `schemars` from chibi-cli deps. deleted `json_input.rs` (module removed, file still on disk). removed `uuid` usage from `OutputHandler` (no longer constructs `TranscriptEntry` for JSON mode).
- **task 11 deviations:** instead of keeping `Flags` as a separate struct, made it a type alias for `ExecutionFlags` (`pub type Flags = ExecutionFlags`). this eliminates the `From<&Flags>` conversion entirely — they're the same type now. `raw` moved to `ChibiInput.raw` (CLI-only field). the `json_input.rs` tests referencing `json_output` and `raw` on `Flags` are dead code (module is no longer compiled); the file can be deleted.
- previous session notes (tasks 1–8) still apply for context.

---

## Phase 1: Core Types — OutputSink + ExecutionFlags

### Task 1: Add `OutputSink` trait to chibi-core

**Files:**
- Create: `crates/chibi-core/src/output.rs`
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: Write the trait and module**

```rust
// crates/chibi-core/src/output.rs

use crate::context::TranscriptEntry;
use std::io;

/// Abstraction over how command results and diagnostics are presented.
///
/// chibi-cli implements this with OutputHandler (text to stdout/stderr, interactive TTY).
/// chibi-json implements this with JsonOutputSink (JSONL to stdout/stderr, auto-approve).
pub trait OutputSink {
    /// Emit a result string (the primary output of a command).
    fn emit_result(&self, content: &str);

    /// Emit a diagnostic message. Only shown when `verbose` is true.
    fn diagnostic(&self, message: &str, verbose: bool);

    /// Emit a diagnostic message unconditionally.
    fn diagnostic_always(&self, message: &str);

    /// Emit a blank line.
    fn newline(&self);

    /// Emit a transcript entry (for JSON-mode structured output).
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;

    /// Whether this sink operates in JSON mode (affects downstream formatting).
    fn is_json_mode(&self) -> bool;

    /// Prompt the user for confirmation. Returns true if confirmed.
    /// JSON-mode implementations should auto-approve (return true).
    fn confirm(&self, prompt: &str) -> bool;
}
```

**Step 2: Register the module in lib.rs**

Add `pub mod output;` to `crates/chibi-core/src/lib.rs` and re-export the trait:
```rust
pub mod output;
pub use output::OutputSink;
```

**Step 3: Run tests to verify compilation**

Run: `cargo test -p chibi-core --lib`
Expected: PASS (trait has no implementation yet, just a definition)

**Step 4: Commit**

```
feat(core): add OutputSink trait

Abstraction over command output presentation. chibi-cli and chibi-json
will provide their own implementations.
```

---

### Task 2: Add `ExecutionFlags` to chibi-core

**Files:**
- Modify: `crates/chibi-core/src/input.rs`
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: Write failing test for ExecutionFlags**

Add to `crates/chibi-core/src/input.rs` in the `tests` module:

```rust
#[test]
fn test_execution_flags_default() {
    let flags = ExecutionFlags::default();
    assert!(!flags.verbose);
    assert!(!flags.no_tool_calls);
    assert!(!flags.show_thinking);
    assert!(!flags.hide_tool_calls);
    assert!(!flags.force_call_agent);
    assert!(!flags.force_call_user);
    assert!(flags.debug.is_empty());
}

#[test]
fn test_execution_flags_serialization() {
    let flags = ExecutionFlags {
        verbose: true,
        no_tool_calls: true,
        show_thinking: false,
        hide_tool_calls: false,
        force_call_agent: true,
        force_call_user: false,
        debug: vec![DebugKey::RequestLog],
    };
    let json = serde_json::to_string(&flags).unwrap();
    let deser: ExecutionFlags = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.verbose, flags.verbose);
    assert_eq!(deser.force_call_agent, flags.force_call_agent);
    assert_eq!(deser.debug.len(), 1);
}

#[test]
fn test_execution_flags_from_flags() {
    let old = Flags {
        verbose: true,
        json_output: true,
        force_call_user: false,
        force_call_agent: true,
        hide_tool_calls: true,
        no_tool_calls: false,
        show_thinking: true,
        raw: true,
        debug: vec![DebugKey::All],
    };
    let new = ExecutionFlags::from(&old);
    assert!(new.verbose);
    assert!(new.force_call_agent);
    assert!(!new.force_call_user);
    assert!(new.hide_tool_calls);
    assert!(!new.no_tool_calls);
    assert!(new.show_thinking);
    assert_eq!(new.debug.len(), 1);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core test_execution_flags`
Expected: FAIL — `ExecutionFlags` not defined

**Step 3: Write `ExecutionFlags` struct and `From<&Flags>` conversion**

Add to `crates/chibi-core/src/input.rs`, after the `Flags` struct:

```rust
/// Execution-only flags — what core needs to run any command.
///
/// Unlike `Flags`, this excludes presentation concerns (`json_output`, `raw`)
/// which belong to the binary layer. Both chibi-cli and chibi-json map their
/// own input types to this.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionFlags {
    /// Show verbose output
    #[serde(default)]
    pub verbose: bool,
    /// Omit tools from API requests entirely
    #[serde(default)]
    pub no_tool_calls: bool,
    /// Show thinking/reasoning content
    #[serde(default)]
    pub show_thinking: bool,
    /// Hide tool call display (verbose overrides)
    #[serde(default)]
    pub hide_tool_calls: bool,
    /// Force handoff to agent
    #[serde(default)]
    pub force_call_agent: bool,
    /// Force handoff to user immediately
    #[serde(default)]
    pub force_call_user: bool,
    /// Debug features to enable
    #[serde(default)]
    pub debug: Vec<DebugKey>,
}

impl From<&Flags> for ExecutionFlags {
    fn from(flags: &Flags) -> Self {
        Self {
            verbose: flags.verbose,
            no_tool_calls: flags.no_tool_calls,
            show_thinking: flags.show_thinking,
            hide_tool_calls: flags.hide_tool_calls,
            force_call_agent: flags.force_call_agent,
            force_call_user: flags.force_call_user,
            debug: flags.debug.clone(),
        }
    }
}
```

**Step 4: Add `ExecutionFlags` to lib.rs re-exports**

Change the existing re-export line in `lib.rs`:
```rust
pub use input::{Command, ExecutionFlags, Flags, Inspectable};
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p chibi-core test_execution_flags`
Expected: PASS

**Step 6: Commit**

```
feat(core): add ExecutionFlags type

Core-only execution modifiers, excluding presentation concerns (json_output, raw).
Includes From<&Flags> for incremental migration.
```

---

## Phase 2: Core Execution + CLI Refactor

### Task 3: Add `ExecutionRequest` to chibi-core

**Files:**
- Create: `crates/chibi-core/src/execution.rs`
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: Write failing test**

Add to a new file `crates/chibi-core/src/execution.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Command;

    #[test]
    fn test_execution_request_default_fields() {
        let req = ExecutionRequest {
            command: Command::NoOp,
            context: "test".to_string(),
            flags: ExecutionFlags::default(),
            username: None,
        };
        assert_eq!(req.context, "test");
        assert!(req.username.is_none());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core test_execution_request`
Expected: FAIL

**Step 3: Write ExecutionRequest struct**

```rust
// crates/chibi-core/src/execution.rs

use crate::input::{Command, ExecutionFlags};

/// What core needs to execute any command.
///
/// Each binary (chibi-cli, chibi-json) maps its own input type to this.
/// Core never sees CLI concepts (sessions, persistent switches) or JSON
/// concepts (always-on JSON mode).
pub struct ExecutionRequest {
    /// The command to execute
    pub command: Command,
    /// Context name — always explicit, already resolved by the caller
    pub context: String,
    /// Execution-only flags
    pub flags: ExecutionFlags,
    /// Runtime username override (already resolved by caller)
    pub username: Option<String>,
}
```

**Step 4: Register in lib.rs**

```rust
pub mod execution;
pub use execution::ExecutionRequest;
```

**Step 5: Run tests**

Run: `cargo test -p chibi-core test_execution_request`
Expected: PASS

**Step 6: Commit**

```
feat(core): add ExecutionRequest type

Unified execution contract for all binaries. Each binary maps its own
input to this before calling core.
```

---

### Task 4: Refactor `OutputHandler` to implement `OutputSink` directly

Replace `OutputHandler`'s inherent methods with a direct `OutputSink` trait implementation. All call sites in chibi-cli switch from calling inherent methods to calling trait methods. This is a clean single-source-of-truth refactor — no delegation layer, no method duplication.

**Files:**
- Modify: `crates/chibi-cli/src/output.rs` — delete inherent methods, implement `OutputSink` directly
- Modify: `crates/chibi-cli/src/main.rs` — update signatures: `&OutputHandler` → `&dyn OutputSink`, add `use chibi_core::OutputSink;`
- Modify: `crates/chibi-cli/src/sink.rs` — `CliResponseSink` field `output: &'a OutputHandler` → `output: &'a dyn OutputSink`

**Step 1: Rewrite `OutputHandler` to implement `OutputSink` directly**

The current `OutputHandler` has these inherent methods (see `crates/chibi-cli/src/output.rs:22-131`):
- `new(json_mode: bool)` — keep as inherent (constructor)
- `is_json_mode(&self)` → delete, replaced by `OutputSink::is_json_mode()`
- `emit(&self, entry)` → becomes `OutputSink::emit_entry()`
- `diagnostic(&self, msg, verbose)` → becomes `OutputSink::diagnostic()`
- `diagnostic_always(&self, msg)` → becomes `OutputSink::diagnostic_always()`
- `newline(&self)` → becomes `OutputSink::newline()`
- `emit_result(&self, content)` → becomes `OutputSink::emit_result()`

Add new trait methods not currently on `OutputHandler`:
- `confirm(&self, prompt)` → delegates to `crate::confirm_action(prompt)`

```rust
use chibi_core::context::TranscriptEntry;
use chibi_core::OutputSink;
use std::io::{self, Write};

/// CLI output handler — text to stdout, diagnostics to stderr.
///
/// Implements `OutputSink` directly; all output goes through trait methods.
pub struct OutputHandler {
    json_mode: bool,
}

impl OutputHandler {
    /// Create a new output handler.
    pub fn new(json_mode: bool) -> Self {
        Self { json_mode }
    }
}

impl OutputSink for OutputHandler {
    fn emit_result(&self, content: &str) {
        println!("{}", content);
    }

    fn diagnostic(&self, message: &str, verbose: bool) {
        if verbose {
            eprintln!("{}", message);
        }
    }

    fn diagnostic_always(&self, message: &str) {
        eprintln!("{}", message);
    }

    fn newline(&self) {
        println!();
    }

    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()> {
        if self.json_mode {
            let json = serde_json::to_string(entry)?;
            println!("{}", json);
            io::stdout().flush()?;
        }
        // In text mode, transcript entries are handled by the response sink
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        self.json_mode
    }

    fn confirm(&self, prompt: &str) -> bool {
        crate::confirm_action(prompt)
    }
}
```

**Step 2: Update `CliResponseSink` to take `&dyn OutputSink`**

In `crates/chibi-cli/src/sink.rs`:
- Change `output: &'a OutputHandler` → `output: &'a dyn OutputSink`
- Update `new()` parameter accordingly
- Remove `use crate::output::OutputHandler;`, add `use chibi_core::OutputSink;`

All method calls on `output` inside `CliResponseSink` (`output.diagnostic()`, `output.diagnostic_always()`, `output.newline()`, `output.emit_entry()`, `output.is_json_mode()`) are already `OutputSink` trait methods — no call site changes needed inside the sink.

**Step 3: Update `main.rs` function signatures**

Functions that take `&OutputHandler`:
- `execute_from_input()` (line 400): `output: &OutputHandler` → `output: &dyn OutputSink`
- `inspect_context()` (line 221): `output: &OutputHandler` → `output: &dyn OutputSink`
- `show_log()` (line 298): `output: &OutputHandler` → `output: &dyn OutputSink`

Add `use chibi_core::OutputSink;` to imports.

`confirm_action` is still called directly in `execute_from_input` for `DestroyContext` — change to `output.confirm(...)` instead.

**Step 4: Update `OutputHandler` tests**

Tests in `output.rs` that call inherent methods need to call trait methods instead. Add `use chibi_core::OutputSink;` to the test module. Tests that called `handler.is_json_mode()`, `handler.diagnostic()`, etc. now call through the trait. The calls look the same syntactically (Rust resolves trait methods when no inherent method exists).

Update `sink.rs` tests: `OutputHandler::new(false)` still works (constructor is inherent). The trait methods are called implicitly through the `&dyn OutputSink` in `CliResponseSink`.

**Step 5: Run all tests**

Run: `cargo test`
Expected: PASS

**Step 6: Commit**

```
refactor(cli): implement OutputSink directly on OutputHandler

Delete inherent methods, implement trait directly. All call sites now
go through the OutputSink trait — single source of truth, no delegation.
```

---

### Task 5: Remove `PromptOptions::json_output`

The design doc says `PromptOptions::json_output` should be removed — the sink's `is_json_mode()` is the source of truth.

**Files:**
- Modify: `crates/chibi-core/src/api/request.rs`
- Modify: `crates/chibi-cli/src/main.rs` (all `PromptOptions::new()` call sites)

**Step 1: Verify `options.json_output` is dead in core**

`PromptOptions::json_output` is set in `new()` but never read elsewhere in core — the streaming code uses `sink.is_json_mode()` instead. Confirm with: `cargo test -p chibi-core` (baseline).

**Step 2: Remove `json_output` from `PromptOptions`**

In `crates/chibi-core/src/api/request.rs`, remove the `json_output` field from the struct and the `new()` constructor. Update `new()` signature to drop the `json_output` parameter.

**Step 3: Update all callers of `PromptOptions::new()` in chibi-cli**

In `crates/chibi-cli/src/main.rs`, every `PromptOptions::new(verbose, use_reflection, json_output, &input.flags.debug, force_markdown)` becomes `PromptOptions::new(verbose, use_reflection, &input.flags.debug, force_markdown)`. There are 4 call sites (SendPrompt, CallTool with agent continuation, CheckInbox, CheckAllInboxes).

Also remove the `let json_output = input.flags.json_output;` line near the top of `execute_from_input` (~line 404).

**Step 4: Run all tests**

Run: `cargo test`
Expected: PASS

**Step 5: Commit**

```
refactor(core): remove PromptOptions::json_output

Dead field — the sink's is_json_mode() is the source of truth.
```

---

## Phase 3: Create chibi-json Crate

### Task 6: Scaffold chibi-json crate

**Files:**
- Create: `crates/chibi-json/Cargo.toml`
- Create: `crates/chibi-json/src/main.rs`
- Create: `crates/chibi-json/src/input.rs`
- Create: `crates/chibi-json/src/output.rs`
- Create: `crates/chibi-json/src/sink.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "chibi-json"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[[bin]]
name = "chibi-json"
path = "src/main.rs"

[dependencies]
chibi-core = { path = "../chibi-core" }
schemars = "0.8"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.49.0", features = ["full"] }

[lints.rust]
dead_code = "deny"
```

**Step 2: Create minimal main.rs**

```rust
use std::io::{self, Read};

mod input;
mod output;
mod sink;

#[tokio::main]
async fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // --json-schema: print input schema and exit
    if args.iter().any(|a| a == "--json-schema") {
        let schema = schemars::schema_for!(input::JsonInput);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
        return Ok(());
    }

    // --version
    if args.iter().any(|a| a == "--version") {
        println!("chibi-json {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Read JSON from stdin
    let mut json_str = String::new();
    io::stdin().read_to_string(&mut json_str)?;

    let json_input: input::JsonInput = serde_json::from_str(&json_str).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid JSON input: {}", e))
    })?;

    // TODO: execute command (Task 7)
    let _ = json_input;
    Ok(())
}
```

**Step 3: Create input module**

Create `crates/chibi-json/src/input.rs`:

```rust
use std::path::PathBuf;

use chibi_core::input::{Command, ExecutionFlags};
use schemars::JsonSchema;
use serde::Deserialize;

/// JSON-mode input — read from stdin, stateless per invocation.
///
/// Unlike ChibiInput (CLI), context is always explicit (no "current" concept),
/// there's no session, and no context selection enum.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct JsonInput {
    /// The command to execute
    pub command: Command,
    /// Context name — required, no "current" concept
    pub context: String,
    /// Execution flags
    #[serde(default)]
    pub flags: ExecutionFlags,
    /// Runtime username override
    #[serde(default)]
    pub username: Option<String>,
    /// Chibi home directory override
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Project root override
    #[serde(default)]
    pub project_root: Option<PathBuf>,
}
```

**Step 4: Create output module**

Create `crates/chibi-json/src/output.rs`:

```rust
use chibi_core::context::TranscriptEntry;
use chibi_core::OutputSink;
use std::io::{self, Write};

/// JSONL output sink for chibi-json.
///
/// Results go to stdout as JSONL, diagnostics go to stderr as JSONL.
/// Confirmation always returns true (trust mode).
pub struct JsonOutputSink;

impl OutputSink for JsonOutputSink {
    fn emit_result(&self, content: &str) {
        let json = serde_json::json!({"type": "result", "content": content});
        println!("{}", json);
    }

    fn diagnostic(&self, message: &str, verbose: bool) {
        if verbose {
            let json = serde_json::json!({"type": "diagnostic", "content": message});
            eprintln!("{}", json);
        }
    }

    fn diagnostic_always(&self, message: &str) {
        let json = serde_json::json!({"type": "diagnostic", "content": message});
        eprintln!("{}", json);
    }

    fn newline(&self) {
        // no-op in JSON mode — whitespace is meaningless
    }

    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()> {
        let json = serde_json::to_string(entry)?;
        println!("{}", json);
        io::stdout().flush()?;
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        true
    }

    fn confirm(&self, _prompt: &str) -> bool {
        true // trust mode — programmatic callers have already decided
    }
}
```

**Step 5: Create response sink**

Create `crates/chibi-json/src/sink.rs`:

```rust
use chibi_core::api::sink::{ResponseEvent, ResponseSink};
use std::io::{self, Write};

/// JSONL response sink for chibi-json.
///
/// Accumulates text chunks and emits complete transcript entries as JSONL.
/// No streaming partial text — programmatic consumers want complete records.
pub struct JsonResponseSink {
    accumulated_text: String,
}

impl JsonResponseSink {
    pub fn new() -> Self {
        Self {
            accumulated_text: String::new(),
        }
    }
}

impl ResponseSink for JsonResponseSink {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(text) => {
                self.accumulated_text.push_str(text);
            }
            ResponseEvent::Reasoning(_) => {
                // Reasoning not emitted in JSON mode
            }
            ResponseEvent::TranscriptEntry(entry) => {
                let json = serde_json::to_string(&entry)?;
                println!("{}", json);
                io::stdout().flush()?;
            }
            ResponseEvent::Finished => {
                self.accumulated_text.clear();
            }
            ResponseEvent::Diagnostic { message, verbose_only } => {
                if !verbose_only {
                    let json = serde_json::json!({"type": "diagnostic", "content": message});
                    eprintln!("{}", json);
                }
            }
            ResponseEvent::ToolStart { name, summary } => {
                let json = serde_json::json!({
                    "type": "tool_start",
                    "name": name,
                    "summary": summary,
                });
                eprintln!("{}", json);
            }
            ResponseEvent::ToolResult { name, result, cached } => {
                let json = serde_json::json!({
                    "type": "tool_result",
                    "name": name,
                    "result": result,
                    "cached": cached,
                });
                eprintln!("{}", json);
            }
            ResponseEvent::Newline | ResponseEvent::StartResponse => {}
        }
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        true
    }
}
```

**Step 6: Add to workspace**

In root `Cargo.toml`, add `"crates/chibi-json"` to workspace members.

**Step 7: Build**

Run: `cargo build -p chibi-json`
Expected: PASS (compiles, doesn't execute commands yet)

**Step 8: Commit**

```
feat: scaffold chibi-json crate

Minimal binary with JsonInput, JsonOutputSink, and JsonResponseSink.
Reads JSON from stdin, schema via --json-schema. No command execution yet.
```

---

### Task 7: Implement command execution in chibi-json

**Files:**
- Modify: `crates/chibi-json/src/main.rs`

**Known resolved blockers** (investigated during planning):

| concern | resolution |
|---------|-----------|
| `Command` field types | all `String` / `Option<String>` — no `&str` issues |
| `now_timestamp` visibility | `pub fn` in `context.rs`, accessible as `chibi_core::context::now_timestamp()` |
| `inspect_context` entangled with CLI | chibi-json handles `Inspectable` variants directly using core methods (`load_system_prompt_for`, `load_reflection`, `load_todos_for`, `load_goals_for`, `get_field`). no markdown rendering — emit raw text. `Inspectable::List` emits variant names directly (no `InspectableExt` needed). `ConfigField` uses core's `ResolvedConfig::get_field()` (not CLI's wrapper). |
| `set_prompt_for_context` file-or-string | copy the 5-line logic (check `Path::is_file()`, read if so). extract to core helper if it feels duplicated after implementation. |
| `ResponseEvent` variants | `Diagnostic { message, verbose_only }` (not `DiagnosticAlways`). `ToolStart { name, summary }`, `ToolResult { name, result, cached }`, `Reasoning(&str)`, `StartResponse`. all handled in task 6's `JsonResponseSink`. |
| `PromptOptions::new()` signature | after task 5: `(verbose, use_reflection, &debug, force_render)`. chibi-json always passes `false` for `force_render`. |
| `ShowLog` | uses `chibi.app.read_jsonl_transcript()` (public) → emit entries via `output.emit_entry()`. `count` field is `isize` (positive = tail, negative = head, zero = all). |

**Step 1: Implement the execution path in main.rs**

Replace the `TODO` in `main.rs` with the full execution flow. This mirrors `execute_from_input` from chibi-cli but is much simpler — no session, no context selection, no markdown, no image cache.

```rust
// In main.rs, replace the TODO block with:

use chibi_core::{Chibi, LoadOptions};
use chibi_core::input::{Command, DebugKey, Inspectable};
use chibi_core::context::{Context, ContextEntry};
use chibi_core::api::{self, PromptOptions};
use chibi_core::tools;
use chibi_core::OutputSink;

// ... inside main(), after parsing json_input:

let output = output::JsonOutputSink;
let verbose = json_input.flags.verbose;

let mut chibi = Chibi::load_with_options(LoadOptions {
    verbose,
    home: json_input.home.clone(),
    project_root: json_input.project_root.clone(),
})?;

// Always trust mode
chibi.set_permission_handler(Box::new(|_, _| true));

// Config flag overrides
json_input.flags.verbose = json_input.flags.verbose || chibi.app.config.verbose;
json_input.flags.hide_tool_calls = json_input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
json_input.flags.no_tool_calls = json_input.flags.no_tool_calls || chibi.app.config.no_tool_calls;

let verbose = json_input.flags.verbose;
let context = &json_input.context;

// Initialize (OnStart hooks)
let _ = chibi.init();

// Auto-destroy expired contexts
let destroyed = chibi.app.auto_destroy_expired_contexts(verbose)?;
if !destroyed.is_empty() {
    chibi.save()?;
    output.diagnostic(
        &format!("[Auto-destroyed {} expired context(s)]", destroyed.len()),
        verbose,
    );
}

// Ensure context dir exists
chibi.app.ensure_context_dir(context)?;

// Touch context with debug destroy settings
let debug_destroy_at = json_input.flags.debug.iter().find_map(|k| match k {
    DebugKey::DestroyAt(ts) => Some(*ts),
    _ => None,
});
let debug_destroy_after = json_input.flags.debug.iter().find_map(|k| match k {
    DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
    _ => None,
});
if !chibi.app.state.contexts.iter().any(|e| e.name == *context) {
    chibi.app.state.contexts.push(ContextEntry::with_created_at(
        context.clone(),
        chibi_core::context::now_timestamp(),
    ));
}
if chibi.app.touch_context_with_destroy_settings(context, debug_destroy_at, debug_destroy_after)? {
    chibi.save()?;
}

output.diagnostic(
    &format!("[Loaded {} tool(s)]", chibi.tool_count()),
    verbose,
);

// SYNC: chibi-cli also dispatches commands — check crates/chibi-cli/src/main.rs
execute_json_command(&mut chibi, &json_input, &output).await?;

// Shutdown (OnEnd hooks)
let _ = chibi.shutdown();

// Automatic cache cleanup
let resolved = chibi.resolve_config(context, None)?;
if resolved.auto_cleanup_cache {
    let removed = chibi.app.cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
    if removed > 0 {
        output.diagnostic(
            &format!(
                "[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                removed, resolved.tool_cache_max_age_days + 1
            ),
            verbose,
        );
    }
}

Ok(())
```

**Step 2: Write `execute_json_command()`**

Add to `main.rs`. Key differences from CLI's `execute_from_input`:
- No session/context selection logic
- No markdown rendering
- `Inspect` handles each variant directly via core methods
- `SetSystemPrompt` uses file-or-string logic inline
- `ShowLog` emits raw transcript entries as JSONL
- `DestroyContext` auto-confirms (trust mode)

```rust
async fn execute_json_command(
    chibi: &mut Chibi,
    input: &input::JsonInput,
    output: &dyn OutputSink,
) -> io::Result<()> {
    let verbose = input.flags.verbose;
    let context = &input.context;

    match &input.command {
        Command::ShowHelp => {
            output.emit_result("Use --json-schema to see the input schema.");
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi-json {}", env!("CARGO_PKG_VERSION")));
        }
        Command::ListContexts => {
            let contexts = chibi.list_contexts();
            for name in contexts {
                let context_dir = chibi.app.context_dir(&name);
                let status = chibi_core::lock::ContextLock::get_status(
                    &context_dir, chibi.app.config.lock_heartbeat_seconds,
                );
                let marker = if &name == context { "* " } else { "  " };
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                output.emit_result(&format!("{}{}{}", marker, name, status_str));
            }
        }
        Command::ListCurrentContext => {
            let ctx = chibi.app.get_or_create_context(context)?;
            let context_dir = chibi.app.context_dir(context);
            let status = chibi_core::lock::ContextLock::get_status(
                &context_dir, chibi.app.config.lock_heartbeat_seconds,
            );
            let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
            output.emit_result(&format!("Context: {}{}", context, status_str));
            output.emit_result(&format!("Messages: {}", ctx.messages.len()));
            if !ctx.summary.is_empty() {
                output.emit_result(&format!(
                    "Summary: {}", ctx.summary.lines().next().unwrap_or("")
                ));
            }
        }
        Command::DestroyContext { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            if !chibi.app.context_dir(ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else {
                // Trust mode: auto-confirm
                chibi.app.destroy_context(ctx_name)?;
                output.emit_result(&format!("Destroyed context: {}", ctx_name));
            }
        }
        Command::ArchiveHistory { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.app.clear_context(ctx_name)?;
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)", ctx_name
            ));
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                api::compact_context_by_name(&chibi.app, ctx_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                let resolved = chibi.resolve_config(context, None)?;
                api::compact_context_with_llm_manual(
                    &chibi.app, context, &resolved, verbose,
                ).await?;
                output.emit_result(&format!("Context '{}' compacted", context));
            }
        }
        Command::RenameContext { old, new } => {
            let old_name = old.as_deref().unwrap_or(context);
            chibi.app.rename_context(old_name, new)?;
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
        }
        Command::ShowLog { context: ctx, count } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            let entries = chibi.app.read_jsonl_transcript(ctx_name)?;
            let selected: Vec<_> = if *count == 0 {
                entries.iter().collect()
            } else if *count > 0 {
                let n = *count as usize;
                entries.iter().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect()
            } else {
                let n = (-*count) as usize;
                entries.iter().take(n).collect()
            };
            for entry in selected {
                output.emit_entry(entry)?;
            }
        }
        Command::Inspect { context: ctx, thing } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            match thing {
                Inspectable::List => {
                    // Emit known inspectable names
                    let names = ["system_prompt", "reflection", "todos", "goals", "home"];
                    for name in &names {
                        output.emit_result(name);
                    }
                    output.emit_result("config.<field> (use 'config.list' to see fields)");
                }
                Inspectable::SystemPrompt => {
                    let prompt = chibi.app.load_system_prompt_for(ctx_name)?;
                    if prompt.is_empty() {
                        output.emit_result("(no system prompt set)");
                    } else {
                        output.emit_result(prompt.trim_end());
                    }
                }
                Inspectable::Reflection => {
                    let reflection = chibi.app.load_reflection()?;
                    if reflection.is_empty() {
                        output.emit_result("(no reflection set)");
                    } else {
                        output.emit_result(reflection.trim_end());
                    }
                }
                Inspectable::Todos => {
                    let todos = chibi.app.load_todos_for(ctx_name)?;
                    if todos.is_empty() {
                        output.emit_result("(no todos)");
                    } else {
                        output.emit_result(todos.trim_end());
                    }
                }
                Inspectable::Goals => {
                    let goals = chibi.app.load_goals_for(ctx_name)?;
                    if goals.is_empty() {
                        output.emit_result("(no goals)");
                    } else {
                        output.emit_result(goals.trim_end());
                    }
                }
                Inspectable::Home => {
                    output.emit_result(&chibi.home_dir().display().to_string());
                }
                Inspectable::ConfigField(field_path) => {
                    let resolved = chibi.resolve_config(ctx_name, input.username.as_deref())?;
                    match resolved.get_field(field_path) {
                        Some(value) => output.emit_result(&value),
                        None => output.emit_result("(not set)"),
                    }
                }
            }
        }
        Command::SetSystemPrompt { context: ctx, prompt } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            let content = if std::path::Path::new(prompt).is_file() {
                std::fs::read_to_string(prompt)?
            } else {
                prompt.clone()
            };
            chibi.app.set_system_prompt_for(ctx_name, &content)?;
            output.emit_result(&format!("System prompt set for context '{}'", ctx_name));
        }
        Command::RunPlugin { name, args } => {
            let tool = tools::find_tool(&chibi.tools, name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("Plugin '{}' not found", name))
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = tools::execute_tool(tool, &args_json, verbose)?;
            output.emit_result(&result);
        }
        Command::CallTool { name, args } => {
            let args_str = args.join(" ");
            let args_json: serde_json::Value = if args_str.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&args_str).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid JSON arguments: {}", e),
                    )
                })?
            };
            let result = chibi.execute_tool(context, name, args_json.clone())?;

            if input.flags.force_call_agent {
                let tool_context = format!(
                    "[User initiated tool call: {}]\n[Arguments: {}]\n[Result: {}]",
                    name, args_json, result
                );
                let mut resolved = chibi.resolve_config(context, input.username.as_deref())?;
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(context);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir, chibi.app.config.lock_heartbeat_seconds,
                )?;
                let fallback = chibi_core::tools::HandoffTarget::Agent {
                    prompt: String::new(),
                };
                let options = PromptOptions::new(
                    verbose, use_reflection, &input.flags.debug, false,
                ).with_fallback(fallback);
                let mut sink = sink::JsonResponseSink::new();
                chibi.send_prompt_streaming(
                    context, &tool_context, &resolved, &options, &mut sink,
                ).await?;
            } else {
                output.emit_result(&result);
            }
        }
        Command::ClearCache { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.app.clear_tool_cache(ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(context, None)?;
            let removed = chibi.app.cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
            output.emit_result(&format!(
                "Removed {} old cache entries (older than {} days)",
                removed, resolved.tool_cache_max_age_days
            ));
        }
        Command::SendPrompt { prompt } => {
            if !chibi.app.context_dir(context).exists() {
                let new_context = Context::new(context.clone());
                chibi.app.save_and_register_context(&new_context)?;
            }
            let mut resolved = chibi.resolve_config(context, input.username.as_deref())?;
            if input.flags.no_tool_calls {
                resolved.no_tool_calls = true;
            }
            let use_reflection = resolved.reflection_enabled;
            let context_dir = chibi.app.context_dir(context);
            let _lock = chibi_core::lock::ContextLock::acquire(
                &context_dir, chibi.app.config.lock_heartbeat_seconds,
            )?;
            let options = PromptOptions::new(
                verbose, use_reflection, &input.flags.debug, false,
            );
            let mut sink = sink::JsonResponseSink::new();
            chibi.send_prompt_streaming(
                context, prompt, &resolved, &options, &mut sink,
            ).await?;
        }
        Command::CheckInbox { context: ctx } => {
            let messages = chibi.app.peek_inbox(ctx)?;
            if messages.is_empty() {
                output.diagnostic(
                    &format!("[No messages in inbox for '{}']", ctx), verbose,
                );
            } else {
                output.diagnostic(
                    &format!(
                        "[Processing {} message(s) from inbox for '{}']",
                        messages.len(), ctx
                    ),
                    verbose,
                );
                if !chibi.app.context_dir(ctx).exists() {
                    let new_context = Context::new(ctx.clone());
                    chibi.app.save_and_register_context(&new_context)?;
                }
                let mut resolved = chibi.resolve_config(ctx, None)?;
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(ctx);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir, chibi.app.config.lock_heartbeat_seconds,
                )?;
                let options = PromptOptions::new(
                    verbose, use_reflection, &input.flags.debug, false,
                );
                let mut sink = sink::JsonResponseSink::new();
                chibi.send_prompt_streaming(
                    ctx, chibi_core::INBOX_CHECK_PROMPT, &resolved, &options, &mut sink,
                ).await?;
            }
        }
        Command::CheckAllInboxes => {
            let contexts = chibi.app.list_contexts();
            let mut processed_count = 0;
            for ctx_name in contexts {
                let messages = chibi.app.peek_inbox(&ctx_name)?;
                if messages.is_empty() {
                    continue;
                }
                output.diagnostic(
                    &format!(
                        "[Processing {} message(s) from inbox for '{}']",
                        messages.len(), ctx_name
                    ),
                    verbose,
                );
                let mut resolved = chibi.resolve_config(&ctx_name, None)?;
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(&ctx_name);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir, chibi.app.config.lock_heartbeat_seconds,
                )?;
                let options = PromptOptions::new(
                    verbose, use_reflection, &input.flags.debug, false,
                );
                let mut sink = sink::JsonResponseSink::new();
                chibi.send_prompt_streaming(
                    &ctx_name, chibi_core::INBOX_CHECK_PROMPT, &resolved, &options, &mut sink,
                ).await?;
                processed_count += 1;
            }
            if processed_count == 0 {
                output.diagnostic("[No messages in any inbox.]", verbose);
            } else {
                output.diagnostic(
                    &format!("[Processed inboxes for {} context(s).]", processed_count),
                    verbose,
                );
            }
        }
        Command::ModelMetadata { model, full } => {
            let resolved = chibi.resolve_config(context, None)?;
            let gateway = chibi_core::gateway::build_gateway(&resolved)?;
            let metadata = chibi_core::model_info::fetch_metadata(&gateway, model).await?;
            output.emit_result(
                chibi_core::model_info::format_model_toml(&metadata, *full).trim_end(),
            );
        }
        Command::NoOp => {}
    }

    Ok(())
}
```

**Step 3: Build and fix**

Run: `cargo build -p chibi-json`

Likely fixes needed during implementation:
- Import paths for types like `ContextEntry`, `now_timestamp`
- Visibility of `chibi.app` fields (already `pub` based on CLI usage)
- `Option<String>` field access patterns on `Command` variants (use `as_deref()` for `&str` conversion)
- `set_permission_handler` signature — check if it takes `Box<dyn Fn>` or something else

The implementer should iterate until compilation succeeds.

**Step 4: Commit**

```
feat(chibi-json): implement command execution

Full command dispatch via JsonOutputSink + JsonResponseSink.
Stateless per invocation, trust mode, JSONL output.
```

---

### Task 8: Add integration tests for chibi-json

**Files:**
- Create: `crates/chibi-json/tests/integration.rs`

**Step 1: Write integration tests**

```rust
use std::process::Command;

/// Helper: run chibi-json with JSON input on stdin
fn run_chibi_json(input: &str) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("Failed to run chibi-json");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

#[test]
fn test_json_schema_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--json-schema")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("JsonInput"));
}

#[test]
fn test_version_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--version")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi-json"));
}

#[test]
fn test_invalid_json_input() {
    let (_, _, success) = run_chibi_json("not json");
    assert!(!success);
}

#[test]
fn test_show_version_command() {
    let input = serde_json::json!({
        "command": {"show_version": {}},
        "context": "default"
    });
    let (stdout, _, success) = run_chibi_json(&input.to_string());
    assert!(success);
    assert!(stdout.contains("chibi-json"));
}
```

**Step 2: Run tests**

Run: `cargo test -p chibi-json`
Expected: PASS

**Step 3: Commit**

```
test(chibi-json): add integration tests

Tests for --json-schema, --version, invalid input, basic commands.
```

---

## Phase 4: Clean Up chibi-cli

### Task 9: Remove JSON paths from chibi-cli

**Files:**
- Modify: `crates/chibi-cli/src/main.rs` — remove `--json-config`, `--json-output`, `--json-schema` handling
- Delete: `crates/chibi-cli/src/json_input.rs`
- Modify: `crates/chibi-cli/src/cli.rs` — remove `--json-config`, `--json-output`, `--json-schema` flags
- Modify: `crates/chibi-cli/src/output.rs` — remove `json_mode` from `OutputHandler`
- Modify: `crates/chibi-cli/Cargo.toml` — remove `schemars` dependency

**Step 1: Remove `--json-schema` early exit from `main()`**

In `main()` (~line 1127), remove the `--json-schema` block.

**Step 2: Remove `--json-config` path from `main()`**

Remove the entire `if is_json_config { ... }` block (~lines 1133-1187). Remove `is_json_config` and `cli_json_output` variables.

**Step 3: Remove `--json-output` flag from CLI parsing**

In `crates/chibi-cli/src/cli.rs`, remove the `--json-output` and `--json-config` and `--json-schema` flag definitions. Remove them from `to_input()` where they set `flags.json_output`.

**Step 4: Remove `json_input` module**

Delete `crates/chibi-cli/src/json_input.rs` and remove `mod json_input;` from `main.rs`.

**Step 5: Simplify `OutputHandler`**

Remove `json_mode: bool` field — CLI is always text mode now.
- `OutputHandler::new(json_mode: bool)` → `OutputHandler::new()` (no args)
- `is_json_mode()` → always returns `false`
- `emit_entry()` → remove JSON branch
- Update all `OutputHandler::new(...)` call sites in `main.rs` (there's one in the normal CLI path)

**Step 6: Remove `schemars` from chibi-cli's `Cargo.toml`**

**Step 7: Run all tests**

Run: `cargo test`
Expected: PASS

**Step 8: Commit**

```
refactor(cli): remove JSON paths from chibi-cli

--json-config, --json-output, --json-schema moved to chibi-json.
OutputHandler simplified to text-only. schemars dependency removed.
```

---

### Task 10: Add SYNC comments and CLI OutputSink marker

**Files:**
- Modify: `crates/chibi-cli/src/main.rs`
- Modify: `crates/chibi-json/src/main.rs`

**Step 1: Add drift prevention comments**

At the top of `execute_from_input()` in chibi-cli:
```rust
// SYNC: chibi-json also dispatches commands — check crates/chibi-json/src/main.rs
```

At the top of `execute_json_command()` in chibi-json:
```rust
// SYNC: chibi-cli also dispatches commands — check crates/chibi-cli/src/main.rs
```

**Step 2: Commit**

```
docs: add SYNC comments for drift prevention between binaries
```

---

## Phase 5: Clean Up Core + Docs

### Task 11: Remove `Flags::json_output` and `Flags::raw` from core

Now that chibi-cli no longer reads `json_output` from `Flags` (removed in task 9) and chibi-json uses `ExecutionFlags`, we can remove the presentation-only fields from core's `Flags`.

**Files:**
- Modify: `crates/chibi-core/src/input.rs`
- Modify: `crates/chibi-cli/src/cli.rs` (stop setting `json_output` and `raw` on `Flags`)
- Modify: `crates/chibi-cli/src/main.rs` (stop reading `input.flags.raw`)
- Modify: `crates/chibi-cli/src/input.rs` (update tests referencing these fields)

**Step 1: Move `raw` to CLI-only**

`raw` is still useful in CLI (disables markdown rendering) but doesn't belong in core's `Flags`. Move it to a separate field on `ChibiInput` or handle it as a local variable extracted from clap args before `execute_from_input`. The implementer should choose the cleanest approach — likely adding `raw: bool` to `ChibiInput` since it's already a CLI-specific type.

**Step 2: Remove `json_output` and `raw` from `Flags`**

Update `Flags` struct in `crates/chibi-core/src/input.rs`. Remove both fields, update `Default` impl, update serialization tests. Update `From<&Flags> for ExecutionFlags` if needed (it shouldn't reference removed fields).

**Step 3: Update chibi-cli**

In `cli.rs` `to_input()`: stop setting `flags.json_output` and `flags.raw`. If `raw` moved to `ChibiInput`, set it there.

In `main.rs` `execute_from_input()`: read `raw` from wherever it moved to instead of `input.flags.raw`.

**Step 4: Run all tests**

Run: `cargo test`
Expected: PASS

**Step 5: Commit**

```
refactor(core): remove json_output and raw from Flags

Presentation-only fields don't belong in core. json_output is dead
(replaced by OutputSink::is_json_mode). raw moved to CLI-only handling.
```

---

### Task 12: Update AGENTS.md, documentation, and file GH issue

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/plans/2026-02-13-milestone-11-implementation.md` (mark task 13 done)

**Step 1: Update AGENTS.md architecture section**

Add chibi-json to the crate topology:
```
chibi-core (library)
    ↑               ↑
chibi-cli (binary)   chibi-json (binary)
```

Add under Architecture:
```
**`crates/chibi-json/`** — Binary crate (JSON-mode, programmatic)
- `main.rs` — Entry point, command dispatch
- `input.rs` — JsonInput (stdin JSON)
- `output.rs` — JsonOutputSink (JSONL OutputSink impl)
- `sink.rs` — JsonResponseSink (JSONL ResponseSink impl)
```

**Step 2: Mark milestone task 13 as done**

Update the progress table in `docs/plans/2026-02-13-milestone-11-implementation.md`.

**Step 3: File GH issue for shared execute_command() extraction**

```bash
gh issue create \
  --title "extract shared execute_command() into chibi-core" \
  --body "$(cat <<'EOF'
Both chibi-cli and chibi-json now have their own command dispatch functions
(`execute_from_input` and `execute_json_command` respectively). There is
significant duplication in the non-presentation command handlers (ListContexts,
DestroyContext, RenameContext, ClearCache, etc.).

**Proposed:** extract the presentation-agnostic command handlers into a shared
`execute_command()` in `chibi-core/src/execution.rs` that takes `&dyn OutputSink`
+ `&dyn ResponseSink`. Each binary would then only handle the commands that
need binary-specific logic (SendPrompt, ShowLog, Inspect for CLI's markdown).

**Blocked on:** seeing the actual duplication pattern after the extraction
stabilizes. Premature extraction risks an awkward API that doesn't fit either
binary well.

**Ref:** design doc `docs/plans/2026-02-14-chibi-json-extraction-design.md`
(execute_command section).
EOF
)" \
  --label enhancement
```

**Step 4: Commit**

```
docs: update architecture for chibi-json extraction
```

---

## Implementation Notes

**Task ordering:** Tasks 1-2 are independent (parallelizable). Task 3 depends on task 2. Task 4 depends on task 1. Task 5 is independent after task 4. Tasks 6-8 depend on tasks 1, 2, 5. Task 9 depends on task 6 (chibi-json must exist before removing JSON from CLI). Tasks 10-12 are cleanup after everything works.

**What's NOT in scope (deferred to GH issue):**
- Extracting shared command dispatch into core `execute_command()` — filed as GH issue. Both binaries have their own dispatch for now; extract when duplication pattern is clear.
- Multi-context orchestration — future concern per design doc.
- `chibi-shared` crate — not needed for v1.
