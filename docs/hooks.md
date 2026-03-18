# Hooks

Chibi supports a hooks system that allows plugins to register for lifecycle events. Hooks can observe events or modify data as it flows through the system.

<!-- BEGIN GENERATED HOOK REFERENCE — do not edit, run `just generate-docs` -->
## Hook Points

### Session Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `on_start` | fires when chibi starts, before any processing | No |
| `on_end` | fires when chibi exits, after all processing | No |

### Message Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_message` | fires before sending a prompt to the LLM | Yes |
| `post_message` | fires after receiving the LLM response | No |

### System Prompt Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_system_prompt` | fires before building the system prompt; can inject content | Yes |
| `post_system_prompt` | fires after building the system prompt; can inject content | Yes |

### Tool Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_tool` | fires before executing a tool; can modify arguments or block | Yes |
| `post_tool` | fires after executing a tool; observe only | No |
| `pre_tool_output` | fires after tool returns, before caching decisions; can modify or block output | Yes |
| `post_tool_output` | fires after tool output processing and caching; observe only | No |

### API Request Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_api_tools` | fires before tools are sent to the API; can filter tools | Yes |
| `pre_api_request` | fires after tool filtering, before HTTP request; can modify request body | Yes |

### Agentic Loop Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_agentic_loop` | fires before each agentic loop iteration; can override fallback and fuel | Yes |
| `post_tool_batch` | fires after processing a batch of tool calls; can override fallback and adjust fuel | Yes |

### File Permission

| Hook | When | Can Modify |
|------|------|------------|
| `pre_file_read` | fires before reading a file outside allowed paths; deny-only permission protocol | Yes |
| `pre_file_write` | fires before write_file or file_edit; deny-only permission protocol | Yes |
| `pre_shell_exec` | fires before shell_exec; deny-only permission protocol | Yes |

### URL Security

| Hook | When | Can Modify |
|------|------|------------|
| `pre_fetch_url` | fires before fetching a sensitive URL or invoking a network-category tool without a URL; deny-only | Yes |

### Sub-Agent Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_spawn_agent` | fires before a sub-agent LLM call; can intercept/replace or block | Yes |
| `post_spawn_agent` | fires after sub-agent returns; observe only | No |

### Tool Output Caching

| Hook | When | Can Modify |
|------|------|------------|
| `pre_cache_output` | fires before caching a large tool output; can provide custom summary | Yes |
| `post_cache_output` | fires after output is cached; observe only | No |

### Message Delivery

| Hook | When | Can Modify |
|------|------|------------|
| `pre_send_message` | fires before delivering an inter-context message; can claim delivery | Yes |
| `post_send_message` | fires after message delivery; observe only | No |

### Index Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `post_index_file` | fires after a file is indexed by the code indexer; observe only | No |

### VFS Write Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_vfs_write` | fires before a VFS file write via tool dispatch; advisory, non-blocking | No |
| `post_vfs_write` | fires after a successful VFS file write via tool dispatch; observe only | No |

### Context Lifecycle

| Hook | When | Can Modify |
|------|------|------------|
| `pre_clear` | fires before clearing a context; observe only | No |
| `post_clear` | fires after clearing a context; observe only | No |
| `pre_compact` | fires before full compaction; observe only | No |
| `post_compact` | fires after full compaction; observe only | No |
| `pre_rolling_compact` | fires before rolling compaction; observe only | No |
| `post_rolling_compact` | fires after rolling compaction; observe only | No |

## Hook Data by Type

### on_start

```json
{
  "chibi_home": "...",  // chibi home directory path
  "project_root": "...",  // project root directory path
  "tool_count": 0  // number of loaded tools
}
```

### on_end

Payload: (empty)

> **Note:** receives empty payload

### pre_message

```json
{
  "prompt": "...",  // the user's prompt
  "context_name": "...",  // active context name
  "summary": "..."  // conversation summary
}
```

**Can return:**
```json
{
  "prompt": "..."  // modified prompt
}
```

### post_message

```json
{
  "prompt": "...",  // original prompt
  "response": "...",  // LLM's response
  "context_name": "..."  // active context name
}
```

### pre_system_prompt

```json
{
  "context_name": "...",  // active context name
  "summary": "...",  // conversation summary
  "flock_goals": []  // array of {flock, goals} objects
}
```

**Can return:**
```json
{
  "inject": "..."  // content to add to system prompt
}
```

> **Note:** flock_goals replaced the old goals field; todos field removed (use VFS task files)

### post_system_prompt

```json
{
  "context_name": "...",  // active context name
  "summary": "...",  // conversation summary
  "flock_goals": []  // array of {flock, goals} objects
}
```

**Can return:**
```json
{
  "inject": "..."  // content to add to system prompt
}
```

> **Note:** same payload/return as pre_system_prompt

### pre_tool

```json
{
  "tool_name": "...",  // name of the tool being called
  "arguments": {}  // tool arguments object
}
```

**Can return:**
```json
{
  "arguments": {},  // modified arguments
  "block": false,  // set true to block execution
  "message": "..."  // message shown when blocked
}
```

### post_tool

```json
{
  "tool_name": "...",  // name of the tool that ran
  "arguments": {},  // tool arguments object
  "result": "...",  // tool output
  "cached": false  // true if output was cached due to size
}
```

### pre_tool_output

```json
{
  "tool_name": "...",  // name of the tool that ran
  "arguments": {},  // tool arguments object
  "output": "..."  // raw tool output
}
```

**Can return:**
```json
{
  "output": "...",  // modified output
  "block": false,  // set true to replace output entirely
  "message": "..."  // replacement message shown to LLM when blocked
}
```

### post_tool_output

```json
{
  "tool_name": "...",  // name of the tool that ran
  "arguments": {},  // tool arguments object
  "output": "...",  // original output after pre_tool_output modifications
  "final_output": "...",  // what the LLM will see (may be truncated if cached)
  "cached": false  // true if output was cached
}
```

### pre_api_tools

```json
{
  "context_name": "...",  // active context name
  "tools": [],  // array of {name, type} tool objects
  "fuel_remaining": 0,  // remaining tool-call budget
  "fuel_total": 0  // total fuel budget
}
```

**Can return:**
```json
{
  "exclude": [],  // tool names to remove (union across hooks)
  "include": []  // allowlist: only these tools remain (intersection across hooks)
}
```

> **Note:** include/exclude are mutually exclusive per response; excludes union, includes intersect across multiple hooks

### pre_api_request

```json
{
  "context_name": "...",  // active context name
  "request_body": {},  // full request body (model, messages, tools, etc.)
  "fuel_remaining": 0,  // remaining tool-call budget
  "fuel_total": 0  // total fuel budget
}
```

**Can return:**
```json
{
  "request_body": {}  // fields to merge into request body (partial override)
}
```

> **Note:** returned fields are merged, not replaced; cache_prompt and exclude_from_output are chibi-internal field names

### pre_agentic_loop

```json
{
  "context_name": "...",  // active context name
  "fuel_remaining": 0,  // remaining tool-call budget
  "fuel_total": 0,  // total fuel budget
  "current_fallback": "...",  // current fallback target (call_agent or call_user)
  "message": "..."  // user message for this loop
}
```

**Can return:**
```json
{
  "fallback": "...",  // override fallback: call_agent or call_user
  "fuel": 0  // set fuel_remaining to this value
}
```

### post_tool_batch

```json
{
  "context_name": "...",  // active context name
  "fuel_remaining": 0,  // remaining tool-call budget
  "fuel_total": 0,  // total fuel budget
  "current_fallback": "...",  // current fallback target
  "tool_calls": []  // array of {name, arguments} for tools that ran
}
```

**Can return:**
```json
{
  "fallback": "...",  // override fallback: call_agent or call_user
  "fuel_delta": 0  // adjust fuel by this amount (positive adds, negative consumes, saturating)
}
```

> **Note:** post_tool_batch output > pre_agentic_loop output > config fallback; last hook to set fallback wins

### pre_file_read

```json
{
  "tool_name": "...",  // file_head, file_tail, or file_lines
  "path": "..."  // absolute path being read
}
```

**Can return:**
```json
{
  "denied": false,  // set true to block the read
  "reason": "..."  // reason shown when denied
}
```

> **Note:** fail-safe deny if no handler; empty {} response falls through to frontend handler

### pre_file_write

```json
{
  "tool_name": "...",  // write_file or file_edit
  "path": "...",  // absolute path being written
  "content": "..."  // file content (null for file_edit)
}
```

**Can return:**
```json
{
  "denied": false,  // set true to block the write
  "reason": "..."  // reason shown when denied
}
```

> **Note:** fail-safe deny if no permission handler configured

### pre_shell_exec

```json
{
  "tool_name": "...",  // shell_exec
  "command": "..."  // shell command string
}
```

**Can return:**
```json
{
  "denied": false,  // set true to block execution
  "reason": "..."  // reason shown when denied
}
```

> **Note:** same deny-only protocol as pre_file_read and pre_file_write

### pre_fetch_url

```json
{
  "tool_name": "...",  // name of the tool making the network call
  "url": "...",  // URL being fetched (absent when safety is "no_url")
  "safety": "...",  // "sensitive" for URL-based calls, "no_url" for network tools without a URL parameter
  "reason": "...",  // classification reason (absent when safety is "no_url")
  "summary": "..."  // human-readable summary from summary_params (present only when safety is "no_url")
}
```

**Can return:**
```json
{
  "denied": false,  // set true to block the fetch
  "reason": "..."  // reason shown when denied
}
```

> **Note:** only fires when no url_policy is configured; url_policy is authoritative when set

### pre_spawn_agent

```json
{
  "system_prompt": "...",  // system prompt for sub-agent
  "input": "...",  // input content to process
  "model": "...",  // model identifier
  "temperature": 0,  // sampling temperature
  "max_tokens": 0  // max tokens for response
}
```

**Can return:**
```json
{
  "response": "...",  // pre-computed response to use instead of LLM call
  "block": false,  // set true to block the sub-agent call
  "message": "..."  // message shown when blocked
}
```

### post_spawn_agent

```json
{
  "system_prompt": "...",  // system prompt used
  "input": "...",  // input content
  "model": "...",  // model identifier
  "response": "..."  // sub-agent's response
}
```

### pre_cache_output

```json
{
  "tool_name": "...",  // tool whose output is being cached
  "arguments": {},  // tool arguments
  "content": "...",  // full output content
  "char_count": 0,  // character count of content
  "line_count": 0  // line count of content
}
```

**Can return:**
```json
{
  "summary": "..."  // custom summary to show LLM instead of full content
}
```

### post_cache_output

```json
{
  "tool_name": "...",  // tool whose output was cached
  "cache_id": "...",  // filename under vfs:///sys/tool_cache/<context>/
  "output_size": 0,  // size of cached output in bytes
  "preview_size": 0  // size of preview shown to LLM
}
```

> **Note:** access cached content with file_head/file_tail/file_lines using full vfs:// URI

### pre_send_message

```json
{
  "from": "...",  // sending context name
  "to": "...",  // recipient context name
  "content": "...",  // message content
  "context_name": "..."  // active context name
}
```

**Can return:**
```json
{
  "delivered": false,  // set true to claim delivery was handled
  "via": "..."  // delivery mechanism name (for logging)
}
```

### post_send_message

```json
{
  "from": "...",  // sending context name
  "to": "...",  // recipient context name
  "content": "...",  // message content
  "context_name": "...",  // active context name
  "delivery_result": "..."  // delivery outcome description
}
```

### post_index_file

```json
{
  "path": "...",  // relative path of indexed file
  "lang": "...",  // detected language
  "symbol_count": 0,  // number of symbols indexed
  "ref_count": 0  // number of references indexed
}
```

### pre_vfs_write

```json
{
  "tool_name": "...",  // write_file or file_edit
  "path": "...",  // VFS path being written
  "content": "...",  // new content (null for file_edit)
  "caller": "..."  // context initiating the write
}
```

> **Note:** only fires for context-initiated writes via send.rs; VfsCaller::System and (harness io) bypass this hook

### post_vfs_write

```json
{
  "tool_name": "...",  // write_file or file_edit
  "path": "...",  // VFS path that was written
  "caller": "..."  // context that initiated the write
}
```

> **Note:** same caller restriction as pre_vfs_write

### pre_clear

```json
{
  "context_name": "...",  // context being cleared
  "message_count": 0,  // number of messages before clear
  "summary": "..."  // existing conversation summary
}
```

### post_clear

```json
{
  "context_name": "...",  // context that was cleared
  "message_count": 0,  // message count before clear
  "summary": "..."  // summary before clear
}
```

### pre_compact

```json
{
  "context_name": "...",  // context being compacted
  "message_count": 0,  // number of messages before compact
  "summary": "..."  // conversation summary
}
```

### post_compact

```json
{
  "context_name": "...",  // context that was compacted
  "message_count": 0,  // message count before compact
  "summary": "..."  // conversation summary
}
```

### pre_rolling_compact

```json
{
  "context_name": "...",  // context being compacted
  "message_count": 0,  // total message count
  "non_system_count": 0,  // non-system message count
  "summary": "..."  // conversation summary
}
```

### post_rolling_compact

```json
{
  "context_name": "...",  // context that was compacted
  "message_count": 0,  // message count after archiving
  "messages_archived": 0,  // number of messages archived
  "summary": "..."  // updated summary
}
```

<!-- END GENERATED HOOK REFERENCE -->

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

## Tein Hook Registration

Synthesised tools (`.scm` files) can register for hooks using the `(harness hooks)` module:

```scheme
(import (harness hooks))

(register-hook 'pre_message
  (lambda (payload)
    ;; payload is an alist parsed from the hook's JSON data.
    ;; return an alist to modify behaviour, or '() for no-op.
    (list (cons "prompt" "modified prompt"))))

(define tool-name "my-tool")
(define tool-description "A tool that also hooks into pre_message")
(define tool-parameters '())
(define (tool-execute args) "ok")
```

Tein hooks follow the same contract as subprocess plugin hooks:
- They receive the hook payload converted from JSON to a scheme alist.
- They return a scheme alist (converted back to JSON), or `'()` (empty list) for no-op.
- Errors in callbacks are caught and skipped silently (same as subprocess hook failures).
- `register-hook` takes a symbol for the hook point name and a one-argument procedure.

**Ordering:** subprocess plugin hooks fire first, then tein hooks, in registration order.

**Re-entrancy:** If a tein hook callback triggers an action that fires the same hook point,
tein callbacks are skipped on the recursive call to prevent infinite loops. Subprocess
hooks still fire normally.

**IO in hook callbacks:** Tein hook callbacks can use `(harness io)` (unsandboxed tier only)
for direct VFS and filesystem IO without triggering hooks. This is the recommended way for
builtin plugins to perform IO during hook execution.

Using `call-tool` from hooks is also possible (when the hook is dispatched from a full async
context) but may trigger hooks on the called tool — use with care to avoid re-entrancy.

**Lifecycle:** Hook registrations are tied to the `.scm` file. When a file is hot-reloaded
or deleted, its hooks are automatically cleared and re-evaluated from the fresh source.

## Hook Execution

When a hook fires, registered plugins are called with:

- `CHIBI_HOOK` env var - Hook point name (e.g., "pre_message")
- stdin - JSON data about the event
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
  data=$(cat)  # Read JSON from stdin
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $CHIBI_HOOK" >> ~/.chibi/hook.log
  echo "$data" | jq '.' >> ~/.chibi/hook.log
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
    data = json.load(sys.stdin)
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
  data=$(cat)  # Read JSON from stdin
  tool_name=$(echo "$data" | jq -r '.tool_name')

  # Block shell_exec for certain patterns
  if [[ "$tool_name" == "shell_exec" ]]; then
    command=$(echo "$data" | jq -r '.arguments.command // ""')
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
  data=$(cat)  # Read JSON from stdin
  context=$(echo "$data" | jq -r '.context_name')

  # Restrict tools in "safe" context
  if [[ "$context" == "safe" ]]; then
    echo '{"include": ["update_goals", "update_reflection"]}'
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
    data = json.load(sys.stdin)
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

## Example: Guardrails (Fallback Override)

A hook that forces user confirmation after dangerous tool calls:

```python
#!/usr/bin/env python3
# ~/.chibi/plugins/guardrails

import sys
import json
import os

if len(sys.argv) > 1 and sys.argv[1] == "--schema":
    print(json.dumps({
        "name": "guardrails",
        "description": "Forces user confirmation for dangerous operations",
        "parameters": {"type": "object", "properties": {}},
        "hooks": ["post_tool_batch"]
    }))
    sys.exit(0)

hook = os.environ.get("CHIBI_HOOK", "")
if hook == "post_tool_batch":
    data = json.load(sys.stdin)
    tool_calls = data.get("tool_calls", [])

    # List of tools that should require user confirmation
    dangerous_tools = ["shell_exec", "write_file", "delete_file"]

    for call in tool_calls:
        if call.get("name") in dangerous_tools:
            # Force return to user after dangerous tool calls
            # Also penalize fuel to discourage repeated dangerous operations
            print(json.dumps({"fallback": "call_user", "fuel_delta": -5}))
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
- **Guardrails** - Override fallback behavior to force user confirmation after risky operations
