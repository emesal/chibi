# Transcript Format

Chibi maintains conversation history using partitioned storage.

## File Roles

```
transcript/           # Authoritative, append-only log (partitioned)
├── manifest.json     # Partition metadata and timestamp ranges
├── active.jsonl      # Current write partition
└── partitions/       # Archived read-only partitions
    ├── <ts>-<ts>.jsonl
    └── <ts>-<ts>.bloom  # Bloom filter for search
context.jsonl         # LLM context window (derived from transcript)
```

### transcript/ (Authoritative)

The authoritative record of all conversation history. Partitioned for scalability—entries are append-only and never modified. Contains anchor entries that mark significant events (context creation, compaction, archival).

Partitions rotate when any threshold is reached:
- Entry count (default: 1000)
- Token count (default: 100,000 estimated LLM tokens)
- Age (default: 30 days)

### context.jsonl (Derived)

The active LLM context window. Derived from transcript starting at the last anchor entry. Rebuilt automatically when stale.

Structure:
1. **Entry 0**: Anchor entry (`context_created`, `compaction`, or `archival`)
2. **Remaining**: Conversation entries (messages, tool calls, tool results, control transfers)

> **Note:** The system prompt is **not** stored in `context.jsonl`. It lives in `system_prompt.md` (source of truth) and is tracked via `context_meta.json`.

## Entry Format

All JSONL entries share this structure:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": 1705123456,
  "from": "alice",
  "to": "default",
  "content": "What is Rust?",
  "entry_type": "message"
}
```

### Fields

| Field | Description |
|-------|-------------|
| `id` | Unique UUID for the entry |
| `timestamp` | Unix timestamp (seconds since epoch) |
| `from` | Source: username, context name, tool name, or "system" |
| `to` | Destination: context name, "user", or tool name |
| `content` | Message content, tool arguments, or tool results |
| `entry_type` | Type of entry (see below) |
| `metadata` | Optional object with additional data (summary, etc.) |
| `tool_call_id` | Optional; present on `tool_call` and `tool_result` entries to correlate pairs |
| `role` | Optional; API role for `entries_to_messages()`: `"user"`, `"agent"`, or `"system"`. Absent on old entries (backwards-compat fallback applies) and non-message types. |
| `flow_control` | Optional boolean; semantic marker for flow control events (user prompts, `call_user` messages, `control_transfer`). Omitted when `false`. |

### Entry Types

#### Conversation Types

| Type | Description | `from` | `to` |
|------|-------------|--------|------|
| `message` | User or assistant message | username or context | context or "user" |
| `tool_call` | LLM calling a tool | context | tool name |
| `tool_result` | Tool returning a result | tool name | context |

#### Flow Control Types

Flow control entries record turn boundaries and handoffs. They appear in both transcript and context.jsonl.

| Type | Description | `from` | `to` | `flow_control` |
|------|-------------|--------|------|----------------|
| `control_transfer` | Control passes between parties (no content) | sender | receiver | `true` |

`control_transfer` entries carry no content and are skipped by `entries_to_messages()`. They are purely informational — recording who handed off to whom.

**Directions:**

| Event | `from` | `to` |
|-------|--------|------|
| User sends a prompt | username | context |
| Agent invokes `call_user` | context | username |

`call_user` also produces a regular `message` entry (with `role: "agent"`, `flow_control: true`) for the message content. That entry is included in API messages as `role: "assistant"`.

> **Note:** `call_agent` is not exposed to the LLM as a callable tool. Its infrastructure (`CALL_AGENT_TOOL_NAME`, `HandoffTarget::Agent`) is retained for the fallback tool mechanism and future inter-agent control transfer.

#### Anchor Types (context.jsonl[0])

| Type | Description | When Created |
|------|-------------|--------------|
| `context_created` | New context initialization | Context first created |
| `compaction` | Context was compacted | After LLM-based or rolling compaction |
| `archival` | Context was archived/cleared | After clear operation |

#### System Types (transcript only)

| Type | Description |
|------|-------------|
| `system_prompt_changed` | System prompt was updated; stored in transcript only, never written to context.jsonl |

## Examples

### User Message

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": 1705123456,
  "from": "alice",
  "to": "default",
  "content": "What is Rust?",
  "entry_type": "message"
}
```

### Assistant Message

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "timestamp": 1705123460,
  "from": "default",
  "to": "alice",
  "content": "Rust is a systems programming language...",
  "entry_type": "message",
  "role": "agent"
}
```

### Tool Call

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440002",
  "timestamp": 1705123465,
  "from": "default",
  "to": "file_head",
  "content": "{\"path\":\"Cargo.toml\"}",
  "entry_type": "tool_call",
  "tool_call_id": "tc_abc123"
}
```

### Tool Result

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440003",
  "timestamp": 1705123466,
  "from": "file_head",
  "to": "default",
  "content": "[package]\nname = \"chibi\"...",
  "entry_type": "tool_result",
  "tool_call_id": "tc_abc123"
}
```

### User Prompt (with control transfer)

```json
{"id": "...", "timestamp": 1705123456, "from": "alice", "to": "default",
 "content": "[20260306-1234+0000] What is Rust?", "entry_type": "message",
 "role": "user", "flow_control": true}

{"id": "...", "timestamp": 1705123456, "from": "alice", "to": "default",
 "content": "", "entry_type": "control_transfer", "flow_control": true}
```

### Call User (agent → user handoff)

When the LLM invokes `call_user`, two entries are written:

```json
{"id": "...", "timestamp": 1705123470, "from": "default", "to": "alice",
 "content": "Task complete.", "entry_type": "message",
 "role": "agent", "flow_control": true}

{"id": "...", "timestamp": 1705123470, "from": "default", "to": "alice",
 "content": "", "entry_type": "control_transfer", "flow_control": true}
```

The `message` entry is included in API history as `role: "assistant"`. The `control_transfer` entry is skipped.

### Context Created Anchor

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440004",
  "timestamp": 1705123400,
  "from": "system",
  "to": "default",
  "content": "Context created",
  "entry_type": "context_created"
}
```

### Compaction Anchor

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440005",
  "timestamp": 1705123500,
  "from": "system",
  "to": "default",
  "content": "Context compacted",
  "entry_type": "compaction",
  "metadata": {
    "summary": "The conversation covered Rust basics including ownership..."
  }
}
```

### Archival Anchor

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440006",
  "timestamp": 1705123600,
  "from": "system",
  "to": "default",
  "content": "Context archived/cleared",
  "entry_type": "archival"
}
```

## Metadata Structure

The optional `metadata` field can contain:

| Field | Used In | Description |
|-------|---------|-------------|
| `summary` | `compaction` | Summary of compacted conversation |
| `transcript_anchor_id` | context.jsonl anchors | Reference to corresponding transcript entry ID |

## Context Rebuilding

When `context.jsonl` is stale, it is rebuilt from the transcript:

1. Find the last anchor entry across all transcript partitions
2. Copy entries from that anchor to end, filtering out `system_prompt_changed` events
3. Write as `context.jsonl`: anchor at entry[0], conversation entries following

The system prompt is injected at API call time from `system_prompt.md` (via `context_meta.json`), not stored as an entry in `context.jsonl`.

## Inbox Format (inbox.jsonl)

Inter-context messages are stored in `inbox.jsonl`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440010",
  "timestamp": 1705123470,
  "from": "main",
  "to": "research",
  "content": "Please look up quantum computing basics"
}
```

Inbox messages are injected into the prompt and cleared after delivery.

## Debug Files

When debug logging is enabled (`--debug request-log`):

### requests.jsonl

Full API request bodies logged to the context directory:

```json
{
  "timestamp": 1705123456,
  "request": {
    "model": "anthropic/claude-sonnet-4",
    "messages": [...],
    "tools": [...],
    "stream": true
  }
}
```

## Working with JSONL

### Viewing with jq

```bash
# Pretty print active partition
cat ~/.chibi/contexts/default/transcript/active.jsonl | jq '.'

# View all partitions (including archived)
cat ~/.chibi/contexts/default/transcript/partitions/*.jsonl \
    ~/.chibi/contexts/default/transcript/active.jsonl | jq '.'

# Filter by entry type
cat transcript/active.jsonl | jq 'select(.entry_type == "message")'

# Get just the content
cat transcript/active.jsonl | jq -r '.content'

# Count entries by type across all partitions
cat transcript/partitions/*.jsonl transcript/active.jsonl 2>/dev/null | \
    jq -s 'group_by(.entry_type) | map({type: .[0].entry_type, count: length})'

# Find anchor entries
cat transcript/active.jsonl | jq 'select(.entry_type == "context_created" or .entry_type == "compaction" or .entry_type == "archival")'
```

### Viewing with chibi

```bash
# Last 10 entries
chibi -g 10

# First 5 entries
chibi -g -5

# From another context
chibi -G research 20
```
