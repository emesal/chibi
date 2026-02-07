# Agentic Workflows

Chibi includes built-in tools and features that enable autonomous, multi-step workflows.

## Built-in Tools

The LLM always has access to these tools (no setup required):

| Tool | Description |
|------|-------------|
| `call_user` | Hand control back to the user (ends the agentic loop) |
| `call_agent` | Continue the agentic loop with a new prompt |
| `update_todos` | Track tasks for the current conversation |
| `update_goals` | Set high-level objectives |
| `update_reflection` | Update persistent memory (when reflection is enabled) |
| `send_message` | Send messages to other contexts |
| `file_head` | Read first N lines from a cached output or file |
| `file_tail` | Read last N lines from a cached output or file |
| `file_lines` | Read a specific line range from a cached output or file |
| `file_grep` | Search for a pattern in a cached output or file |
| `cache_list` | List all cached tool outputs for the current context |
| `write_file` | Write content to a file (requires `file_tools_allowed_paths`, gated by `pre_file_write` hook) |
| `patch_file` | Find-and-replace in a file (requires `file_tools_allowed_paths`, gated by `pre_file_write` hook) |
| `spawn_agent` | Spawn a sub-agent with a custom system prompt to process input |
| `retrieve_content` | Read a file/URL and process content through a sub-agent |

## External Plugins

These plugins are available in [chibi-plugins](https://github.com/emesal/chibi-plugins) and must be installed separately:

| Plugin | Description |
|--------|-------------|
| `read_context` | Read another context's state (read-only) |
| `sub-agent` | Spawn sub-agents in another context |

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
     [calls update_todos with content: "- [x] Read the config file\n- [ ] Analyze the structure\n- [ ] Write report"]
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
  "from": "main"  // optional, defaults to current context
}
```

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

These commands work with `--json-config`:

```json
{"command": "check_all_inboxes"}
{"command": {"check_inbox": {"context": "work"}}}
```

### Sending Messages from External Programs

External programs can deliver messages to any context's inbox using the `-P` flag to call the `send_message` tool directly:

```bash
# Send a message to a context
chibi -P send_message '{"to": "work-assistant", "content": "New task: review PR #123"}'

# Optionally specify a sender
chibi -P send_message '{"to": "work-assistant", "content": "Build failed", "from": "ci-bot"}'
```

This also works with `--json-config` for programmatic use:

```bash
echo '{"command": {"call_tool": {"name": "send_message", "args": ["{\"to\": \"work-assistant\", \"content\": \"Hello!\"}"]}}}' | chibi --json-config
```

The `from` field defaults to the current context name if not specified.

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

**`retrieve_content`** — Read a file or fetch a URL, then process through a sub-agent:
```json
{
  "source": "https://example.com/api-docs",
  "instructions": "Summarize the authentication section"
}
```

Both tools accept optional `model`, `temperature`, and `max_tokens` overrides. Without overrides, the parent's model and settings are used.

Sub-agent calls are non-streaming (results returned as tool output). Plugins can intercept or replace sub-agent calls via `pre_spawn_agent` / `post_spawn_agent` hooks — see [hooks.md](hooks.md#pre_spawn_agent).

### Ephemeral Context Flag

Use `-C` to spawn agents without affecting global context state:

```bash
# Run a task in another context
chibi -C research "Find information about quantum computing"

# Set system prompt and send task
chibi -C coding -y "You are a code reviewer" "Review this function"
```

### sub-agent Plugin

The `sub-agent` plugin (from chibi-plugins) provides a convenient wrapper for the LLM:

```
Main: [calls sub-agent with context: "research", task: "Find info about X"]
      ... sub-agent runs in "research" context ...
Main: [calls read_context with context_name: "research"]
Main: "The sub-agent found: ..."
```

### read_context Plugin

Allows reading another context's state without switching:

```json
{
  "context_name": "research",
  "include": ["todos", "goals", "summary", "messages"]
}
```

## Rolling Compaction

When auto-compaction is enabled and context size exceeds the threshold, rolling compaction kicks in:

### Process

1. LLM analyzes all messages
2. Decides which to archive based on:
   - Current goals and todos (keeps relevant messages)
   - Message recency (prefers keeping recent context)
   - Content importance (preserves key decisions)
3. Selected messages are archived and summarized
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
     [calls sub-agent: context="research", task="Find quantum computing basics"]
     [calls call_agent: "Check research results and continue"]

Round 2:
LLM: [calls read_context: "research"]
     Updates todos: "- [x] Search for materials\n- [ ] Synthesize findings"
     [calls call_agent: "Write the summary"]

Round 3:
LLM: Writes summary report
     Updates todos: "- [x] Search\n- [x] Synthesize\n- [x] Write report"
     Clears goals
     Returns final response to user
```

## Tool Output Caching

When tool outputs exceed the configured threshold (default: 4000 chars), they're automatically cached to disk and a truncated preview is sent to the LLM.

### How It Works

1. Tool produces large output (e.g., `fetch_url` returns a large webpage)
2. Output is cached to `~/.chibi/contexts/<name>/tool_cache/`
3. LLM receives a truncated message with:
   - Cache ID for later reference
   - Size and line count statistics
   - Preview of first ~500 chars
   - Instructions to use file tools for examination

### Examining Cached Content

The LLM uses built-in file tools to examine cached content surgically:

```
[Output cached: fetch_url_abc123_def456]
Tool: fetch_url | Size: 50000 chars, ~12500 tokens | Lines: 1200
Preview:
---
<!DOCTYPE html>
<html>
<head>
  <title>Example Page</title>
...
---
Use file_head, file_tail, file_lines, file_grep with cache_id to examine.
```

The LLM can then:
- `file_head(cache_id="fetch_url_abc123_def456", lines=100)` - See first 100 lines
- `file_grep(cache_id="...", pattern="class.*Button")` - Search for patterns
- `file_lines(cache_id="...", start=500, end=550)` - Read specific section

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
