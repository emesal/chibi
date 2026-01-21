# Writing Plugins for Chibi

Plugins extend chibi by providing tools the LLM can use and hooks that run at specific lifecycle points.

## Quick Start

A plugin is any executable in `~/.chibi/plugins/`. When called with `--schema`, it outputs a JSON schema describing itself.

```python
#!/usr/bin/env python3
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "greet",
        "description": "Greet someone by name",
        "parameters": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Name to greet"}
            },
            "required": ["name"]
        }
    }))
    sys.exit(0)

params = json.loads(os.environ["CHIBI_TOOL_ARGS"])
print(f"Hello, {params['name']}!")
```

Make it executable and drop it in `~/.chibi/plugins/`.

## Schema Format

The schema JSON must include:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Tool name (used by LLM to call it) |
| `description` | string | What the tool does (shown to LLM) |
| `parameters` | object | JSON Schema for parameters |
| `hooks` | array | Optional list of hooks to register for |

### Parameters

Parameters follow [JSON Schema](https://json-schema.org/) format:

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "limit": {
      "type": "integer",
      "description": "Max results",
      "default": 10
    }
  },
  "required": ["query"]
}
```

## Environment Variables

When your plugin runs, chibi sets:

| Variable | When | Contents |
|----------|------|----------|
| `CHIBI_TOOL_ARGS` | Tool call | JSON object with parameters |
| `CHIBI_TOOL_NAME` | Tool call | Tool name (for multi-tool plugins) |
| `CHIBI_HOOK` | Hook execution | Hook name (e.g., `on_start`) |
| `CHIBI_HOOK_DATA` | Hook execution | JSON object with hook-specific data |
| `CHIBI_VERBOSE` | Always (if `-v`) | Set to `1` when verbose mode is on |

## Hooks

Plugins can register for lifecycle hooks by including a `hooks` array in the schema:

```json
{
  "name": "my_plugin",
  "description": "...",
  "parameters": {...},
  "hooks": ["on_start", "pre_message", "post_system_prompt"]
}
```

### Available Hooks

| Hook | When | Hook Data |
|------|------|-----------|
| `on_start` | Chibi starts | `{current_context, verbose}` |
| `on_end` | Chibi exits | `{current_context}` |
| `pre_message` | Before sending user message to LLM | `{prompt, context_name, summary}` |
| `post_message` | After receiving LLM response | `{prompt, response, context_name}` |
| `pre_tool` | Before executing a tool | `{tool_name, arguments}` |
| `post_tool` | After tool execution | `{tool_name, arguments, result}` |
| `on_context_switch` | Context changes | `{from_context, to_context, is_sub_context}` |
| `pre_clear` | Before clearing context | `{context_name, message_count, summary}` |
| `post_clear` | After clearing context | `{context_name}` |
| `pre_compact` | Before manual compaction | `{context_name, message_count, summary}` |
| `post_compact` | After manual compaction | `{context_name, message_count, summary}` |
| `pre_rolling_compact` | Before auto-compaction | `{context_name, message_count, non_system_count, summary}` |
| `post_rolling_compact` | After auto-compaction | `{context_name, message_count, messages_archived, summary}` |
| `pre_system_prompt` | Building system prompt | `{context_name, summary, todos, goals}` |
| `post_system_prompt` | After system prompt built | `{context_name, summary, todos, goals}` |
| `pre_send_message` | Before inter-context message | `{from, to, content, context_name}` |
| `post_send_message` | After inter-context message | `{from, to, content, context_name, delivery_result}` |

### Hook Output

Hooks can return JSON to stdout. For most hooks, this is informational. Some hooks have special behavior:

- `pre_message`: Return `{"prompt": "text"}` to modify the user's prompt before sending to LLM
- `pre_tool`: Return `{"block": true, "message": "reason"}` to prevent tool execution
- `pre_tool`: Return `{"arguments": {...}}` to modify tool arguments before execution
- `pre_system_prompt` / `post_system_prompt`: Return `{"inject": "text"}` to add content to the system prompt
- `pre_send_message`: Return `{"delivered": true, "via": "..."}` to intercept message delivery

Return empty output or `{}` if you have nothing to contribute.

### Hook Example

```python
#!/usr/bin/env python3
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "context_logger",
        "description": "Logs context switches",
        "parameters": {"type": "object", "properties": {}},
        "hooks": ["on_context_switch"]
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook == "on_context_switch":
    data = json.loads(os.environ["CHIBI_HOOK_DATA"])
    with open("/tmp/context_log.txt", "a") as f:
        f.write(f"{data['from_context']} -> {data['to_context']}\n")
    print("{}")
    sys.exit(0)

# Tool call (this plugin is hook-only, but handle anyway)
print("This plugin only handles hooks")
```

## Direct Invocation

Users can run plugins directly without the LLM using `-p` (plugin) or `-P` (call-tool):

```bash
chibi -p myplugin arg1 arg2    # Run a plugin directly
chibi -P update_todos '{"content": "..."}'  # Call any tool (plugin or built-in)
```

The args are passed as `{"args": ["arg1", "arg2"]}` in `CHIBI_TOOL_ARGS`. Design your plugin to handle both LLM calls (structured parameters) and direct calls (args array) if needed:

```python
params = json.loads(os.environ["CHIBI_TOOL_ARGS"])

if "args" in params:
    # Direct invocation: chibi -P myplugin list --all
    args = params["args"]
    # Parse args as needed
else:
    # LLM invocation: structured parameters
    name = params["name"]
```

## I/O Conventions

- **stdout**: Tool output returned to LLM (or printed for `-P`)
- **stderr**: Diagnostics, prompts, progress (goes to terminal)
- **stdin**: Available for user interaction (confirmations, input)

Example with user confirmation:

```python
print("Are you sure? [y/N] ", end="", file=sys.stderr)
response = input()
if response.lower() != "y":
    print("Cancelled")
    sys.exit(0)
```

## Python with uv

For Python plugins with dependencies, use [uv](https://github.com/astral-sh/uv) script mode:

```python
#!/usr/bin/env -S uv run --quiet --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["requests", "beautifulsoup4"]
# ///

import json, os, sys
import requests
from bs4 import BeautifulSoup

# ... rest of plugin
```

This auto-installs dependencies on first run.

## Bash Plugins

Plugins can be any executable. Here's a bash example:

```bash
#!/bin/bash

if [[ "$1" == "--schema" ]]; then
    cat <<'EOF'
{
  "name": "disk_usage",
  "description": "Show disk usage for a path",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {"type": "string", "description": "Path to check", "default": "."}
    }
  }
}
EOF
    exit 0
fi

# Parse JSON args (requires jq)
path=$(echo "$CHIBI_TOOL_ARGS" | jq -r '.path // "."')
du -sh "$path"
```

## Testing Plugins

Test schema output:
```bash
./my_plugin --schema | jq .
```

Test tool execution:
```bash
CHIBI_TOOL_ARGS='{"name": "world"}' ./my_plugin
```

Test hooks:
```bash
CHIBI_HOOK="on_start" CHIBI_HOOK_DATA='{"current_context": "default", "verbose": true}' ./my_plugin
```

## Built-in Tools

Chibi provides several built-in tools that don't require plugins:

- `update_todos` - Manage per-context todo list
- `update_goals` - Manage per-context goals
- `update_reflection` - Update LLM's persistent memory
- `send_message` - Send messages between contexts

These are always available to the LLM.
