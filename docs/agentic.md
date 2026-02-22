# Agentic Workflows

Chibi includes built-in tools and features that enable autonomous, multi-step workflows.

## Built-in Tools

The LLM always has access to these tools (no setup required):

### Core

| Tool | Description |
|------|-------------|
| `call_user` | End the turn and return control to the user |
| `call_agent` | Continue the agentic loop with a new prompt |
| `update_todos` | Track tasks for the current conversation |
| `update_goals` | Set high-level objectives |
| `update_reflection` | Update persistent memory (when reflection is enabled) |
| `send_message` | Send messages to other contexts |
| `read_context` | Read another context's state (summary, todos, goals, messages) |
| `model_info` | Look up model metadata (context window, pricing, capabilities, parameters) |

### File

| Tool | Description |
|------|-------------|
| `file_head` | Read first N lines from a file or cached output (accepts `vfs:///` URIs) |
| `file_tail` | Read last N lines from a file or cached output (accepts `vfs:///` URIs) |
| `file_lines` | Read a specific line range from a file or cached output (accepts `vfs:///` URIs) |
| `file_grep` | Search for a pattern in a file or cached output (accepts `vfs:///` URIs) |
| `write_file` | Write content to a file (requires `file_tools_allowed_paths`, gated by `pre_file_write` hook) |

### Coding

| Tool | Description |
|------|-------------|
| `shell_exec` | Execute a shell command; returns stdout, stderr, exit code, and timeout status |
| `dir_list` | List a directory tree with file sizes; respects depth limit |
| `glob_files` | Find files matching a glob pattern, honouring `.gitignore` |
| `grep_files` | Search files for a regex pattern, honouring `.gitignore` |
| `file_edit` | Structured file editing (insert, replace, delete ranges) |
| `fetch_url` | HTTP GET request returning the response body |
| `index_update` | Index the codebase for symbol search |
| `index_query` | Search the index by symbol name or pattern |
| `index_status` | Show index metadata (file count, last updated) |

### Agent

| Tool | Description |
|------|-------------|
| `spawn_agent` | Spawn a sub-agent with a custom system prompt to process input |
| `summarize_content` | Read a file or URL and process its content through a sub-agent |

### VFS

| Tool | Description |
|------|-------------|
| `vfs_list` | List a VFS directory |
| `vfs_info` | Get VFS entry metadata |
| `vfs_copy` | Copy a file within VFS |
| `vfs_move` | Move or rename a VFS entry |
| `vfs_mkdir` | Create a VFS directory |
| `vfs_delete` | Delete a VFS entry |

### Tool Filtering

All tool categories are included by default. Use the `[tools]` config section to restrict them:

```toml
[tools]
# Allowlist (only these tools are sent to the LLM)
include = ["call_agent", "shell_exec", "file_head"]

# Blocklist (remove specific tools)
exclude = ["shell_exec"]

# Remove entire categories: "builtin", "file", "coding", "agent", "vfs", "mcp", "plugin"
exclude_categories = ["vfs", "coding"]
```

Plugins can also filter tools dynamically via the `pre_api_tools` hook — see [hooks.md](hooks.md#pre_api_tools).

## External Plugins

The [chibi-plugins](https://github.com/emesal/chibi-plugins) repository provides ready-to-install plugins:

| Plugin | Description |
|--------|-------------|
| `agent-skills` | Agent Skills marketplace — install and invoke skills from `SKILL.md` |
| `bofh_in_the_shell` | No comment |
| `coffee-table` | Shared inter-context communication space |
| `file-permission` | Prompts for user confirmation on file writes (hook) |
| `hello_chibi` | XMPP bridge via mcabber — send/receive XMPP messages |
| `hook-inspector` | Debug hook — logs all hook events to file |
| `web_search` | Web search via DuckDuckGo |

See [plugins.md](plugins.md) for installation and authoring details.

## MCP Tools

Chibi integrates with any [MCP](https://modelcontextprotocol.io/)-compatible server. MCP tools are
discovered automatically and presented to the LLM alongside built-in tools and plugins — the LLM
uses them the same way as any other tool, with no special handling required.

A standalone daemon (`chibi-mcp-bridge`) manages MCP server lifecycles and proxies tool calls over
TCP. Chibi starts it automatically when MCP servers are configured.

See [mcp.md](mcp.md) for setup and configuration.

## Todos and Goals

Each context can have its own todos and goals stored in markdown files:

- **Todos** (`~/.chibi/contexts/<name>/todos.md`) - Short-term tasks
- **Goals** (`~/.chibi/contexts/<name>/goals.md`) - Long-term objectives

These are automatically included in the system prompt, so the LLM always knows what it's working toward.

### Viewing

```bash
chibi -n todos    # View current todos
chibi -n goals    # View current goals
```

### How It Works

The LLM can call `update_todos` or `update_goals` with new markdown content. The content completely replaces the existing file (it's not appended).

Example LLM behavior:
```
LLM: "Let me update my task list."
     [calls update_todos with content: "- [x] Read the config file\n- [ ] Analyse the structure\n- [ ] Write report"]
```

## Reflection (Persistent Memory)

Reflection gives the LLM persistent memory across all contexts and sessions.

### How It Works

- Stored in `~/.chibi/prompts/reflection.md`
- Automatically appended to the system prompt
- LLM uses `update_reflection` tool to modify it
- Has a configurable character limit

### Configuration

In `config.toml`:

```toml
reflection_enabled = true
reflection_character_limit = 10000
```

### Use Cases

- Remember user preferences
- Store important facts
- Keep notes for future conversations
- Build up knowledge over time

### Viewing

```bash
chibi -n reflection
```

## Inter-Context Communication

### send_message Tool

The built-in `send_message` tool lets contexts communicate:

```json
{
  "to": "research",
  "content": "Please look up quantum computing basics",
  "from": "main"
}
```

The `from` field defaults to the current context name if not specified.

Messages are delivered to the recipient's inbox and injected into their next prompt.

### Inbox

Each context has an inbox (`~/.chibi/contexts/<name>/inbox.jsonl`). When a context receives a message:

1. Message is stored in inbox
2. Next time that context is used, inbox messages are injected into the prompt
3. Inbox is cleared after injection

### Checking Inboxes

Chibi can check context inboxes and automatically process any pending messages:

- `-b` / `--check-all-inboxes`: Check all context inboxes. For each context with pending messages, the LLM is activated to process them.
- `-B <context>` / `--check-inbox-for <context>`: Check only the specified context's inbox.

When messages are found, the LLM receives the inbox messages followed by a system prompt instructing it to take appropriate action. Contexts with empty inboxes are silently skipped.

This is useful for scheduled tasks (e.g., cron jobs) that periodically wake up contexts to handle inter-context communication:

```bash
# Check all inboxes every hour
0 * * * * chibi -b

# Check specific context inbox
chibi -B work-assistant
```

These commands work with `chibi-json`:

```bash
echo '{"command": "check_all_inboxes"}' | chibi-json
echo '{"command": {"check_inbox": {"context": "work"}}}' | chibi-json
```

### Sending Messages from External Programs

External programs can deliver messages to any context's inbox using the `-P` flag to call the `send_message` tool directly:

```bash
# Send a message to a context
chibi -P send_message '{"to": "work-assistant", "content": "New task: review PR #123"}'

# Optionally specify a sender
chibi -P send_message '{"to": "work-assistant", "content": "Build failed", "from": "ci-bot"}'
```

This also works with `chibi-json` for programmatic use:

```bash
echo '{"command": {"call_tool": {"name": "send_message", "args": ["{\"to\": \"work-assistant\", \"content\": \"Hello!\"}"]}}}' | chibi-json
```

**Example: CI/CD integration**

```bash
#!/bin/bash
# Notify chibi context when build completes
chibi -P send_message "{\"to\": \"dev-assistant\", \"content\": \"Build $BUILD_ID completed with status: $STATUS\", \"from\": \"jenkins\"}"

# Optionally trigger immediate processing
chibi -B dev-assistant
```

### Hooks for Message Routing

The `pre_send_message` hook can intercept delivery for custom routing:

```json
{
  "delivered": true,
  "via": "slack-bridge"
}
```

See [hooks.md](hooks.md#pre_send_message) for details.

## Sub-Agents

### Built-in Agent Tools

Chibi provides two built-in tools for spawning sub-agents — separate LLM calls with their own system prompts that return results as tool output.

**`spawn_agent`** — General-purpose sub-agent spawning:
```json
{
  "system_prompt": "You are a code reviewer. Be concise.",
  "input": "Review this function:\ndef add(a, b): return a + b",
  "model": "anthropic/claude-haiku",
  "temperature": 0.3
}
```

**`summarize_content`** — Read a file or fetch a URL, then process through a sub-agent:
```json
{
  "source": "https://example.com/api-docs",
  "instructions": "Summarise the authentication section"
}
```

Both tools accept optional `model`, `temperature`, and `max_tokens` overrides. Without overrides, the parent's model and settings are used.

### Model Presets

Instead of specifying a model directly, `spawn_agent` accepts a `preset` parameter — a capability name like `"fast"` or `"reasoning"`. The actual model and default parameters are resolved from your ratatoskr preset configuration using the `subagent_cost_tier` set in `config.toml` (default: `"free"`).

Available capability names are listed in the `preset` parameter description when the tool is active — the LLM sees the valid options at runtime. Explicit `model`, `temperature`, and `max_tokens` arguments always override preset defaults.

```json
{
  "system_prompt": "You are a fast summariser.",
  "input": "Summarise the following document...",
  "preset": "fast"
}
```

To change the cost tier for sub-agents, set `subagent_cost_tier` in `config.toml` or `local.toml`:

```toml
subagent_cost_tier = "standard"
```

Sub-agent calls are non-streaming (results returned as tool output). Plugins can intercept or replace sub-agent calls via `pre_spawn_agent` / `post_spawn_agent` hooks — see [hooks.md](hooks.md#pre_spawn_agent).

### Ephemeral Context Flag

Use `-C` to spawn agents without affecting global context state:

```bash
# Run a task in another context
chibi -C research "Find information about quantum computing"

# Set system prompt and send task
chibi -C coding -y "You are a code reviewer" "Review this function"
```

### read_context Tool

Allows reading another context's state without switching:

```json
{
  "context_name": "research",
  "include_messages": "true",
  "num_messages": 5
}
```

## Rolling Compaction

When auto-compaction is enabled and context size exceeds the threshold, rolling compaction kicks in:

### Process

1. LLM analyses all messages
2. Decides which to archive based on:
   - Current goals and todos (keeps relevant messages)
   - Message recency (prefers keeping recent context)
   - Content importance (preserves key decisions)
3. Selected messages are archived and summarised
4. Summary is integrated with existing conversation summary

### Fallback

If LLM decision fails, falls back to archiving the oldest N% of messages (configured by `rolling_compact_drop_percentage`).

### Configuration

```toml
auto_compact = true
auto_compact_threshold = 80.0
rolling_compact_drop_percentage = 50.0
```

### Manual Compaction

```bash
chibi -z  # Compact current context with LLM summary
chibi -Z other-context  # Simple archive without LLM
```

## Example Workflow

A complex autonomous workflow might look like:

```
User: "Research quantum computing and write a summary report"

Round 1:
LLM: Sets goals: "Research quantum computing, write summary report"
     Sets todos: "- [ ] Search for introductory materials"
     [calls spawn_agent: system_prompt="You are a researcher", input="Find quantum computing basics"]
     [calls call_agent: "Check research results and continue"]

Round 2:
LLM: [calls read_context: "research"]
     Updates todos: "- [x] Search for materials\n- [ ] Synthesise findings"
     [calls call_agent: "Write the summary"]

Round 3:
LLM: Writes summary report
     Updates todos: "- [x] Search\n- [x] Synthesise\n- [x] Write report"
     Clears goals
     Returns final response to user
```

## Tool Output Caching

When tool outputs exceed the configured threshold (default: 4000 chars), they're automatically cached to VFS and a truncated preview is sent to the LLM.

### How It Works

1. Tool produces large output (e.g., `fetch_url` returns a large webpage)
2. Output is written to `vfs:///sys/tool_cache/<context>/<id>` by SYSTEM
3. LLM receives a truncated message with:
   - `vfs:///` URI for later reference
   - Size and line count statistics
   - Preview of first ~500 chars
   - Instructions to use file tools with the URI

### Examining Cached Content

The LLM uses built-in file tools with the `vfs:///` URI:

```
[Output cached: vfs:///sys/tool_cache/default/fetch_url_abc123_def456]
Tool: fetch_url | Size: 50000 chars, ~12500 tokens | Lines: 1200
Preview:
---
<!DOCTYPE html>
<html>
<head>
  <title>Example Page</title>
...
---
Use file_head, file_tail, file_lines, file_grep with path="vfs:///sys/tool_cache/..." to examine.
```

The LLM can then:
- `file_head(path="vfs:///sys/tool_cache/default/fetch_url_abc123_def456", lines=100)` - See first 100 lines
- `file_grep(path="vfs:///sys/tool_cache/default/fetch_url_abc123_def456", pattern="class.*Button")` - Search for patterns
- `file_lines(path="vfs:///sys/tool_cache/default/fetch_url_abc123_def456", start=500, end=550)` - Read specific section

### Configuration

```toml
# Threshold above which outputs are cached (chars)
tool_output_cache_threshold = 4000

# Max age for cached entries before cleanup (days)
tool_cache_max_age_days = 7

# Preview size in truncated message (chars)
tool_cache_preview_chars = 500

# Allow file tools to access files outside cache
# file_tools_allowed_paths = ["~", "/tmp"]
```

### Cache Management

```bash
chibi --clear-cache           # Clear current context's cache
chibi --clear-cache-for other # Clear specific context's cache
chibi --cleanup-cache         # Remove old entries across all contexts
```

## Best Practices

1. **Clear Goals** - Help the LLM stay focused by encouraging goal-setting
2. **Incremental Todos** - Breaking work into small tasks helps track progress
3. **Reasonable Recursion Limits** - Balance autonomy vs. runaway loops
4. **Use Reflection Wisely** - Store genuinely useful long-term knowledge
5. **Monitor with Verbose Mode** - Use `-v` to see what the agent is doing
6. **Leverage Caching** - Large outputs are automatically cached for surgical access
