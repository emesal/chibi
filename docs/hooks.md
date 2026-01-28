# Hooks

Chibi supports a hooks system that allows plugins to register for lifecycle events. Hooks can observe events or modify data as it flows through the system.

## Hook Points

### Session Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `on_start` | When chibi starts (before any processing) | No |
| `on_end` | When chibi exits (after all processing) | No |

### Message Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_message` | Before sending a prompt to the LLM | Yes (prompt) |
| `post_message` | After receiving LLM response | No |

### System Prompt Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_system_prompt` | Before building system prompt | Yes (inject content) |
| `post_system_prompt` | After building system prompt | Yes (inject content) |

### Tool Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_tool` | Before executing a tool | Yes (arguments, can block) |
| `post_tool` | After executing a tool | No |
| `pre_tool_output` | After tool execution, before caching | Yes (output, can block/replace) |
| `post_tool_output` | After tool output is processed | No |

### API Request Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_api_tools` | Before tools are sent to API | Yes (filter tools) |
| `pre_api_request` | Before API request is sent | Yes (modify request body) |

### Tool Output Caching

| Hook | When | Can Modify |
|------|------|------------|
| `pre_cache_output` | Before caching large tool output | Yes (can provide custom summary) |
| `post_cache_output` | After output is cached | No |

### Message Delivery

| Hook | When | Can Modify |
|------|------|------------|
| `pre_send_message` | Before delivering inter-context message | Yes (can claim delivery) |
| `post_send_message` | After message delivery | No |

### Context Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `on_context_switch` | When switching contexts | No |
| `pre_clear` | Before clearing context | No |
| `post_clear` | After clearing context | No |
| `pre_compact` | Before full compaction | No |
| `post_compact` | After full compaction | No |
| `pre_rolling_compact` | Before rolling compaction | No |
| `post_rolling_compact` | After rolling compaction | No |

## Registering for Hooks

Plugins register for hooks via their `--schema` JSON output:

```json
{
  "name": "my_tool",
  "description": "Tool description",
  "parameters": {
    "type": "object",
    "properties": {}
  },
  "hooks": ["on_start", "pre_message", "post_message"]
}
```

## Hook Execution

When a hook fires, registered plugins are called with environment variables:

- `CHIBI_HOOK` - Hook point name (e.g., "pre_message")
- `CHIBI_HOOK_DATA` - JSON data about the event

## Hook Data by Type

### on_start / on_end

```json
{
  "current_context": "default",
  "verbose": true
}
```

### pre_message

```json
{
  "prompt": "user's prompt",
  "context_name": "default",
  "summary": "conversation summary..."
}
```

**Can return:**
```json
{
  "prompt": "modified prompt"
}
```

### post_message

```json
{
  "prompt": "original prompt",
  "response": "LLM's response",
  "context_name": "default"
}
```

### pre_system_prompt / post_system_prompt

```json
{
  "context_name": "default",
  "summary": "conversation summary...",
  "todos": "current todos...",
  "goals": "current goals..."
}
```

**Can return:**
```json
{
  "inject": "content to add to system prompt"
}
```

### pre_api_tools

Called before tools are sent to the API. Allows dynamic filtering of which tools the LLM can use.

```json
{
  "context_name": "default",
  "tools": [
    {"name": "update_todos", "type": "builtin"},
    {"name": "file_head", "type": "file"},
    {"name": "my_plugin", "type": "plugin"}
  ],
  "recursion_depth": 0
}
```

Tool types are:
- `builtin`: update_todos, update_goals, update_reflection, send_message
- `file`: file_head, file_tail, file_lines, file_grep, cache_list
- `plugin`: Tools loaded from the plugins directory

**Can return (to filter tools):**
```json
{
  "exclude": ["file_grep", "my_plugin"]
}
```

Or to use allowlist mode:
```json
{
  "include": ["update_todos", "update_goals"]
}
```

When multiple hooks respond:
- Includes are **intersected** (most restrictive wins)
- Excludes are **unioned** (all excluded tools are removed)

### pre_api_request

Called after tools are filtered but before the HTTP request is sent. Allows modification of any part of the request body.

```json
{
  "context_name": "default",
  "request_body": {
    "model": "anthropic/claude-sonnet-4",
    "messages": [...],
    "tools": [...],
    "temperature": 0.7,
    ...
  },
  "recursion_depth": 0
}
```

**Can return (to modify request):**
```json
{
  "request_body": {
    "temperature": 0.3,
    "max_tokens": 2000
  }
}
```

Returned fields are **merged** into the request body, not replaced entirely. This allows targeted modifications without needing to echo back the entire body.

### pre_tool

```json
{
  "tool_name": "read_file",
  "arguments": {"path": "/etc/passwd"}
}
```

**Can return:**
```json
{
  "arguments": {"path": "/safe/path"}
}
```

Or to block execution:
```json
{
  "block": true,
  "message": "This operation is not allowed"
}
```

### post_tool

```json
{
  "tool_name": "read_file",
  "arguments": {"path": "Cargo.toml"},
  "result": "file contents...",
  "cached": false
}
```

Note: `cached` is `true` if the output was cached due to size.

### pre_tool_output

Called immediately after a tool returns its output, before any caching decisions. Can modify or replace the output entirely.

```json
{
  "tool_name": "read_file",
  "arguments": {"path": "Cargo.toml"},
  "output": "raw tool output..."
}
```

**Can return (to modify output):**
```json
{
  "output": "modified output..."
}
```

Or to block/replace entirely:
```json
{
  "block": true,
  "message": "Replacement message shown to LLM"
}
```

### post_tool_output

Called after tool output processing (including any pre_tool_output modifications and caching decisions). Observe only.

```json
{
  "tool_name": "read_file",
  "arguments": {"path": "Cargo.toml"},
  "output": "original output (after pre_tool_output modifications)",
  "final_output": "what the LLM will see (may be truncated if cached)",
  "cached": false
}
```

### pre_cache_output

Called before caching a large tool output. Can provide a custom summary.

```json
{
  "tool_name": "fetch_url",
  "arguments": {"url": "https://example.com"},
  "content": "full output content...",
  "char_count": 50000,
  "line_count": 1200
}
```

**Can return (to provide custom summary):**
```json
{
  "summary": "Custom summary of the content..."
}
```

### post_cache_output

Notification after output has been cached.

```json
{
  "tool_name": "fetch_url",
  "cache_id": "fetch_url_abc123_def456",
  "char_count": 50000,
  "token_estimate": 12500,
  "line_count": 1200
}
```

### pre_send_message

```json
{
  "from": "default",
  "to": "research",
  "content": "message content",
  "context_name": "default"
}
```

**Can return (to claim delivery):**
```json
{
  "delivered": true,
  "via": "external-service"
}
```

### post_send_message

```json
{
  "from": "default",
  "to": "research",
  "content": "message content",
  "context_name": "default",
  "delivery_result": "Message delivered to 'research' via local inbox"
}
```

### on_context_switch

```json
{
  "from_context": "default",
  "to_context": "coding",
  "is_transient": false
}
```

### pre_clear / post_clear

```json
{
  "context_name": "default",
  "message_count": 10,
  "summary": "existing summary..."
}
```

### pre_compact / post_compact

```json
{
  "context_name": "default",
  "message_count": 20,
  "summary": "conversation summary..."
}
```

### pre_rolling_compact / post_rolling_compact

```json
{
  "context_name": "default",
  "message_count": 50,
  "non_system_count": 48,
  "summary": "conversation summary..."
}
```

For `post_rolling_compact`:
```json
{
  "context_name": "default",
  "message_count": 25,
  "messages_archived": 25,
  "summary": "updated summary..."
}
```

## Example Hook Plugin

A minimal hook plugin that logs events:

```bash
#!/bin/bash
# ~/.chibi/plugins/logger

if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "logger",
  "description": "Logs lifecycle events",
  "parameters": {"type": "object", "properties": {}},
  "hooks": ["on_start", "on_end", "pre_message", "post_message"]
}
EOF
  exit 0
fi

# Handle hook call
if [[ -n "$CHIBI_HOOK" ]]; then
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $CHIBI_HOOK" >> ~/.chibi/hook.log
  echo "$CHIBI_HOOK_DATA" | jq '.' >> ~/.chibi/hook.log
  echo "{}"  # Return empty JSON (no modifications)
  exit 0
fi

# Normal tool call (this plugin is hook-only)
echo "This tool only handles hooks"
```

## Example: Prompt Modifier

A hook that adds context to every prompt:

```python
#!/usr/bin/env python3
# ~/.chibi/plugins/context_injector

import sys
import json
import os

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "context_injector",
        "description": "Injects additional context into prompts",
        "parameters": {"type": "object", "properties": {}},
        "hooks": ["pre_message"]
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook == "pre_message":
    data = json.loads(os.environ.get("CHIBI_HOOK_DATA", "{}"))
    prompt = data.get("prompt", "")

    # Add timestamp to every prompt
    from datetime import datetime
    modified = f"[{datetime.now().isoformat()}]\n{prompt}"

    print(json.dumps({"prompt": modified}))
    sys.exit(0)

print("{}")
```

## Example: Tool Blocker

A hook that blocks certain tool operations:

```bash
#!/bin/bash
# ~/.chibi/plugins/safety_guard

if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "safety_guard",
  "description": "Blocks dangerous tool operations",
  "parameters": {"type": "object", "properties": {}},
  "hooks": ["pre_tool"]
}
EOF
  exit 0
fi

if [[ "$CHIBI_HOOK" == "pre_tool" ]]; then
  tool_name=$(echo "$CHIBI_HOOK_DATA" | jq -r '.tool_name')

  # Block run_command for certain patterns
  if [[ "$tool_name" == "run_command" ]]; then
    command=$(echo "$CHIBI_HOOK_DATA" | jq -r '.arguments.command // ""')
    if [[ "$command" == *"rm -rf"* ]]; then
      echo '{"block": true, "message": "Blocked: rm -rf commands are not allowed"}'
      exit 0
    fi
  fi

  echo '{}'
  exit 0
fi

echo '{}'
```

## Example: Tool Filter

A hook that restricts available tools dynamically:

```bash
#!/bin/bash
# ~/.chibi/plugins/tool_filter

if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "tool_filter",
  "description": "Filters available tools based on context",
  "parameters": {"type": "object", "properties": {}},
  "hooks": ["pre_api_tools"]
}
EOF
  exit 0
fi

if [[ "$CHIBI_HOOK" == "pre_api_tools" ]]; then
  context=$(echo "$CHIBI_HOOK_DATA" | jq -r '.context_name')

  # Restrict tools in "safe" context
  if [[ "$context" == "safe" ]]; then
    echo '{"include": ["update_todos", "update_goals"]}'
    exit 0
  fi

  # Exclude file tools in all contexts
  echo '{"exclude": ["file_head", "file_tail", "file_lines", "file_grep"]}'
  exit 0
fi

echo '{}'
```

## Example: Temperature Override

A hook that modifies API request parameters:

```python
#!/usr/bin/env python3
# ~/.chibi/plugins/temp_override

import sys
import json
import os

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "temp_override",
        "description": "Overrides temperature based on context",
        "parameters": {"type": "object", "properties": {}},
        "hooks": ["pre_api_request"]
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook == "pre_api_request":
    data = json.loads(os.environ.get("CHIBI_HOOK_DATA", "{}"))
    context = data.get("context_name", "")

    # Use low temperature for "coding" context
    if context == "coding":
        print(json.dumps({"request_body": {"temperature": 0.1}}))
        sys.exit(0)

    # Use high temperature for "creative" context
    if context == "creative":
        print(json.dumps({"request_body": {"temperature": 1.2}}))
        sys.exit(0)

print("{}")
```

## Use Cases

- **Logging** - Record all interactions for debugging or auditing
- **Metrics** - Track tool usage, message counts, context switches
- **Integration** - Notify external systems about events
- **Validation** - Pre-check messages or tool arguments before execution
- **Backup** - Save state before destructive operations
- **Security** - Block or modify dangerous operations
- **Enrichment** - Add context or metadata to prompts
- **Tool Restriction** - Filter available tools based on context or permissions
- **API Customization** - Modify temperature, max_tokens, or other API parameters
