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

See [hooks.md](hooks.md) for the full hook reference — payloads, return values, and all 31 hook points.

### Hook Output

Hooks can return JSON to stdout. For most hooks, this is informational. Some hooks have special behaviour:

- `pre_message`: Return `{"prompt": "text"}` to modify the user's prompt before sending to LLM
- `pre_tool`: Return `{"block": true, "message": "reason"}` to prevent tool execution
- `pre_tool`: Return `{"arguments": {...}}` to modify tool arguments before execution
- `pre_system_prompt` / `post_system_prompt`: Return `{"inject": "text"}` to add content to the system prompt
- `pre_send_message`: Return `{"delivered": true, "via": "..."}` to intercept message delivery
- `pre_spawn_agent`: Return `{"response": "..."}` to replace the LLM call, or `{"block": true, "message": "..."}` to block it
- `pre_cache_output`: Return `{"summary": "..."}` to provide a custom summary instead of caching
- `pre_api_tools`: Return `{"remove": ["tool_name"]}` to filter tools from the API request
- `pre_agentic_loop` / `post_tool_batch`: Return `{"handoff": "user"|"agent"|"none"}` to override the fallback

Return empty output or `{}` if you have nothing to contribute.

### Hook Example

```python
#!/usr/bin/env python3
import json, os, sys

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "startup_logger",
        "description": "Logs chibi startup info",
        "parameters": {"type": "object", "properties": {}},
        "hooks": ["on_start"]
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook == "on_start":
    data = json.load(sys.stdin)
    with open("/tmp/chibi_starts.log", "a") as f:
        f.write(f"started: project_root={data['project_root']}\n")
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

Chibi provides built-in tools that don't require plugins:

**Agentic tools:**
- `update_todos` - Manage per-context todo list
- `update_goals` - Manage per-context goals
- `update_reflection` - Update LLM's persistent memory
- `send_message` - Send messages between contexts
- `spawn_agent` - Spawn a sub-agent with a custom system prompt

**File tools** (for reading and writing files and cached tool outputs):
- `file_head` - Read first N lines (accepts `vfs:///` URIs for cached outputs)
- `file_tail` - Read last N lines (accepts `vfs:///` URIs)
- `file_lines` - Read specific line range (accepts `vfs:///` URIs)
- `file_grep` - Search for patterns (accepts `vfs:///` URIs)
- `write_file` - Write content to a file or VFS path

**Coding tools** (project-aware, path-relative to project root):
- `shell_exec` - Execute a shell command
- `dir_list` - List a directory tree
- `glob_files` - Find files matching a glob pattern
- `grep_files` - Regex-search files with context lines
- `file_edit` - Structured file edits (replace_lines, insert_before, insert_after, delete_lines, replace_string)
- `fetch_url` - Fetch content from a URL

**Index tools:**
- `index_update` - Walk and index the project for symbol search
- `index_query` - Search the codebase index for symbols or references
- `index_status` - Show index summary (file counts, symbol totals)

See [agentic.md](agentic.md) for details on sub-agents and tool output caching.

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
