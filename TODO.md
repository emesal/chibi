# Implemented

* [x] stdin prompt support
  - `echo "prompt here" | chibi -s context`
  - if both stdin and arg prompt provided, concatenate them

* [x] `-s new` creates auto-named context
  - name format: YYYYMMDD_HHMMSS (underscores, not hyphens)
  - if collision, append _N (e.g., 20240115_143022_2)
  - optional prefix: `-s new:prefix` creates prefix_YYYYMMDD_HHMMSS
  - the literal name "new" is reserved

* [x] todos and goals as part of context state
  - stored as separate files in context directory (todos.md, goals.md)
  - built-in tools: update_todos, update_goals
  - automatically included in system prompt

* [x] conversation summary maintained in context state
  - stored in context.json as "summary" field
  - included in system prompt automatically

* [x] rolling compaction (LLM-guided stripping)
  - triggered by existing auto_compact_threshold in config.toml (when auto_compact = true)
  - if auto_compact = false, this never triggers (current behavior preserved)
  - manual compaction via -c still available for full context compaction
  - strips oldest half of messages
  - LLM integrates stripped content into summary, guided by goals/todos

* [x] agentic workflow
  - built-in continue_processing tool
  - LLM can recurse without returning control to user
  - includes "note to self" for next round

* [x] sub-agents via wrapper tool
  - read_context tool for cross-context state inspection (read-only)
  - sub-agents can be spawned via external tool that calls chibi
  - main agent reads sub-agent results via read_context

# Future Ideas

* workspace concept - grouping related contexts
* shared goals across workspace
* maximum recursion depth for continue_processing
* built-in spawn_agent tool (currently requires external script)
