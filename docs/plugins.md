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
- `pre_api_tools`: Return `{"exclude": ["tool_name"]}` or `{"include": ["tool_name"]}` to filter tools
- `pre_agentic_loop` / `post_tool_batch`: Return `{"fallback": "call_user"|"call_agent"}` to override the fallback, or `{"fuel": N}` / `{"fuel_delta": N}` to adjust fuel

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
chibi -P update_goals '{}'           # Call tool with empty JSON
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

## Synthesised Tools (Scheme)

Synthesised tools are written in R7RS Scheme and live in the VFS under `/tools/`. Chibi evaluates them via [tein](https://github.com/emesal/tein) and registers them alongside regular plugins.

### File Locations

| VFS Path | Tier | Visibility |
|----------|------|------------|
| `/tools/shared/*.scm` | sandboxed (default) | all contexts |
| `/tools/home/<context>/*.scm` | sandboxed | owner context only |
| `/tools/flocks/<flock>/*.scm` | sandboxed | flock members only |

Subdirectories are scanned recursively. Files are loaded at startup and hot-reloaded when written via the VFS.

### Convention Format (single tool per file)

```scheme
(import (scheme base))

(define tool-name "word_count")
(define tool-description "Count words in text")
(define tool-parameters
  '((text . ((type . "string") (description . "text to count")))))

(define (tool-execute args)
  (let ((text (cdr (assoc "text" args))))
    (number->string (string-length text))))
```

### Multi-Tool Format (`define-tool`)

Use `(import (harness tools))` to access the `define-tool` macro. A single file can define multiple tools:

```scheme
(import (scheme base))
(import (harness tools))

(define-tool greet
  (description "Greet someone")
  (parameters '((name . ((type . "string") (description . "name")))))
  (execute (lambda (args)
    (string-append "Hello, " (cdr (assoc "name" args)) "!"))))

(define-tool farewell
  (description "Say goodbye")
  (parameters '((name . ((type . "string") (description . "name")))))
  (execute (lambda (args)
    (string-append "Goodbye, " (cdr (assoc "name" args)) "!"))))
```

Optional keywords `category` and `summary-params` can follow `description`:

```scheme
(define-tool fetch-data
  (description "Fetch data from an API endpoint")
  (category "network")
  (summary-params ("endpoint" "method"))
  (parameters '((endpoint . ((type . "string") (description . "API endpoint path")))
                (method   . ((type . "string") (description . "HTTP method")))))
  (execute (lambda (args) ...)))
```

- **`category`** — string: `"network"`, `"shell"`, or `"synthesised"` (default). Network tools fire `pre_fetch_url` before execution.
- **`summary-params`** — list of parameter names used to build the human-readable permission-prompt summary for network tools that have no `url` parameter.

### `(harness tools)` Module

The `(harness tools)` module exposes:

- **`define-tool`** — macro for defining multiple tools in one file. Registers the tool into an internal `%tool-registry%` list that chibi reads after evaluation.
- **`call-tool`** — procedure `(call-tool name args)` for calling other registered tools from within a tool's `execute` body. `args` is an alist of `("key" . value)` pairs. Returns the tool's string output.

`call-tool` bridges synchronously into chibi's async tool dispatch. It is available in both sandboxed and unsandboxed tiers.

### `(harness io)` Module — Privileged IO (Unsandboxed Only)

Available at `Unsandboxed` tier only. Provides direct VFS and local filesystem IO that bypasses the tool dispatch and hook layers. Intended for builtin plugins that need IO during hook callbacks where `call-tool` would cause re-entrancy.

```scheme
(import (harness io))

(io-read path)           ; → string or #f (not found)
(io-write path data)     ; → #t, raises on error
(io-append path data)    ; → #t, raises on error
(io-list path)           ; → list of entry name strings, '() for nonexistent
(io-exists? path)        ; → boolean
```

**Path dispatch:**

| Prefix | Destination | Caller |
|--------|-------------|--------|
| `"vfs://..."` | VFS backend | `VfsCaller::System` (bypasses zone permissions) |
| Bare absolute path | Local filesystem via `tokio::fs` | — |

**Example:**

```scheme
(import (scheme base))
(import (harness io))

; Write and read a VFS file
(io-write "vfs:///shared/notes.txt" "my note")
(io-read  "vfs:///shared/notes.txt")  ; => "my note"

; Check existence
(io-exists? "vfs:///shared/notes.txt")  ; => #t
(io-exists? "vfs:///shared/missing")    ; => #f

; List a directory
(io-list "vfs:///shared")  ; => ("notes.txt")
```

Normal tein tools should use `call-tool` for IO when possible — it goes through the regular tool dispatch and honours hooks. Use `(harness io)` when the tool runs *inside* a hook callback and direct VFS access is needed without triggering further hooks.

### Harness Helpers

The harness also injects these foreign functions into every synthesised tool context:

| Procedure | Returns | Description |
|-----------|---------|-------------|
| `(generate-id)` | `"a3f2b1c9"` (8 hex chars) | Short unique ID from uuid v4 |
| `(current-timestamp)` | `"20260308-1423z"` | Current UTC time as `YYYYMMDD-HHMMz` |
| `%context-name%` | `"alice"` | Mutable binding holding the calling context's name; updated before each call |

Use `%context-name%` to resolve VFS paths relative to the calling context's home directory (e.g. `(string-append "/home/" %context-name% "/tasks")`).

### Task Plugin

The bundled task plugin (`plugins/tasks.scm` in the repo) provides structured task management. Install it to the VFS:

```bash
# install globally (visible to all contexts)
chibi -P write_file '{"path": "vfs:///tools/shared/tasks.scm", "content": "<paste file contents>"}'
```

Or copy to `/tools/shared/tasks.scm` in your VFS root directly.

**Tools:**

| Tool | Parameters | Description |
|------|------------|-------------|
| `task_create` | `path`, `body`, `priority`, `assigned-to`, `depends-on` | Create a task; returns id and VFS path |
| `task_update` | `id`, `status`, `priority`, `body`, `assigned-to` | Update task fields by ID |
| `task_view` | `id` | Read full task metadata and body |
| `task_list` | `status`, `priority`, `assigned-to` (optional filters) | List tasks |
| `task_delete` | `id` | Remove a task file |

**Path conventions:**
- `auth/login` → `/home/<ctx>/tasks/auth/login.task`
- `flock:infra/deploy` → `/flocks/infra/tasks/deploy.task`

Tasks are automatically summarised and injected as ephemeral context before each prompt. See [vfs.md](vfs.md) for the `.task` file format.

### File History Plugin

The bundled history plugin (`plugins/history.scm`) automatically snapshots VFS files before each write and exposes tools for browsing, diffing, and reverting to prior revisions.

**Install:**

```bash
chibi -P write_file '{"path": "vfs:///tools/shared/history.scm", "content": "<paste file contents>"}'
```

The plugin auto-configures to `unsandboxed` tier when installed at `vfs:///tools/shared/history.scm` (see `BUILTIN_UNSANDBOXED` in `config.rs`). No `[tools.tiers]` entry required.

**How it works:**

Registers a `pre_vfs_write` hook that reads the current file content and writes it as a numbered revision before each overwrite. Hook is best-effort — errors are silently swallowed to avoid blocking writes.

**Storage layout:**

```
<file-dir>/.chibi/history/<filename>/<N>   — revision N (full content)
<file-dir>/.chibi/history/<filename>/meta  — alist: ((next . N))
```

Revisions are kept under the same VFS directory as the file itself. The `.chibi/` prefix ensures they are hidden from `vfs_list` output. `io-list` and direct addressing still reach them.

At most 10 revisions are kept (`%history-keep%`); oldest are pruned automatically.

**Tools:**

| Tool | Parameters | Description |
|------|------------|-------------|
| `file_history_log` | `path` | List revision numbers for a VFS file (newest first) |
| `file_history_show` | `path`, `revision` | Show full file content at a specific revision |
| `file_history_diff` | `path`, `revision` (optional) | Unified diff between a revision and current content |
| `file_history_revert` | `path`, `revision` | Restore file to a previous revision (fires hook so revert itself is snapshotted) |

**Requires:** `(harness io)` and `(chibi diff)` — unsandboxed tier only.

### Sandbox Tiers

Tools run in one of two tiers, configured per-path in `[tools.tiers]` (see [configuration.md](configuration.md)):

| Tier | Access | Step Limit |
|------|--------|------------|
| `sandboxed` (default) | `Modules::Safe` subset of R7RS | 10,000,000 steps |
| `unsandboxed` | Full R7RS | None |

`Modules::Safe` allows `(scheme base)`, `(scheme write)`, `(scheme read)`, `(scheme char)`, and other pure modules. It blocks modules with `default_safe: false` — notably `(scheme regex)`, `(tein modules)`, and network/filesystem access.

### Parameters Format

The `tool-parameters` / `parameters` value is a Scheme alist that chibi converts to JSON Schema:

```scheme
'((text  . ((type . "string")  (description . "input text")))
  (count . ((type . "integer") (description . "how many") (required . #f))))
```

All parameters are required by default. Add `(required . #f)` to an attribute alist to make a parameter optional.

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
