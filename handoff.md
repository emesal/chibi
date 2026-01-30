# Handoff: Make chibi-core Stateless

## Status: Phase 5 Complete, Phase 6 In Progress

chibi-core compiles (1 unused import warning). chibi-cli does NOT compile — needs Session integration.

## Completed Work

### Phase 1: Session struct in CLI ✓
- Created `crates/chibi-cli/src/session.rs`
- `Session` struct with `current_context`, `previous_context`
- Methods: `load()`, `save()`, `switch_context()`, `swap_with_previous()`
- Full unit tests

### Phases 2-4: Core API parameterization ✓
All core methods now take `context_name` parameter instead of using `self.state.current_context`.

### Phase 5: Remove ContextState session fields ✓

**Removed from `ContextState` (context.rs):**
- `current_context: String` field
- `previous_context: Option<String>` field
- `switch_context()` method

**Updated `AppState` (state/mod.rs):**
- `load()` no longer initializes current/previous context
- `sync_state_with_filesystem()` removed phases 3-4 (context validation)
- `auto_destroy_expired_contexts()` no longer skips "current" context
- `destroy_context()` simplified — just deletes, returns `bool`, no fallback logic
- `rename_context()` no longer updates current_context (now `&mut self`)
- `clear_context(context_name)` — parameterized
- `resolve_config(context_name, ...)` — parameterized

**Removed deprecated methods:**
- `get_current_context()` — use `get_or_create_context(name)`
- `save_current_context()` — use `save_context()`
- `append_to_current_transcript_and_context()` — use parameterized version
- `load_current_todos/goals()` — use `load_todos/goals(name)`
- `save_current_todos/goals()` — use `save_todos/goals(name, content)`
- `load_system_prompt()` — use `load_system_prompt_for(name)`

**Updated entry creation methods (state/mod.rs):**
```rust
create_user_message_entry(context_name, content, username)
create_assistant_message_entry(context_name, content)
create_tool_call_entry(context_name, tool_name, arguments)
create_tool_result_entry(context_name, tool_name, result)
```

**Updated builtin.rs:**
```rust
execute_builtin_tool(app, context_name, tool_name, args)
```

**Updated logging.rs:**
```rust
log_request_if_enabled(app, context_name, debug, request_body)
log_response_meta_if_enabled(app, context_name, debug, response_meta)
```

**Updated inbox.rs:**
- `send_inbox_message_from(from_context, to_context, message)` — replaces `send_inbox_message()`
- Removed `load_and_clear_current_inbox()`

**Removed from Chibi facade (chibi.rs):**
- `switch_context()`
- `swap_with_previous()`
- `current_context()`
- `current_context_name()`

## Phase 6: Integrate Session into CLI (IN PROGRESS)

**File:** `crates/chibi-cli/src/main.rs`

The CLI currently has ~30 usages of removed methods/fields that need updating:

### Pattern replacements needed:

1. **Load Session at startup** (after `Chibi::load()`):
```rust
let mut session = Session::load(chibi.home_dir())?;
```

2. **Replace `chibi.current_context_name()`** with `&session.current_context`

3. **Replace `chibi.app.state.current_context`** with `session.current_context.clone()`

4. **Replace `chibi.switch_context(&name)`** with:
```rust
session.switch_context(name.to_string());
chibi.app.ensure_context_dir(&session.current_context)?;
```

5. **Replace `chibi.swap_with_previous()`** with:
```rust
session.swap_with_previous()?
```

6. **Handle destroy fallback** (when destroying current context):
```rust
if session.current_context == ctx_to_destroy {
    let fallback = session.previous_context
        .as_ref()
        .filter(|p| *p != ctx_to_destroy && chibi.app.context_dir(p).exists())
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    session.current_context = fallback;
    session.previous_context = None;
}
```

7. **Save session** at appropriate points (after context switch, destroy, etc.)

8. **Update hook data** to use session values

### CLI locations to update:

Search patterns to find all usages:
```bash
grep -n "current_context_name\|current_context\|switch_context\|swap_with_previous" crates/chibi-cli/src/main.rs
```

Key lines (approximate, may have shifted):
- Line ~152: `chibi.current_context_name()` in CLI config load
- Line ~351: `chibi.app.state.current_context` in hook data
- Lines ~373-404: context switching logic
- Line ~462+: various `current_context_name()` calls
- Line ~536: `clear_context()` call needs context name
- Line ~623: `execute_tool()` call
- Line ~649+: more context name usages
- Line ~709: hook data

### Also update:
- `chibi.app.clear_context()` → `chibi.app.clear_context(&session.current_context)`
- `chibi.resolve_config(a, b)` → `chibi.resolve_config(&session.current_context, a, b)`

## Phase 7: Cleanup & Tests (NOT STARTED)

1. Fix unused import warning in chibi.rs (remove `Context` from import)
2. Update all tests in state/mod.rs that reference removed fields/methods
3. Update tests that call parameterized methods with old signatures
4. Run full test suite: `cargo test`
5. Manual verification

## Files Modified

**chibi-core:**
- `src/context.rs` — ContextState struct, removed tests
- `src/state/mod.rs` — many method changes
- `src/chibi.rs` — removed facade methods
- `src/inbox.rs` — parameterized send_inbox_message
- `src/api/send.rs` — uses parameterized methods
- `src/api/compact.rs` — uses parameterized methods
- `src/api/logging.rs` — parameterized logging functions
- `src/tools/builtin.rs` — parameterized execute_builtin_tool

**chibi-cli:**
- `src/session.rs` — already complete
- `src/main.rs` — needs phase 6 updates

## Build Status

```bash
cargo check  # chibi-core: OK (1 warning), chibi-cli: FAILS
```

CLI errors are all "method not found" or "no field" for the removed session state.

## Git Status

On branch: `refactor/stateless-core`
Uncommitted changes in both crates.
