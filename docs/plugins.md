# Writing Plugins for Chibi

Plugins extend chibi by providing tools the LLM can use and hooks that run at specific lifecycle points.

## Quick Start

A plugin is any executable in `~/.chibi/plugins/`. When called with `--schema`, it outputs a JSON schema describing itself.

```python
#!/usr/bin/env python3
import json, sys

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

params = json.load(sys.stdin)
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

## Communication

**Tool calls:** Parameters are passed as JSON via stdin. Read with `json.load(sys.stdin)` (Python) or `jq` (bash).

**Hooks:** Hook data is also passed via stdin as JSON. The `CHIBI_HOOK` env var identifies which hook is firing.

## Environment Variables

When your plugin runs, chibi sets:

| Variable | When | Contents |
|----------|------|----------|
| `CHIBI_TOOL_NAME` | Tool call | Tool name (for multi-tool plugins) |
| `CHIBI_HOOK` | Hook execution | Hook name (e.g., `on_start`) |
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
| `on_start` | Chibi starts | `{implied_context, working_context, verbose}` |
| `on_end` | Chibi exits | `{implied_context, working_context}` |
| `pre_message` | Before sending user message to LLM | `{prompt, context_name, summary}` |
| `post_message` | After receiving LLM response | `{prompt, response, context_name}` |
| `pre_tool` | Before executing a tool | `{tool_name, arguments}` |
| `post_tool` | After tool execution | `{tool_name, arguments, result}` |
| `on_context_switch` | Context changes | `{from_context, to_context, is_ephemeral}` |
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
| `pre_spawn_agent` | Before sub-agent LLM call | `{system_prompt, input, model, temperature, max_tokens}` |
| `post_spawn_agent` | After sub-agent LLM call | `{system_prompt, input, model, response}` |

### Hook Output

Hooks can return JSON to stdout. For most hooks, this is informational. Some hooks have special behavior:

- `pre_message`: Return `{"prompt": "text"}` to modify the user's prompt before sending to LLM
- `pre_tool`: Return `{"block": true, "message": "reason"}` to prevent tool execution
- `pre_tool`: Return `{"arguments": {...}}` to modify tool arguments before execution
- `pre_system_prompt` / `post_system_prompt`: Return `{"inject": "text"}` to add content to the system prompt
- `pre_send_message`: Return `{"delivered": true, "via": "..."}` to intercept message delivery
- `pre_spawn_agent`: Return `{"response": "..."}` to replace the LLM call, or `{"block": true, "message": "..."}` to block it

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
    data = json.load(sys.stdin)
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
chibi -p myplugin "arg1 arg2"        # Run a plugin with args (shell-style split)
chibi -p myplugin "'with spaces'"    # Args with spaces need inner quotes
chibi -p myplugin ""                 # No args (empty string required)
chibi -P update_todos '{}'           # Call tool with empty JSON
chibi -P send '{"to":"x"}'           # Call tool with JSON args
```

Both flags take exactly 2 arguments. For `-p`, the second argument is a shell-style
args string that gets split into an array. For `-P`, it's a JSON object passed directly.

This allows mixing with other flags in any order:

```bash
chibi -p myplugin "list --all" -v    # -v works as verbose flag
chibi -P send '{}' -C ephemeral      # -C works for context
```

The args are passed as `{"args": ["arg1", "arg2"]}` via stdin. Design your plugin to handle both LLM calls (structured parameters) and direct calls (args array) if needed:

```python
params = json.load(sys.stdin)

if "args" in params:
    # Direct invocation: chibi -p myplugin "list --all"
    args = params["args"]  # ["list", "--all"]
    # Parse args as needed
else:
    # LLM invocation: structured parameters
    name = params["name"]
```

## I/O Conventions

- **stdin**: JSON parameters (tool calls) or hook data
- **stdout**: Tool output returned to LLM (or printed for `-P`)
- **stderr**: Diagnostics, prompts, progress (goes to terminal)

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

# Parse JSON args from stdin (requires jq)
path=$(jq -r '.path // "."')
du -sh "$path"
```

## Testing Plugins

Test schema output:
```bash
./my_plugin --schema | jq .
```

Test tool execution:
```bash
echo '{"name": "world"}' | ./my_plugin
```

Test hooks:
```bash
echo '{}' | CHIBI_HOOK="on_start" ./my_plugin
```

## Built-in Tools

Chibi provides several built-in tools that don't require plugins:

**Agentic tools:**
- `update_todos` - Manage per-context todo list
- `update_goals` - Manage per-context goals
- `update_reflection` - Update LLM's persistent memory
- `send_message` - Send messages between contexts

**File tools** (for examining cached tool outputs and allowed paths):
- `file_head` - Read first N lines
- `file_tail` - Read last N lines
- `file_lines` - Read specific line range
- `file_grep` - Search for patterns
- `cache_list` - List cached outputs

**Agent tools** (for spawning sub-agents):
- `spawn_agent` - Spawn a sub-agent with a custom system prompt
- `retrieve_content` - Read a file/URL and process it through a sub-agent

These are always available to the LLM. See [agentic.md](agentic.md) for details on sub-agents and tool output caching.

## Language Plugins

Language plugins provide symbol extraction for the codebase index. Core handles all database writes.

**Convention:** plugins named `lang_<language>` (e.g. `lang_rust`, `lang_python`).

**Input** (stdin, JSON):
```json
{"files": [{"path": "src/foo.rs", "content": "..."}]}
```

**Output** (stdout, JSON):
```json
{
  "symbols": [
    {"name": "parse", "kind": "function", "parent": "Parser",
     "line_start": 42, "line_end": 67, "signature": "fn parse(&self) -> Result<AST>", "visibility": "public"}
  ],
  "refs": [
    {"from_line": 55, "to_name": "TokenStream::new", "kind": "call"}
  ]
}
```

**Symbol fields:** `name` (required), `kind` (required), `line_start`/`line_end` (optional), `parent` (optional, for nesting), `signature`/`visibility` (optional).

**Ref fields:** `from_line`, `to_name`, `kind` (all optional but recommended).

The `post_index_file` hook fires after each file is indexed with `{"path", "lang", "symbol_count", "ref_count"}`.
