# zero-config chibi — design

issue: #141
branch: `feature/M1.5-basic-composable-agent`
depends on: [ratatoskr#24](https://github.com/emesal/ratatoskr/issues/24) (merged in v0.2.3)

## progress

- [x] §1 config struct changes
- [x] §2 config loading
- [x] §3 config resolution
- [x] §4 gateway building
- [x] §5 context_window_limit resolution
- [ ] §6 models.toml cleanup
- [ ] §7 -m/-M output
- [ ] §8 api_key Option propagation
- [ ] §9 docs updates

**status:** §1–5 done, 446 tests + 10 doctests passing. ratatoskr updated to v0.2.3 (Cargo.lock). §8 partially done (CLI accessor + test updates landed with §1–4).

### what's already done

- `Config.{api_key, model, context_window_limit}` → `Option` with `serde(default)`, `Config` derives `Default`
- `ResolvedConfig.api_key` → `Option<String>`
- `ConfigDefaults::{MODEL, WARN_THRESHOLD_PERCENT, CONTEXT_WINDOW_LIMIT}` added
- `AppState::load()` returns `Config::default()` when config.toml missing
- `resolve_config()` uses `unwrap_or`/`unwrap_or_else` with `ConfigDefaults`
- `build_gateway()` passes `config.api_key.as_deref()` to ratatoskr `openrouter()`
- `should_warn()`, `remaining_tokens()` guard against `context_window_limit == 0`
- `should_auto_compact()` skips when `context_window_limit == 0`
- CLI `api_key()` accessor returns `Option<&str>`
- all test `Config` struct literals + assertions updated with `Some()` wrapping
- `resolve_context_window()` in `gateway.rs` — sync registry lookup fills `context_window_limit` when 0
- `resolve_cli_config()` calls `resolve_context_window()` after config resolution — single choke point for CLI

### what remains

- §6: remove `context_window` and `supports_tool_calls` from chibi's `ModelMetadata`
- §7: update `format_model_toml()` — context_window as comment not settable field
- §8: `get_field("api_key")` handle `None` (display "unset") — rest already done
- §9: docs (getting-started.md, configuration.md, AGENTS.md)

## goal

`cargo install chibi && chibi "hello"` just works. no config.toml, no API key, no models.toml. free-tier openrouter via ratatoskr presets.

## what becomes optional

| field | before | after |
|---|---|---|
| `config.toml` | required (hard error if missing) | optional (defaults applied) |
| `api_key` | required | optional (`None` = keyless openrouter) |
| `model` | required | optional (default: `ratatoskr:free/agentic`) |
| `context_window_limit` | required | optional (default: `0` = fetch from ratatoskr) |
| `warn_threshold_percent` | required | serde default `80.0` |

## design

### 1. Config struct changes (`config.rs`) ✅

`Config` (deserialized from config.toml):

- `api_key: String` → `api_key: Option<String>` with `serde(default)`
- `model: String` → `model: Option<String>` with `serde(default)`
- `context_window_limit: usize` → `context_window_limit: Option<usize>` with `serde(default)`
- `warn_threshold_percent: f32` — gains `serde(default = "default_warn_threshold_percent")`
- `Config` gains a `Default` impl

`ResolvedConfig`:

- `api_key: String` → `api_key: Option<String>` (keyless is a valid state)
- `model: String` — unchanged (always resolved from default or config)
- `context_window_limit: usize` — unchanged (`0` = sentinel for "fetch at runtime")

`ConfigDefaults` gains:

```rust
pub const MODEL: &'static str = "ratatoskr:free/agentic";
pub const WARN_THRESHOLD_PERCENT: f32 = 80.0;
pub const CONTEXT_WINDOW_LIMIT: usize = 0;  // fetch from ratatoskr
```

### 2. config loading (`state/mod.rs`) ✅

`AppState::load()`: missing config.toml returns `Config::default()` instead of a hard error.

### 3. config resolution (`state/config_resolution.rs`) ✅

`resolve_config()`:

- `api_key`: pass through as `Option<String>`
- `model`: `self.config.model.clone().unwrap_or_else(|| ConfigDefaults::MODEL.to_string())`
- `context_window_limit`: `self.config.context_window_limit.unwrap_or(ConfigDefaults::CONTEXT_WINDOW_LIMIT)`
- `warn_threshold_percent`: already defaulted by serde, no change needed
- local.toml override for api_key wraps in `Some()`

### 4. gateway building (`gateway.rs`) ✅

```rust
pub fn build_gateway(config: &ResolvedConfig) -> io::Result<EmbeddedGateway> {
    Ratatoskr::builder()
        .openrouter(config.api_key.as_deref())  // Option<&str> — None = keyless
        .build()
        .map_err(|e| io::Error::other(format!("Failed to build gateway: {}", e)))
}
```

depends on ratatoskr#24: `openrouter(Option<impl Into<String>>)`.

### 5. context_window_limit resolution ✅

`gateway::resolve_context_window(&mut config, &gateway)` — sync registry lookup, no network I/O.

called from `resolve_cli_config()` (CLI's single config choke point) when `context_window_limit == 0`. if the model isn't in the registry, limit stays 0 and existing guards (skip compaction/warnings) remain in effect.

### 6. models.toml cleanup

remove from `ModelMetadata` (the config struct, not ratatoskr's):

- `context_window: Option<usize>` — now from ratatoskr runtime metadata
- `supports_tool_calls: Option<bool>` — now from ratatoskr capabilities

`resolve_config()` drops the code reading those fields. models.toml keeps only per-model API param overrides.

### 7. `-m`/`-M` output (`model_info.rs`)

`format_model_toml()`:

- `-m` (minimal): remove `context_window` from settable output. only API params.
- `-M` (full): show `context_window` as informational comment, not settable field. same for tool call support.

### 8. `api_key: Option<String>` propagation

small blast radius:

- `build_gateway()` — covered above
- CLI `api_key()` accessor — return type becomes `Option<&str>`
- `get_field("api_key")` — handle `None` (display "unset")
- tests — mechanical updates wrapping in `Some()`

### 9. docs updates

- **`docs/getting-started.md`** — new zero-config quick-start path (`cargo install chibi && chibi "hello"`). existing config example becomes "customization" section.
- **`docs/configuration.md`** — api_key, model, context_window_limit, warn_threshold_percent documented as optional with defaults. models.toml section trimmed (remove context_window/supports_tool_calls).
- **`AGENTS.md`** — minor updates if needed to reflect optional config.

## non-goals (separate issues)

- config resolution pattern refactor (the `if let Some` field-by-field merging)
- models.toml elimination (folding into config.toml)
- env var support for api_key (`CHIBI_API_KEY`)

## resolution chain

```
defaults (ratatoskr:free/agentic, keyless, warn_threshold 80%)
  <- config.toml (optional, user overrides)
    <- local.toml (context-level overrides)
      <- ratatoskr metadata (context_window, capabilities — runtime, authoritative)
```
