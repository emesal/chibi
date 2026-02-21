# Subagent Preset Support Design

## Context

Closes #117. Ratatoskr v0.3.2 now exposes `ModelGateway::list_presets()` and
`ModelGateway::resolve_preset(tier, capability)` via the `ModelGateway` trait.
Presets are a two-level map: cost tier → capability name → `PresetEntry`
(model ID + optional `PresetParameters` defaults).

## Design

### Cost Tier: User-Controlled, LLM-Opaque

The LLM never sees or sets the cost tier. It is a user policy decision,
configured via `config.toml` (global) or `JsonInput.config` (per-invocation
from chibi-json). The CLI has no flag for it — there's no clean per-invocation
UX for it there.

New field: `subagent_cost_tier: String`, default `"free"`.
Flows through the standard config stack: `Config` → `LocalConfig` →
`ResolvedConfig`, via the existing `apply_option_overrides!` macro.

### Capability: LLM-Chosen, Baked Into Tool Description

The LLM passes a `preset` parameter (just the capability name string) to
`spawn_agent`. Available capability names are listed directly in the `preset`
parameter's description at tool-assembly time, so the LLM has full context
without a separate discovery call.

Because tool descriptions are currently static (`&'static str` in
`AGENT_TOOL_DEFS`), the agent tool assembly function is refactored to accept
the available capability names and build the description dynamically.

### Scope

- `spawn_agent`: gains `preset: Option<String>` in `SpawnOptions` and the
  tool schema.
- `retrieve_content`: no change. It delegates to `spawn_agent` internally but
  must support tool calls (summariser models aren't guaranteed to), so it stays
  pinned to the parent model or an explicit `model` override.

### Resolution Order in `apply_spawn_options`

Explicit overrides always win over preset defaults:

1. If `preset` is set, call `gateway.resolve_preset(&config.subagent_cost_tier, &capability)`.
2. Apply `resolution.model` as the model (unless `opts.model` is also set — explicit wins).
3. Apply `resolution.parameters` as defaults to `ApiParams` (fill `None` fields only).
4. Apply explicit `opts.model` / `opts.temperature` / `opts.max_tokens` overrides on top.

`apply_spawn_options` grows an `Option<&EmbeddedGateway>` parameter. At the
call site in `spawn_agent`, the gateway is built from the parent config (same
as `gateway::chat` does internally — no extra cost since we need it anyway for
the chat call).

If no preset is configured for the requested tier/capability, fall back
gracefully: log a warning, proceed with parent config unchanged.

### Dynamic Tool Description

`all_agent_tools_to_api_format()` becomes
`all_agent_tools_to_api_format(preset_capabilities: &[&str])`.

The `preset` parameter description reads:
- If capabilities non-empty: `"Preset capability name — one of: fast, reasoning, … (cost tier set by config)"`
- If empty: `"Preset capability name (no presets configured)"` — param still present but LLM knows it's a no-op.

The call site in `send.rs` passes capability names obtained via
`gateway.list_presets()` (already available there from `build_gateway`).

### `ApiParams` Defaults Application

`PresetParameters::apply_defaults_to_chat` operates on ratatoskr's
`ChatOptions`. Chibi works at the `ApiParams` level. A small private helper
`apply_preset_defaults(params: &PresetParameters, api: &mut ApiParams)` fills
`None` fields in `ApiParams` from `PresetParameters` — mirrors the same
"fill None, never overwrite Some" semantics, inline in `agent_tools.rs`.

## Files Changed

- `crates/chibi-core/src/config.rs` — add `subagent_cost_tier` to `Config`,
  `LocalConfig`, `ResolvedConfig`; wire through overrides, get/list fields,
  resolve_config initialiser
- `crates/chibi-core/src/tools/agent_tools.rs` — add `preset` to
  `SpawnOptions` + `from_args`; refactor `apply_spawn_options` to accept
  `Option<&EmbeddedGateway>`; add `apply_preset_defaults` helper; update tool
  schemas and `all_agent_tools_to_api_format` signature
- `crates/chibi-core/src/api/send.rs` — pass preset capabilities to
  `all_agent_tools_to_api_format`
- `docs/configuration.md` — document `subagent_cost_tier`
- `docs/agentic.md` — document preset usage

## Testing

- Unit: `apply_spawn_options` with preset — model set, params applied as
  defaults, explicit overrides win
- Unit: `apply_preset_defaults` — fills None, preserves Some
- Unit: `all_agent_tools_to_api_format` with/without capabilities — description
  correct in both cases
- Unit: `subagent_cost_tier` flows through config override chain
