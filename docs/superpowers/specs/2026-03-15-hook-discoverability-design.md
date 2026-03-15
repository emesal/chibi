# Hook Discoverability via `(harness docs)` Module

**Date:** 2026-03-15
**Status:** Approved
**Scope:** chibi-core (`hooks.rs`, `synthesised.rs`, `eval.rs`), `docs/hooks.md`, `justfile`

## Problem

Chibi has many hook points with rich payload/return contracts, but this information is only available in `docs/hooks.md` — a large markdown file invisible to LLMs operating inside the scheme environment. An LLM using `scheme_eval` or writing a synthesised tool knows `register-hook` exists (via `harness-tools-docs`) but cannot discover which hooks are available, what payloads they receive, or what return values modify behaviour.

There is also a single-source-of-truth risk: `docs/hooks.md` is hand-authored prose that can drift from the actual `HookPoint` enum and the hook dispatch code.

## Solution

Make hook metadata a rust data structure (`const HOOK_METADATA`) that serves as the single canonical source. From it, generate:

1. A scheme docs alist (`hooks-docs`) injected into the tein runtime, discoverable via `(describe hooks-docs)` and `(module-doc hooks-docs 'pre_message)`
2. The hook reference section of `docs/hooks.md`, via a `just generate-docs` task

A new `(harness docs)` r7rs module exports both `hooks-docs` and the existing `harness-tools-docs` alist (moved from the preamble top-level). This module is safe-tier and available in all contexts.

## Design

### Data Model — `hooks.rs`

Two structs and a const array, next to the `HookPoint` enum. `HookPoint` also needs `EnumIter` (strum) added to its derives for the completeness test:

```rust
pub(crate) struct FieldMeta {
    pub name: &'static str,
    pub typ: &'static str,         // "string", "number", "bool", "object", "array"
    pub description: &'static str,
}

pub(crate) struct HookMeta {
    pub point: HookPoint,
    pub category: &'static str,    // "session", "message", "tool", "api", etc.
    pub description: &'static str, // one-liner
    pub can_modify: bool,
    pub payload_fields: &'static [FieldMeta],
    pub return_fields: &'static [FieldMeta],  // empty if observe-only
    pub notes: &'static str,       // extra context, empty if none
}

pub(crate) const HOOK_METADATA: &[HookMeta] = &[
    HookMeta {
        point: HookPoint::PreMessage,
        category: "message",
        description: "before sending prompt to LLM",
        can_modify: true,
        payload_fields: &[
            FieldMeta { name: "prompt", typ: "string", description: "the user's prompt" },
            FieldMeta { name: "context_name", typ: "string", description: "active context" },
            FieldMeta { name: "summary", typ: "string", description: "conversation summary" },
        ],
        return_fields: &[
            FieldMeta { name: "prompt", typ: "string", description: "modified prompt" },
        ],
        notes: "",
    },
    // ... all hooks
];
```

### Scheme Alist Generation — `hooks.rs`

```rust
pub(crate) fn generate_hooks_docs_alist() -> String
```

Iterates `HOOK_METADATA`, produces a scheme alist string following the `introspect-docs` convention:

```scheme
(define hooks-docs
  '((__module__ . "hook points — lifecycle hooks for plugins and synthesised tools")
    (pre_message . "category: message | fires before sending prompt to LLM | can modify: prompt
  payload: prompt (string), context_name (string), summary (string)
  returns: prompt (string)")
    ...))
```

Each value is a human-readable multi-line string — compatible with `(describe ...)` and `(module-doc ...)` which just display strings.

### `(harness docs)` Module — `synthesised.rs`

A new module constant:

```rust
pub(crate) const HARNESS_DOCS_MODULE: &str = r#"
(define-library (harness docs)
  (import (scheme base))
  (export hooks-docs harness-tools-docs)
  (begin #t))
"#;
```

The actual bindings (`hooks-docs`, `harness-tools-docs`) are top-level defines in `HARNESS_PREAMBLE`, same pattern as `register-hook`. The module re-exports them.

### HARNESS_PREAMBLE Changes

- **`harness-tools-docs`** stays as a top-level define in the preamble (unchanged content). It is now also re-exported via `(harness docs)` — the module is the canonical access path.
- **Add `hooks-docs`** — the generated alist string, spliced in via `format!` or `LazyLock`.
- Both bindings are top-level (not inside a library) so they're accessible before any imports, and the `(harness docs)` module can re-export them.

This is a **convention change**: the canonical way to access these docs is now `(import (harness docs))` rather than referencing top-level bindings directly. The top-level bindings still work but are no longer documented. AGENTS.md and `chibi.md` get updated accordingly.

### Preamble Assembly

Since `hooks-docs` is generated at runtime from `HOOK_METADATA`, the preamble can no longer be a plain `const &str`. Options:

- **`LazyLock<String>`**: built once on first access. `HARNESS_PREAMBLE` becomes a function or lazy static that splices the generated alist into the template.
- **`format!` in `build_tein_context`**: generate the alist string and format it into the preamble each time a context is built.

`LazyLock<String>` is cleaner — one allocation, reused across all context builds.

### Module Registration — `synthesised.rs`

In `build_tein_context`, after registering `HARNESS_HOOKS_MODULE`:

```rust
ctx.register_module(HARNESS_DOCS_MODULE)?;
```

### Safe Tier

`(harness docs)` exports only data (alists), no IO — safe for all tiers. Module availability is controlled by `register_module` in `build_tein_context` (already covered above), same as the other harness modules. No changes to tein's `Modules::Safe` allowlist needed — runtime-registered modules bypass it.

### Markdown Generation — `hooks.rs`

```rust
pub fn generate_hooks_markdown() -> String
```

Produces the hook reference section with the same structure as today's `docs/hooks.md` lines 1–112: category-grouped tables with "when" and "can modify" columns, then per-hook payload/return JSON blocks.

### `docs/hooks.md` Structure

```markdown
# Hooks

<!-- BEGIN GENERATED HOOK REFERENCE — do not edit, run `just generate-docs` -->
...generated from HOOK_METADATA...
<!-- END GENERATED HOOK REFERENCE -->

## Registering for Hooks
...hand-authored, unchanged...

## Examples
...hand-authored, unchanged...
```

### Justfile Task

```just
generate-docs:
    cargo test -p chibi-core --test generate_docs -- --nocapture > /dev/null
```

Or a small binary/integration test that writes the generated section. The exact mechanism is an implementation detail — the key contract is `just generate-docs` updates the file, and CI can run `just generate-docs --check` (or a test that asserts freshness).

### EVAL_PRELUDE Update — `synthesised.rs`

Add `(import (harness docs))` to `EVAL_PRELUDE` so that `hooks-docs` and `harness-tools-docs` are available in `scheme_eval` contexts without manual import (same as `(harness tools)` is today).

### System Prompt Update — `chibi.md`

Replace the `harness-tools-docs` pointer with:

> Use `(import (harness docs))` then `(describe hooks-docs)` to list available hooks, or `(module-doc hooks-docs 'pre_message)` for a specific hook's contract. `(describe harness-tools-docs)` lists the harness API (`define-tool`, `call-tool`, `register-hook`, etc.).

### AGENTS.md Updates

- Update the `harness-tools-docs` quirk to reference `(harness docs)` as the canonical import
- Add note about `hooks-docs` and the generation pipeline
- Note: `(module-exports '(harness docs))` will error (same runtime-registration limitation)

## Tests

1. **Completeness**: iterate all `HookPoint` variants (via strum), assert each has an entry in `HOOK_METADATA`. Adding a hook variant without metadata fails the test.
2. **Scheme alist**: generate the alist, evaluate in a sandboxed tein context, call `(describe hooks-docs)`, verify non-empty output containing all hook point names.
3. **Module availability**: in both sandboxed and unsandboxed contexts, verify `(import (harness docs))` succeeds and both `hooks-docs` and `harness-tools-docs` are bound.
4. **Markdown freshness**: a test that generates the markdown and asserts it matches the content between the begin/end markers in `docs/hooks.md`.

## Files Changed

| File | Change |
|------|--------|
| `crates/chibi-core/src/tools/hooks.rs` | Add `EnumIter` derive to `HookPoint`, add `HookMeta`, `FieldMeta`, `HOOK_METADATA`, `generate_hooks_docs_alist()`, `generate_hooks_markdown()` |
| `crates/chibi-core/src/tools/synthesised.rs` | Add `HARNESS_DOCS_MODULE`, update preamble to splice `hooks-docs`, register module in `build_tein_context`, add `(import (harness docs))` to `EVAL_PRELUDE` |
| `crates/chibi-core/src/tools/eval.rs` | Update tests for `(harness docs)` module |
| `crates/chibi-core/prompts/chibi.md` | Update discoverability pointers |
| `docs/hooks.md` | Add begin/end markers, replace reference section with generated content |
| `AGENTS.md` | Update quirks for `(harness docs)` |
| `justfile` | Add `generate-docs` task |
