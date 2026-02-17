# remove is_json_mode() from OutputSink/ResponseSink

resolves #148

## problem

`is_json_mode()` exists on both `OutputSink` and `ResponseSink` traits, but
CLI always returns false and JSON always returns true. it's a compile-time
property queried at runtime — an architectural smell that leaks presentation
concerns into core.

## approach: sink-side formatting

remove `is_json_mode()` entirely. core emits structured data; each sink
formats as appropriate.

## changes

### trait removal

- **`OutputSink`** (`core/output.rs`): remove `is_json_mode()`. update
  `emit_entry` doc — it's now the universal transcript entry path, not
  "JSON-mode structured output".
- **`ResponseSink`** (`core/api/sink.rs`): remove `is_json_mode()` + default
  impl.

### sink impls

- **`OutputHandler`** (`cli/output.rs`): unit struct becomes
  `{ verbose: bool }`. `new()` takes `verbose: bool`. `emit_entry()` gains
  the human-readable formatting currently in `show_log` (match on entry type,
  verbose/compact tool call display, markdown rendering).
- **`CliResponseSink`** (`cli/sink.rs`): remove `is_json_mode()` impl.
- **`JsonOutputSink`** (`json/output.rs`): remove `is_json_mode()` impl.
- **`JsonResponseSink`** (`json/sink.rs`): remove `is_json_mode()` impl.

### core simplification

- **`show_log`** (`core/execution.rs`): drop `verbose` param and all
  formatting logic (~40 lines). becomes: select entries by count, call
  `output.emit_entry()` for each.
- **`send.rs`** (`core/api/send.rs`): remove `json_mode` variable and guard
  around `TextChunk` emission (line ~619). remove `is_json_mode()` guard
  around `Finished` emission (line ~1909). both sinks already handle these
  events correctly as no-ops / real handlers.

### tests

- remove `test_new_text_mode` (asserted `!is_json_mode()`)
- remove `test_is_json_mode_normal` from sink tests
- update `test_emit_entry_noop` — `emit_entry` is no longer a no-op in CLI;
  test that it produces formatted output
- add verbose vs compact rendering tests for `emit_entry`
- update all `OutputHandler::new()` calls to pass `verbose`

## files touched

| file | change |
|------|--------|
| `crates/chibi-core/src/output.rs` | remove `is_json_mode` from trait |
| `crates/chibi-core/src/api/sink.rs` | remove `is_json_mode` from trait |
| `crates/chibi-core/src/api/send.rs` | remove two guards |
| `crates/chibi-core/src/execution.rs` | simplify `show_log` |
| `crates/chibi-cli/src/output.rs` | stateful `OutputHandler`, formatting in `emit_entry` |
| `crates/chibi-cli/src/sink.rs` | remove `is_json_mode` impl |
| `crates/chibi-cli/src/main.rs` | pass `verbose` to `OutputHandler::new()` |
| `crates/chibi-json/src/output.rs` | remove `is_json_mode` impl |
| `crates/chibi-json/src/sink.rs` | remove `is_json_mode` impl |
