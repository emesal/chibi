# Design: `SetModel` CLI Command

**Date:** 2026-02-20
**Status:** Approved

## Summary

Add `--set-model` / `-m` (current context) and `--set-model-for-context` / `-M` (named context) to
chibi-cli, and the corresponding `set_model` command to the shared `Command` enum for chibi-json.
Model names are validated live via ratatoskr's network API before being written to `local.toml`.
Short flags `-m` / `-M` are reassigned from `--model-metadata` / `--model-metadata-full`, which
become long-only flags.

## Motivation

Currently there is no CLI way to persistently set the model for a context short of editing
`local.toml` by hand. The analogous username operation (`-u` / `--set-username`) and system-prompt
operation (`-y` / `--set-current-system-prompt`) already have this pattern established.

## Design

### Command variant (chibi-core `input.rs`)

```rust
/// Set model for a context (-m/--set-model, -M/--set-model-for-context)
SetModel {
    context: Option<String>,
    model: String,
},
```

`context: None` = current context (CLI); `context: Some(name)` = named context (CLI + JSON).

### Event variant (chibi-core `output.rs`)

```rust
/// Model saved to local.toml for a context (verbose-tier).
ModelSet { model: String, context: String },
```

Follows the same verbose-tier convention as `SystemPromptSet` and `UsernameSaved`.

### Execution handler (chibi-core `execution.rs`)

```rust
Command::SetModel { context: ctx, model } => {
    let ctx_name = ctx.as_deref().unwrap_or(context);
    let gateway = crate::gateway::build_gateway(resolved)?;
    // Live validation via ratatoskr (registry → cache → network).
    // Unknown / invalid model IDs are rejected here.
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

`save_local_config` already uses `safe_io::atomic_write_text` — no additional locking needed.

### chibi-cli flag changes

| Old | New |
|-----|-----|
| `-m, --model-metadata <MODEL>` | `--model-metadata <MODEL>` (long only) |
| `-M, --model-metadata-full <MODEL>` | `--model-metadata-full <MODEL>` (long only) |
| *(new)* | `-m, --set-model <MODEL>` — set model for current context (combinable with prompt) |
| *(new)* | `-M, --set-model-for-context <CTX> <MODEL>` — set model for named context (implies `--no-chibi`) |

`-m` / `-M` remain in `ATTACHED_FLAGS` (they still accept an attached value).

**Implied `--no-chibi`:** add `set_model_for_context.is_some()` (named-context variant only, same
rule as `-Y` / `--set-system-prompt`). The current-context form `-m` is combinable with a prompt,
like `-y`.

### chibi-json

No code changes. `Command::SetModel` is in the shared enum; the handler in `execution.rs` covers
both binaries. Context is always explicit in chibi-json; `context: None` is only reachable from
the CLI path.

Ephemeral model selection in chibi-json continues via the existing `config.model` (typed) or
`overrides.model` (string) fields in `JsonInput` — these are per-invocation only and do not write
to `local.toml`.

## Ephemeral vs. Persistent Summary

| | Ephemeral | Persistent |
|---|---|---|
| **chibi-cli** | `-s model=<model>` | `-m <model>` / `--set-model` |
| **chibi-json** | `config.model` or `overrides.model` | `command: set_model` |

## Validation

Model names are validated via `model_info::fetch_metadata`, which calls
`EmbeddedGateway::fetch_model_metadata` (embedded registry → OpenRouter network on miss). An
unknown or unreachable model ID produces an `io::Error` and no write occurs.

## Documentation changes

- `docs/cli-reference.md` — add **Model** section for `-m`/`-M`; update **Model Metadata** section
  to remove short flags; update the implied `-x` and combinable lists; update chibi-json command
  list to include `set_model`.
- `docs/configuration.md` — update **Per-Context Configuration** section to document CLI shortcut
  for model setting (parallel to the existing username example).
