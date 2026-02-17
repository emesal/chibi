# Context Management

Contexts are separate conversations. Each context maintains its own:
- Message history
- Summary (from compaction)
- Todos and goals
- System prompt (optional override)
- Configuration (optional override via `local.toml`)

## Switching Contexts

```bash
# Switch to a context (creates if it doesn't exist)
chibi -c rust-learning

# Continue in that context
chibi "Tell me about ownership"

# Switch to another context
chibi -c web-project

# Switch back to previous context
chibi -c -
```

The current context is persisted in `~/.chibi/session.json`, along with the `previous_context` for quick switching.

### Auto-Named Contexts

```bash
# Create a new context with timestamp name (e.g., 20240115_143022)
chibi -c new

# Create with a prefix (e.g., bugfix_20240115_143022)
chibi -c new:bugfix
```

### Previous Context Reference

Use `-` as a shortcut to reference the previous context. When using `-c -`, it works just like `cd -` in bash - the current and previous contexts **swap places**, allowing you to toggle back and forth:

```bash
chibi -c dev          # Switch to 'dev', previous='default'
chibi -c production   # Switch to 'production', previous='dev'
chibi -c -            # Switch to 'dev', previous='production' (swapped!)
chibi -c -            # Switch to 'production', previous='dev' (swapped back!)
```

**Swap Behavior:**
The swap behavior means you can quickly toggle between two contexts:
```bash
chibi -c work         # Working on work project
chibi -c personal     # Switch to personal project (work is now previous)
chibi -c -            # Back to work (personal is now previous)
chibi -c -            # Back to personal (work is now previous)
# Keep toggling as needed!
```

**Using with Other Commands:**
The `-` reference works with any command that accepts a context name:
```bash
chibi -c staging      # Switch to staging
chibi -D -            # Delete the previous context (production)
chibi -G - 20         # Show last 20 log entries from previous context
chibi -C - "query"    # Ephemerally run a query in previous context (no swap)
```

**How it works:**
- `session.json` tracks `implied_context` and `previous_context` fields
- When using `-c -`, implied and previous contexts swap (like `cd -`)
- Ephemeral switches (`-C -`) use previous but don't swap or persist changes
- Other commands (`-D -`, `-G -`, etc.) just resolve to the previous context name
- `-` is a reserved name and cannot be used as an actual context name
- Error if no previous context exists (e.g., on first invocation)

## Ephemeral Contexts

Use `-C` to run in a context without changing your current context:

```bash
# Current context: default
chibi -C research "Find info about quantum computing"
# Still in: default (research was used only for that command)
```

This is useful for:
- Running one-off tasks in other contexts
- Spawning sub-agents
- Scripts that shouldn't affect user's current context

## Listing Contexts

```bash
# List all contexts
chibi -L

# Output shows lock status:
# * default [active]    # Currently in use by a chibi process
#   coding [stale]      # Lock exists but process likely crashed
#   research            # No lock, not in use
```

```bash
# Show current context info (name, message count, todos, goals)
chibi -l
```

## Context Operations

### Rename

```bash
# Rename current context
chibi -r new-name

# Rename a specific context
chibi -R old-name new-name
```

### Delete

```bash
# Delete current context (switches to default first)
chibi -d

# Delete a specific context
chibi -D old-project
```

### Archive (Clear History)

Archiving saves messages to transcript and clears the active context:

```bash
# Archive current context
chibi -a

# Archive a specific context
chibi -A old-context
```

### Compact

Compacting summarizes messages and starts fresh:

```bash
# Compact current context (uses LLM to summarize)
chibi -z

# Compact a specific context (simple archive, no LLM summary)
chibi -Z other-context
```

## Context Locking

When chibi is actively using a context, it creates a lock file to prevent concurrent access.

### How It Works

1. When chibi starts, it tries to acquire a lock on the context
2. A background thread updates the lock timestamp periodically (every `lock_heartbeat_seconds`)
3. Other chibi processes will fail if they try to use a locked context

### Lock Status

The `-L` flag shows lock status:

- **`[active]`** - Lock was updated recently (within 1.5x heartbeat interval)
- **`[stale]`** - Lock exists but is old (process likely crashed)
- No indicator - No lock, context is free

### Stale Lock Handling

If a process crashes, its lock becomes stale. Chibi automatically cleans up stale locks and acquires a new one.

## Activity Tracking & Auto-Destroy

Chibi tracks when each context was last used. This enables automatic cleanup of test contexts.

### Activity Tracking

Every time chibi runs with a context, it updates the `last_activity_at` timestamp in `state.json` (context metadata). This happens automatically during normal usage.

### Auto-Destroy (Debug Feature)

Contexts can be marked for automatic destruction using `--debug` flags. This is primarily for test cleanup:

```bash
# Create a context that auto-destroys after 60 seconds of inactivity
chibi --debug destroy_after_seconds_inactive=60 -c test-context

# Create a context that auto-destroys at a specific timestamp
chibi --debug destroy_at=1737820800 -c ephemeral-context
```

**How it works:**
- Auto-destroy checks run at the start of every chibi invocation
- Only non-current contexts are eligible for destruction
- A context is destroyed if:
  - `destroy_at >= 1` and current time > `destroy_at`, OR
  - `destroy_after_seconds_inactive >= 1` and current time > `last_activity_at + destroy_after_seconds_inactive`
- Values of 0 disable the respective feature (default)

**Use case:** Integration tests can create contexts with short inactivity timeouts. Subsequent normal chibi usage automatically cleans them up.

## Per-Context System Prompts

Each context can have its own system prompt:

```bash
# View current context's system prompt
chibi -n system_prompt

# Set from text
chibi -y "You are a helpful coding assistant"

# Set from file
chibi -y ~/prompts/coder.md

# Set for a specific context
chibi -Y research "You are a research assistant"
```

Custom prompts are stored in `~/.chibi/contexts/<name>/system_prompt.md`. If not set, the default from `~/.chibi/prompts/chibi.md` is used.

### Example: Different Personalities

```bash
chibi -c coding
chibi -y "You are a senior software engineer. Be precise and technical."

chibi -c creative
chibi -y "You are a creative writing assistant. Be imaginative and playful."

chibi -c default  # Uses the default chibi.md prompt
```

## Per-Context Configuration

Each context can override global settings. See [configuration.md](configuration.md#per-context-configuration-localtoml) for details.

```bash
# The config file location
~/.chibi/contexts/<name>/local.toml
```

## Inspecting Contexts

```bash
# Inspect current context
chibi -n system_prompt   # View system prompt
chibi -n reflection      # View reflection (global)
chibi -n todos           # View todos
chibi -n goals           # View goals
chibi -n list            # List what can be inspected

# Inspect a specific context
chibi -N research todos
chibi -N coding system_prompt
```

## Viewing History

```bash
# Show last N entries from current context
chibi -g 10

# Show last N entries from a specific context
chibi -G research 20

# Negative numbers show from the beginning
chibi -g -5  # First 5 entries
```

## Storage Structure

```
~/.chibi/contexts/<name>/
├── transcript/          # Authoritative conversation log (partitioned)
│   ├── manifest.json    # Partition metadata
│   ├── active.jsonl     # Current write partition
│   └── partitions/      # Archived read-only partitions
├── context.jsonl        # LLM context window (derived from transcript)
├── context_meta.json    # Metadata (created_at timestamp)
├── local.toml           # Per-context config overrides (optional)
├── summary.md           # Conversation summary (from compaction)
├── todos.md             # Current todos
├── goals.md             # Current goals
├── inbox.jsonl          # Messages from other contexts
├── system_prompt.md     # Custom system prompt (optional)
├── tool_cache/          # Cached large tool outputs
├── .lock                # Lock file (when active)
└── .dirty               # Marker for context rebuild (temporary)
```

See [transcript-format.md](transcript-format.md) for details on the JSONL file formats.
