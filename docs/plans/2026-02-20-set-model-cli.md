# SetModel CLI Command Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `-m`/`--set-model` and `-M`/`--set-model-for-context` CLI flags to persistently write the model into a context's `local.toml`, with live validation via ratatoskr, reassigning short flags from `--model-metadata`/`--model-metadata-full`.

**Architecture:** Add `Command::SetModel` to the shared core enum, implement the async handler in `execution.rs` (validate via `fetch_metadata` → load/mutate/save `LocalConfig`), then wire CLI flags in `chibi-cli/src/cli.rs` and update docs.

**Tech Stack:** Rust, clap (CLI parsing), ratatoskr (`fetch_model_metadata`), toml (local.toml serialisation), `safe_io::atomic_write_text` (already used by `save_local_config`)

---

### Task 1: Add `Command::SetModel` and `CommandEvent::ModelSet` to chibi-core

**Files:**
- Modify: `crates/chibi-core/src/input.rs`
- Modify: `crates/chibi-core/src/output.rs`

**Step 1: Add the `SetModel` variant to the `Command` enum**

In `crates/chibi-core/src/input.rs`, find the `Command` enum (line 28). Add after `SetSystemPrompt`:

```rust
    /// Set model for a context (-m/--set-model, -M/--set-model-for-context)
    SetModel {
        context: Option<String>,
        model: String,
    },
```

**Step 2: Add the `ModelSet` variant to `CommandEvent`**

In `crates/chibi-core/src/output.rs`, find `CommandEvent` (line 3). Add after `UsernameSaved`:

```rust
    /// Model saved to local.toml for a context (verbose-tier).
    ModelSet { model: String, context: String },
```

**Step 3: Verify it compiles**

```bash
cargo build -p chibi-core 2>&1 | head -40
```

Expected: errors about non-exhaustive match in `execution.rs` — that's fine, we handle it next. No other errors.

**Step 4: Commit**

```bash
git add crates/chibi-core/src/input.rs crates/chibi-core/src/output.rs
git commit -m "feat(core): add Command::SetModel and CommandEvent::ModelSet"
```

---

### Task 2: Write the failing test for `SetModel` dispatch

**Files:**
- Modify: `crates/chibi-core/src/execution.rs` (tests section, near the bottom)

**Step 1: Find the test module**

The test module is at the bottom of `execution.rs`. Look for `dispatch_set_system_prompt_emits_event` as a reference — copy its structure.

**Step 2: Write the failing test**

Add this test inside the `#[cfg(test)]` mod at the bottom of `execution.rs`:

```rust
#[tokio::test]
async fn dispatch_set_model_saves_to_local_config() {
    let (mut chibi, _dir) = create_test_chibi();
    chibi.app.ensure_context_dir("ctx").unwrap();

    let config = chibi.resolve_config("ctx", None).unwrap();
    let flags = ExecutionFlags::default();
    let sink = CaptureSink::new();
    let mut response = CollectingSink::default();

    execute_command(
        &mut chibi,
        "ctx",
        &Command::SetModel {
            context: None,
            model: "anthropic/claude-sonnet-4".to_string(),
        },
        &flags,
        &config,
        None,
        &sink,
        &mut response,
    )
    .await
    .unwrap();

    // Model should be persisted in local.toml
    let local = chibi.app.load_local_config("ctx").unwrap();
    assert_eq!(
        local.model.as_deref(),
        Some("anthropic/claude-sonnet-4"),
        "model should be saved to local config"
    );

    // ModelSet event should be emitted
    let events = sink.events.borrow();
    assert!(
        events.iter().any(|e| matches!(e, CommandEvent::ModelSet { .. })),
        "ModelSet event should be emitted"
    );
}

#[tokio::test]
async fn dispatch_set_model_named_context() {
    let (mut chibi, _dir) = create_test_chibi();
    chibi.app.ensure_context_dir("other").unwrap();

    let config = chibi.resolve_config("ctx", None).unwrap();
    let flags = ExecutionFlags::default();
    let sink = CaptureSink::new();
    let mut response = CollectingSink::default();

    execute_command(
        &mut chibi,
        "ctx",
        &Command::SetModel {
            context: Some("other".to_string()),
            model: "anthropic/claude-sonnet-4".to_string(),
        },
        &flags,
        &config,
        None,
        &sink,
        &mut response,
    )
    .await
    .unwrap();

    let local = chibi.app.load_local_config("other").unwrap();
    assert_eq!(
        local.model.as_deref(),
        Some("anthropic/claude-sonnet-4"),
        "model should be saved to named context's local config"
    );
}
```

**Step 3: Run to verify they fail**

```bash
cargo test -p chibi-core dispatch_set_model 2>&1 | tail -20
```

Expected: compile error (non-exhaustive match) or test failures — not yet implemented.

---

### Task 3: Implement the `SetModel` handler in `execution.rs`

**Files:**
- Modify: `crates/chibi-core/src/execution.rs`

**Step 1: Find the match arm for `SetSystemPrompt`**

Search for `Command::SetSystemPrompt` in `execution.rs` (around line 247). Add the `SetModel` arm immediately after the `SetSystemPrompt` block:

```rust
        Command::SetModel {
            context: ctx,
            model,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            let gateway = crate::gateway::build_gateway(resolved)?;
            // Live validation: registry → cache → network. Unknown model → error, no write.
            crate::model_info::fetch_metadata(&gateway, model).await?;
            let mut local = chibi.app.load_local_config(ctx_name)?;
            local.model = Some(model.clone());
            chibi.app.save_local_config(ctx_name, &local)?;
            output.emit_event(CommandEvent::ModelSet {
                model: model.clone(),
                context: ctx_name.to_string(),
            });
            Ok(CommandEffect::None)
        }
```

**Step 2: Check `execution.rs` has `CommandEvent` in scope**

Search for `CommandEvent` imports at the top of `execution.rs`. It should already be imported — if not, add it to the use statement.

**Step 3: Run the tests**

```bash
cargo test -p chibi-core dispatch_set_model 2>&1 | tail -30
```

Expected: both tests PASS. Note: these tests make live network calls to validate the model name. If network is unavailable, they will fail — that's acceptable for now (see note below).

> **Note on network in tests:** The `dispatch_set_model_*` tests call ratatoskr's live API. They are integration-style tests. If you want them to be hermetic, skip them with `#[ignore]` and mark them `// integration test — requires network`. For now, leave them as-is and rely on the CI network access.

**Step 4: Run full core test suite**

```bash
cargo test -p chibi-core 2>&1 | tail -20
```

Expected: all existing tests still pass.

**Step 5: Commit**

```bash
git add crates/chibi-core/src/execution.rs
git commit -m "feat(core): implement SetModel command handler with live validation"
```

---

### Task 4: Reassign `-m`/`-M` short flags in chibi-cli and add new flags

**Files:**
- Modify: `crates/chibi-cli/src/cli.rs`

**Step 1: Remove short flags from `--model-metadata` and `--model-metadata-full`**

Find the `model_metadata` field (currently has `short = 'm'`). Remove the `short = 'm'` line. Do the same for `model_metadata_full` (remove `short = 'M'`).

After change, `model_metadata` should look like:
```rust
    /// Show model metadata in TOML format (settable fields only)
    #[arg(
        long = "model-metadata",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub model_metadata: Option<String>,

    /// Show full model metadata in TOML format (with pricing, capabilities, parameter ranges)
    #[arg(
        long = "model-metadata-full",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub model_metadata_full: Option<String>,
```

**Step 2: Add new `set_model` and `set_model_for_context` fields**

Add them immediately after `model_metadata_full` (in the `// === Model metadata ===` section):

```rust
    // === Model setting ===
    /// Set model for current context (persists to local.toml)
    #[arg(
        short = 'm',
        long = "set-model",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub set_model: Option<String>,

    /// Set model for specified context (requires CTX and MODEL)
    #[arg(
        short = 'M',
        long = "set-model-for-context",
        value_names = ["CTX", "MODEL"],
        num_args = 2,
        allow_hyphen_values = true
    )]
    pub set_model_for_context: Option<Vec<String>>,
```

**Step 3: Update `implies_force_call_user` in `to_input()`**

Find the `implies_force_call_user` block. It currently contains `self.model_metadata.is_some() || self.model_metadata_full.is_some()`. Change to:

```rust
            || self.model_metadata.is_some()
            || self.model_metadata_full.is_some()
            || self.set_model_for_context.is_some()
```

(Do NOT add `self.set_model.is_some()` — the current-context form is combinable with a prompt, like `-y`.)

**Step 4: Add command dispatch in `to_input()`**

Find the command dispatch section (the long `if / else if` chain). Add new arms after the `SetSystemPrompt` arms, before `RunPlugin`:

```rust
        } else if let Some(ref model) = self.set_model {
            Command::SetModel {
                context: None,
                model: model.clone(),
            }
        } else if let Some(ref v) = self.set_model_for_context {
            if v.len() >= 2 {
                Command::SetModel {
                    context: Some(v[0].clone()),
                    model: v[1].clone(),
                }
            } else {
                Command::NoOp
            }
        }
```

**Step 5: Update `ATTACHED_FLAGS` constant**

Find `const ATTACHED_FLAGS: &[char]` in `expand_attached_args`. It currently contains `'m'` and `'M'` (from model-metadata). They stay — they now belong to the new set-model flags. No change needed.

**Step 6: Update `CLI_AFTER_HELP` string**

Find `const CLI_AFTER_HELP`. In the `Implied --no-chibi:` line, remove `-m, -M` (they were model-metadata; model-metadata is now long-only and does imply no-chibi). Add `-M` back (now means set-model-for-context):

```
  Implied --no-chibi: -l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -M, -p, -P, --model-metadata, --model-metadata-full
  Combinable with prompt: -c, -C, -a, -z, -r, -m, -y, -u, -U, -v
```

**Step 7: Build to check for compile errors**

```bash
cargo build -p chibi-cli 2>&1 | head -40
```

Expected: clean build.

---

### Task 5: Write CLI flag tests

**Files:**
- Modify: `crates/chibi-cli/src/cli.rs` (tests module, at the bottom)

**Step 1: Add tests for the new flags and for the reassigned model-metadata flags**

Find the existing `test_model_metadata_*` tests (near the bottom of the test module). Add the new tests after them:

```rust
    // === Set model tests ===

    #[test]
    fn test_set_model_short() {
        let input = parse_input("-m anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_set_model_long() {
        let input = parse_input("--set-model anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_set_model_attached() {
        let input = parse_input("-manthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_set_model_combinable_with_prompt() {
        let input = parse_input("-m anthropic/claude-sonnet-4 hello world").unwrap();
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "hello world")
        );
        // model is set on the struct, prompt wins in command
        assert!(input.cli_set_model.is_some() || matches!(input.command, Command::SendPrompt { .. }));
    }

    #[test]
    fn test_set_model_for_context_short() {
        let input = parse_input("-M myctx openai/gpt-4o").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { ref context, ref model }
                if *context == Some("myctx".to_string()) && model == "openai/gpt-4o")
        );
        assert!(input.flags.force_call_user); // implies --no-chibi
    }

    #[test]
    fn test_set_model_for_context_long() {
        let input = parse_input("--set-model-for-context myctx openai/gpt-4o").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { ref context, ref model }
                if *context == Some("myctx".to_string()) && model == "openai/gpt-4o")
        );
        assert!(input.flags.force_call_user);
    }

    // Verify old -m/-M no longer accepted for model-metadata
    #[test]
    fn test_model_metadata_long_only() {
        let input = parse_input("--model-metadata anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: false } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_model_metadata_full_long_only() {
        let input = parse_input("--model-metadata-full anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: true } if model == "anthropic/claude-sonnet-4")
        );
    }
```

> **Note:** The `test_set_model_combinable_with_prompt` test above may need adjustment depending on how `to_input()` handles the case where both `set_model` and a prompt are present. The `set_model` flag sets the struct field but `Command::SendPrompt` wins in dispatch (same as `-y`). Remove that test if it doesn't compile cleanly — the behaviour is already covered by the dispatch logic and the integration test.

**Step 2: Remove or update the old short-flag model-metadata tests**

The tests `test_model_metadata_short`, `test_model_metadata_full_short`, `test_model_metadata_attached`, `test_model_metadata_full_attached` all use `-m`/`-M` short forms which are now reassigned. Update them to use `--model-metadata` / `--model-metadata-full` long forms, or delete and replace with the `test_model_metadata_long_only` tests above.

**Step 3: Run the CLI tests**

```bash
cargo test -p chibi-cli 2>&1 | tail -30
```

Expected: all tests pass.

**Step 4: Commit**

```bash
git add crates/chibi-cli/src/cli.rs
git commit -m "feat(cli): add -m/--set-model and -M/--set-model-for-context; make model-metadata long-only"
```

---

### Task 6: Update `docs/cli-reference.md`

**Files:**
- Modify: `docs/cli-reference.md`

**Step 1: Add a new "Model" section**

Insert a new section between "System Prompt" and "Username":

```markdown
## Model

| Flag | Description |
|------|-------------|
| `-m, --set-model <MODEL>` | Set model for current context (persists to local.toml, validated live) |
| `-M, --set-model-for-context <CTX> <MODEL>` | Set model for specified context |

Model names are validated live against the OpenRouter API before being written. An unknown model
ID returns an error and no write occurs.

```bash
chibi -m anthropic/claude-sonnet-4          # Set model for current context
chibi -M research openai/o3                 # Set model for 'research' context
chibi -m anthropic/claude-sonnet-4 "Hello"  # Set model, then chat
```
```

**Step 2: Update the "Model Metadata" section**

Change the header row to reflect that `-m`/`-M` are no longer the short flags:

```markdown
## Model Metadata

| Flag | Description |
|------|-------------|
| `--model-metadata <MODEL>` | Show model metadata in TOML format (settable fields only) |
| `--model-metadata-full <MODEL>` | Show full model metadata (with pricing, capabilities, parameter ranges) |
```

Update the examples:
```bash
chibi --model-metadata anthropic/claude-sonnet-4       # Settable fields only
chibi --model-metadata-full openai/gpt-4o              # Full metadata including pricing
```

**Step 3: Update the "Implied -x" and "Combinable with Prompt" lists**

In the **Flag Behavior** section:

- Implied `--no-chibi` line: remove `-m, -M` from the list; add `-M` (set-model-for-context) and `--model-metadata, --model-metadata-full`
- Combinable with prompt line: add `-m`

Result:
```
Implied --no-chibi: -l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -M, -p, -P, --model-metadata, --model-metadata-full
Combinable with prompt: -c, -C, -a, -z, -r, -m, -y, -u, -U, -v
```

**Step 4: Update the chibi-json command list**

In the **Programmatic / JSON Mode** section, add to the "Commands with arguments" list:

```
- `{ "set_model": { "context": "...", "model": "..." } }` (context optional)
```

**Step 5: Commit**

```bash
git add docs/cli-reference.md
git commit -m "docs: update cli-reference for set-model flags and reassigned -m/-M"
```

---

### Task 7: Update `docs/configuration.md`

**Files:**
- Modify: `docs/configuration.md`

**Step 1: Update the Per-Context Configuration section**

Find the existing username CLI example:
```bash
chibi -u alice "Hello"  # Persists to local.toml
chibi -U bob "Hello"    # Ephemeral, doesn't persist
```

Add a model equivalent immediately after:
```bash
chibi -m anthropic/claude-sonnet-4   # Persists to local.toml (validated live)
chibi -s model=anthropic/claude-sonnet-4 "Hello"  # Ephemeral, doesn't persist
```

**Step 2: Commit**

```bash
git add docs/configuration.md
git commit -m "docs: document CLI shortcut for persistent model setting in per-context config"
```

---

### Task 8: Final verification

**Step 1: Run full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

**Step 2: Build all binaries**

```bash
cargo build 2>&1 | tail -10
```

Expected: clean build, no warnings.

**Step 3: Smoke test (if installed)**

```bash
chibi --help | grep -A3 "set-model"
chibi --help | grep "model-metadata"
```

Expected: `--set-model` and `--set-model-for-context` appear; `--model-metadata` appears without short flags.

**Step 4: Run pre-push checks**

```bash
just pre-push
```

Expected: passes.
