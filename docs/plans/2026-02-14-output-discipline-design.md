# Output Discipline Design

> **Issue:** #14 (redefined — JSON-specific work deferred to chibi-json, task 13)
>
> **Goal:** Route all chibi-cli output through `OutputHandler` for consistency, testability, and clean separation when chibi-json is extracted.

## Problem

Several output paths in chibi-cli bypass `OutputHandler` with raw `println!`/`print!`/`eprintln!` calls. This makes output behaviour inconsistent and harder to test.

## Gaps

| # | Location | Issue |
|---|----------|-------|
| 1 | `inspect_context()` | 8 raw `println!`/`print!` — doesn't receive `OutputHandler` |
| 2 | `show_log()` | 10 raw `println!` — doesn't receive `OutputHandler` |
| 3 | `ModelMetadata` command | raw `print!` with TOML string |
| 4 | verbose tool list in `main()` | raw `eprintln!` — `OutputHandler` constructed too late |
| 5 | `--json-config` version output | raw `println!` — early exit, acceptable |
| 6 | `--json-schema` output | raw `println!` — early exit, acceptable |

## Design

**No new abstractions.** Pass `&OutputHandler` to functions that need it, convert raw print calls to `output.emit_result()` or `output.diagnostic()`.

### Gap 1: `inspect_context()`

Add `output: &OutputHandler` parameter. Convert all `println!`/`print!` to `output.emit_result()`. The markdown-rendered paths (todos, goals) already use `render_markdown_output()` which writes directly — wrap the result string through `emit_result` instead in normal mode, or keep render_markdown_output for TTY rendering and note this as a known limitation (markdown rendering is inherently a TTY concern).

Actually, looking more carefully: `render_markdown_output` writes directly to stdout for the TTY rendering case. In a pure CLI app this is correct — the rendered markdown *is* the human output. So: convert the simple text outputs (`println!`) to `emit_result()`, and leave the markdown rendering paths as-is. They're already doing the right thing for a CLI app.

### Gap 2: `show_log()`

Add `output: &OutputHandler` parameter. Convert all `println!` to `output.emit_result()`. The formatted strings (`[USER]`, `[TOOL: name]`, etc.) stay as-is — they're human-readable formatting, which is correct for chibi-cli.

### Gap 3: `ModelMetadata`

Convert `print!(...)` to `output.emit_result(...)` in the `Command::ModelMetadata` match arm.

### Gap 4: verbose tool list

Move `OutputHandler::new(input.flags.json_output)` up, right after flag resolution (after line 1213). Then convert the `eprintln!` calls to `output.diagnostic(..., verbose)`.

### Gaps 5–6: early exits

Leave as-is. These are schema/version dumps that exit before any normal processing. Not worth the complexity.

## Out of Scope

- JSON-specific output formatting (deferred to chibi-json, task 13)
- `--json-output` flag deprecation (deferred to task 13)
- New output abstractions or traits
