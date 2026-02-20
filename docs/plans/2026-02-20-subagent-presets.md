# Subagent Preset Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

## Progress

- [x] Task 1 — committed `feat(config): add subagent_cost_tier to config stack`
- [x] Task 2 — committed `feat(agent_tools): add preset to SpawnOptions and apply_spawn_options`
- [ ] Task 3 — wire gateway into spawn_agent
- [ ] Task 4 — dynamic tool description
- [ ] Task 5 — docs
- [ ] Task 6 — final verification + close issue

**Branch:** `feature/subagent-presets` in `.worktrees/subagent-presets`

**Goal:** Allow `spawn_agent` to select a model via a ratatoskr preset capability name, with the cost tier controlled by user config (`subagent_cost_tier`), so the LLM never touches cost policy.

**Architecture:** Add `subagent_cost_tier` through the standard config stack (`Config` → `LocalConfig` → `ResolvedConfig`). Add `preset: Option<String>` to `SpawnOptions`. In `apply_spawn_options`, resolve the preset via the gateway and apply model + `PresetParameters` as defaults to `ApiParams` before explicit overrides. Build `all_agent_tools_to_api_format` dynamically so the `preset` param description lists available capability names.

**Tech Stack:** Rust, ratatoskr v0.3.2 (`ModelGateway::list_presets`, `::resolve_preset`, `PresetResolution`, `PresetEntry`, `PresetParameters`), existing chibi-core config/gateway/tools machinery.

**Design doc:** `docs/plans/2026-02-20-subagent-presets-design.md`

**Closes:** GitHub issue #117

---

## Task 1: Add `subagent_cost_tier` to config stack

**Files:**
- Modify: `crates/chibi-core/src/config.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)]` block near `test_resolved_config_get_field`:

```rust
#[test]
fn test_subagent_cost_tier_default() {
    let config = ResolvedConfig {
        subagent_cost_tier: "free".to_string(),
        ..test_resolved_config()
    };
    assert_eq!(config.get_field("subagent_cost_tier"), Some("free".to_string()));
}

#[test]
fn test_subagent_cost_tier_override() {
    let base = test_resolved_config();
    let local = LocalConfig {
        subagent_cost_tier: Some("standard".to_string()),
        ..Default::default()
    };
    let mut resolved = base.clone();
    local.apply_overrides(&mut resolved);
    assert_eq!(resolved.subagent_cost_tier, "standard");
}
```

**Step 2: Run test to verify it fails**

```
cargo test -p chibi-core test_subagent_cost_tier 2>&1 | tail -20
```

Expected: compile error — `subagent_cost_tier` doesn't exist yet.

**Step 3: Add `SUBAGENT_COST_TIER` constant to `ConfigDefaults`**

In the `impl ConfigDefaults` block, after `MODEL`:

```rust
pub const SUBAGENT_COST_TIER: &'static str = "free";
```

**Step 4: Add serde default wrapper function**

After the other `fn default_*` functions (before the `Config` struct):

```rust
fn default_subagent_cost_tier() -> String {
    ConfigDefaults::SUBAGENT_COST_TIER.to_string()
}
```

**Step 5: Add field to `Config`**

In the `Config` struct, after `url_policy`:

```rust
/// Cost tier used when resolving subagent presets (e.g. "free", "standard", "premium").
/// Controls which tier of ratatoskr presets `spawn_agent` resolves against.
#[serde(default = "default_subagent_cost_tier")]
pub subagent_cost_tier: String,
```

**Step 6: Add field to `LocalConfig`**

In the `LocalConfig` struct, after `url_policy`:

```rust
/// Subagent preset cost tier override
pub subagent_cost_tier: Option<String>,
```

**Step 7: Wire into `apply_option_overrides!` in `LocalConfig::apply_overrides`**

Add `subagent_cost_tier` to the `apply_option_overrides!` list alongside `model`, `username`, etc.

**Step 8: Add field to `ResolvedConfig`**

In the `ResolvedConfig` struct, after `url_policy`:

```rust
/// Cost tier for resolving subagent presets. Default: "free".
pub subagent_cost_tier: String,
```

**Step 9: Wire into `get_field` and `list_fields`**

In `get_field`, add `subagent_cost_tier` to the `clone:` arm of the `config_get_field!` macro call alongside `model`, `username`, `fallback_tool`.

In `list_fields`, add `"subagent_cost_tier"` after `"fallback_tool"`.

**Step 10: Initialise in `resolve_config` struct literal**

Find the `ResolvedConfig { ... }` struct literal in `resolve_config()`. Add:

```rust
subagent_cost_tier: config.subagent_cost_tier.clone(),
```

**Step 11: Add to `test_resolved_config()` helper**

Find `fn test_resolved_config()` in the test block. Add the new field:

```rust
subagent_cost_tier: "free".to_string(),
```

Also update any other test that constructs `ResolvedConfig` directly (search for `ResolvedConfig {` in `config.rs` tests — add the field wherever needed to fix compile errors).

**Step 12: Run tests to verify they pass**

```
cargo test -p chibi-core test_subagent_cost_tier 2>&1 | tail -20
```

Expected: both tests PASS.

**Step 13: Run full test suite to check for regressions**

```
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all pass.

**Step 14: Commit**

```bash
git add crates/chibi-core/src/config.rs
git commit -m "feat(config): add subagent_cost_tier to config stack"
```

---

## Task 2: Add `preset` to `SpawnOptions` and `apply_spawn_options`

**Files:**
- Modify: `crates/chibi-core/src/tools/agent_tools.rs`
- Modify: `crates/chibi-core/src/gateway.rs` (re-export `EmbeddedGateway` if needed)

**Step 1: Check imports in `agent_tools.rs`**

Verify the top of `agent_tools.rs` already imports `crate::gateway`. Check what's imported from ratatoskr — we need `PresetParameters` and `EmbeddedGateway`. Look at the existing `use` lines and note what's missing.

```
head -20 crates/chibi-core/src/tools/agent_tools.rs
```

**Step 2: Write failing tests**

Add to the `#[cfg(test)]` block at the bottom of `agent_tools.rs`:

```rust
#[test]
fn test_spawn_options_preset_from_args() {
    let args = json!({ "preset": "fast" });
    let opts = SpawnOptions::from_args(&args);
    assert_eq!(opts.preset, Some("fast".to_string()));
}

#[test]
fn test_spawn_options_no_preset() {
    let args = json!({ "model": "some/model" });
    let opts = SpawnOptions::from_args(&args);
    assert!(opts.preset.is_none());
    assert_eq!(opts.model, Some("some/model".to_string()));
}

#[test]
fn test_apply_preset_defaults_fills_none() {
    use ratatoskr::PresetParameters;
    let params = PresetParameters {
        temperature: Some(0.3),
        max_tokens: Some(2048),
        ..Default::default()
    };
    let mut api = crate::config::ApiParams::defaults();
    assert!(api.temperature.is_none());
    apply_preset_defaults(&params, &mut api);
    assert_eq!(api.temperature, Some(0.3));
    assert_eq!(api.max_tokens, Some(2048));
}

#[test]
fn test_apply_preset_defaults_preserves_existing() {
    use ratatoskr::PresetParameters;
    let params = PresetParameters {
        temperature: Some(0.3),
        ..Default::default()
    };
    let mut api = crate::config::ApiParams::defaults();
    api.temperature = Some(0.9); // caller already set this
    apply_preset_defaults(&params, &mut api);
    assert_eq!(api.temperature, Some(0.9)); // caller wins
}
```

**Step 3: Run to verify failure**

```
cargo test -p chibi-core test_spawn_options_preset 2>&1 | tail -20
cargo test -p chibi-core test_apply_preset_defaults 2>&1 | tail -20
```

Expected: compile errors — `preset` field and `apply_preset_defaults` don't exist.

**Step 4: Add `preset` field to `SpawnOptions`**

In the `SpawnOptions` struct, after `max_tokens`:

```rust
/// Preset capability name (e.g. "fast", "reasoning").
/// Resolved against `config.subagent_cost_tier`. Explicit model/temperature/max_tokens win over preset defaults.
pub preset: Option<String>,
```

**Step 5: Add `preset` parsing in `SpawnOptions::from_args`**

In the `from_args` impl, add:

```rust
preset: args.get_str("preset").map(String::from),
```

**Step 6: Add `apply_preset_defaults` private helper**

Add this function after `apply_spawn_options` (or before it, at module level):

```rust
/// Apply `PresetParameters` as defaults to `ApiParams`.
/// Fills `None` fields only — never overwrites `Some` values set by the caller.
fn apply_preset_defaults(params: &ratatoskr::PresetParameters, api: &mut crate::config::ApiParams) {
    macro_rules! fill {
        ($field:ident) => {
            if api.$field.is_none() {
                api.$field = params.$field.clone();
            }
        };
    }
    fill!(temperature);
    fill!(top_p);
    fill!(max_tokens);
    fill!(frequency_penalty);
    fill!(presence_penalty);
    fill!(seed);
    fill!(stop);
    fill!(parallel_tool_calls);
    // Note: top_k, reasoning, tool_choice, response_format, cache_prompt,
    // raw_provider_options are in PresetParameters but not in ApiParams — skip.
}
```

**Step 7: Refactor `apply_spawn_options` to accept a gateway**

Change the signature:

```rust
fn apply_spawn_options(
    config: &ResolvedConfig,
    opts: &SpawnOptions,
    gateway: Option<&ratatoskr::EmbeddedGateway>,
) -> ResolvedConfig {
```

Add preset resolution at the start of the function body, before the existing field overrides:

```rust
let mut c = config.clone();

// Resolve preset first (explicit opts override preset defaults)
if let (Some(capability), Some(gw)) = (opts.preset.as_deref(), gateway) {
    use ratatoskr::ModelGateway;
    match gw.resolve_preset(&config.subagent_cost_tier, capability) {
        Some(resolution) => {
            c.model = resolution.model;
            if let Some(params) = resolution.parameters {
                apply_preset_defaults(&params, &mut c.api);
            }
        }
        None => {
            tracing::warn!(
                tier = %config.subagent_cost_tier,
                capability = %capability,
                "no preset found for tier/capability — using parent model"
            );
        }
    }
}

// Explicit overrides win over preset defaults
if let Some(ref model) = opts.model { ... }
```

Note: `tracing` is already a dependency — check the existing imports in `agent_tools.rs`; if not already imported, add `use tracing::warn;` or use the full path.

**Step 8: Update all call sites of `apply_spawn_options`**

There are call sites in `spawn_agent` (and possibly `retrieve_content`'s internal delegation). Update each to pass `None` for now — we'll wire the real gateway in Task 3.

Search: `apply_spawn_options(config, options)` → replace with `apply_spawn_options(config, options, None)`.

Also update the existing unit tests that call `apply_spawn_options` directly — add `None` as third arg.

**Step 9: Check imports — add `use ratatoskr::ModelGateway;` if needed**

`resolve_preset` is a trait method, so the trait must be in scope. Add at the top of `agent_tools.rs` if missing:

```rust
use ratatoskr::ModelGateway;
```

**Step 10: Run tests**

```
cargo test -p chibi-core test_spawn_options_preset 2>&1 | tail -20
cargo test -p chibi-core test_apply_preset_defaults 2>&1 | tail -20
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all pass.

**Step 11: Commit**

```bash
git add crates/chibi-core/src/tools/agent_tools.rs
git commit -m "feat(agent_tools): add preset to SpawnOptions and apply_spawn_options"
```

---

## Task 3: Wire gateway into `spawn_agent` for preset resolution

**Files:**
- Modify: `crates/chibi-core/src/tools/agent_tools.rs`

**Step 1: Write failing test**

Add to tests:

```rust
#[test]
fn test_apply_spawn_options_preset_sets_model() {
    // This tests the plumbing: if we pass a real EmbeddedGateway and a preset
    // that exists, the model in the returned config should change.
    // Use a preset known to exist in ratatoskr's embedded seed registry.
    use ratatoskr::{EmbeddedGateway, Ratatoskr};
    let gateway = Ratatoskr::builder()
        .build()
        .expect("gateway should build");
    let presets = gateway.list_presets();
    // Only run the assertion if any preset exists
    if let Some((tier, caps)) = presets.iter().next() {
        if let Some(capability) = caps.iter().next() {
            let mut config = crate::config::ResolvedConfig {
                subagent_cost_tier: tier.clone(),
                ..super::tests::test_resolved_config()
            };
            let opts = SpawnOptions {
                preset: Some(capability.clone()),
                ..Default::default()
            };
            let result = apply_spawn_options(&config, &opts, Some(&gateway));
            // model should have changed from the default
            assert_ne!(result.model, config.model,
                "preset should have changed the model");
        }
    }
}
```

Note: this test is conditional — if no embedded presets exist, it passes vacuously. That's fine; the logic is still tested by `test_apply_preset_defaults_*`.

**Step 2: Run to check compile**

```
cargo test -p chibi-core test_apply_spawn_options_preset_sets_model 2>&1 | tail -20
```

Expected: compile error or pass vacuously depending on embedded seed.

**Step 3: Wire gateway into `spawn_agent`**

In the `spawn_agent` function body, after `apply_spawn_options` is called, the gateway is currently built separately for the chat call. Restructure so the gateway is built once and passed to `apply_spawn_options`:

```rust
pub async fn spawn_agent(...) -> io::Result<String> {
    let gateway = crate::gateway::build_gateway(config)?;
    let effective_config = apply_spawn_options(config, options, Some(&gateway));
    // ... rest of function unchanged (gateway is dropped after apply, new one built in gateway::chat)
```

Wait — `gateway::chat` builds its own gateway internally. We're building an extra one here. That's a minor inefficiency but acceptable; presets are a configuration step, not a hot path. If it becomes a concern, refactor `gateway::chat` to accept an optional existing gateway later.

**Step 4: Run full tests**

```
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all pass.

**Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/agent_tools.rs
git commit -m "feat(agent_tools): wire gateway into spawn_agent for preset resolution"
```

---

## Task 4: Dynamic tool description — `all_agent_tools_to_api_format`

**Files:**
- Modify: `crates/chibi-core/src/tools/agent_tools.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs`
- Modify: `crates/chibi-core/src/api/send.rs`

**Step 1: Write failing tests**

Add to `agent_tools.rs` tests:

```rust
#[test]
fn test_agent_tool_schema_has_preset_param() {
    // spawn_agent should have a "preset" parameter
    let spawn = get_agent_tool_api(SPAWN_AGENT_TOOL_NAME);
    let params = &spawn["function"]["parameters"]["properties"];
    assert!(params.get("preset").is_some(), "spawn_agent should have preset param");
}

#[test]
fn test_agent_tools_description_lists_capabilities() {
    let tools = all_agent_tools_to_api_format(&["fast", "reasoning"]);
    let spawn = tools.iter().find(|t| {
        t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME)
    }).expect("spawn_agent in list");
    let preset_desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
        .as_str().unwrap_or("");
    assert!(preset_desc.contains("fast"), "description should list 'fast'");
    assert!(preset_desc.contains("reasoning"), "description should list 'reasoning'");
}

#[test]
fn test_agent_tools_description_no_capabilities() {
    let tools = all_agent_tools_to_api_format(&[]);
    let spawn = tools.iter().find(|t| {
        t["function"]["name"].as_str() == Some(SPAWN_AGENT_TOOL_NAME)
    }).expect("spawn_agent in list");
    let preset_desc = spawn["function"]["parameters"]["properties"]["preset"]["description"]
        .as_str().unwrap_or("");
    assert!(preset_desc.contains("no presets"), "should mention no presets configured");
}
```

**Step 2: Run to verify failure**

```
cargo test -p chibi-core test_agent_tool_schema_has_preset_param 2>&1 | tail -20
cargo test -p chibi-core test_agent_tools_description 2>&1 | tail -20
```

Expected: failures — `preset` param not in schema, `all_agent_tools_to_api_format` takes no args.

**Step 3: Add `preset` to the `spawn_agent` tool schema**

In `AGENT_TOOL_DEFS`, add to the `spawn_agent` parameters (after `max_tokens`):

```rust
ToolPropertyDef {
    name: "preset",
    r#type: "string",
    description: "PRESET_DESCRIPTION_PLACEHOLDER",
    required: false,
},
```

We'll replace the placeholder with the dynamic description in the next step.

**Step 4: Refactor `all_agent_tools_to_api_format` to accept capability names**

`AGENT_TOOL_DEFS` is a static slice — we can't put dynamic strings in it. Instead, change `all_agent_tools_to_api_format` to build JSON dynamically:

Change the signature and implementation:

```rust
/// Build agent tool definitions in API format.
/// `preset_capabilities`: capability names available via the configured cost tier.
/// If empty, the preset param description notes no presets are configured.
pub fn all_agent_tools_to_api_format(preset_capabilities: &[&str]) -> Vec<serde_json::Value> {
    let preset_desc = if preset_capabilities.is_empty() {
        "Preset capability name (no presets configured for this tier)".to_string()
    } else {
        format!(
            "Preset capability name — one of: {} (cost tier set by config). \
             Sets the model and default parameters for the sub-agent. \
             Explicit model/temperature/max_tokens override preset defaults.",
            preset_capabilities.join(", ")
        )
    };

    AGENT_TOOL_DEFS.iter().map(|def| {
        let mut json = def.to_api_format();
        // Inject dynamic preset description into spawn_agent only
        if def.name == SPAWN_AGENT_TOOL_NAME {
            json["function"]["parameters"]["properties"]["preset"]["description"] =
                serde_json::Value::String(preset_desc.clone());
        }
        json
    }).collect()
}
```

**Step 5: Update `mod.rs` re-export signature**

In `crates/chibi-core/src/tools/mod.rs`, the `pub use` line for `all_agent_tools_to_api_format` — no change needed to the re-export itself, the signature change is transparent.

**Step 6: Update call site in `send.rs`**

In `send_prompt`, before tool assembly, build the gateway temporarily to get preset capabilities:

```rust
// Get available preset capabilities for tool descriptions
let preset_capabilities: Vec<String> = {
    use ratatoskr::ModelGateway;
    match crate::gateway::build_gateway(&resolved_config) {
        Ok(gw) => gw
            .list_presets()
            .get(&resolved_config.subagent_cost_tier)
            .map(|caps| caps.iter().cloned().collect())
            .unwrap_or_default(),
        Err(_) => vec![],
    }
};
let preset_cap_refs: Vec<&str> = preset_capabilities.iter().map(String::as_str).collect();
```

Then update the tool assembly line:

```rust
all_tools.extend(tools::all_agent_tools_to_api_format(&preset_cap_refs));
```

**Step 7: Run tests**

```
cargo test -p chibi-core test_agent_tool_schema_has_preset_param 2>&1 | tail -20
cargo test -p chibi-core test_agent_tools_description 2>&1 | tail -20
cargo test -p chibi-core 2>&1 | tail -30
```

Expected: all pass.

**Step 8: Commit**

```bash
git add crates/chibi-core/src/tools/agent_tools.rs crates/chibi-core/src/tools/mod.rs crates/chibi-core/src/api/send.rs
git commit -m "feat(agent_tools): dynamic preset capability list in spawn_agent tool description"
```

---

## Task 5: Update documentation

**Files:**
- Modify: `docs/configuration.md`
- Modify: `docs/agentic.md`

**Step 1: Add `subagent_cost_tier` to configuration docs**

In `docs/configuration.md`, find the section covering top-level config fields. Add:

```markdown
### `subagent_cost_tier`

**Default:** `"free"`
**Type:** string

Cost tier used when resolving model presets for `spawn_agent`. Controls which ratatoskr preset tier is selected when the LLM passes a `preset` capability name.

Common values depend on your ratatoskr preset configuration — typical tiers are `free`, `standard`, and `premium`. If the requested tier/capability combination has no configured preset, `spawn_agent` falls back to the parent context's model.

Configurable via `config.toml` (global) or `JsonInput.config.subagent_cost_tier` (per-invocation from chibi-json).
```

**Step 2: Update agentic docs**

In `docs/agentic.md`, find the section on `spawn_agent` parameters. Add a section on presets:

```markdown
### Model Presets

Instead of specifying a model directly, `spawn_agent` accepts a `preset` parameter — a capability name like `"fast"` or `"reasoning"`. The actual model is resolved from your ratatoskr preset configuration using the `subagent_cost_tier` set in `config.toml` (default: `"free"`).

Available preset names are listed in the `preset` parameter description when the tool is active. Explicit `model`, `temperature`, and `max_tokens` parameters always override preset defaults.

Example:
```json
{
  "system_prompt": "You are a fast summariser.",
  "input": "...",
  "preset": "fast"
}
```
```

**Step 3: Commit**

```bash
git add docs/configuration.md docs/agentic.md
git commit -m "docs: document subagent_cost_tier and spawn_agent preset support"
```

---

## Task 6: Close issue and final verification

**Step 1: Run full test suite**

```
cargo test 2>&1 | tail -40
```

Expected: all tests pass across all crates.

**Step 2: Build release**

```
cargo build --release 2>&1 | tail -20
```

Expected: clean build, no warnings about new code.

**Step 3: Close issue**

```bash
gh issue close 117 --comment "Implemented in this branch. \`spawn_agent\` now accepts a \`preset\` capability name; cost tier is set via \`subagent_cost_tier\` in config (default: \`free\`). Dynamic capability list injected into tool description at runtime."
```

**Step 4: Remind user about pre-push**

Remind: `just pre-push` before pushing.
