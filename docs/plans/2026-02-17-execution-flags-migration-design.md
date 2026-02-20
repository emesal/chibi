# design: migrate ExecutionFlags overlap into config overrides

issue: #161
date: 2026-02-17

## context

`ExecutionFlags` and `ResolvedConfig` duplicate four behavioural fields (`verbose`,
`hide_tool_calls`, `no_tool_calls`, `show_thinking`), with manual `flag || config`
merge logic duplicated in both `chibi-cli/src/main.rs` and `chibi-json/src/main.rs`.

`set_field` (from #157) already provides a universal string-keyed config override
interface on core's `ResolvedConfig`.

## design

### ExecutionFlags shrinks to ephemeral modifiers

remove `verbose`, `hide_tool_calls`, `no_tool_calls`, `show_thinking` from
`ExecutionFlags`. what remains:

```rust
pub struct ExecutionFlags {
    pub force_call_agent: bool,
    pub force_call_user: bool,
    pub debug: Vec<DebugKey>,
}
```

these are true command modifiers — imperative, not config.

### show_thinking moves to core's ResolvedConfig

add `show_thinking: bool` to:
- `ChibiConfig` (with serde default)
- `ContextOverrides`
- config resolution (`resolve()`)
- core `ResolvedConfig`
- `set_field` / `get_field` / `list_fields`

CLI's `ResolvedConfig` drops its own `show_thinking`, delegates to `core.show_thinking`.

### binary merge logic replaced with set_field

both mains currently do:
```rust
input.flags.verbose = input.flags.verbose || config.verbose;
input.flags.hide_tool_calls = input.flags.hide_tool_calls || config.hide_tool_calls;
input.flags.no_tool_calls = input.flags.no_tool_calls || config.no_tool_calls;
```

after: CLI/JSON flags that are `true` call `config.set_field("verbose", "true")` etc.
config already holds file-based defaults; set_field overrides. no duplicated merge.

### core reads from config, not flags

- `execution.rs`: `flags.verbose` → `config.verbose`
- `flags.no_tool_calls` → `config.no_tool_calls` (the copy into `resolved` becomes
  unnecessary since config already has it)
- `PromptOptions::new()` gets `verbose` from `config.verbose`
- `hide_tool_calls` and `show_thinking` were never read in core — no change needed

### CLI sink reads from config

```rust
let verbose = cli_config.core.verbose;
let show_tool_calls = !cli_config.core.hide_tool_calls || verbose;
let show_thinking = cli_config.core.show_thinking || verbose;
```

### JSON schema change

shrinking `ExecutionFlags` changes chibi-json's input schema. the removed fields
move to the `config` map processed by `set_field`. acceptable — pre-alpha, no
backwards compat requirement.
