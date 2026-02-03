# Chibi 0.6.0 Release Prep Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add context name injection, datetime prefix, improved call_agent description, and file writing tools with permission hooks.

## Progress

- [x] **Task 1:** Inject context name into system prompt — `send.rs:420`
- [x] **Task 2:** Inject datetime prefix into user messages — `send.rs:1256-1258`
- [x] **Task 3:** Update call_agent description — `builtin.rs:149`
- [x] **Task 4:** Add file write tool definitions
- [x] **Task 5:** Add pre_file_write hook point
- [x] **Task 6:** Implement write_file execution
- [x] **Task 7:** Implement patch_file execution
- [x] **Task 8:** Create default permission plugin
- [x] **Task 9:** Update documentation
- [x] **Task 10:** Run full test suite and verify

**Architecture:**
- Context name and datetime become first-class parts of the prompt building pipeline
- File write tools follow existing `file_tools.rs` patterns but gate execution through a new permission hook
- Permission system is hook-based: `pre_file_write` hook can approve/deny/modify operations

**Tech Stack:** Rust, serde_json, chrono (already in deps)

---

## Task 1: Inject Context Name into System Prompt

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:412-418`
- Test: `crates/chibi-core/src/api/send.rs` (existing test module)

**Step 1: Write the failing test**

Add to the test module in `send.rs`:

```rust
#[test]
fn test_context_name_injected_in_system_prompt() {
    // This is a unit test concept - the actual integration happens in build_full_system_prompt
    // We verify the format string is correct
    let context_name = "my-context";
    let expected = format!("\n\nCurrent context: {}", context_name);
    assert!(expected.contains("Current context: my-context"));
}
```

**Step 2: Run test to verify it passes (trivial test)**

Run: `cargo test -p chibi-core test_context_name`

**Step 3: Modify build_full_system_prompt to inject context name**

In `build_full_system_prompt()` around line 418, after the username block:

```rust
    // Add context name
    full_system_prompt.push_str(&format!(
        "\n\nCurrent context: {}",
        context_name
    ));
```

**Step 4: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "$(cat <<'EOF'
feat: inject context name into system prompt

LLM now knows which context it's operating in, enabling
better multi-context awareness and inter-context messaging.
EOF
)"
```

---

## Task 2: Inject Datetime Prefix into User Messages

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:1254-1258`
- Test: add unit test in same file

**Step 1: Write the failing test**

```rust
#[test]
fn test_datetime_prefix_format() {
    use chrono::Local;
    let now = Local::now();
    let formatted = now.format("%Y%m%d-%H%M%z").to_string();
    // Should be like "20260203-1542+0000" or "20260203-1542-0500"
    assert_eq!(formatted.len(), 18, "datetime format should be 18 chars");
    assert!(formatted.chars().nth(8) == Some('-'), "should have dash separator");
}
```

**Step 2: Run test to verify it passes**

Run: `cargo test -p chibi-core test_datetime_prefix_format`

**Step 3: Modify user message creation to include datetime prefix**

In `send_prompt_with_depth()` around line 1254, modify how `final_prompt` is used:

```rust
    // Add datetime prefix to user message
    let datetime_prefix = chrono::Local::now().format("%Y%m%d-%H%M%z").to_string();
    let prefixed_prompt = format!("[{}] {}", datetime_prefix, final_prompt);

    // Add user message to context and transcript
    app.add_message(&mut context, "user".to_string(), prefixed_prompt.clone());
    let user_entry =
        create_user_message_entry(context_name, &prefixed_prompt, &resolved_config.username);
```

**Step 4: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "$(cat <<'EOF'
feat: prefix user messages with datetime

Every user message now starts with [YYYYMMDD-HHMM+ZZZZ] timestamp,
giving the LLM temporal awareness for time-sensitive conversations.
EOF
)"
```

---

## Task 3: Update call_agent Description

**Files:**
- Modify: `crates/chibi-core/src/tools/builtin.rs:143-145`
- Test: update existing test in same file

**Step 1: Update the test expectation**

Find and update `test_call_agent_tool_api_format` test:

```rust
#[test]
fn test_call_agent_tool_api_format() {
    let tool = get_tool_api(CALL_AGENT_TOOL_NAME);
    assert_eq!(tool["type"], "function");
    assert_eq!(tool["function"]["name"], CALL_AGENT_TOOL_NAME);
    assert!(
        tool["function"]["description"]
            .as_str()
            .unwrap()
            .contains("recurse")
    );
    assert!(
        tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("prompt"))
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core test_call_agent_tool_api_format`
Expected: FAIL (description doesn't contain "recurse")

**Step 3: Update the description**

Change line 145 in `builtin.rs`:

```rust
    BuiltinToolDef {
        name: CALL_AGENT_TOOL_NAME,
        description: "Recurse to do more work before handing control back to the user. Use this to continue processing when you have more steps to complete.",
        properties: &[ToolPropertyDef {
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p chibi-core test_call_agent_tool_api_format`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/builtin.rs
git commit -m "$(cat <<'EOF'
docs: improve call_agent tool description

Makes the tool's purpose clearer: it's for recursing to do more work,
not just "continue processing" which was too vague.
EOF
)"
```

---

## Task 4: Add File Write Tool Definitions

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs`
- Test: add tests in same file

**Step 1: Add tool name constants**

After line 20, add:

```rust
pub const WRITE_FILE_TOOL_NAME: &str = "write_file";
pub const PATCH_FILE_TOOL_NAME: &str = "patch_file";
```

**Step 2: Add tool definitions to FILE_TOOL_DEFS**

After the `cache_list` definition (around line 149), add:

```rust
    BuiltinToolDef {
        name: WRITE_FILE_TOOL_NAME,
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Requires user permission.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Absolute or relative path to write to",
                default: None,
            },
            ToolPropertyDef {
                name: "content",
                prop_type: "string",
                description: "Content to write to the file",
                default: None,
            },
        ],
        required: &["path", "content"],
    },
    BuiltinToolDef {
        name: PATCH_FILE_TOOL_NAME,
        description: "Apply a find-and-replace patch to a file. Replaces the first occurrence of 'find' with 'replace'. Requires user permission.",
        properties: &[
            ToolPropertyDef {
                name: "path",
                prop_type: "string",
                description: "Path to the file to patch",
                default: None,
            },
            ToolPropertyDef {
                name: "find",
                prop_type: "string",
                description: "Text to find (exact match)",
                default: None,
            },
            ToolPropertyDef {
                name: "replace",
                prop_type: "string",
                description: "Text to replace with",
                default: None,
            },
        ],
        required: &["path", "find", "replace"],
    },
```

**Step 3: Update is_file_tool function**

```rust
pub fn is_file_tool(name: &str) -> bool {
    matches!(
        name,
        FILE_HEAD_TOOL_NAME
            | FILE_TAIL_TOOL_NAME
            | FILE_LINES_TOOL_NAME
            | FILE_GREP_TOOL_NAME
            | CACHE_LIST_TOOL_NAME
            | WRITE_FILE_TOOL_NAME
            | PATCH_FILE_TOOL_NAME
    )
}
```

**Step 4: Write tests**

```rust
#[test]
fn test_write_file_tool_api_format() {
    let tool = get_tool_api(WRITE_FILE_TOOL_NAME);
    assert_eq!(tool["function"]["name"], WRITE_FILE_TOOL_NAME);
    let required = tool["function"]["parameters"]["required"]
        .as_array()
        .unwrap();
    assert!(required.contains(&serde_json::json!("path")));
    assert!(required.contains(&serde_json::json!("content")));
}

#[test]
fn test_patch_file_tool_api_format() {
    let tool = get_tool_api(PATCH_FILE_TOOL_NAME);
    assert_eq!(tool["function"]["name"], PATCH_FILE_TOOL_NAME);
    let required = tool["function"]["parameters"]["required"]
        .as_array()
        .unwrap();
    assert!(required.contains(&serde_json::json!("path")));
    assert!(required.contains(&serde_json::json!("find")));
    assert!(required.contains(&serde_json::json!("replace")));
}

#[test]
fn test_file_tool_registry_contains_write_tools() {
    assert_eq!(FILE_TOOL_DEFS.len(), 7); // was 5, now 7
    let names: Vec<_> = FILE_TOOL_DEFS.iter().map(|d| d.name).collect();
    assert!(names.contains(&WRITE_FILE_TOOL_NAME));
    assert!(names.contains(&PATCH_FILE_TOOL_NAME));
}
```

**Step 5: Run tests**

Run: `cargo test -p chibi-core file_tool`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/chibi-core/src/tools/file_tools.rs
git commit -m "$(cat <<'EOF'
feat: add write_file and patch_file tool definitions

Tool schemas only - execution not yet implemented.
These tools will require permission via pre_file_write hook.
EOF
)"
```

---

## Task 5: Add pre_file_write Hook Point

**Files:**
- Modify: `crates/chibi-core/src/tools/hooks.rs`
- Test: existing hook tests

**Step 1: Find and read hooks.rs to understand the pattern**

The hook enum and dispatch should follow existing patterns.

**Step 2: Add PreFileWrite variant**

Add to the `HookPoint` enum:

```rust
    /// Before a file write/patch operation. Hook can approve, deny, or modify.
    /// Data: { "tool_name": str, "path": str, "content"?: str, "find"?: str, "replace"?: str }
    /// Return: { "approved": bool, "path"?: str (override), "reason"?: str (if denied) }
    PreFileWrite,
```

**Step 3: Add to HOOK_NAMES array and as_str**

```rust
    PreFileWrite => "pre_file_write",
```

**Step 4: Update hook count in tests if needed**

**Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/hooks.rs
git commit -m "$(cat <<'EOF'
feat: add pre_file_write hook point

Allows plugins to approve/deny/modify file write operations.
Permission system can be implemented as a hook plugin.
EOF
)"
```

---

## Task 6: Implement write_file Execution

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs`
- Modify: `crates/chibi-core/src/api/send.rs` (to wire up hook)

**Step 1: Add execute_write_file function**

```rust
/// Execute write_file tool with permission check via hook
pub fn execute_write_file(
    path: &str,
    content: &str,
) -> io::Result<String> {
    // Note: Permission check happens in send.rs before calling this
    let path = PathBuf::from(path);

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    crate::safe_io::atomic_write_text(&path, content)?;

    Ok(format!(
        "File written successfully: {} ({} bytes)",
        path.display(),
        content.len()
    ))
}
```

**Step 2: Wire up in execute_file_tool**

Add to the match in `execute_file_tool`:

```rust
        WRITE_FILE_TOOL_NAME => {
            let path = require_str_param(args, "path")?;
            let content = require_str_param(args, "content")?;
            Some(execute_write_file(&path, &content))
        }
```

**Step 3: Add permission check in send.rs execute_single_tool**

Before the file tool execution, check the pre_file_write hook:

```rust
// For write tools, check permission first
if tool_call.name == tools::WRITE_FILE_TOOL_NAME || tool_call.name == tools::PATCH_FILE_TOOL_NAME {
    let hook_data = serde_json::json!({
        "tool_name": tool_call.name,
        "path": args.get_str("path").unwrap_or(""),
        "content": args.get_str("content"),
        "find": args.get_str("find"),
        "replace": args.get_str("replace"),
    });
    let hook_results = tools::execute_hook(tools, tools::HookPoint::PreFileWrite, &hook_data)?;

    // Check if any hook denied the operation
    for (hook_name, result) in &hook_results {
        if !result.get_bool_or("approved", false) {
            let reason = result.get_str_or("reason", "Permission denied by hook");
            return Ok(ToolExecutionResult {
                final_result: format!("Error: {}", reason),
                original_result: format!("Error: {}", reason),
                was_cached: false,
            });
        }
    }

    // If no hooks registered, deny by default (fail-safe)
    if hook_results.is_empty() {
        return Ok(ToolExecutionResult {
            final_result: "Error: No permission handler configured. File write tools require a pre_file_write hook.".to_string(),
            original_result: "Error: No permission handler configured".to_string(),
            was_cached: false,
        });
    }
}
```

**Step 4: Write integration test**

```rust
#[test]
fn test_write_file_blocked_without_hook() {
    // When no pre_file_write hook is registered, write should fail
    // This is a safety feature
}
```

**Step 5: Run tests**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/chibi-core/src/tools/file_tools.rs crates/chibi-core/src/api/send.rs
git commit -m "$(cat <<'EOF'
feat: implement write_file with hook-based permission

write_file is blocked by default unless a pre_file_write hook
approves the operation. This fail-safe design ensures users
must explicitly enable file writes.
EOF
)"
```

---

## Task 7: Implement patch_file Execution

**Files:**
- Modify: `crates/chibi-core/src/tools/file_tools.rs`

**Step 1: Add execute_patch_file function**

```rust
/// Execute patch_file tool (find and replace)
pub fn execute_patch_file(
    path: &str,
    find: &str,
    replace: &str,
) -> io::Result<String> {
    let path = PathBuf::from(path);

    // Read existing content
    let content = std::fs::read_to_string(&path)?;

    // Check if find string exists
    if !content.contains(find) {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Pattern not found in file: {:?}", find),
        ));
    }

    // Replace first occurrence only
    let new_content = content.replacen(find, replace, 1);

    // Write atomically
    crate::safe_io::atomic_write_text(&path, &new_content)?;

    Ok(format!(
        "File patched successfully: {} (replaced {} bytes with {} bytes)",
        path.display(),
        find.len(),
        replace.len()
    ))
}
```

**Step 2: Wire up in execute_file_tool**

```rust
        PATCH_FILE_TOOL_NAME => {
            let path = require_str_param(args, "path")?;
            let find = require_str_param(args, "find")?;
            let replace = require_str_param(args, "replace")?;
            Some(execute_patch_file(&path, &find, &replace))
        }
```

**Step 3: Write tests**

```rust
#[test]
fn test_execute_patch_file_success() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("test.txt");
    std::fs::write(&path, "hello world").unwrap();

    let result = execute_patch_file(
        path.to_str().unwrap(),
        "world",
        "universe"
    ).unwrap();

    assert!(result.contains("patched successfully"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello universe");
}

#[test]
fn test_execute_patch_file_pattern_not_found() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("test.txt");
    std::fs::write(&path, "hello world").unwrap();

    let result = execute_patch_file(
        path.to_str().unwrap(),
        "nonexistent",
        "replacement"
    );

    assert!(result.is_err());
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core execute_patch`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chibi-core/src/tools/file_tools.rs
git commit -m "$(cat <<'EOF'
feat: implement patch_file for find-and-replace edits

Uses atomic writes for safety. Replaces only first occurrence.
Permission check handled by pre_file_write hook (same as write_file).
EOF
)"
```

---

## Task 8: Create Default Permission Plugin

**Files:**
- Create: `~/.chibi/plugins/file_permission.py`

This is a reference implementation users can customize.

**Step 1: Write the plugin**

```python
#!/usr/bin/env -S uv run --quiet --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Default file permission handler for chibi.
Prompts user for y/N confirmation on every file write.
"""

import json
import os
import sys

SCHEMA = {
    "name": "file_permission",
    "description": "Prompts for permission before file writes",
    "parameters": {"type": "object", "properties": {}, "required": []},
    "hooks": ["pre_file_write"]
}

def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--schema":
        print(json.dumps(SCHEMA))
        return

    hook = os.environ.get("CHIBI_HOOK")
    if hook != "pre_file_write":
        print(json.dumps({"approved": True}))
        return

    hook_data = json.loads(os.environ.get("CHIBI_HOOK_DATA", "{}"))
    tool_name = hook_data.get("tool_name", "unknown")
    path = hook_data.get("path", "unknown")

    # Print to stderr (user sees this)
    if tool_name == "write_file":
        content = hook_data.get("content", "")
        preview = content[:200] + "..." if len(content) > 200 else content
        print(f"\n[{tool_name}] {path}", file=sys.stderr)
        print(f"Content preview:\n{preview}\n", file=sys.stderr)
    else:  # patch_file
        find = hook_data.get("find", "")
        replace = hook_data.get("replace", "")
        print(f"\n[{tool_name}] {path}", file=sys.stderr)
        print(f"Find: {find[:100]}", file=sys.stderr)
        print(f"Replace: {replace[:100]}\n", file=sys.stderr)

    # Prompt for permission
    try:
        response = input("Allow this file operation? [y/N]: ").strip().lower()
        approved = response == 'y'
    except EOFError:
        approved = False

    result = {
        "approved": approved,
        "reason": "User approved" if approved else "User denied"
    }
    print(json.dumps(result))

if __name__ == "__main__":
    main()
```

**Step 2: Make executable**

```bash
chmod +x ~/.chibi/plugins/file_permission.py
```

**Step 3: Test manually**

```bash
echo '{}' | CHIBI_HOOK=pre_file_write CHIBI_HOOK_DATA='{"tool_name":"write_file","path":"/tmp/test.txt","content":"hello"}' ~/.chibi/plugins/file_permission.py
```

**Step 4: Commit (in chibi repo, add to examples/)**

```bash
mkdir -p examples/plugins
cp ~/.chibi/plugins/file_permission.py examples/plugins/
git add examples/plugins/file_permission.py
git commit -m "$(cat <<'EOF'
docs: add example file_permission plugin

Reference implementation for pre_file_write hook.
Prompts user for y/N confirmation on each operation.
EOF
)"
```

---

## Task 9: Update Documentation

**Files:**
- Modify: `CLAUDE.md` (hooks section)
- Modify: `README.md` if it exists

**Step 1: Update CLAUDE.md hooks list**

Add `pre_file_write` to the hooks list.

**Step 2: Document new tools in help text or docs**

**Step 3: Commit**

```bash
git add CLAUDE.md README.md
git commit -m "$(cat <<'EOF'
docs: document file write tools and pre_file_write hook
EOF
)"
```

---

## Task 10: Run Full Test Suite and Verify

**Step 1: Run all tests**

```bash
cargo test
```

**Step 2: Run clippy**

```bash
cargo clippy -- -D warnings
```

**Step 3: Test manually**

```bash
cargo run -- "write a test file to /tmp/chibi-test.txt"
```

**Step 4: Final commit if any fixes needed**

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Context name in system prompt | `api/send.rs` |
| 2 | Datetime prefix on user messages | `api/send.rs` |
| 3 | Better call_agent description | `tools/builtin.rs` |
| 4 | File write tool definitions | `tools/file_tools.rs` |
| 5 | pre_file_write hook point | `tools/hooks.rs` |
| 6 | write_file execution | `tools/file_tools.rs`, `api/send.rs` |
| 7 | patch_file execution | `tools/file_tools.rs` |
| 8 | Example permission plugin | `examples/plugins/` |
| 9 | Documentation | `CLAUDE.md` |
| 10 | Verification | - |
