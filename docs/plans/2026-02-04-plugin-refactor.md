# plugin communication and cleanup refactor

**date:** 2026-02-04
**status:** complete

## overview

pre-stability milestone cleanup focusing on:
1. plugin stdin/stdout communication (breaking change)
2. ~~ToolCallAccumulator → ratatoskr::ToolCall migration~~ ✓
3. ~~dead code removal in gateway.rs~~ ✓
4. test coverage gaps

## 1. plugin stdin/stdout communication

### current state

`plugins.rs:execute_tool()` (lines 244-283):
- params via `CHIBI_TOOL_ARGS` env var
- results from stdout (piped)
- stdin inherited "for user interaction"

`hooks.rs:execute_hook()` (lines 44-91):
- data via `CHIBI_HOOK` + `CHIBI_HOOK_DATA` env vars
- results from stdout

### change

**tools:** write JSON params to stdin, read results from stdout
- remove `CHIBI_TOOL_ARGS` env var
- close stdin after writing (signals EOF)
- keep `CHIBI_TOOL_NAME` env var (multi-tool plugins need this)
- keep `CHIBI_VERBOSE` env var

**hooks:** same pattern — write hook data to stdin
- remove `CHIBI_HOOK_DATA` env var
- keep `CHIBI_HOOK` env var (identifies which hook is firing)

### files to modify

- `crates/chibi-core/src/tools/plugins.rs` — `execute_tool()` function
- `crates/chibi-core/src/tools/hooks.rs` — `execute_hook()` function
- `CLAUDE.md` — update plugin documentation

### migration

clean break (pre-alpha). existing plugins must update to read from stdin.

**new plugin pattern:**
```python
#!/usr/bin/env -S uv run --quiet --script
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "tool_name",
        "description": "...",
        "parameters": {"type": "object", "properties": {}, "required": []},
    }))
    sys.exit(0)

if os.environ.get("CHIBI_HOOK"):
    data = json.load(sys.stdin)
    # process hook...
    print("{}")
    sys.exit(0)

params = json.load(sys.stdin)
# process tool call...
print("result")
```

## 2. ToolCallAccumulator → ratatoskr::ToolCall ✓

**completed:** `0ca398f`

- deleted `llm.rs`
- updated `send.rs` to use `ratatoskr::ToolCall` directly
- note: ratatoskr's ToolCall already has Default, so no initialization changes needed

## 3. dead code removal ✓

**completed:** `ae228e4`

- deleted `from_ratatoskr_message()` (47 lines)
- deleted `to_tool_definition()` (7 lines)
- deleted `test_from_ratatoskr_message_roundtrip()`
- cleaned up unused imports

## 4. test coverage

### missing tests identified

| function | file | status |
|----------|------|--------|
| `execute_tool()` | plugins.rs:244-283 | **no tests** |
| `execute_hook()` | hooks.rs:44-91 | **no tests** |

### plan

add integration-style tests that:
- create temp plugin scripts
- verify stdin receives params
- verify stdout is captured as result
- verify env vars are set correctly (`CHIBI_TOOL_NAME`, `CHIBI_HOOK`)
- test error handling (non-zero exit, invalid utf-8, etc.)

these tests will be written *after* the stdin/stdout refactor, testing the new behaviour.

## execution order

1. ~~ToolCallAccumulator migration (smallest, isolated)~~ ✓
2. ~~dead code removal (small, isolated)~~ ✓
3. ~~plugin stdin/stdout refactor (largest)~~ ✓
4. ~~add tests for plugin execution~~ ✓
5. ~~update CLAUDE.md plugin documentation~~ ✓

## risks

- **plugin breakage:** all existing plugins need updating. acceptable for pre-alpha.
- **streaming edge cases:** stdin write must complete before process reads. should be fine with `.write_all()` + close.
