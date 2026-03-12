# Tein Hook Registration

**Issue:** #220 — synthesised tein tools participating in hook dispatch

## Problem

Hook dispatch (`execute_hook` in `hooks.rs`) only supports subprocess-based
plugin tools. Synthesised tools (tein/scheme `.scm` files) cannot register for
hook points. This blocks #197 (VCS plugin) and any future tein plugin that
needs to observe or modify hook payloads.

## Decision

Add a `(harness hooks)` tein module exposing `register-hook`, backed by a
rust-side `SynthesisedHookRegistry`. Hook dispatch is extended to evaluate
tein callbacks alongside subprocess plugins, with a re-entrancy guard to
prevent recursive hook invocation.

Rejected alternatives:
- **Subprocess shim** (wrap tein in a subprocess): defeats the purpose of
  in-process tein; adds IPC overhead and loses sandbox integration.
- **Callback on `ScmChangeCallback`** pattern: too narrow — only fires on
  `.scm` writes, not a general hook mechanism.

## Design

### 1. Scheme API — `(harness hooks)`

```scheme
(import (harness hooks))

(register-hook 'pre_vfs_write
  (lambda (payload)
    ;; payload: scheme value parsed from hook JSON
    ;; return: alist/value serialised back to JSON (same contract as subprocess hooks)
    ;; return '() for no-op
    '()))
```

- `register-hook` takes a symbol (hook point name) and a procedure.
- Multiple registrations per hook point allowed (executed in registration order).
- Available in all tiers — what the callback *can do* is limited by the
  sandbox (e.g. sandboxed code cannot import `(harness io)` (#219)).

### 2. Rust bridge — `SynthesisedHookRegistry`

```
Global: Arc<RwLock<SynthesisedHookRegistry>>
Structure: HashMap<HookPoint, Vec<SynthesisedHookEntry>>
```

Each `SynthesisedHookEntry` holds:
- The tein environment/closure reference needed to invoke the callback.
- The VFS path of the `.scm` file that registered it (for lifecycle management).
- The tool name or identifier (for result attribution in the `(String, Value)` pairs).

**Lifecycle:** registrations are tied to the `.scm` file that created them.
When a file is hot-reloaded (`reload_tool_from_content`) or unregistered, its
hook registrations are cleared before re-eval. This ensures stale callbacks
never fire.

**Population:** `register-hook` calls from scheme land write into the registry
during eval time (when the `.scm` file is first loaded or hot-reloaded).

### 3. Re-entrancy guard

A thread-local or registry-level `HashSet<HookPoint>` tracks which hook points
are currently executing tein callbacks. If a hook point is already in the set
when dispatch begins, tein callbacks are skipped for that point. Subprocess
hooks still fire normally.

This gives a clean, deterministic contract: *a tein hook callback will never be
invoked recursively for the same hook point.*

### 4. Dispatch integration

`execute_hook` (or a new unified dispatcher wrapping it) runs:

1. Subprocess plugin hooks (existing path, unchanged).
2. Synthesised tein hooks (new path, same JSON in → JSON out contract).

Results from both are merged into the same `Vec<(String, Value)>` return.

Ordering: subprocess hooks first, then tein hooks. This is arbitrary but
deterministic; if ordering becomes important, a priority system can be added
later.

### 5. JSON ↔ Scheme bridge

Hook payloads are `serde_json::Value`. The bridge needs:
- `Value → Scheme`: convert JSON to tein's native representation (alist or
  similar). This may already exist for `call-tool` payloads.
- `Scheme → Value`: convert the callback's return value back to JSON.

If no conversion utilities exist, they'll need to be added to the harness
bridge layer.

## Scope

### In scope
- `(harness hooks)` module with `register-hook`
- `SynthesisedHookRegistry` in rust
- Re-entrancy guard
- Dispatch integration in `execute_hook`
- JSON ↔ scheme conversion (if not already present)
- Lifecycle management (clear on hot-reload/unregister)
- Tests

### Out of scope
- `(harness io)` privileged module (#219)
- `pre_vfs_write` / `post_vfs_write` hook points (part of #197)
- VCS plugin itself (#197)
- Hook priority/ordering system

## Files likely affected

- `crates/chibi-core/src/tools/hooks.rs` — dispatch integration
- `crates/chibi-core/src/tools/synthesised.rs` — registry, lifecycle, module
- Tein harness preamble or module registration for `(harness hooks)`

## Implementation Plan

See `docs/plans/2026-03-09-tein-hook-registration.md`
