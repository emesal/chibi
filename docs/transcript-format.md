# Transcript Format

Chibi maintains conversation history in two formats: human-readable markdown and machine-readable JSONL.

## Human-Readable (transcript.md)

The `transcript.md` file stores conversation history in a format similar to the LLM's context:

```
[USER]: What is Rust?

[ASSISTANT]: Rust is a systems programming language...

[USER]: Tell me more about ownership.

[ASSISTANT]: Ownership is Rust's key feature...
```

This file is appended to during archiving and compaction operations. It provides a readable archive of past conversations.

## Machine-Readable (context.jsonl)

The `context.jsonl` file stores the active conversation in JSON Lines format with metadata:

```json
{"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":1705123456,"from":"alice","to":"default","content":"What is Rust?","entry_type":"message"}
{"id":"550e8400-e29b-41d4-a716-446655440001","timestamp":1705123460,"from":"default","to":"user","content":"Rust is a systems programming language...","entry_type":"message"}
```

### Entry Fields

| Field | Description |
|-------|-------------|
| `id` | Unique UUID for the entry |
| `timestamp` | Unix timestamp (seconds since epoch) |
| `from` | Source: username, context name, or tool name |
| `to` | Destination: context name, "user", or tool name |
| `content` | Message content, tool arguments, or tool results |
| `entry_type` | Type of entry (see below) |
| `metadata` | Optional additional data |

### Entry Types

| Type | Description | Example `from` | Example `to` |
|------|-------------|----------------|--------------|
| `message` | User or assistant message | `alice` | `default` |
| `tool_call` | LLM calling a tool | `default` | `read_file` |
| `tool_result` | Tool returning a result | `read_file` | `default` |
| `compaction` | Compaction marker | `system` | `default` |

### Message Examples

**User message:**
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

**Assistant message:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "timestamp": 1705123460,
  "from": "default",
  "to": "user",
  "content": "Rust is a systems programming language...",
  "entry_type": "message"
}
```

**Tool call:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440002",
  "timestamp": 1705123465,
  "from": "default",
  "to": "read_file",
  "content": "{\"path\":\"Cargo.toml\"}",
  "entry_type": "tool_call"
}
```

**Tool result:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440003",
  "timestamp": 1705123466,
  "from": "read_file",
  "to": "default",
  "content": "[package]\nname = \"chibi\"...",
  "entry_type": "tool_result"
}
```

**Compaction marker:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440004",
  "timestamp": 1705123500,
  "from": "system",
  "to": "default",
  "content": "",
  "entry_type": "compaction",
  "metadata": {
    "summary": "The conversation covered Rust basics including ownership..."
  }
}
```

## Archive Files

### transcript_archive.jsonl

When compaction occurs, older entries are moved to `transcript_archive.jsonl`. This file has the same format as `context.jsonl` but contains archived entries.

### Relationship Between Files

```
context.jsonl           # Active conversation (rebuilt from transcript on load)
transcript_archive.jsonl # Archived entries from compaction
transcript.md           # Human-readable archive (append-only)
```

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

Inbox messages are injected into the prompt and cleared after use.

## Debug Files

When debug logging is enabled (`--debug`):

### requests.jsonl

Full API request bodies:

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

### response_meta.jsonl

Response metadata (usage stats, model info):

```json
{
  "timestamp": 1705123460,
  "response": {
    "id": "chatcmpl-abc123",
    "model": "anthropic/claude-sonnet-4",
    "usage": {
      "prompt_tokens": 1234,
      "completion_tokens": 567,
      "total_tokens": 1801
    }
  }
}
```

## Working with JSONL

### Viewing with jq

```bash
# Pretty print all entries
cat ~/.chibi/contexts/default/context.jsonl | jq '.'

# Filter by entry type
cat context.jsonl | jq 'select(.entry_type == "message")'

# Get just the content
cat context.jsonl | jq -r '.content'

# Count entries by type
cat context.jsonl | jq -s 'group_by(.entry_type) | map({type: .[0].entry_type, count: length})'
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
