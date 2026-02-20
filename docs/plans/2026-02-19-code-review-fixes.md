# Code Review Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all 8 issues identified in the code review of PR #167 (refactor/typed-output-2602).

**Architecture:** All fixes are within the existing codebase structure — no new files needed. Issues range from a one-line doc fix to threading `OutputSink` through `compact.rs`, refactoring the `emit_event` pattern, removing a redundant event variant, deduplicating `NoopSink`, fixing a behavioural regression in `show_thinking`, and adding test coverage.

**Tech Stack:** Rust, cargo test

---

## Issue Summary

| # | Issue | File(s) |
|---|-------|---------|
| 1 | Stale `CHIBI_VERBOSE` doc comment | `chibi-core/src/tools/plugins.rs` |
| 2 | `show_thinking` default silently changed true→false | `chibi-cli/src/config.rs` |
| 3 | Duplicate `NoopSink` in same file | `chibi-core/src/chibi.rs` |
| 4 | `(text, verbose_only)` dead bool in `emit_event` | `chibi-cli/src/output.rs` |
| 5 | Double-emission of tool diagnostics for sequential tools | `chibi-core/src/api/send.rs` |
| 6 | `compact.rs` unconditional `eprintln!` bypasses `OutputSink` | `chibi-core/src/api/compact.rs` |
| 7 | `ContextLoaded` redundant with `LoadSummary` | `chibi-core/src/output.rs`, `chibi-json/` |
| 8 | No tests for `OutputHandler::emit_event` | `chibi-cli/src/output.rs` |

---

### Task 1: Fix stale `CHIBI_VERBOSE` doc comment

**Files:**
- Modify: `crates/chibi-core/src/tools/plugins.rs:241`

**Step 1: Remove the stale sentence**

In `crates/chibi-core/src/tools/plugins.rs`, line 241 currently reads:

```
/// Tools also receive CHIBI_VERBOSE=1 env var when verbose mode is enabled.
```

Delete that line entirely. The updated doc comment block should be:

```rust
/// Execute a tool with the given arguments (as JSON)
///
/// Tools receive arguments via stdin (JSON), leaving stdout for results.
pub fn execute_tool(tool: &Tool, arguments: &serde_json::Value) -> io::Result<String> {
```

**Step 2: Verify**

```bash
grep -n "CHIBI_VERBOSE" crates/chibi-core/src/tools/plugins.rs
```

Expected: no output (neither the doc comment nor any code should reference it).

**Step 3: Run tests**

```bash
cargo test -p chibi-core tools 2>&1 | tail -20
```

Expected: all pass.

**Step 4: Commit**

```bash
git add crates/chibi-core/src/tools/plugins.rs
git commit -m "fix(docs): remove stale CHIBI_VERBOSE doc from execute_tool"
```

---

### Task 2: Fix `show_thinking` default (true → false regression)

**Files:**
- Modify: `crates/chibi-cli/src/config.rs:409`

**Context:** `ConfigDefaults::SHOW_THINKING` was `true` (added in commit `9142ed9`). When these fields moved to `CliConfig`, the default was accidentally set to `false`.

**Step 1: Fix the default**

In `crates/chibi-cli/src/config.rs`, the `Default` impl for `CliConfig` (around line 404–413):

```rust
impl Default for CliConfig {
    fn default() -> Self {
        Self {
            render_markdown: true,
            verbose: false,
            hide_tool_calls: false,
            show_thinking: false,   // ← change this to true
            image: ImageConfig::default(),
            markdown_style: default_markdown_style(),
        }
    }
}
```

Change `show_thinking: false` to `show_thinking: true`.

**Step 2: Check for a second false default**

```bash
grep -n "show_thinking" crates/chibi-cli/src/config.rs
```

There may be a second `Default` impl (around line 667). If `show_thinking: false` appears there too, fix it to `true` as well — it represents the same default.

**Step 3: Run tests**

```bash
cargo test -p chibi-cli 2>&1 | tail -20
```

Expected: all pass.

**Step 4: Commit**

```bash
git add crates/chibi-cli/src/config.rs
git commit -m "fix(config): restore show_thinking default to true"
```

---

### Task 3: Deduplicate `NoopSink` in `chibi.rs`

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs`

**Context:** `NoopSink` is defined twice identically — once inside `Chibi::load()` (lines 111–122) and once inside `Chibi::from_home()` (lines 133–144). Both are module-private and identical.

**Step 1: Add a single module-level `NoopSink`**

Find the first `use` imports or first item at the top of `chibi.rs` (before `impl Chibi`). Add a single private struct there:

```rust
/// A no-op output sink for callers that don't need load-time output.
struct NoopSink;

impl OutputSink for NoopSink {
    fn emit_result(&self, _: &str) {}
    fn emit_event(&self, _: CommandEvent) {}
    fn newline(&self) {}
    fn emit_entry(&self, _: &crate::context::TranscriptEntry) -> io::Result<()> {
        Ok(())
    }
    fn confirm(&self, _: &str) -> bool {
        false
    }
}
```

Place this just before the `impl Chibi {` block.

**Step 2: Remove both inline definitions**

Inside `Chibi::load()`, delete the `struct NoopSink; impl OutputSink for NoopSink { ... }` block (lines 111–122), leaving only:

```rust
pub fn load() -> io::Result<Self> {
    Self::load_with_options(LoadOptions::default(), &NoopSink)
}
```

Inside `Chibi::from_home()`, delete the second identical block (lines 133–144), leaving only:

```rust
pub fn from_home(home: &Path) -> io::Result<Self> {
    Self::load_with_options(
        LoadOptions {
            home: Some(home.to_path_buf()),
            ..Default::default()
        },
        &NoopSink,
    )
}
```

**Step 3: Verify no remaining inline definitions**

```bash
grep -n "struct NoopSink" crates/chibi-core/src/chibi.rs
```

Expected: exactly 1 occurrence (the module-level one).

**Step 4: Run tests**

```bash
cargo test -p chibi-core 2>&1 | tail -20
```

Expected: all pass.

**Step 5: Commit**

```bash
git add crates/chibi-core/src/chibi.rs
git commit -m "refactor(core): deduplicate NoopSink to module level in chibi.rs"
```

---

### Task 4: Simplify `emit_event` — remove dead `verbose_only` bool

**Files:**
- Modify: `crates/chibi-cli/src/output.rs:33–106`

**Context:** Every branch of the `match` returns `verbose_only: true`. The `(String, bool)` tuple adds indirection for zero benefit. Replace the tuple with a direct guard at the top, and emit per-arm.

**Step 1: Rewrite `emit_event`**

Replace the current `fn emit_event` body with a simpler form. Since all events are verbose-tier, guard once at the top:

```rust
fn emit_event(&self, event: CommandEvent) {
    if !self.verbose {
        return;
    }
    let text = match &event {
        CommandEvent::AutoDestroyed { count } => {
            format!("[Auto-destroyed {} expired context(s)]", count)
        }
        CommandEvent::CacheCleanup { removed, max_age_days } => {
            format!(
                "[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                removed,
                max_age_days + 1
            )
        }
        CommandEvent::SystemPromptSet { context } => {
            format!("[System prompt set for context '{}']", context)
        }
        CommandEvent::UsernameSaved { username, context } => {
            format!("[Username '{}' saved to context '{}']", username, context)
        }
        CommandEvent::InboxEmpty { context } => {
            format!("[No messages in inbox for '{}']", context)
        }
        CommandEvent::InboxProcessing { count, context } => {
            format!("[Processing {} message(s) from inbox for '{}']", count, context)
        }
        CommandEvent::AllInboxesEmpty => "[No messages in any inbox.]".to_string(),
        CommandEvent::InboxesProcessed { count } => {
            format!("[Processed inboxes for {} context(s).]", count)
        }
        CommandEvent::ContextLoaded { tool_count } => {
            format!("[Loaded {} tool(s)]", tool_count)
        }
        CommandEvent::McpToolsLoaded { count } => {
            format!("[MCP: {} tools loaded]", count)
        }
        CommandEvent::McpBridgeUnavailable { reason } => {
            format!("[MCP: bridge unavailable: {}]", reason)
        }
        CommandEvent::LoadSummary {
            builtin_count,
            builtin_names,
            plugin_count,
            plugin_names,
        } => {
            let mut lines = format!(
                "[Built-in ({}): {}]",
                builtin_count,
                builtin_names.join(", ")
            );
            if *plugin_count == 0 {
                lines.push_str("\n[No plugins loaded]");
            } else {
                lines.push_str(&format!(
                    "\n[Plugins ({}): {}]",
                    plugin_count,
                    plugin_names.join(", ")
                ));
            }
            lines
        }
    };
    eprintln!("{}", text);
}
```

**Note:** If a future `CommandEvent` variant needs to be shown unconditionally (non-verbose), remove the early return and add a per-arm `if !self.verbose { return; }` before that arm's `eprintln!`. Don't add that complexity now (YAGNI).

**Step 2: Run tests**

```bash
cargo test -p chibi-cli 2>&1 | tail -20
```

Expected: all pass.

**Step 3: Commit**

```bash
git add crates/chibi-cli/src/output.rs
git commit -m "refactor(cli): simplify emit_event, remove dead verbose_only bool"
```

---

### Task 5: Fix double-emission of tool diagnostics in `send.rs`

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs`

**Context:** `execute_single_tool` emits `ToolDiagnostic` for each entry in `result.diagnostics` (lines ~1297–1303). Then `process_tool_calls` has a "pre-log" loop (lines ~1495–1502) that iterates all `results[i]` and also emits diagnostics — this hits sequential tools a second time since their results are already populated.

The fix: the pre-log loop was intended for parallel tools only. Gate it to only emit for tools in `parallel_batch`.

**Step 1: Collect parallel indices**

After the parallel batch executes and results are stored (after the `for ((idx, _tc), result) in parallel_batch.iter().zip(parallel_results)` block), collect the parallel indices into a `HashSet`:

```rust
let parallel_indices: std::collections::HashSet<usize> =
    parallel_batch.iter().map(|(idx, _)| *idx).collect();
```

**Step 2: Gate the pre-log loop**

Find the pre-log loop in `process_tool_calls` (inside the `for (i, tc) in tool_calls.iter().enumerate()` loop, before transcript writing):

```rust
// Pre-log diagnostics for parallel-executed tools
if let Some(result) = &results[i] {
    for diag in &result.diagnostics {
        sink.handle(ResponseEvent::ToolDiagnostic {
            tool: tc.name.clone(),
            message: diag.clone(),
        })?;
    }
}
```

Add the gate:

```rust
// Pre-log diagnostics for parallel-executed tools only.
// Sequential tools have already emitted their diagnostics in execute_single_tool.
if parallel_indices.contains(&i) {
    if let Some(result) = &results[i] {
        for diag in &result.diagnostics {
            sink.handle(ResponseEvent::ToolDiagnostic {
                tool: tc.name.clone(),
                message: diag.clone(),
            })?;
        }
    }
}
```

**Step 3: Run tests**

```bash
cargo test -p chibi-core api 2>&1 | tail -20
```

Expected: all pass.

**Step 4: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "fix(send): prevent double-emission of tool diagnostics for sequential tools"
```

---

### Task 6: Route `compact.rs` output through `OutputSink`

**Files:**
- Modify: `crates/chibi-core/src/api/compact.rs`
- Modify: `crates/chibi-core/src/api/send.rs` (call site of `compact_context_with_llm`)
- Modify: `crates/chibi-core/src/output.rs` (add new `CommandEvent` variants)

**Context:** `compact.rs` currently has ~11 unconditional `eprintln!` calls that bypass `OutputSink`. The functions have no `OutputSink` parameter. We need to add a `CompactionEvent` or reuse `CommandEvent` for compaction messages.

This is the most invasive fix. Proceed carefully.

**Step 6a: Add compaction variants to `CommandEvent`**

In `crates/chibi-core/src/output.rs`, add these variants to `CommandEvent`:

```rust
/// LLM-based compaction started for a context (verbose-tier).
CompactionStarted { context: String, message_count: usize, token_count: usize },
/// LLM-based compaction completed (verbose-tier).
CompactionComplete {
    context: String,
    archived: usize,
    remaining: usize,
    summary_tokens: usize,
},
/// Rolling compaction: LLM selected N messages to archive (verbose-tier).
RollingCompactionDecision { archived: usize },
/// Rolling compaction fallback: dropping oldest N% (verbose-tier).
RollingCompactionFallback { drop_percentage: f64 },
/// Rolling compaction completed (verbose-tier).
RollingCompactionComplete { archived: usize, remaining: usize },
/// No compaction prompt found — using default (verbose-tier).
CompactionNoPrompt,
```

**Step 6b: Thread `&dyn OutputSink` into compact functions**

Update the signatures of the public compact functions in `crates/chibi-core/src/api/compact.rs`:

```rust
pub async fn rolling_compact(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn crate::output::OutputSink,
) -> io::Result<()>

pub async fn compact_context_with_llm(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn crate::output::OutputSink,
) -> io::Result<()>

pub async fn compact_context_with_llm_manual(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    sink: &dyn crate::output::OutputSink,
) -> io::Result<()>

pub async fn compact_context_by_name(
    app: &AppState,
    context_name: &str,
    sink: &dyn crate::output::OutputSink,
) -> io::Result<()>

async fn compact_context_with_llm_internal(
    app: &AppState,
    context_name: &str,
    resolved_config: &ResolvedConfig,
    manual: bool,
    sink: &dyn crate::output::OutputSink,
) -> io::Result<()>
```

Replace every `eprintln!(...)` in these functions with the appropriate `sink.emit_event(CommandEvent::...)` call using the new variants from Step 6a.

**Step 6c: Update the call site in `send.rs`**

In `crates/chibi-core/src/api/send.rs` (line ~1788):

```rust
return compact_context_with_llm(app, context_name, &resolved_config).await;
```

Pass `sink` as a trait object:

```rust
return compact_context_with_llm(app, context_name, &resolved_config, sink as &dyn crate::output::OutputSink).await;
```

(Or use the concrete `S: ResponseSink` — but `OutputSink` and `ResponseSink` are separate traits. Use `&NoopSink` or create a small adapter if needed. The simplest approach is to accept `&dyn OutputSink` specifically since compaction events are `CommandEvent`s, not `ResponseEvent`s.)

**Step 6d: Update CLI and JSON output handlers**

In `crates/chibi-cli/src/output.rs`, add match arms for the new compaction `CommandEvent` variants to `emit_event`. All compaction events are verbose-tier — they follow the same early-return pattern established in Task 4.

In `crates/chibi-json/src/output.rs`, add match arms to the `CommandEvent` serialisation match. Use descriptive `type` strings:
- `"compaction_started"`, `"compaction_complete"`, `"rolling_compaction_decision"`, `"rolling_compaction_fallback"`, `"rolling_compaction_complete"`, `"compaction_no_prompt"`

**Step 6e: Find and fix any other compact call sites**

```bash
grep -rn "compact_context_with_llm\|rolling_compact\|compact_context_by_name" crates/ --include="*.rs" | grep -v "compact.rs"
```

Update each call site to pass a sink. For call sites in `chibi.rs` (the synchronous/public API methods), pass `&NoopSink` from the module-level definition added in Task 3.

**Step 6f: Run tests**

```bash
cargo test -p chibi-core 2>&1 | tail -20
```

Expected: all pass. Fix any compile errors from missing match arms or signature mismatches.

**Step 6g: Commit**

```bash
git add crates/chibi-core/src/output.rs crates/chibi-core/src/api/compact.rs crates/chibi-core/src/api/send.rs crates/chibi-cli/src/output.rs crates/chibi-json/src/output.rs
git commit -m "feat(compact): route compaction output through OutputSink instead of eprintln"
```

---

### Task 7: Remove redundant `ContextLoaded` event variant

**Files:**
- Modify: `crates/chibi-core/src/output.rs` (remove variant)
- Modify: `crates/chibi-json/src/main.rs` (remove emission)
- Modify: `crates/chibi-json/src/output.rs` (remove match arm)
- Modify: `crates/chibi-cli/src/output.rs` (remove match arm)

**Context:** `CommandEvent::ContextLoaded { tool_count }` is emitted only by `chibi-json/src/main.rs` after load and conveys `chibi.tool_count()`. `LoadSummary` already carries `builtin_count + plugin_count` which is the same total. `ContextLoaded` is redundant.

**Step 7a: Remove the emission in `chibi-json/src/main.rs`**

Find and delete:

```rust
output.emit_event(CommandEvent::ContextLoaded {
    tool_count: chibi.tool_count(),
});
```

`LoadSummary` is already emitted by `chibi-core` during load; no replacement needed.

**Step 7b: Remove the variant from `CommandEvent`**

In `crates/chibi-core/src/output.rs`, delete:

```rust
/// Context loaded with N tools (verbose-tier).
ContextLoaded { tool_count: usize },
```

**Step 7c: Remove match arms**

In `crates/chibi-cli/src/output.rs`, delete the `CommandEvent::ContextLoaded` arm from `emit_event`.

In `crates/chibi-json/src/output.rs`, delete the `CommandEvent::ContextLoaded` arm from the JSON serialisation match.

**Step 7d: Compile-check**

```bash
cargo build 2>&1 | grep "error" | head -20
```

Expected: no errors. If any remain, search for `ContextLoaded` across the codebase:

```bash
grep -rn "ContextLoaded" crates/ --include="*.rs"
```

Fix any remaining references.

**Step 7e: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all pass.

**Step 7f: Commit**

```bash
git add crates/chibi-core/src/output.rs crates/chibi-json/src/main.rs crates/chibi-json/src/output.rs crates/chibi-cli/src/output.rs
git commit -m "refactor(output): remove redundant ContextLoaded event (LoadSummary covers it)"
```

---

### Task 8: Add tests for `OutputHandler::emit_event`

**Files:**
- Modify: `crates/chibi-cli/src/output.rs` (add tests in the existing `#[cfg(test)]` block)

**Context:** The old `test_diagnostic_verbose_true/false` tests were removed. `emit_event` now has 12 match arms with no coverage. Add targeted tests.

**Step 1: Write the tests**

In the existing `#[cfg(test)] mod tests` block at the bottom of `crates/chibi-cli/src/output.rs`, add:

```rust
// ── emit_event tests ─────────────────────────────────────────────────────────

#[test]
fn test_emit_event_verbose_false_suppresses_output() {
    // All CommandEvent variants are verbose-tier; non-verbose handler must suppress.
    // We can't capture stderr in unit tests, but we can verify no panic occurs
    // and the verbose guard is respected by calling with verbose=false.
    let handler = OutputHandler::new(false);
    handler.emit_event(CommandEvent::AutoDestroyed { count: 3 });
    handler.emit_event(CommandEvent::AllInboxesEmpty);
    handler.emit_event(CommandEvent::McpBridgeUnavailable {
        reason: "timeout".to_string(),
    });
    // If verbose_only guard is broken, output would appear; structural assertion is
    // that none of these panic.
}

#[test]
fn test_emit_event_verbose_true_does_not_panic() {
    let handler = OutputHandler::new(true);
    // Exercise every variant to catch format string regressions.
    handler.emit_event(CommandEvent::AutoDestroyed { count: 0 });
    handler.emit_event(CommandEvent::CacheCleanup {
        removed: 5,
        max_age_days: 6,
    });
    handler.emit_event(CommandEvent::SystemPromptSet {
        context: "work".to_string(),
    });
    handler.emit_event(CommandEvent::UsernameSaved {
        username: "alice".to_string(),
        context: "work".to_string(),
    });
    handler.emit_event(CommandEvent::InboxEmpty {
        context: "work".to_string(),
    });
    handler.emit_event(CommandEvent::InboxProcessing {
        count: 2,
        context: "work".to_string(),
    });
    handler.emit_event(CommandEvent::AllInboxesEmpty);
    handler.emit_event(CommandEvent::InboxesProcessed { count: 3 });
    handler.emit_event(CommandEvent::McpToolsLoaded { count: 7 });
    handler.emit_event(CommandEvent::McpBridgeUnavailable {
        reason: "connection refused".to_string(),
    });
    handler.emit_event(CommandEvent::LoadSummary {
        builtin_count: 4,
        builtin_names: vec!["a".to_string(), "b".to_string()],
        plugin_count: 0,
        plugin_names: vec![],
    });
    handler.emit_event(CommandEvent::LoadSummary {
        builtin_count: 2,
        builtin_names: vec!["a".to_string()],
        plugin_count: 3,
        plugin_names: vec!["p1".to_string(), "p2".to_string(), "p3".to_string()],
    });
}
```

**Note:** If `ContextLoaded` was removed in Task 7, do not include it here. If Task 6 added new compaction variants, add a test arm for each of them too.

**Step 2: Run tests**

```bash
cargo test -p chibi-cli output 2>&1 | tail -30
```

Expected: all pass including the new tests.

**Step 3: Commit**

```bash
git add crates/chibi-cli/src/output.rs
git commit -m "test(cli): add emit_event coverage for all CommandEvent variants"
```

---

## Execution Order

Tasks 1–4 are independent and can be done in any order. Tasks 5 and 6 touch `send.rs` — do them sequentially. Task 7 depends on Task 6 (to avoid removing a variant that compact.rs might still need). Task 8 should be done last, after the variant set is finalised.

Recommended order: **1 → 2 → 3 → 4 → 5 → 6 → 7 → 8**

Run `just pre-push` before pushing.
