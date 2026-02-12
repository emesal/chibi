# CLI Reference

Chibi uses a lowercase/UPPERCASE pattern: lowercase operates on current context, UPPERCASE operates on a specified context.

## Context Operations

| Flag | Description |
|------|-------------|
| `-c, --switch-context <NAME>` | Switch to a context (persistent); `new` for auto-name, `new:prefix` for prefixed, `-` for previous |
| `-C, --ephemeral-context <NAME>` | Use context for this invocation only (doesn't change global state) |
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

### Previous Context Reference

The special context name `-` can be used to reference the previous context in any command that accepts a context name (`-c`, `-C`, `-D`, `-A`, `-Z`, `-R`, `-G`, `-N`, `-Y`, `--clear-cache-for`). The previous context is tracked in `session.json` and updated whenever you use `-c` to switch contexts.

**Examples:**
```bash
chibi -c dev          # Switch to 'dev', previous='default'
chibi -c production   # Switch to 'production', previous='dev'
chibi -c -            # Switch to 'dev', previous='production' (swaps!)
chibi -c -            # Switch to 'production', previous='dev' (swaps back!)
```

**Swap Behavior (like `cd -`):**
When using `-c -` to switch contexts, the current and previous contexts swap places, just like the `cd -` command in bash. This allows you to toggle back and forth between two contexts repeatedly:
```bash
chibi -c work         # current='work', previous='personal'
chibi -c -            # current='personal', previous='work'
chibi -c -            # current='work', previous='personal'
```

**Notes:**
- `-` is a reserved name and cannot be used as a literal context name
- If no previous context exists (first invocation), you'll get an error: "No previous context available"
- Only persistent switches (`-c`) update `previous_context` in session.json and use swap behavior
- Ephemeral switches (`-C -`) resolve to previous context but don't swap or persist changes
- Works with all context name parameters: `-D -` deletes previous context, `-G - 10` shows previous context's log, etc.
- Attached flag syntax works: both `-xc-` and `-xc -` are valid

## Inspection & History

| Flag | Description |
|------|-------------|
| `-g, --show-current-log <N>` | Show last N log entries from current context (negative = from start) |
| `-G, --show-log <CTX> <N>` | Show last N log entries from specified context |
| `-n, --inspect-current <THING>` | Inspect: `system_prompt`, `reflection`, `todos`, `goals`, `home`, `list`, or config fields |
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
| `-U, --ephemeral-username <NAME>` | Set username for this invocation only |

## Plugins & Tools

| Flag | Description |
|------|-------------|
| `-p, --plugin <NAME> "ARGS"` | Run a plugin directly with shell-style args |
| `-P, --call-tool <TOOL> JSON` | Call a tool directly with JSON arguments |

## Cache Management

| Flag | Description |
|------|-------------|
| `--clear-cache` | Clear the tool output cache for current context |
| `--clear-cache-for <CTX>` | Clear the tool output cache for specified context |
| `--cleanup-cache` | Remove old cache entries across all contexts |

## Model Metadata

| Flag | Description |
|------|-------------|
| `-m, --model-metadata <MODEL>` | Show model metadata in TOML format (settable fields only) |
| `-M, --model-metadata-full <MODEL>` | Show full model metadata (with pricing, capabilities, parameter ranges) |

Model metadata is fetched via ratatoskr's gateway (embedded registry → cache → OpenRouter API on miss). The `-m` flag shows only fields you can set in `models.toml`, while `-M` includes everything (pricing, capabilities, parameter ranges).

```bash
chibi -m anthropic/claude-sonnet-4       # Settable fields only
chibi -M openai/gpt-4o                   # Full metadata including pricing
```

## Control Flags

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Show extra info (tools loaded, warnings, etc.) |
| `--hide-tool-calls` | Hide tool call display (tool calls are shown by default; verbose overrides) |
| `--no-tool-calls` | Omit tools from API requests entirely (pure text mode) |
| `-x, --no-chibi` | Don't invoke the LLM |
| `-X, --force-chibi` | Force LLM invocation (overrides implied -x) |
| `--raw` | Disable markdown rendering (plain text output) |
| `-h, --help` | Show help message |
| `--version` | Show version |

## JSON Modes

| Flag | Description |
|------|-------------|
| `--json-config` | Read input as JSON from stdin (for programmatic use) |
| `--json-output` | Output in JSONL format (structured output) |
| `--json-schema` | Print the JSON schema for `--json-config` input and exit |

### JSON Input Format (--json-config)

When using `--json-config`, pass a JSON object to stdin:

```json
{
  "command": { "send_prompt": { "prompt": "Hello" } },
  "context": { "switch": { "name": "coding" } },
  "flags": { "verbose": true }
}
```

**Flags:** `"verbose"`, `"json_output"`, `"force_call_user"`, `"force_call_agent"`, `"hide_tool_calls"`, `"no_tool_calls"`, `"raw"`

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

**Context selection:** `"current"`, `{ "switch": { "name": "..." } }`, `{ "ephemeral": { "name": "..." } }`

**Username:** `{ "persistent": "name" }`, `{ "ephemeral": "name" }`

**Home directory:** Use `--home` alongside `--json-config` (cannot be set in JSON):
```bash
echo '{"command": "list_contexts"}' | chibi --home /path/to/alt --json-config
```

### JSON Schema (--json-schema)

Print the full JSON Schema describing the input format accepted by `--json-config`, then exit. This is useful for editor integration, validation, and code generation. All other flags are ignored.

```bash
chibi --json-schema              # Print schema to stdout
chibi --json-schema > schema.json  # Save to file
chibi --json-schema | jq .definitions.Command  # Inspect a specific type
```

## Directory Override

| Flag | Description |
|------|-------------|
| `--home <PATH>` | Override chibi home directory (default: `~/.chibi`) |

The home directory is resolved in this order:
1. `--home` CLI flag (highest priority)
2. `CHIBI_HOME` environment variable
3. `~/.chibi` default

Use `-n home` to inspect the resolved path.

## Debug Flags

| Flag | Description |
|------|-------------|
| `--debug <KEY[,KEY,...]>` | Enable debug features (comma-separated, see below) |

### Debug Keys

| Key | Description |
|-----|-------------|
| `request-log` | Log full API request bodies to `requests.jsonl` |
| `response-meta` | Log response metadata/usage stats to `response_meta.jsonl` |
| `all` | Enable all logging features above |
| `md=<FILENAME>` | Render a markdown file and quit (implies `-x`, forces rendering even without TTY) |
| `force-markdown` | Force markdown rendering even when stdout is not a TTY |
| `destroy_at=<TIMESTAMP>` | Set auto-destroy timestamp on current context |
| `destroy_after_seconds_inactive=<SECS>` | Set inactivity timeout on current context |

### Debug Logging

Debug output is written to files in the context directory:
- `requests.jsonl` - Full API request bodies (with `request-log` or `all`)
- `response_meta.jsonl` - Response metadata, usage stats, model info (with `response-meta` or `all`)

### Markdown Rendering

You can render a markdown file without invoking the LLM:

```bash
# Render a markdown file
chibi --debug md=README.md

# Works with any markdown file
chibi --debug md=docs/guide.md
```

This is useful for:
- Previewing markdown files with terminal rendering
- Testing markdown rendering without starting a conversation
- Quick markdown file viewing

The `md=<FILENAME>` feature automatically:
- Implies `-x` (no LLM invocation) and exits after rendering
- Forces markdown rendering even when stdout is not a TTY (e.g., when piped or in CI)

#### Force Markdown Rendering

By default, markdown rendering only activates when stdout is a TTY (terminal). To force rendering even when piped or redirected:

```bash
# Force markdown rendering in a normal conversation
chibi --debug force-markdown "Tell me about Rust"

# Useful for piping formatted output
chibi --debug force-markdown "List the files" | less -R
```

Note: `--debug md=<file>` automatically forces rendering, so you don't need to combine them.

### Combining Debug Keys

Multiple debug keys can be combined with commas:

```bash
# Enable request logging and force markdown rendering
chibi --debug request-log,force-markdown "Tell me about Rust"

# Log requests and response metadata
chibi --debug request-log,response-meta "Hello"
```

### Auto-Destroy (Test Cleanup)

Contexts can be marked for automatic destruction, primarily for test cleanup:

```bash
# Destroy context after 60 seconds of inactivity
chibi --debug destroy_after_seconds_inactive=60 -c test-ctx

# Destroy context at a specific timestamp
chibi --debug destroy_at=1234567890 -c test-ctx
```

Auto-destroy runs automatically at every chibi invocation. It checks all non-current contexts and destroys those that meet the criteria. This prevents test contexts from accumulating.

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
