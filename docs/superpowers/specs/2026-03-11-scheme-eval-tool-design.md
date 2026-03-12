# scheme_eval Builtin Tool

**Date:** 2026-03-11
**Status:** Draft

## Summary

A builtin tool that lets the LLM evaluate Scheme (R7RS) expressions in a sandboxed tein environment. The environment persists for the lifetime of the process, allowing the LLM to define variables, build up state, and compose computations across multiple tool calls.

## Tool Schema

```
name:        scheme_eval
category:    Eval
description: Evaluate a Scheme (R7RS) expression in a persistent sandboxed
             environment. State persists across calls — define variables, build
             data structures, compose computations. Returns the result of the
             last expression. Additional safe modules can be imported with
             (import ...).

parameters:
  code: string (required) — Scheme expression(s) to evaluate

required:       [code]
summary_params: [code]
```

## Pre-Imported Modules

These are auto-imported — no `(import ...)` needed:

| Module | Purpose |
|--------|---------|
| `(scheme base)` | Core R7RS: lists, arithmetic, control, strings |
| `(scheme write)` | `display`, `write`, `write-shared` |
| `(scheme read)` | S-expression parsing from strings |
| `(scheme char)` | Character predicates, case conversion |
| `(scheme case-lambda)` | Multi-arity procedure dispatch |
| `(tein json)` | JSON parse/emit — structured data interchange |
| `(tein safe-regexp)` | Non-backtracking regex (no ReDoS risk) |
| `(srfi 1)` | List library: `filter`, `fold`, `map`, `partition`, etc. |
| `(srfi 130)` | String library: `string-contains`, `string-split`, etc. |
| `(chibi match)` | Pattern matching |
| `(harness tools)` | `call-tool` — dispatch to any registered chibi tool |

The LLM can `(import ...)` any other module in the `Modules::Safe` allowlist.

## Sandbox Configuration

- **Tier:** `Sandboxed` — `Modules::Safe` allowlist enforced
- **Step limit:** 10,000,000 (matches synthesised tools)
- **No `(harness io)`** — all I/O goes through `call-tool`

## Persistence Model

- Tein contexts stored in a process-global `LazyLock<Mutex<HashMap<String, (Arc<ThreadLocalContext>, ThreadId)>>>`, keyed by context name. `ThreadLocalContext` is not `Clone` — `Arc` provides cheap sharing.
- Created on first `scheme_eval` call for a given context.
- Lives until process exit.
- Each context gets its own independent environment (no cross-context state leakage).
- The `ThreadId` stored alongside each context is the tein **worker thread's** ID, captured during `build_managed(init)`. This is the key used by `CallContextGuard` / `BRIDGE_CALL_CTX` — the bridge function `call_tool_fn` looks up the context by `std::thread::current().id()` from the worker thread.
- `ToolMetadata::parallel` is `false` — `ThreadLocalContext` serialises calls internally, and `CallContextGuard` is keyed by worker thread ID, so concurrent eval calls for the same context would collide on the `BRIDGE_CALL_CTX` entry.

## call-tool Bridge

The eval handler sets a `CallContextGuard` before entering tein, same mechanism as `execute_synthesised`. This lets scheme code call back into rust tool dispatch:

```scheme
(call-tool "file_head" '(("path" . "/some/file") ("lines" . 10)))
```

The guard is set with the stored worker `ThreadId` (not the caller's thread). Dropped after each eval call returns. The `ToolCallContext` (app state, config, registry) is threaded through from the handler's `ToolCall` argument.

Before each `evaluate()` call, `%context-name%` is injected via `(set! %context-name% "...")` so that `call-tool` dispatches resolve VFS paths relative to the calling context (same pattern as `execute_synthesised`).

## Result Format

- **Success:** `display` representation of the last expression's value.
- **Empty code:** returns empty string (not an error).
- **Error:** Error message prefixed with `error: ` (syntax errors, runtime errors, step limit exceeded).

## Implementation

### New File

`crates/chibi-core/src/tools/eval.rs` — follows the `fs_read.rs` pattern:

- `EVAL_TOOL_NAME` constant
- `EVAL_TOOL_DEFS` static array (one tool)
- `register_eval_tools(registry)` — registers the handler
- `execute_scheme_eval()` — creates or retrieves the tein context, sets `CallContextGuard`, evaluates code, returns result
- Module-global context store (the `LazyLock<Mutex<HashMap<...>>>`)

### New ToolCategory

`ToolCategory::Eval` — no permission gating (sandboxed execution, LLM already trusted to call tools).

### Cargo Feature Change

Add `regex` to chibi-core's tein dependency: `features = ["json", "regex"]`.

### Registration

`register_eval_tools()` called alongside existing `register_*_tools()` in the registry builder. Tool exposed in the tool list sent to the LLM (added to the appropriate `*_TOOL_DEFS` collection or registered independently).

### Tein Context Init

```rust
Context::builder()
    .standard_env()
    .sandboxed(Modules::Safe)
    .step_limit(10_000_000)
    .build_managed(|ctx| {
        // capture worker thread ID for BRIDGE_CALL_CTX keying
        ctx.define_fn_variadic("call-tool", __tein_call_tool_fn)?;
        ctx.define_fn_variadic("generate-id", __tein_generate_id_fn)?;
        ctx.define_fn_variadic("current-timestamp", __tein_current_timestamp_fn)?;
        ctx.evaluate(HARNESS_PREAMBLE)?;
        ctx.register_module(HARNESS_TOOLS_MODULE)?;
        ctx.register_module(HARNESS_HOOKS_MODULE)?;
        ctx.evaluate(EVAL_PRELUDE)?;  // the (import ...) block
        Ok(())
    })
```

`EVAL_PRELUDE` is a const string:
```scheme
(import (scheme base)
        (scheme write)
        (scheme read)
        (scheme char)
        (scheme case-lambda)
        (tein json)
        (tein safe-regexp)
        (srfi 1)
        (srfi 130)
        (chibi match)
        (harness tools))
```

## Integration Points

- `tools/mod.rs` — add `Eval` variant to `ToolCategory` (incl. `as_str()`), re-export eval module, add `EVAL_TOOL_DEFS` to `builtin_tool_names()` and `builtin_summary_params()` chains
- `tools/registry.rs` — no changes needed; `register_eval_tools()` called from `chibi.rs`
- `chibi.rs` — call `register_eval_tools(&mut registry)` alongside existing `register_*_tools()` calls
- `send.rs` — `ToolCategory::Eval` falls through the existing wildcard `_ => None` arm (no permission gating), no code change needed
- `Cargo.toml` — add `regex` feature to tein dep
- tests — update `test_tool_category_debug` and tool count assertions in registry.rs

## Future Considerations

- **Persistent environments:** Serialise tein state to the context directory between runs. Deferred — requires tein serialisation support.
- **Per-prompt reset option:** A parameter to reset the environment. Not needed initially — the LLM can manage its own state.
