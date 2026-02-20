# Structured Error Output Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Route chibi-json errors and completion signals to stderr as structured JSONL, leaving stdout as a pure result stream.

**Architecture:** Add `emit_done(&self, result: &io::Result<()>)` to `OutputSink` in chibi-core with a default no-op. `JsonOutputSink` overrides it to emit `{"type":"done","ok":...}` (plus `code`/`message` on failure) to stderr. `main()` in chibi-json calls `emit_done` then exits — removing the old stdout error emission.

**Tech Stack:** Rust, `serde_json::json!`, `io::ErrorKind`, existing `OutputSink` trait.

---

### Task 1: Add `emit_done` to `OutputSink` trait in chibi-core

**Files:**
- Modify: `crates/chibi-core/src/output.rs`

**Step 1: Write a failing test**

In `crates/chibi-core/src/output.rs`, add at the bottom inside the existing `#[cfg(test)]` block (or create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingSink {
        done_called: std::cell::Cell<bool>,
    }

    impl RecordingSink {
        fn new() -> Self { Self { done_called: std::cell::Cell::new(false) } }
    }

    impl OutputSink for RecordingSink {
        fn emit_result(&self, _: &str) {}
        fn emit_event(&self, _: CommandEvent) {}
        fn newline(&self) {}
        fn emit_entry(&self, _: &TranscriptEntry) -> std::io::Result<()> { Ok(()) }
        fn confirm(&self, _: &str) -> bool { true }
        fn emit_done(&self, _: &std::io::Result<()>) {
            self.done_called.set(true);
        }
    }

    #[test]
    fn emit_done_default_is_noop() {
        // NoopSink uses the default impl — calling it must not panic
        let sink = NoopSink;
        sink.emit_done(&Ok(()));
        sink.emit_done(&Err(std::io::Error::new(std::io::ErrorKind::NotFound, "x")));
    }

    #[test]
    fn emit_done_can_be_overridden() {
        let sink = RecordingSink::new();
        sink.emit_done(&Ok(()));
        assert!(sink.done_called.get());
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p chibi-core emit_done 2>&1 | head -20
```

Expected: compile error — `emit_done` not found on trait.

**Step 3: Add `emit_done` to the trait with a default no-op**

In `crates/chibi-core/src/output.rs`, add after the `emit_markdown` default method inside the `OutputSink` trait:

```rust
    /// Signal command completion. Called once, after all output has been emitted.
    ///
    /// Default: no-op — chibi-cli handles completion via its own UX.
    fn emit_done(&self, result: &io::Result<()>) {
        let _ = result;
    }
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p chibi-core emit_done 2>&1
```

Expected: 2 tests pass.

**Step 5: Confirm no other crates broken**

```bash
cargo build 2>&1
```

Expected: clean build (default impl covers all existing `OutputSink` implementors).

**Step 6: Commit**

```bash
git add crates/chibi-core/src/output.rs
git commit -m "feat(core): add emit_done to OutputSink trait with default no-op"
```

---

### Task 2: Implement `emit_done` and `error_code` in `JsonOutputSink`

**Files:**
- Modify: `crates/chibi-json/src/output.rs`

**Step 1: Write failing tests**

Add at the bottom of `crates/chibi-json/src/output.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn error_code_not_found() {
        let e = io::Error::new(io::ErrorKind::NotFound, "x");
        assert_eq!(error_code(&e), "not_found");
    }

    #[test]
    fn error_code_invalid_input() {
        let e = io::Error::new(io::ErrorKind::InvalidInput, "x");
        assert_eq!(error_code(&e), "invalid_input");
    }

    #[test]
    fn error_code_permission_denied() {
        let e = io::Error::new(io::ErrorKind::PermissionDenied, "x");
        assert_eq!(error_code(&e), "permission_denied");
    }

    #[test]
    fn error_code_invalid_data() {
        let e = io::Error::new(io::ErrorKind::InvalidData, "x");
        assert_eq!(error_code(&e), "invalid_data");
    }

    #[test]
    fn error_code_already_exists() {
        let e = io::Error::new(io::ErrorKind::AlreadyExists, "x");
        assert_eq!(error_code(&e), "already_exists");
    }

    #[test]
    fn error_code_fallback() {
        let e = io::Error::new(io::ErrorKind::BrokenPipe, "x");
        assert_eq!(error_code(&e), "internal_error");
    }
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p chibi-json error_code 2>&1 | head -20
```

Expected: compile error — `error_code` not defined.

**Step 3: Add `error_code` helper and `emit_done` impl**

In `crates/chibi-json/src/output.rs`, add after the imports and before the `JsonOutputSink` struct:

```rust
/// Map `io::ErrorKind` to a stable coarse-grained error code string.
fn error_code(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidData => "invalid_data",
        io::ErrorKind::AlreadyExists => "already_exists",
        _ => "internal_error",
    }
}
```

Then add `emit_done` inside `impl OutputSink for JsonOutputSink`, after `emit_markdown`:

```rust
    fn emit_done(&self, result: &io::Result<()>) {
        let json = match result {
            Ok(()) => serde_json::json!({"type": "done", "ok": true}),
            Err(e) => serde_json::json!({
                "type": "done",
                "ok": false,
                "code": error_code(e),
                "message": e.to_string(),
            }),
        };
        eprintln!("{}", json);
    }
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p chibi-json 2>&1
```

Expected: all tests pass including the new `error_code_*` tests.

**Step 5: Commit**

```bash
git add crates/chibi-json/src/output.rs
git commit -m "feat(json): implement emit_done with coarse error codes on stderr"
```

---

### Task 3: Wire `emit_done` in `chibi-json/src/main.rs`

**Files:**
- Modify: `crates/chibi-json/src/main.rs`

**Step 1: Read the current `main` function**

Locate `crates/chibi-json/src/main.rs` lines 9–22. The current body is:

```rust
fn main() {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    match rt.block_on(run()) {
        Ok(()) => {}
        Err(e) => {
            let json = serde_json::json!({
                "type": "error",
                "message": e.to_string(),
            });
            println!("{}", json);
            std::process::exit(1);
        }
    }
}
```

**Step 2: Replace `main` body**

Replace the entire `main` function with:

```rust
fn main() {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let result = rt.block_on(run());
    let output = output::JsonOutputSink;
    output.emit_done(&result);
    if result.is_err() {
        std::process::exit(1);
    }
}
```

**Step 3: Build to verify**

```bash
cargo build -p chibi-json 2>&1
```

Expected: clean build.

**Step 4: Smoke test — success path**

```bash
echo '{"command":"list_contexts","context":"default"}' | cargo run -p chibi-json 2>/tmp/chibi-stderr.json
echo "stdout exit: $?"
cat /tmp/chibi-stderr.json | tail -1
```

Expected:
- stdout: zero or more result lines
- stderr last line: `{"ok":true,"type":"done"}`
- exit code: 0

**Step 5: Smoke test — error path**

```bash
echo 'not valid json' | cargo run -p chibi-json 2>/tmp/chibi-stderr-err.json; echo "exit: $?"
cat /tmp/chibi-stderr-err.json | tail -1
```

Expected:
- stdout: empty
- stderr last line: `{"code":"invalid_input","message":"Invalid JSON input: ...","ok":false,"type":"done"}`
- exit code: 1

**Step 6: Commit**

```bash
git add crates/chibi-json/src/main.rs
git commit -m "feat(json): wire emit_done in main, remove old stdout error emission"
```

---

### Task 4: Update docs

**Files:**
- Modify: `docs/hooks.md` or relevant section in `docs/architecture.md` that describes chibi-json output format

**Step 1: Find the relevant doc section**

```bash
grep -n "stderr\|stdout\|error\|JSONL" docs/architecture.md | head -30
grep -n "stderr\|stdout\|error\|JSONL" docs/hooks.md | head -30
```

**Step 2: Update to reflect new stdout/stderr contract**

Find the chibi-json output format description and update it to document:

- stdout: result stream (`result`, transcript entries) — silent on error
- stderr: diagnostic stream (events, `done` signal)
- `done` format on success: `{"type":"done","ok":true}`
- `done` format on failure: `{"type":"done","ok":false,"code":"...","message":"..."}`
- error codes: `not_found`, `invalid_input`, `permission_denied`, `invalid_data`, `already_exists`, `internal_error`

**Step 3: Commit**

```bash
git add docs/
git commit -m "docs: update chibi-json stdout/stderr contract and done signal"
```

---

### Task 5: Close issue

**Step 1: Comment on #149 with what landed**

```bash
gh issue comment 149 --body "## landed on dev

- errors no longer emitted on stdout
- stderr is now the diagnostic channel: events + terminal \`done\` signal
- \`{\"type\":\"done\",\"ok\":true}\` on success
- \`{\"type\":\"done\",\"ok\":false,\"code\":\"...\",\"message\":\"...\"}\` on failure
- coarse error codes from \`io::ErrorKind\`: \`not_found\`, \`invalid_input\`, \`permission_denied\`, \`invalid_data\`, \`already_exists\`, \`internal_error\`
- fine-grained semantic codes (e.g. \`context_not_found\`) remain future work requiring proper error variants in chibi-core"
```

**Step 2: Close issue if fully resolved (or leave open for fine-grained codes)**

Leave open — fine-grained codes remain tracked per the existing comment.
