# Model Metadata CLI Flag Design

**Date:** 2026-02-05
**Issues:** #87, #88
**Status:** Ready for implementation

## Overview

Add CLI flags to query model metadata from ratatoskr's registry and output in TOML format for easy copy-paste into `models.toml`.

## Background

Issues #87 and #88 originally proposed a model-info command and embedded registry. Since then, ratatoskr was created to handle LLM infrastructure, including a `ModelRegistry` with embedded seed data. Chibi now consumes ratatoskr's registry rather than maintaining its own.

## Design

### CLI Flags

- `-m <MODEL>` / `--model-metadata <MODEL>` — minimal output, just settable fields
- `-M <MODEL>` / `--model-metadata-full <MODEL>` — full output with pricing, capabilities, parameter ranges as comments

Both flags imply `--no-chibi` (no LLM invocation).

### Output Format

**Minimal (`-m`):**
```toml
[models."anthropic/claude-sonnet-4"]
context_window = 200000

[models."anthropic/claude-sonnet-4".api]
max_tokens = 16384
```

**Full (`-M`):**
```toml
# anthropic/claude-sonnet-4
# provider: openrouter
# capabilities: Chat
# pricing: $3.00 / $15.00 per MTok (prompt/completion)

[models."anthropic/claude-sonnet-4"]
context_window = 200000

[models."anthropic/claude-sonnet-4".api]
max_tokens = 16384
# temperature: 0.0 - 1.0 (default: 1.0)
# top_p: 0.0 - 1.0
# top_k: min 1.0
# reasoning: supported
```

### Command Variant

```rust
// crates/chibi-core/src/input.rs
pub enum Command {
    // ... existing variants ...
    ModelMetadata { model: String, full: bool },
}
```

### Execution Flow

1. `cli.rs` parses `-m`/`-M` flag, sets `Command::ModelMetadata`
2. `main.rs` matches command, builds gateway via `build_gateway()`
3. Calls `gateway.model_metadata(&model)` (from `ModelGateway` trait)
4. If `None`: print error to stderr, exit 1
5. If `Some`: format as TOML, print to stdout

### Error Handling

- Model not in registry: `"model 'X' not found in registry"` to stderr, exit 1
- Gateway build failure: existing error handling

## Files to Change

- `crates/chibi-cli/src/cli.rs` — add flags and parsing
- `crates/chibi-core/src/input.rs` — add `Command::ModelMetadata` variant
- `crates/chibi-core/src/model_info.rs` — new file, TOML formatting logic
- `crates/chibi-cli/src/main.rs` — dispatch the command
- `crates/chibi-core/src/lib.rs` — export new module

## Out of Scope

- Refresh command (ratatoskr handles caching/fetching internally)
- Chibi-side registry (ratatoskr owns it)
- Changes to existing `models.toml` handling

## Dependencies

- ratatoskr v0.1.1+ with `ModelGateway::model_metadata()` and `ModelRegistry::with_embedded_seed()`
