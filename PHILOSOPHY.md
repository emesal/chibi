# Chibi Project Philosophy

## Core Principles

### 1. Unix Philosophy First
- **Clean output separation**: stdout only contains LLM responses (pipeable), stderr for diagnostics (with `-v`)
- **Composable**: Designed to work in pipelines (`cat error.log | chibi "explain this"`)
- **Consistent CLI:** Any argv string not beginning with `-` is interpreted as the start of the prompt (can be forced with `--`)
- **Do one thing well**: Focused on LLM conversations without overreaching

### 2. Extensibility Over Safety Rails
- **Zero restrictions on tools**: Tools receive full trust from the framework
- **User responsibility**: "You are expected to understand the tools you install" (README.md)
- **Tools self-govern**: Each tool handles its own safety (e.g., `run_command` asks for confirmation, tools check `CHIBI_VERBOSE`)
- **Environment-based arguments**: `CHIBI_TOOL_ARGS` frees stdin for user interaction

### 3. Persistence as First-Class Concern
- **Context isolation**: Separate conversations per project/topic
- **Dual transcript formats**: Human-readable `transcript.txt` + machine-readable `transcript.jsonl` with metadata (from/to, IDs, timestamps)
- **Cross-context reflection**: Persistent memory spanning all sessions via `update_reflection`
- **State preservation**: Everything saved atomically with context locks for multi-process safety

### 4. Agentic Autonomy
- **Built-in task tracking**: `update_todos`, `update_goals` guide autonomous work
- **Recursion without hand-holding**: `recurse` tool enables multi-step workflows with depth limits
- **Cross-agent messaging**: `send_message` + inbox system for inter-context communication
- **Sub-context isolation**: `-S` flag spawns ephemeral agents without affecting global state

### 5. Context-Aware Architecture
- **Hierarchical configuration**: CLI flags → `local.toml` → `config.toml` → `models.toml` → defaults
- **Per-context customization**: Different models, prompts, usernames per conversation
- **Context window management**: Warnings, auto-compaction, rolling compaction with LLM summarization
- **Goals/todos integration**: Automatically injected into system prompts

### 6. Observability and Extensibility Through Hooks
- **Many hook points**: `on_start`, `pre_message`, `pre_tool`, `post_compact`, etc.
- **Non-intrusive**: Hooks use same tool discovery mechanism (`--schema`)
- **Observation and modification**: Pre-hooks can modify data, post-hooks observe

## Design Values

| Value | Manifestation |
|--------|---------------|
| **Reliability** | Context locks with heartbeat, transcript preservation before destructive ops |
| **Transparency** | Streaming output, verbose mode, all data in plain text/JSON files |
| **Flexibility** | Tools register themselves via JSON schema, hooks inject into system prompts |
| **User Control** | Manual compaction (`-c`), context switching (`-s`), custom prompts per context |
| **Incremental Growth** | Features added via external tools, not built-in bloat |
| **Performance** | Streaming responses, async Rust, minimal runtime overhead |

## Anti-Patterns Avoided

1. **No opaque binary formats**: All data in JSON/TOML/Markdown
2. **No forced workflows**: Optional features (auto-compact, reflection, tools)
3. **No opinionated defaults**: Minimal baked-in behavior, user configures
4. **No hidden complexity**: File structure is simple and inspectable
5. **No dependency sprawl**: Minimal crate set (reqwest, tokio, serde, dirs-next)

## The "Danger Zone" Philosophy (README:196)

> "Chibi does not impose any restrictions on tools. NONE. Each tool is responsible for its own safety measures."

This reflects deliberate design: trust the user, provide mechanisms, not mandates. Safety is opt-in at the tool level (`run_command` confirms, `github-mcp` has safe-lists), not enforced by the framework.

## Summary

Chibi embodies a **minimalist yet extensible** CLI philosophy: a reliable, pipeable foundation for LLM conversations that gets out of the way while providing sophisticated mechanisms (hooks, contexts, inbox, compaction) for building agentic workflows. It treats users as sophisticated operators who understand their tools, rather than children needing protection.
