# structured error output for chibi-json

**issue:** #149
**date:** 2026-02-19
**status:** approved

## problem

chibi-json currently emits fatal errors as `{"type": "error", "message": "..."}` on stdout, with no semantic code field. programmatic consumers can't reliably classify failures without scraping message strings.

additionally, events (verbose-tier diagnostics) already go to stderr, but there is no terminal signal that tells consumers the command has completed and whether it succeeded.

## design

### stdout/stderr split

- **stdout**: pure result stream — `result`, transcript entries, and any other structured output. silent on error.
- **stderr**: diagnostic/control stream — events, errors, and a terminal `done` signal as the last line emitted.

### `OutputSink::emit_done` (chibi-core)

a new method added to the `OutputSink` trait in `crates/chibi-core/src/output.rs`:

```rust
/// Signal command completion. Called once, after all output has been emitted.
/// Default: no-op (chibi-cli handles completion via its own UX).
fn emit_done(&self, result: &io::Result<()>) {
    let _ = result;
}
```

the default no-op preserves existing behaviour for chibi-cli and chibi-mcp-bridge.

### `JsonOutputSink::emit_done` (chibi-json)

overrides the default to emit a terminal `done` line to stderr:

```json
{"type": "done", "ok": true}
```

on success, or on failure:

```json
{"type": "done", "ok": false, "code": "not_found", "message": "context 'foo' does not exist"}
```

### error code mapping

coarse-grained codes derived from `io::ErrorKind` via a private `error_code` helper in chibi-json:

| `io::ErrorKind`    | `"code"`             |
|--------------------|----------------------|
| `NotFound`         | `"not_found"`        |
| `InvalidInput`     | `"invalid_input"`    |
| `PermissionDenied` | `"permission_denied"`|
| `InvalidData`      | `"invalid_data"`     |
| `AlreadyExists`    | `"already_exists"`   |
| everything else    | `"internal_error"`   |

fine-grained semantic codes (e.g. `"context_not_found"`) are deferred — they require proper error variants in chibi-core and are noted in #149 as future work.

### chibi-json main.rs

the existing `match rt.block_on(run())` error handler (which emits `{"type": "error", ...}` to stdout) is replaced:

```rust
let result = rt.block_on(run());
let output = output::JsonOutputSink;
output.emit_done(&result);
if result.is_err() {
    std::process::exit(1);
}
```

`run()` no longer emits errors — `emit_done` owns that responsibility.

## files changed

- `crates/chibi-core/src/output.rs` — add `emit_done` with default no-op
- `crates/chibi-json/src/output.rs` — implement `emit_done`, add `error_code` helper
- `crates/chibi-json/src/main.rs` — wire `emit_done`, remove old error emission

## out of scope

- fine-grained `ChibiError` variants in chibi-core (future, tracked in #149)
- chibi-cli or chibi-mcp-bridge changes (default no-op covers them)
- changes to how events are emitted (already on stderr via `eprintln!`)
