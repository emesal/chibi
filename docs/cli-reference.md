# CLI Reference

Chibi uses a lowercase/UPPERCASE pattern: lowercase operates on current context, UPPERCASE operates on a specified context.

## Context Operations

| Flag | Description |
|------|-------------|
| `-c, --switch-context <NAME>` | Switch to a context (persistent); `new` for auto-name, `new:prefix` for prefixed |
| `-C, --transient-context <NAME>` | Use context for this invocation only (doesn't change global state) |
| `-l, --list-current-context` | Show current context info (name, message count, todos, goals) |
| `-L, --list-contexts` | List all contexts (shows `[active]` or `[stale]` lock status) |
| `-d, --delete-current-context` | Delete the current context |
| `-D, --delete-context <CTX>` | Delete a specified context |
| `-a, --archive-current-history` | Archive current context history (saves to transcript) |
| `-A, --archive-history <CTX>` | Archive specified context's history |
| `-z, --compact-current-context` | Compact current context (LLM summarizes) |
| `-Z, --compact-context <CTX>` | Compact specified context (simple archive) |
| `-r, --rename-current-context <NEW>` | Rename current context |
| `-R, --rename-context <OLD> <NEW>` | Rename specified context |

## Inspection & History

| Flag | Description |
|------|-------------|
| `-g, --show-current-log <N>` | Show last N log entries from current context (negative = from start) |
| `-G, --show-log <CTX> <N>` | Show last N log entries from specified context |
| `-n, --inspect-current <THING>` | Inspect current context: `system_prompt`, `reflection`, `todos`, `goals`, `list` |
| `-N, --inspect <CTX> <THING>` | Inspect specified context |

## System Prompt

| Flag | Description |
|------|-------------|
| `-y, --set-current-system-prompt <PROMPT>` | Set system prompt for current context (text or file path) |
| `-Y, --set-system-prompt <CTX> <PROMPT>` | Set system prompt for specified context |

## Username

| Flag | Description |
|------|-------------|
| `-u, --set-username <NAME>` | Set username (persists to context's local.toml) |
| `-U, --transient-username <NAME>` | Set username for this invocation only |

## Plugins & Tools

| Flag | Description |
|------|-------------|
| `-p, --plugin <NAME> [ARGS...]` | Run a plugin directly (bypasses LLM) |
| `-P, --call-tool <TOOL> [ARGS...]` | Call a tool directly (plugin or built-in) |

## Cache Management

| Flag | Description |
|------|-------------|
| `--clear-cache` | Clear the tool output cache for current context |
| `--clear-cache-for <CTX>` | Clear the tool output cache for specified context |
| `--cleanup-cache` | Remove old cache entries across all contexts |

## Control Flags

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Show extra info (tools loaded, warnings, etc.) |
| `-x, --no-chibi` | Don't invoke the LLM |
| `-X, --force-chibi` | Force LLM invocation (overrides implied -x) |
| `-h, --help` | Show help message |
| `--version` | Show version |

## JSON Modes

| Flag | Description |
|------|-------------|
| `--json-config` | Read input as JSON from stdin (for programmatic use) |
| `--json-output` | Output in JSONL format (structured output) |

### JSON Input Format (--json-config)

When using `--json-config`, pass a JSON object to stdin:

```json
{
  "command": { "send_prompt": { "prompt": "Hello" } },
  "context": { "switch": { "name": "coding" } },
  "flags": { "verbose": true }
}
```

**Simple commands:** `"list_contexts"`, `"list_current_context"`, `"no_op"`

**Commands with arguments:**
- `{ "send_prompt": { "prompt": "..." } }`
- `{ "delete_context": { "name": "..." } }` (name optional, null = current)
- `{ "archive_history": { "name": "..." } }`
- `{ "compact_context": { "name": "..." } }`
- `{ "rename_context": { "old": "...", "new": "..." } }`
- `{ "show_log": { "context": "...", "count": 10 } }`
- `{ "inspect": { "context": "...", "thing": "todos" } }`
- `{ "set_system_prompt": { "context": "...", "prompt": "..." } }`
- `{ "run_plugin": { "name": "...", "args": [...] } }`
- `{ "call_tool": { "name": "...", "args": [...] } }`

**Context selection:** `"current"`, `{ "switch": { "name": "..." } }`, `{ "transient": { "name": "..." } }`

**Username:** `{ "persistent": "name" }`, `{ "transient": "name" }`

## Debug Flags

| Flag | Description |
|------|-------------|
| `--debug <KEY>` | Enable debug logging: `request-log`, `response-meta`, `all` |

Debug output is written to files in the context directory:
- `requests.jsonl` - Full API request bodies (with `request-log` or `all`)
- `response_meta.jsonl` - Response metadata, usage stats, model info (with `response-meta` or `all`)

## Flag Behavior

### Implied --no-chibi

These flags produce output or operate on other contexts, so they imply `--no-chibi`:

`-l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -p, -P`

### Combinable with Prompt

These flags can be combined with a prompt (execute operation, then invoke LLM):

`-c, -C, -a, -z, -r, -y, -u, -U, -v`

### Force Override

Use `-X/--force-chibi` to override implied `--no-chibi`:

```bash
# Normally -L implies -x (no LLM)
chibi -L

# Force LLM invocation after listing
chibi -L -X "Now help me with something"
```

## Prompt Input

### Command Line

```bash
# Single argument (can contain spaces)
chibi What is Rust?

# Multiple words become one prompt
chibi Tell me about ownership in Rust

# Use -- to force prompt interpretation
chibi -- -v is not a flag here
```

### Interactive

```bash
# Start interactive mode (end with . on empty line)
chibi
Enter your prompt:
- Line 1
- Line 2
.
```

### Piped

```bash
# Pipe content as prompt
echo "What is 2+2?" | chibi

# Combine piped input with argument
cat file.txt | chibi "Summarize this"
```

## Output Philosophy

- **stdout**: Only LLM responses (clean, pipeable)
- **stderr**: Diagnostics (only with `-v`)

```bash
# Pipe to another command
chibi "Generate JSON" | jq .

# Save to file
chibi "Write a poem" > poem.txt

# Use in scripts
result=$(chibi "What is 2+2? Just the number.")
```

## Examples

### Basic Usage

```bash
chibi What are the benefits of using Rust?
```

### Context Management

```bash
chibi -c rust-learning          # Switch context
chibi -c new                    # Auto-named context
chibi -c new:bugfix             # Prefixed auto-name
chibi -L                        # List all contexts
chibi -l                        # Current context info
```

### Custom Prompts

```bash
chibi -c coding
chibi -y "You are a senior engineer."
chibi -n system_prompt          # View it
```

### Tool Debugging

```bash
chibi -v "Read my Cargo.toml"
# stderr: [Loaded 1 tool(s): read_file]
# stderr: [Tool: read_file]
```

### Scripting

```bash
cat error.log | chibi "explain this"
git diff | chibi "review these changes"
chibi "List 5 numbers as JSON" | jq '.[0]'
```

### Sub-Agents

```bash
# Run in another context without switching
chibi -C research "Find info about X"

# With custom system prompt
chibi -C coding -y "You are a reviewer" "Review this code"
```
