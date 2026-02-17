# Remove is_json_mode() Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove `is_json_mode()` from `OutputSink` and `ResponseSink` traits, moving presentation logic to sink implementations.

**Architecture:** Core emits structured data unconditionally via `emit_entry()` and response events. Each sink (CLI/JSON) handles formatting internally. No runtime format queries — the type system handles it.

**Tech Stack:** Rust, cargo workspace (chibi-core, chibi-cli, chibi-json)

---

### Task 1: Remove `is_json_mode()` from `OutputSink` trait

**Files:**
- Modify: `crates/chibi-core/src/output.rs:23-24` (remove method)
- Modify: `crates/chibi-core/src/output.rs:20-21` (update `emit_entry` doc)

**Step 1: Remove `is_json_mode` from the trait and update `emit_entry` doc**

In `crates/chibi-core/src/output.rs`, remove the `is_json_mode` method from the `OutputSink` trait and update the `emit_entry` doc comment:

```rust
    /// Emit a transcript entry for display.
    ///
    /// Each sink formats entries appropriately for its output medium:
    /// CLI renders human-readable text, JSON emits structured JSONL.
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;
```

Remove:
```rust
    /// Whether this sink operates in JSON mode (affects downstream formatting).
    fn is_json_mode(&self) -> bool;
```

**Step 2: Verify it doesn't compile (expected — impls still reference it)**

Run: `cargo check -p chibi-core 2>&1 | head -20`
Expected: Compiles (trait itself is fine, impls are in other crates)

---

### Task 2: Remove `is_json_mode()` from `ResponseSink` trait

**Files:**
- Modify: `crates/chibi-core/src/api/sink.rs:90-96` (remove method + default impl)
- Modify: `crates/chibi-core/src/api/sink.rs` (remove test)

**Step 1: Remove `is_json_mode` from the trait**

In `crates/chibi-core/src/api/sink.rs`, remove the method and its doc comment from the `ResponseSink` trait:

```rust
    /// Returns true if the sink is in JSON output mode.
    ///
    /// When in JSON mode, text chunks should typically not be streamed
    /// to the terminal, as the output will be formatted as JSON instead.
    fn is_json_mode(&self) -> bool {
        false
    }
```

**Step 2: Remove the `test_is_json_mode_default` test**

Remove from the `tests` module:
```rust
    #[test]
    fn test_is_json_mode_default() {
        let sink = CollectingSink::new();
        assert!(!sink.is_json_mode());
    }
```

**Step 3: Verify core compiles**

Run: `cargo check -p chibi-core 2>&1 | head -20`
Expected: PASS (send.rs will fail — that's task 4)

Actually expected: FAIL because `send.rs` still calls `sink.is_json_mode()`. That's fine — tasks 1-3 set up the trait changes, task 4 fixes the call sites.

---

### Task 3: Remove `is_json_mode()` from all sink implementations

**Files:**
- Modify: `crates/chibi-cli/src/output.rs:54-56` (remove from `OutputHandler`)
- Modify: `crates/chibi-cli/src/sink.rs:144-146` (remove from `CliResponseSink`)
- Modify: `crates/chibi-json/src/output.rs:39-41` (remove from `JsonOutputSink`)
- Modify: `crates/chibi-json/src/sink.rs:70-72` (remove from `JsonResponseSink`)

**Step 1: Remove from `OutputHandler` impl**

In `crates/chibi-cli/src/output.rs`, remove:
```rust
    fn is_json_mode(&self) -> bool {
        false
    }
```

**Step 2: Remove from `CliResponseSink` impl**

In `crates/chibi-cli/src/sink.rs`, remove:
```rust
    fn is_json_mode(&self) -> bool {
        self.output.is_json_mode()
    }
```

**Step 3: Remove from `JsonOutputSink` impl**

In `crates/chibi-json/src/output.rs`, remove:
```rust
    fn is_json_mode(&self) -> bool {
        true
    }
```

**Step 4: Remove from `JsonResponseSink` impl**

In `crates/chibi-json/src/sink.rs`, remove:
```rust
    fn is_json_mode(&self) -> bool {
        true
    }
```

---

### Task 4: Remove `is_json_mode()` guards in `send.rs`

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:619` (remove `json_mode` variable)
- Modify: `crates/chibi-core/src/api/send.rs:642` (remove guard around `TextChunk`)
- Modify: `crates/chibi-core/src/api/send.rs:1909` (remove guard around `Finished`)

**Step 1: Remove `json_mode` variable and guard around `TextChunk`**

In `collect_streaming_response`, change:
```rust
    let json_mode = sink.is_json_mode();
```
to: (delete the line entirely)

And change:
```rust
                full_response.push_str(&text);
                if !json_mode {
                    sink.handle(ResponseEvent::TextChunk(&text))?;
                }
```
to:
```rust
                full_response.push_str(&text);
                sink.handle(ResponseEvent::TextChunk(&text))?;
```

**Step 2: Remove guard around `Finished`**

Change:
```rust
            // Signal streaming finished
            if !sink.is_json_mode() {
                sink.handle(ResponseEvent::Finished)?;
            }
```
to:
```rust
            // Signal streaming finished
            sink.handle(ResponseEvent::Finished)?;
```

**Step 3: Verify core compiles**

Run: `cargo check -p chibi-core`
Expected: PASS

---

### Task 5: Simplify `show_log` in core

**Files:**
- Modify: `crates/chibi-core/src/execution.rs:468-559`

**Step 1: Replace `show_log` body**

Replace the entire function with:
```rust
/// Show log entries for a context.
///
/// Selects entries by count and emits each via `emit_entry()`.
/// Formatting is the responsibility of the sink implementation.
fn show_log(
    chibi: &Chibi,
    context: &str,
    count: isize,
    output: &dyn OutputSink,
) -> io::Result<()> {
    let entries = chibi.app.read_jsonl_transcript(context)?;

    let selected: Vec<_> = if count == 0 {
        entries.iter().collect()
    } else if count > 0 {
        let n = count as usize;
        entries
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        let n = (-count) as usize;
        entries.iter().take(n).collect()
    };

    for entry in selected {
        output.emit_entry(entry)?;
    }
    Ok(())
}
```

**Step 2: Update the call site to drop `verbose`**

In `dispatch_command` (~line 241), change:
```rust
            show_log(chibi, ctx_name, *count, verbose, output)?;
```
to:
```rust
            show_log(chibi, ctx_name, *count, output)?;
```

**Step 3: Verify core compiles**

Run: `cargo check -p chibi-core`
Expected: PASS

---

### Task 6: Make `OutputHandler` stateful with `verbose`

**Files:**
- Modify: `crates/chibi-cli/src/output.rs`
- Modify: `crates/chibi-cli/src/main.rs:529`

**Step 1: Add `verbose` field to `OutputHandler`**

Change struct and constructors:
```rust
/// CLI output handler — text to stdout, diagnostics to stderr.
///
/// Implements `OutputSink` directly; all output goes through trait methods.
/// Always operates in text mode — JSON output belongs to chibi-json.
pub struct OutputHandler {
    verbose: bool,
}

impl Default for OutputHandler {
    fn default() -> Self {
        Self { verbose: false }
    }
}

impl OutputHandler {
    /// Create a new output handler.
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}
```

**Step 2: Implement `emit_entry` with human-readable formatting**

Replace the `emit_entry` impl:
```rust
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()> {
        use chibi_core::context;

        match entry.entry_type.as_str() {
            context::ENTRY_TYPE_MESSAGE => {
                self.emit_result(&format!("[{}]", entry.from.to_uppercase()));
                self.emit_markdown(&entry.content)?;
                self.newline();
            }
            context::ENTRY_TYPE_TOOL_CALL => {
                if self.verbose {
                    self.emit_result(&format!("[TOOL CALL: {}]\n{}\n", entry.to, entry.content));
                } else {
                    let args_preview = if entry.content.len() > 60 {
                        format!("{}...", &entry.content[..60])
                    } else {
                        entry.content.clone()
                    };
                    self.emit_result(&format!("[TOOL: {}] {}", entry.to, args_preview));
                }
            }
            context::ENTRY_TYPE_TOOL_RESULT => {
                if self.verbose {
                    self.emit_result(&format!(
                        "[TOOL RESULT: {}]\n{}\n",
                        entry.from, entry.content
                    ));
                } else {
                    let size = entry.content.len();
                    let size_str = if size > 1024 {
                        format!("{:.1}kb", size as f64 / 1024.0)
                    } else {
                        format!("{}b", size)
                    };
                    self.emit_result(&format!("  -> {}", size_str));
                }
            }
            "compaction" => {
                if self.verbose {
                    self.emit_result(&format!("[COMPACTION]: {}\n", entry.content));
                }
            }
            _ => {
                if self.verbose {
                    self.emit_result(&format!(
                        "[{}]: {}\n",
                        entry.entry_type.to_uppercase(),
                        entry.content
                    ));
                }
            }
        }
        Ok(())
    }
```

**Step 3: Update `main.rs` to pass `verbose`**

In `crates/chibi-cli/src/main.rs`, change:
```rust
    let output = OutputHandler::new();
```
to:
```rust
    let output = OutputHandler::new(verbose);
```

**Step 4: Verify CLI compiles**

Run: `cargo check -p chibi-cli`
Expected: FAIL — tests still use `OutputHandler::new()` without args. That's task 8.

---

### Task 7: Verify `chibi-json` compiles

**Files:** (no changes needed — just verification)

**Step 1: Check compilation**

Run: `cargo check -p chibi-json`
Expected: PASS (JsonOutputSink/JsonResponseSink already had `is_json_mode` removed in task 3, and `emit_entry` is unchanged)

---

### Task 8: Update tests

**Files:**
- Modify: `crates/chibi-cli/src/output.rs` (tests module)
- Modify: `crates/chibi-cli/src/sink.rs` (tests module)

**Step 1: Update output.rs tests**

Replace the entire `#[cfg(test)]` module in `crates/chibi-cli/src/output.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_entry_message() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "user".to_string(),
            to: "assistant".to_string(),
            content: "Hello".to_string(),
            entry_type: "message".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Should not panic — formats as human-readable text
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_call_compact() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "assistant".to_string(),
            to: "shell_exec".to_string(),
            content: r#"{"command":"ls"}"#.to_string(),
            entry_type: "tool_call".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Compact mode: shows tool name + truncated args
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_call_verbose() {
        let handler = OutputHandler::new(true);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "assistant".to_string(),
            to: "shell_exec".to_string(),
            content: r#"{"command":"ls -la"}"#.to_string(),
            entry_type: "tool_call".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Verbose mode: shows full content
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_result_compact() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "shell_exec".to_string(),
            to: "assistant".to_string(),
            content: "file1.rs\nfile2.rs\n".to_string(),
            entry_type: "tool_result".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Compact mode: shows size only
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_skips_compaction_when_not_verbose() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "system".to_string(),
            to: "system".to_string(),
            content: "compacted 10 entries".to_string(),
            entry_type: "compaction".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Non-verbose: compaction entries are silently skipped
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_diagnostic_verbose_false() {
        let handler = OutputHandler::new(false);
        // Should not panic (no output when verbose is false)
        handler.diagnostic("Test message", false);
    }

    #[test]
    fn test_diagnostic_verbose_true() {
        let handler = OutputHandler::new(false);
        // Should not panic
        handler.diagnostic("Test message", true);
    }
}
```

**Step 2: Update sink.rs tests**

In `crates/chibi-cli/src/sink.rs`, remove the `test_is_json_mode_normal` test and update all `OutputHandler::new()` calls to `OutputHandler::new(false)`:

Remove:
```rust
    #[test]
    fn test_is_json_mode_normal() {
        let output = OutputHandler::new();
        let sink = CliResponseSink::new(&output, None, false, true, false);
        assert!(!sink.is_json_mode());
    }
```

Replace all occurrences of `OutputHandler::new()` with `OutputHandler::new(false)` in the tests module.

**Step 3: Run all tests**

Run: `cargo test`
Expected: PASS

---

### Task 9: Final verification and commit

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

**Step 2: Run clippy**

Run: `cargo clippy --all-targets`
Expected: No warnings

**Step 3: Commit**

```bash
git add -A
git commit -m "refactor: remove is_json_mode() from OutputSink/ResponseSink

Move presentation logic to sink implementations. Core now emits
structured data unconditionally; each sink formats as appropriate.

OutputHandler gains a verbose field and handles human-readable
transcript formatting in emit_entry(). show_log simplified to
entry selection only.

Resolves #148"
```
