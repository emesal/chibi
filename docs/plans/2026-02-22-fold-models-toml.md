# Fold `models.toml` into `config.toml` / `local.toml` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove `models.toml` as a separate file by folding per-model API param overrides into `config.toml` (global) and `local.toml` (per-context), adding per-context per-model support as a bonus.

**Architecture:** Add a `models: HashMap<String, ModelMetadata>` field to both `Config` and `LocalConfig`. Update `AppState::load()` to drop `models_config` and emit a deprecation warning if `models.toml` still exists. Update `resolve_config()` to look up the model in `config.models` then `local.models`, applying local on top.

**Tech Stack:** Rust, `serde`/`toml`, `chibi-core`

---

### Task 1: Add `models` field to `Config` and `LocalConfig`

**Files:**
- Modify: `crates/chibi-core/src/config.rs`

**Step 1: Add `models` to `Config`**

In `Config` struct (around line 669, after `subagent_cost_tier`), add:

```rust
/// Per-model API parameter overrides. Keyed by model ID.
#[serde(default)]
pub models: HashMap<String, ModelMetadata>,
```

`HashMap` is already imported in this file. `ModelMetadata` is defined just below `Config`.

**Step 2: Add `models` to `LocalConfig`**

In `LocalConfig` struct (around line 718, after `subagent_cost_tier`), add:

```rust
/// Per-model API parameter overrides for this context. Keyed by model ID.
/// These take precedence over global `config.toml` model overrides.
#[serde(default)]
pub models: HashMap<String, ModelMetadata>,
```

**Step 3: Update `ModelMetadata` doc comment**

Change the doc comment on `ModelMetadata` (line 773) from:

```rust
/// Model metadata from ~/.chibi/models.toml.
```

to:

```rust
/// Per-model API parameter overrides.
///
/// Configured under `[models."<model-id>"]` in `config.toml` (global) or
/// `local.toml` (per-context). Contains only API parameter overrides; model
/// capabilities come from ratatoskr's registry.
```

**Step 4: Verify it compiles**

```bash
cargo build -p chibi-core 2>&1 | head -30
```

Expected: compile error about `Config` missing `models` field in struct literals (tests). That's expected — fix in Task 2.

---

### Task 2: Update `AppState` — drop `models_config`, add deprecation warning

**Files:**
- Modify: `crates/chibi-core/src/state/mod.rs`

**Step 1: Remove `ModelsConfig` import and `models_config` field**

In `use crate::config::{Config, ConfigDefaults, ModelsConfig, ResolvedConfig};` — remove `ModelsConfig`.

Remove the `pub models_config: ModelsConfig,` field from `AppState`.

**Step 2: Update `AppState::from_dir` (test constructor)**

Remove:
```rust
models_config: ModelsConfig::default(),
```
from the `Ok(AppState { ... })` initialiser.

**Step 3: Update `AppState::load`**

Replace the entire `models.toml` loading block:

```rust
let models_path = chibi_dir.join("models.toml");
// ...
// Load models.toml (optional)
let models_config: ModelsConfig = if models_path.exists() { ... } else { ModelsConfig::default() };
```

With a deprecation warning:

```rust
let models_path = chibi_dir.join("models.toml");
if models_path.exists() {
    eprintln!(
        "[WARN] ~/.chibi/models.toml is deprecated. \
         Move your [models] sections into config.toml and delete models.toml."
    );
}
```

Remove `models_config` from the `AppState { ... }` initialiser at the bottom of `load()`.

**Step 4: Verify it compiles**

```bash
cargo build -p chibi-core 2>&1 | head -40
```

Expected: errors in `config_resolution.rs` about `self.models_config` — fix in Task 3.

---

### Task 3: Update `resolve_config` to use `config.models` and `local.models`

**Files:**
- Modify: `crates/chibi-core/src/state/config_resolution.rs`

**Step 1: Remove `resolve_model_name`**

Delete the entire `resolve_model_name` method (lines 43–53) — it's a no-op and no longer needed.

**Step 2: Update `resolve_config`**

Find the block at the bottom of `resolve_config` (around line 142):

```rust
// Resolve model name and potentially override context window + API params
resolved.model = self.resolve_model_name(&resolved.model);
if let Some(model_meta) = self.models_config.models.get(&resolved.model) {
    let model_api = resolved.api.merge_with(&model_meta.api);
    resolved.api = if let Some(ref local_api) = local.api {
        model_api.merge_with(local_api)
    } else {
        model_api
    };
}
```

Replace with:

```rust
// Apply per-model API param overrides (global config.models, then local.models on top)
// Both layers are overridden by explicit local.api (context-level params always win).
if let Some(model_meta) = self.config.models.get(&resolved.model) {
    let model_api = resolved.api.merge_with(&model_meta.api);
    resolved.api = if let Some(ref local_api) = local.api {
        model_api.merge_with(local_api)
    } else {
        model_api
    };
}
if let Some(model_meta) = local.models.get(&resolved.model) {
    let model_api = resolved.api.merge_with(&model_meta.api);
    resolved.api = if let Some(ref local_api) = local.api {
        model_api.merge_with(local_api)
    } else {
        model_api
    };
}
```

**Step 3: Verify it compiles**

```bash
cargo build -p chibi-core 2>&1 | head -40
```

Expected: errors in tests about `models_config`. Fix in Task 4.

---

### Task 4: Update tests

**Files:**
- Modify: `crates/chibi-core/src/state/tests.rs`

**Step 1: Find the two test functions that use `models_config`**

Search for `models_config` in the test file — there are two tests:
- `test_resolve_config_hierarchy_model_level` (around line 560)
- `test_resolve_config_hierarchy_context_over_model` (around line 632)

**Step 2: Update `test_resolve_config_hierarchy_model_level`**

Replace:
```rust
app.models_config.models.insert(
    "test-model".to_string(),
    crate::config::ModelMetadata { api: ApiParams { ... } },
);
```

With (insert into `config.models` directly — `Config` is `pub`, so mutate it):
```rust
app.config.models.insert(
    "test-model".to_string(),
    crate::config::ModelMetadata {
        api: ApiParams {
            temperature: Some(0.5),
            reasoning: crate::config::ReasoningConfig {
                effort: Some(crate::config::ReasoningEffort::High),
                ..Default::default()
            },
            ..Default::default()
        },
    },
);
```

**Step 3: Update `test_resolve_config_hierarchy_context_over_model`**

Same pattern — replace `app.models_config.models.insert(...)` with `app.config.models.insert(...)`.

**Step 4: Add a new test for local.models overriding config.models**

After `test_resolve_config_hierarchy_context_over_model`, add:

```rust
#[test]
#[serial_test::serial]
fn test_resolve_config_local_models_override_global_models() {
    // local.models should override config.models for the same model key
    let temp_dir = TempDir::new().unwrap();
    let mut config = Config {
        model: Some("test-model".to_string()),
        ..Config::default()
    };
    config.models.insert(
        "test-model".to_string(),
        crate::config::ModelMetadata {
            api: ApiParams {
                temperature: Some(0.3),
                max_tokens: Some(500),
                ..Default::default()
            },
        },
    );

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    // local.models overrides temperature but not max_tokens
    let mut local = LocalConfig::default();
    local.models.insert(
        "test-model".to_string(),
        crate::config::ModelMetadata {
            api: ApiParams {
                temperature: Some(0.9),
                ..Default::default()
            },
        },
    );
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();

    assert_eq!(resolved.api.temperature, Some(0.9)); // local.models wins
    assert_eq!(resolved.api.max_tokens, Some(500));  // config.models preserved
}
```

**Step 5: Run the tests**

```bash
cargo test -p chibi-core resolve_config 2>&1
```

Expected: all passing.

**Step 6: Commit**

```bash
git add crates/chibi-core/src/config.rs \
        crates/chibi-core/src/state/mod.rs \
        crates/chibi-core/src/state/config_resolution.rs \
        crates/chibi-core/src/state/tests.rs
git commit -m "refactor(config): fold models.toml into config.toml and local.toml

Per-model API param overrides now live under [models.\"<id>\"] in
config.toml (global) and local.toml (per-context). local.models takes
precedence over config.models. models.toml is deprecated with a warning."
```

---

### Task 5: Remove `ModelsConfig` from public API and clean up `lib.rs`

**Files:**
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: Remove `ModelsConfig` from the pub use line**

Find (line 61):
```rust
pub use config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig, ToolsConfig};
```

Remove `ModelsConfig`:
```rust
pub use config::{ApiParams, Config, LocalConfig, ResolvedConfig, ToolsConfig};
```

**Step 2: Update the module doc comment**

In `src/lib.rs` (line 10), the comment mentions `models.toml`. Update it to remove that reference.

**Step 3: Update `src/chibi.rs` doc comment** (line 10 and 143)

Both mention `models.toml` — remove those references. The new minimal example just needs `config.toml`.

**Step 4: Delete `ModelsConfig` struct from `config.rs`**

Remove the entire `ModelsConfig` struct (lines 784–789):
```rust
/// Models config containing model aliases/metadata
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default)]
    pub models: HashMap<String, ModelMetadata>,
}
```

**Step 5: Verify full build**

```bash
cargo build 2>&1 | head -30
```

Expected: clean.

**Step 6: Commit**

```bash
git add crates/chibi-core/src/lib.rs \
        crates/chibi-core/src/config.rs \
        crates/chibi-core/src/chibi.rs
git commit -m "chore: remove ModelsConfig from public API, clean up doc references to models.toml"
```

---

### Task 6: Update `model_info.rs` — fix TOML output comment

**Files:**
- Modify: `crates/chibi-core/src/model_info.rs`

**Step 1: Fix the module doc comment**

Line 4:
```rust
//! - **TOML** ([`format_model_toml`]): For `models.toml` copy-paste (CLI `-m`/`-M`).
```

Change to:
```rust
//! - **TOML** ([`format_model_toml`]): For copy-pasting into `config.toml` (CLI `-m`/`-M`).
```

**Step 2: Fix `format_model_toml` doc comment**

Line 32:
```rust
/// Format model metadata as TOML for `models.toml`.
```

Change to:
```rust
/// Format model metadata as TOML for copy-pasting into `config.toml`.
```

**Step 3: Run model_info tests**

```bash
cargo test -p chibi-core model_info 2>&1
```

Expected: all passing (output format unchanged, only comments differ).

**Step 4: Commit**

```bash
git add crates/chibi-core/src/model_info.rs
git commit -m "docs(model_info): update TOML output comments to reference config.toml"
```

---

### Task 7: Update example files

**Files:**
- Modify: `examples/config.example.toml`
- Delete: `examples/models.example.toml`

**Step 1: Add `[models]` section to `config.example.toml`**

Append to the end of `examples/config.example.toml`:

```toml
# =============================================================================
# Per-Model API Parameter Overrides
# =============================================================================
# Override API parameters for specific models. Keys match the model IDs used
# in `model` or local.toml. These are applied after global [api] settings but
# before per-context local.toml settings.
#
# Use `chibi -M <model>` to see what parameters a model supports.

# Claude with extended thinking
# [models."anthropic/claude-sonnet-4".api.reasoning]
# max_tokens = 32000

# OpenAI reasoning models
# [models."openai/o3".api]
# max_tokens = 100000
#
# [models."openai/o3".api.reasoning]
# effort = "high"

# xAI Grok reasoning
# [models."x-ai/grok-3-beta".api.reasoning]
# effort = "high"
```

**Step 2: Update the comment on line 28 of `config.example.toml`**

Find:
```toml
# If using models.toml, the context_window from there takes precedence
```

Remove that line (it's no longer accurate and `context_window` in models.toml was already removed).

**Step 3: Delete `examples/models.example.toml`**

```bash
git rm examples/models.example.toml
```

**Step 4: Commit**

```bash
git add examples/config.example.toml
git commit -m "chore(examples): fold models.example.toml into config.example.toml, delete old file"
```

---

### Task 8: Update docs

**Files:**
- Modify: `docs/configuration.md`
- Modify: `docs/architecture.md`
- Modify: `docs/cli-reference.md`

**Step 1: Update `docs/configuration.md`**

Find the `## Model Metadata (models.toml)` section (around line 254). Replace the section header and intro with:

```markdown
## Per-Model API Parameters

Per-model API parameter overrides under `[models."<model-id>"]` in `config.toml` (global)
or `local.toml` (per-context). Local overrides take precedence over global.
```

Update the priority stack comment (around line 513) — remove `models.toml` as a separate layer and replace with `config.toml [models]` and `local.toml [models]` in the right positions.

Remove any reference to `~/.chibi/models.toml` as a file path.

**Step 2: Update `docs/architecture.md`**

Find the storage layout (around line 85):
```
├── config.toml, models.toml
```

Change to:
```
├── config.toml
```

**Step 3: Update `docs/cli-reference.md`**

Find the `--model-metadata` / `-m`/`-M` description (around line 112). It mentions "only fields you can set in `models.toml`". Change to "only fields you can set under `[models]` in `config.toml`".

**Step 4: Run tests to make sure nothing broke**

```bash
cargo test 2>&1 | tail -20
```

Expected: all passing.

**Step 5: Commit**

```bash
git add docs/configuration.md docs/architecture.md docs/cli-reference.md
git commit -m "docs: update configuration docs to reflect models.toml removal"
```

---

### Task 9: Final check

**Step 1: Search for any remaining `models.toml` references in non-plan source files**

```bash
grep -r "models\.toml" --include="*.rs" --include="*.md" \
  --exclude-dir="docs/plans" .
```

Expected: only the deprecation warning string in `state/mod.rs` and possibly `docs/plans/` — nothing else.

**Step 2: Full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all passing.

**Step 3: Run `just pre-push` if available**

```bash
just pre-push
```
