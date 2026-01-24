# Agentic Workflows

Chibi includes built-in tools and features that enable autonomous, multi-step workflows.

## Built-in Tools

The LLM always has access to these tools (no setup required):

| Tool | Description |
|------|-------------|
| `update_todos` | Track tasks for the current conversation |
| `update_goals` | Set high-level objectives |
| `update_reflection` | Update persistent memory (when reflection is enabled) |
| `send_message` | Send messages to other contexts |

## External Plugins

These plugins are available in [chibi-plugins](https://github.com/emesal/chibi-plugins) and must be installed separately:

| Plugin | Description |
|--------|-------------|
| `recurse` | Continue working without returning to user |
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

## Recurse (Autonomous Mode)

The `recurse` plugin (from chibi-plugins) lets the LLM work autonomously across multiple rounds.

### How It Works

1. LLM calls `recurse` with a note about what to do next
2. Current response finishes
3. A new round starts automatically with the note injected
4. LLM continues working

```
Round 1:
LLM: "I need to check test results next."
     [calls recurse with note: "Check the test results"]

Round 2:
LLM: (sees note) "Continuing. Note to self: Check the test results"
     ... continues working ...
```

### Safety Limits

The `max_recursion_depth` config limits how many rounds can happen (default: 30).

```toml
max_recursion_depth = 30
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

### Transient Context Flag

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
     [calls recurse: "Check research results and continue"]

Round 2:
LLM: [calls read_context: "research"]
     Updates todos: "- [x] Search for materials\n- [ ] Synthesize findings"
     [calls recurse: "Write the summary"]

Round 3:
LLM: Writes summary report
     Updates todos: "- [x] Search\n- [x] Synthesize\n- [x] Write report"
     Clears goals
     Returns final response to user
```

## Best Practices

1. **Clear Goals** - Help the LLM stay focused by encouraging goal-setting
2. **Incremental Todos** - Breaking work into small tasks helps track progress
3. **Reasonable Recursion Limits** - Balance autonomy vs. runaway loops
4. **Use Reflection Wisely** - Store genuinely useful long-term knowledge
5. **Monitor with Verbose Mode** - Use `-v` to see what the agent is doing
