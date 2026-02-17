# ExecutionFlags Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove behavioural config fields from `ExecutionFlags`, making `ResolvedConfig` the single source of truth for `verbose`, `hide_tool_calls`, `no_tool_calls`, `show_thinking`.

**Architecture:** CLI/JSON flags become sugar that calls `config.set_field(...)` instead of writing to `ExecutionFlags`. Core reads from `config` instead of `flags`. `ExecutionFlags` shrinks to three ephemeral command modifiers: `force_call_agent`, `force_call_user`, `debug`.

**Tech Stack:** Rust, serde, schemars (JsonSchema)

---

### Task 1: Add `show_thinking` to core config layer

Add the field to `Config`, `LocalConfig`, `ResolvedConfig`, config resolution, `set_field`, `get_field`, `list_fields`.

**Files:**
- Modify: `crates/chibi-core/src/config.rs`
- Modify: `crates/chibi-core/src/state/config_resolution.rs:75` (resolve_config struct literal)

**Step 1: Add default function**

In `crates/chibi-core/src/config.rs`, after `default_no_tool_calls` (around line 519), add:

```rust
fn default_show_thinking() -> bool {
    false
}
```

**Step 2: Add to `Config` struct**

In `Config` (line 590-592 area, after `no_tool_calls`), add:

```rust
    /// Show thinking/reasoning content (default: false, verbose overrides)
    #[serde(default = "default_show_thinking")]
    pub show_thinking: bool,
```

**Step 3: Add to `LocalConfig` struct**

In `LocalConfig` (line 659-660 area, after `no_tool_calls`), add:

```rust
    /// Per-context show thinking override
    pub show_thinking: Option<bool>,
```

**Step 4: Add to `LocalConfig::apply_overrides` macro invocation**

In `apply_overrides` (around line 720), add `show_thinking` to the `apply_option_overrides!` list, after `no_tool_calls`.

**Step 5: Add to `ResolvedConfig` struct**

In `ResolvedConfig` (line 780-781 area, after `no_tool_calls`), add:

```rust
    /// Show thinking/reasoning content (default: false, verbose overrides)
    pub show_thinking: bool,
```

**Step 6: Add to config resolution**

In `crates/chibi-core/src/state/config_resolution.rs`, in the `resolve_config` struct literal (around line 89, after `no_tool_calls`), add:

```rust
            show_thinking: self.config.show_thinking,
```

**Step 7: Add to `get_field` macro**

In `get_field` (line 828), add `show_thinking` to the `display:` list:

```
display: verbose, hide_tool_calls, no_tool_calls, show_thinking, auto_compact, ...
```

**Step 8: Add to `list_fields`**

In `list_fields` (around line 912), add `"show_thinking"` after `"no_tool_calls"`.

**Step 9: Add to `set_field` macro**

In `set_field` (line 964), add `show_thinking` to the `bool:` list:

```
bool: verbose, hide_tool_calls, no_tool_calls, show_thinking, auto_compact, ...
```

**Step 10: Add to all `ResolvedConfig { ... }` test literals**

Add `show_thinking: false,` after `no_tool_calls` in every struct literal:
- `crates/chibi-core/src/config.rs` (~lines 1192, 1322)
- `crates/chibi-core/src/gateway.rs` (~line 362)
- `crates/chibi-core/src/tools/agent_tools.rs` (~line 560)
- `crates/chibi-core/src/tools/security.rs` (~line 400)
- `crates/chibi-core/src/tools/file_tools.rs` (~line 710)

Add `show_thinking: false,` to `Config` literals:
- `crates/chibi-core/src/chibi.rs` (~line 487)

Add `show_thinking: None,` to `LocalConfig` literals:
- `crates/chibi-core/src/state/tests.rs` (check for any `LocalConfig` literals; likely via `Default` so may be fine)

**Step 11: Build and test**

Run: `cargo build && cargo test -p chibi-core`
Expected: all pass

**Step 12: Commit**

```
git add -A && git commit -m "feat: add show_thinking to core config layer"
```

---

### Task 2: Shrink `ExecutionFlags`

Remove `verbose`, `hide_tool_calls`, `no_tool_calls`, `show_thinking` from `ExecutionFlags`. Update all construction sites.

**Files:**
- Modify: `crates/chibi-core/src/input.rs:135-163` (struct + tests)

**Step 1: Replace `ExecutionFlags` struct body**

```rust
/// Execution flags — ephemeral command modifiers.
///
/// These are per-invocation imperative commands, not behavioural config.
/// Behavioural settings (verbose, hide_tool_calls, no_tool_calls, show_thinking)
/// live in `ResolvedConfig` and are set via `set_field` / `-s` / config files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionFlags {
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
```

**Step 2: Update tests in `input.rs`**

Fix the two `ExecutionFlags { ... }` construction sites in tests (~lines 187, 497) to remove the four deleted fields. Update assertions that reference `flags.verbose`, `flags.no_tool_calls`, `flags.hide_tool_calls`, `flags.show_thinking`.

Also update the `ExecutionFlags::default()` assertions (~lines 178, 485) to remove assertions about the deleted fields.

**Step 3: Build core (expect compilation errors in other crates)**

Run: `cargo build -p chibi-core 2>&1 | head -40`
Expected: chibi-core builds, other crates have errors (we fix those in subsequent tasks)

**Step 4: Commit**

```
git add crates/chibi-core/src/input.rs && git commit -m "refactor: shrink ExecutionFlags to ephemeral modifiers only"
```

---

### Task 3: Update core `execution.rs` to read from config

Replace `flags.verbose` and `flags.no_tool_calls` reads with `config.verbose` and `config.no_tool_calls`.

**Files:**
- Modify: `crates/chibi-core/src/execution.rs`

**Step 1: In `execute_command` (line 58)**

Change `let verbose = flags.verbose;` → `let verbose = config.verbose;`

**Step 2: In `dispatch_command` (line 152)**

Change `let verbose = flags.verbose;` → `let verbose = config.verbose;`

**Step 3: In `send_prompt` (lines 444, 455)**

Remove the `if flags.no_tool_calls { resolved.no_tool_calls = true; }` block (line 444-446) — config already has the correct value.

Change `flags.verbose` → `config.verbose` in the `PromptOptions::new(...)` call (line 455).

**Step 4: Build core**

Run: `cargo build -p chibi-core`
Expected: pass

**Step 5: Commit**

```
git add crates/chibi-core/src/execution.rs && git commit -m "refactor: execution reads verbose/no_tool_calls from config"
```

---

### Task 4: Update `chibi-cli`

Remove merge logic, use `set_field` for CLI flag overrides, read from config in sink setup.

**Files:**
- Modify: `crates/chibi-cli/src/cli.rs:665` (flags construction)
- Modify: `crates/chibi-cli/src/main.rs:247-249, 519-528, 347-353`
- Modify: `crates/chibi-cli/src/input.rs` (test literals)
- Modify: `crates/chibi-cli/src/config.rs` (drop `show_thinking` from CLI ResolvedConfig)

**Step 1: Update `cli.rs` — flags construction**

In `cli.rs` around line 665, the `ExecutionFlags { ... }` block: remove the four deleted fields, keep `force_call_user`, `force_call_agent`, `debug`.

The four removed boolean values (`self.verbose`, `self.hide_tool_calls`, `self.show_thinking`, `self.no_tool_calls`) need to be captured as config overrides instead. Add them to the `config_overrides` vec that's built from `--set` pairs:

```rust
        // CLI boolean flags → config overrides
        let mut config_overrides: Vec<(String, String)> = self
            .set
            .iter()
            .map(|s| {
                let (k, v) = s.split_once('=').ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("--set value must be KEY=VALUE, got: {}", s),
                    )
                })?;
                Ok((k.to_string(), v.to_string()))
            })
            .collect::<io::Result<_>>()?;

        // Boolean CLI flags as config overrides (only when set to true)
        if self.verbose {
            config_overrides.push(("verbose".to_string(), "true".to_string()));
        }
        if self.hide_tool_calls {
            config_overrides.push(("hide_tool_calls".to_string(), "true".to_string()));
        }
        if self.no_tool_calls {
            config_overrides.push(("no_tool_calls".to_string(), "true".to_string()));
        }
        if self.show_thinking {
            config_overrides.push(("show_thinking".to_string(), "true".to_string()));
        }
```

**Step 2: Update `main.rs` — remove merge logic**

Remove the merge block (~lines 525-528):
```rust
    let verbose = input.flags.verbose || chibi.app.config.verbose;
    input.flags.verbose = verbose;
    input.flags.hide_tool_calls = input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
    input.flags.no_tool_calls = input.flags.no_tool_calls || chibi.app.config.no_tool_calls;
```

The `LoadOptions` verbose (line 519) should use the raw CLI flag value for tool-loading diagnostics. Check if `input.config_overrides` contains verbose, or just use a dedicated local:

```rust
    let load_verbose = input.config_overrides.iter().any(|(k, v)| k == "verbose" && v == "true");
```

**Step 3: Update `main.rs` — execute_from_input reads from config**

In `execute_from_input` (~lines 247-249), replace:
```rust
    let verbose = input.flags.verbose;
    let show_tool_calls = !input.flags.hide_tool_calls || verbose;
    let show_thinking_flag = input.flags.show_thinking || verbose;
```

With reads from the resolved config (which happens after `resolve_cli_config` + `apply_overrides_from_pairs`). Move these lines after config resolution (~line 335). Use:
```rust
    let verbose = cli_config.core.verbose;
    let show_tool_calls = !cli_config.core.hide_tool_calls || verbose;
    let show_thinking = cli_config.core.show_thinking || verbose;
```

**Step 4: Update sink construction**

In the `CliResponseSink::new(...)` call (~line 353), change:
```rust
    show_thinking_flag || cli_config.show_thinking,
```
To just:
```rust
    show_thinking,
```
(since `show_thinking` already incorporates the verbose override and config value)

**Step 5: Drop `show_thinking` from CLI's `ResolvedConfig`**

In `crates/chibi-cli/src/config.rs`:
- Remove `show_thinking` field from `ResolvedConfig` struct (~line 267-268)
- Update `get_field` to delegate `"show_thinking"` to `self.core.get_field(path)` (remove the explicit match arm ~line 288)
- Update `list_fields` to remove `"show_thinking"` from the CLI-specific list (~line 311)
- Remove `show_thinking` from `RawCliConfig` (~line 377), `CliConfig` (~line 389), `CliConfig::default()` (~line 398), `CliConfig::merge_with` (~line 410), `CliConfigOverride` (~line 421)
- Remove from `load_cli_config` (~line 511)

Update CLI `ResolvedConfig` construction in `main.rs` (~line 171): remove `show_thinking` field.

**Step 6: Update CLI input tests**

In `crates/chibi-cli/src/input.rs`, fix `ExecutionFlags { ... }` test literals to remove the four deleted fields.

**Step 7: Update CLI cli.rs tests**

In `crates/chibi-cli/src/cli.rs` tests, fix any assertions that reference `flags.verbose`, `flags.hide_tool_calls`, `flags.show_thinking`, `flags.no_tool_calls`. These should now be checked via `config_overrides` instead.

**Step 8: Build and test**

Run: `cargo build -p chibi-cli && cargo test -p chibi-cli`
Expected: pass

**Step 9: Commit**

```
git add crates/chibi-cli/ && git commit -m "refactor(cli): use set_field for flag→config overrides"
```

---

### Task 5: Update `chibi-json`

Remove merge logic, let callers pass these via `config`/`overrides` fields.

**Files:**
- Modify: `crates/chibi-json/src/main.rs:54-70`

**Step 1: Remove merge logic**

Remove (~lines 64-68):
```rust
    json_input.flags.verbose = json_input.flags.verbose || chibi.app.config.verbose;
    json_input.flags.hide_tool_calls =
        json_input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
    json_input.flags.no_tool_calls =
        json_input.flags.no_tool_calls || chibi.app.config.no_tool_calls;
```

**Step 2: Update verbose for LoadOptions and diagnostics**

The `LoadOptions { verbose: json_input.flags.verbose, ... }` (line 55) no longer works since verbose isn't in flags. Use:

```rust
    // Check for verbose in overrides map (string-keyed) or typed config
    let load_verbose = json_input
        .overrides
        .as_ref()
        .and_then(|o| o.get("verbose"))
        .map(|v| v == "true")
        .or_else(|| {
            json_input
                .config
                .as_ref()
                .and_then(|c| c.verbose)
        })
        .unwrap_or(false);
```

Use `load_verbose` for `LoadOptions` and the early `let verbose = ...` line.

After config resolution, use `resolved.verbose` for the actual verbose state:
```rust
    let verbose = resolved.verbose;
```

**Step 3: Build and test**

Run: `cargo build -p chibi-json && cargo test -p chibi-json`
Expected: pass

**Step 4: Commit**

```
git add crates/chibi-json/ && git commit -m "refactor(json): remove flag merge logic, use config overrides"
```

---

### Task 6: Update `lib.rs` exports and docstrings

**Files:**
- Modify: `crates/chibi-core/src/lib.rs:63`

**Step 1: Check if `ExecutionFlags` still needs to be re-exported**

It's still used by both binaries, so keep the re-export. Just verify the docstring on `ExecutionFlags` (already updated in Task 2) matches reality.

**Step 2: Commit (if changes)**

```
git add crates/chibi-core/src/lib.rs && git commit -m "docs: update ExecutionFlags re-export"
```

---

### Task 7: Full build + test + cleanup

**Files:** all crates

**Step 1: Full build**

Run: `cargo build`
Expected: clean

**Step 2: Full test**

Run: `cargo test`
Expected: all pass

**Step 3: Format**

Run: `cargo fmt`

**Step 4: Clippy**

Run: `cargo clippy -- -W warnings`
Expected: clean

**Step 5: Commit (if formatting/clippy fixes)**

```
git add -A && git commit -m "chore: fmt + clippy fixes"
```

---

### Task 8: Update documentation

**Files:**
- Modify: `docs/configuration.md` (add `show_thinking` to core config table)
- Modify: `AGENTS.md` or relevant docs if they reference `ExecutionFlags` fields

**Step 1: Check docs for references to removed fields**

Search for `ExecutionFlags`, `verbose`, `show_thinking`, `hide_tool_calls`, `no_tool_calls` in docs/ and update any references that describe the old architecture.

**Step 2: Add `show_thinking` to config docs**

In `docs/configuration.md`, add `show_thinking` to the config field table alongside `verbose`, `hide_tool_calls`, `no_tool_calls`.

**Step 3: Commit**

```
git add docs/ && git commit -m "docs: update for ExecutionFlags migration (#161)"
```
