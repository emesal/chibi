# Handoff: Make chibi-core Stateless

## Status: COMPLETE ✓

All phases complete. Both crates compile and all tests pass.

## Summary

Moved "current context" and "previous context" session state from chibi-core to chibi-cli.

### Key Changes

**New file: `crates/chibi-cli/src/session.rs`**
- `Session` struct with `current_context: String`, `previous_context: Option<String>`
- Loads/saves `~/.chibi/session.json`
- Methods: `switch_context()`, `swap_with_previous()`, `is_current()`, `is_previous()`

**chibi-core (now stateless):**
- `ContextState` no longer has `current_context` or `previous_context` fields
- All methods that operated on "current context" now take `context_name: &str` parameter
- Removed methods: `switch_context()`, `swap_with_previous()`, `current_context()`, `current_context_name()`
- `destroy_context()` now returns `bool` (success/failure), no longer handles fallback logic
- `auto_destroy_expired_contexts()` no longer skips "current" context (CLI handles session updates)
- Entry creation methods (`create_user_message_entry`, etc.) take `context_name` as first param
- `resolve_config()` takes `context_name` as first param

**chibi-cli:**
- `Session` loaded at startup after `Chibi::load()`
- All context operations use `session.current_context`
- Context switching: `session.switch_context(name)` + `chibi.app.ensure_context_dir()`
- Destroy fallback logic moved to CLI (if destroying current, switch to previous or "default")
- Session saved after persistent context switches, destroys, renames

## API Changes

```rust
// Before
chibi.current_context_name()
chibi.switch_context(&name)?
chibi.swap_with_previous()?
chibi.resolve_config(None, None)?

// After (CLI)
session.current_context.clone()
session.switch_context(name)
session.swap_with_previous()?
chibi.resolve_config(&session.current_context, None, None)?
```

## Files Modified

**chibi-core:**
- `src/chibi.rs` - removed facade methods, updated doctests
- `src/context.rs` - removed `current_context`/`previous_context` from `ContextState`
- `src/state/mod.rs` - parameterized methods, updated tests
- `src/api/send.rs` - uses parameterized methods
- `src/api/compact.rs` - uses parameterized methods
- `src/api/logging.rs` - parameterized logging functions
- `src/tools/builtin.rs` - parameterized `execute_builtin_tool`
- `src/inbox.rs` - parameterized `send_inbox_message_from`
- `src/lib.rs` - updated doctest

**chibi-cli:**
- `src/session.rs` - new Session struct with tests
- `src/main.rs` - integrated Session throughout

## Verification

```bash
cargo check   # Both crates: OK (no warnings)
cargo test    # All tests pass (761 tests)
```

## Manual Testing Checklist

- [ ] `chibi -c test "hello"` - creates context, sends prompt
- [ ] `chibi -c other "hi"` then `chibi -c - "back"` - swap works
- [ ] `chibi --destroy test` - destroys, session updates
- [ ] `chibi -l` - lists contexts correctly
- [ ] `chibi` (no args) - uses session.current_context
- [ ] Check `~/.chibi/session.json` exists with correct content
- [ ] Verify `~/.chibi/state.json` no longer has current_context/previous_context
