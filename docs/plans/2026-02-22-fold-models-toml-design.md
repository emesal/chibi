# Design: Fold `models.toml` into `config.toml` / `local.toml`

## Context

`models.toml` predates ratatoskr. It originally held context window sizes, capability flags, and
model aliases — all of which ratatoskr now owns. What remains is one legitimate feature:
per-model API parameter overrides (e.g. reasoning effort, max tokens for thinking).

The file is now a one-trick pony that adds a second config file for a feature that fits naturally
into the existing two-file hierarchy (`config.toml` global, `local.toml` per-context). Folding it
in also unlocks a new capability: per-context per-model overrides, which `models.toml` never
supported.

## What Changes

### `Config` (config.toml)

Add a `models` field:

```rust
#[serde(default)]
pub models: HashMap<String, ModelMetadata>,
```

Example usage:

```toml
[models."anthropic/claude-sonnet-4".api.reasoning]
max_tokens = 32000

[models."x-ai/grok-3-beta".api.reasoning]
effort = "high"
```

### `LocalConfig` (local.toml)

Add the same `models` field:

```rust
#[serde(default)]
pub models: HashMap<String, ModelMetadata>,
```

Example usage (per-context override):

```toml
[models."openai/o3".api.reasoning]
effort = "low"
```

### Deleted

- `ModelsConfig` struct
- `AppState.models_config` field
- Loading of `~/.chibi/models.toml` in `AppState::load()`
- `examples/models.example.toml` (content folded into `examples/config.example.toml`)

## Resolution Order

```
runtime override           (highest priority)
local.toml [models]        per-context per-model api params
local.toml                 per-context general settings
env vars                   CHIBI_API_KEY, CHIBI_MODEL
config.toml [models]       global per-model api params
config.toml                global settings
defaults
```

The merge in `resolve_config`:
1. Look up resolved model in `self.config.models` → apply api params on top of global api
2. Look up resolved model in `local.models` → apply api params on top, winning over global model params
3. Re-apply `local.api` on top (existing behaviour, context-level params always win)

This is the same double-merge pattern already used for `api` params between global and local.

## Migration

- On startup, if `~/.chibi/models.toml` exists → emit a one-time deprecation warning telling the
  user to move their settings into `config.toml` and delete the file.
- No auto-migration. Pre-alpha; backwards compatibility is not a priority.

## Docs to Update

- `docs/configuration.md` — remove `models.toml` section; add `[models]` to `config.toml` and
  `local.toml` sections
- `docs/architecture.md` — remove `models.toml` from storage layout
- `docs/cli-reference.md` — update `--model-metadata` / `-m`/`-M` blurb (no longer mentions
  `models.toml` as the destination)
- Module-level doc comments in `chibi.rs` and `lib.rs` that reference `models.toml`
- `examples/config.example.toml` — add `[models]` section with examples
