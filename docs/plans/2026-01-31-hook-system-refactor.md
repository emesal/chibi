# Hook System Refactor: Move All Hooks to Core

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move all hook execution from CLI to core, achieving clean separation where plugins only interact with chibi-core.

**Architecture:** Add lifecycle methods (`init()`, `shutdown()`) to `Chibi` that execute `OnStart`/`OnEnd` hooks. Move `PreClear`/`PostClear` hooks into `clear_context()`. Remove `OnContextSwitch` hook entirely. Remove `verbose` parameter from hook execution (internal logging only).

**Tech Stack:** Rust, chibi-core, chibi-cli

**Related Issues:** #74, #78

**Imprtant:** Do not commit to git.

---

## Task 1: Remove OnContextSwitch from HookPoint enum

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs:14-38`

**Step 1: Remove OnContextSwitch variant**

In `hooks.rs`, remove `OnContextSwitch` from the enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum HookPoint {
    PreMessage,
    PostMessage,
    PreTool,
    PostTool,
    PreToolOutput,
    PostToolOutput,
    // OnContextSwitch removed
    PreClear,
    PostClear,
    PreCompact,
    PostCompact,
    PreRollingCompact,
    PostRollingCompact,
    OnStart,
    OnEnd,
    PreSystemPrompt,
    PostSystemPrompt,
    PreSendMessage,
    PostSendMessage,
    PreCacheOutput,
    PostCacheOutput,
    PreApiTools,
    PreApiRequest,
}
```

**Step 2: Update test constants**

In the same file, update `ALL_HOOKS` constant in tests (around line 109):

```rust
const ALL_HOOKS: &[(&str, HookPoint)] = &[
    ("pre_message", HookPoint::PreMessage),
    ("post_message", HookPoint::PostMessage),
    ("pre_tool", HookPoint::PreTool),
    ("post_tool", HookPoint::PostTool),
    ("pre_tool_output", HookPoint::PreToolOutput),
    ("post_tool_output", HookPoint::PostToolOutput),
    // on_context_switch removed
    ("pre_clear", HookPoint::PreClear),
    ("post_clear", HookPoint::PostClear),
    ("pre_compact", HookPoint::PreCompact),
    ("post_compact", HookPoint::PostCompact),
    ("pre_rolling_compact", HookPoint::PreRollingCompact),
    ("post_rolling_compact", HookPoint::PostRollingCompact),
    ("on_start", HookPoint::OnStart),
    ("on_end", HookPoint::OnEnd),
    ("pre_system_prompt", HookPoint::PreSystemPrompt),
    ("post_system_prompt", HookPoint::PostSystemPrompt),
    ("pre_send_message", HookPoint::PreSendMessage),
    ("post_send_message", HookPoint::PostSendMessage),
    ("pre_cache_output", HookPoint::PreCacheOutput),
    ("post_cache_output", HookPoint::PostCacheOutput),
    ("pre_api_tools", HookPoint::PreApiTools),
    ("pre_api_request", HookPoint::PreApiRequest),
];
```

**Step 3: Run tests**

Run: `cargo test -p chibi-core hooks`
Expected: All hook tests pass

---

## Task 2: Remove OnContextSwitch calls from CLI

**Files:**
- Modify: `crates/chibi-cli/src/main.rs`

**Step 1: Find and remove OnContextSwitch hook calls**

Search for `OnContextSwitch` in main.rs and remove the hook execution blocks.

Remove the ephemeral context switch hook (around line 377-380):
```rust
// DELETE this block:
let hook_data = serde_json::json!({
    "from_context": &session.implied_context,
    "to_context": &actual_name,
    "is_ephemeral": true,
});
let _ = chibi.execute_hook(tools::HookPoint::OnContextSwitch, &hook_data, verbose);
```

Remove the persistent context switch hook (around line 403-406):
```rust
// DELETE this block:
let hook_data = serde_json::json!({
    "from_context": &from_context,
    "to_context": &session.implied_context,
    "is_ephemeral": !persistent,
});
let _ = chibi.execute_hook(tools::HookPoint::OnContextSwitch, &hook_data, verbose);
```

**Step 2: Build to verify**

Run: `cargo build -p chibi-cli`
Expected: Build succeeds

---

## Task 3: Add init() and shutdown() methods to Chibi

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs`

**Step 1: Add init() method**

Add after `load_with_options()` (around line 133):

```rust
    /// Initialize the session.
    ///
    /// Executes `OnStart` hooks. Call this once at the start of a session,
    /// before any prompts are sent.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use chibi_core::Chibi;
    ///
    /// # fn example() -> std::io::Result<()> {
    /// let chibi = Chibi::load()?;
    /// chibi.init()?;
    /// // ... use chibi ...
    /// chibi.shutdown()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn init(&self) -> io::Result<Vec<(String, serde_json::Value)>> {
        let hook_data = serde_json::json!({});
        tools::execute_hook(&self.tools, tools::HookPoint::OnStart, &hook_data)
    }

    /// Shutdown the session.
    ///
    /// Executes `OnEnd` hooks. Call this once at the end of a session,
    /// after all prompts are complete.
    pub fn shutdown(&self) -> io::Result<Vec<(String, serde_json::Value)>> {
        let hook_data = serde_json::json!({});
        tools::execute_hook(&self.tools, tools::HookPoint::OnEnd, &hook_data)
    }
```

**Step 2: Remove verbose from execute_hook signature**

First, update the internal `execute_hook` function in `hooks.rs` to remove verbose parameter. The function should use internal logging only (or none for now):

In `crates/chibi-core/src/tools/hooks.rs`, change:

```rust
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        let output = Command::new(&tool.path)
            .env("CHIBI_HOOK", hook.as_ref())
            .env("CHIBI_HOOK_DATA", data.to_string())
            .env_remove("CHIBI_TOOL_ARGS")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| {
                io::Error::other(format!(
                    "Failed to execute hook {} on {}: {}",
                    hook.as_ref(),
                    tool.name,
                    e
                ))
            })?;

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();

        if trimmed.is_empty() {
            continue;
        }

        let value: serde_json::Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|_| serde_json::Value::String(trimmed.to_string()));

        results.push((tool.name.clone(), value));
    }

    Ok(results)
}
```

**Step 3: Update Chibi::execute_hook wrapper**

In `chibi.rs`, update the `execute_hook` method signature (around line 314):

```rust
    pub fn execute_hook(
        &self,
        hook: tools::HookPoint,
        data: &serde_json::Value,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        tools::execute_hook(&self.tools, hook, data)
    }
```

**Step 4: Build to check for errors**

Run: `cargo build -p chibi-core`
Expected: Errors about missing verbose argument in callers

---

## Task 4: Update all execute_hook callers to remove verbose

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs`
- Modify: `crates/chibi-core/src/api/compact.rs`
- Modify: `crates/chibi-cli/src/main.rs`

**Step 1: Update send.rs**

Find all `execute_hook` calls and remove the `verbose` argument. There are approximately 12 calls.

Example change:
```rust
// Before:
tools::execute_hook(tools, tools::HookPoint::PreMessage, &hook_data, verbose)?;

// After:
tools::execute_hook(tools, tools::HookPoint::PreMessage, &hook_data)?;
```

**Step 2: Update compact.rs**

Same pattern - remove `verbose` from all `execute_hook` calls (approximately 4 calls).

**Step 3: Update main.rs in CLI**

Remove `verbose` from remaining hook calls (OnStart, OnEnd, PreClear, PostClear).

**Step 4: Build everything**

Run: `cargo build`
Expected: Build succeeds

**Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

---

## Task 5: Move PreClear/PostClear hooks into clear_context()

**Files:**
- Modify: `crates/chibi-core/src/state/context_ops.rs`
- Modify: `crates/chibi-cli/src/main.rs`

**Step 1: Update clear_context signature**

The `clear_context` method is on `AppState`, but needs access to tools for hooks. We have two options:
1. Pass tools to `clear_context`
2. Add a new method on `Chibi` that wraps `clear_context` with hooks

Option 2 is cleaner. Add to `chibi.rs`:

```rust
    /// Clear a context, executing PreClear/PostClear hooks.
    ///
    /// This wraps `AppState::clear_context` with hook execution.
    pub fn clear_context(&self, context_name: &str) -> io::Result<()> {
        // Get context info for hook data before clearing
        let context = self.app.get_or_create_context(context_name)?;

        let pre_hook_data = serde_json::json!({
            "context_name": context_name,
            "message_count": context.messages.len(),
            "summary": context.summary,
        });
        let _ = tools::execute_hook(&self.tools, tools::HookPoint::PreClear, &pre_hook_data);

        self.app.clear_context(context_name)?;

        let post_hook_data = serde_json::json!({
            "context_name": context_name,
        });
        let _ = tools::execute_hook(&self.tools, tools::HookPoint::PostClear, &post_hook_data);

        Ok(())
    }
```

**Step 2: Update CLI to use new method**

In `main.rs`, change the clear command handling (around line 540-555):

```rust
// Before:
let hook_data = serde_json::json!({...});
let _ = chibi.execute_hook(tools::HookPoint::PreClear, &hook_data, verbose);
chibi.app.clear_context(&ctx_name)?;
let hook_data = serde_json::json!({...});
let _ = chibi.execute_hook(tools::HookPoint::PostClear, &hook_data, verbose);

// After:
chibi.clear_context(&ctx_name)?;
```

**Step 3: Build and test**

Run: `cargo build && cargo test`
Expected: All pass

---

## Task 6: Move OnStart/OnEnd from CLI to use init/shutdown

**Files:**
- Modify: `crates/chibi-cli/src/main.rs`

**Step 1: Replace OnStart hook call with init()**

Find the OnStart hook execution (around line 338-342) and replace:

```rust
// Before:
let hook_data = serde_json::json!({
    "implied_context": &session.implied_context,
    "verbose": verbose,
});
let _ = chibi.execute_hook(tools::HookPoint::OnStart, &hook_data, verbose);

// After:
let _ = chibi.init();
```

**Step 2: Replace OnEnd hook call with shutdown()**

Find the OnEnd hook execution (around line 872-875) and replace:

```rust
// Before:
let hook_data = serde_json::json!({
    "working_context": &working_context,
});
let _ = chibi.execute_hook(tools::HookPoint::OnEnd, &hook_data, verbose);

// After:
let _ = chibi.shutdown();
```

**Step 3: Build and test**

Run: `cargo build && cargo test`
Expected: All pass

---

## Task 7: Remove execute_hook from Chibi public API (optional)

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs`

**Step 1: Check if execute_hook is still needed publicly**

After the refactor, CLI should no longer call `execute_hook` directly. Check:

Run: `grep -r "execute_hook" crates/chibi-cli/`
Expected: No matches (or only in comments)

**Step 2: Make execute_hook private or remove if unused**

If CLI no longer uses it, consider making it `pub(crate)` or removing the wrapper entirely (the internal `tools::execute_hook` is still used by core).

```rust
    // Change from pub to pub(crate) if still needed internally
    pub(crate) fn execute_hook(
        &self,
        hook: tools::HookPoint,
        data: &serde_json::Value,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        tools::execute_hook(&self.tools, hook, data)
    }
```

**Step 3: Build**

Run: `cargo build`
Expected: Build succeeds

---

## Task 8: Update documentation

**Files:**
- Modify: `docs/hooks.md`
- Modify: `plugins/hook-inspector/hook-inspector`

**Step 1: Update hooks.md**

Update the documentation to reflect:
- 21 hooks (not 23)
- OnContextSwitch removed
- OnStart/OnEnd now have empty payloads `{}`
- PreClear/PostClear are now called by core

**Step 2: Update hook-inspector example plugin**

Remove `on_context_switch` from the hooks array in the schema.

---

## Task 9: Final verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

**Step 2: Manual smoke test**

```bash
cargo install --path crates/chibi-cli
chibi "hello"
```

Expected: Works normally, hook-inspector (if installed) logs hooks

**Step 3: Verify no hook code remains in CLI**

Run: `grep -n "HookPoint\|execute_hook" crates/chibi-cli/src/*.rs`
Expected: No matches (CLI is hook-free)

**Step 4: Run pre-push checks**

Run: `just pre-push`
Expected: All checks pass
