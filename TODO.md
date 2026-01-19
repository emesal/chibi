* rolling context window with LLM-guided stripping of the oldest bits
  - triggered by existing auto_compact_threshold in config.toml (when auto_compact = true)
  - if auto_compact = false, this never triggers (current behavior preserved)
  - manual compaction via -c still available for full context compaction
  - the LLM decides what to drop based on todos and goals (see below)
  - the LLM is then tasked to integrate the dropped bits into the summary (see below)
* a summary of the stripped chat history is maintained in the context state
* agentic workflow
  - todos as part of context state (this round?)
  - goals as part of context state (between rounds?)
  - recurse switch that allows the llm to respond without losing control
* sub-agents:
  - use wrapper tool approach (simpler, fits unix philosophy)
  - sub-agents receive JSON parameters indicating which other contexts they might relate to
  - read-only context access tool for cross-context state inspection
  - use cross-context state tool BEFORE reflection (reflection is for personality, not tasks)
* cross-context task memory?
  - reflection is personality/preferences, not for coordinating tasks
  - do we need a separate shared task state? or is read-only context access sufficient?
  - could be: shared goals file, or a "workspace" concept grouping related contexts
* `-s new` creates auto-named context
  - name format: YYYYMMDD_HHMMSS (underscores, not hyphens)
  - if collision, append _N (e.g., 20240115_143022_2)
  - optional prefix: `-s new:prefix` creates prefix_YYYYMMDD_HHMMSS
  - the literal name "new" is reserved
* stdin prompt support
  - `chibi -s context "prompt here"` (current)
  - `echo "prompt here" | chibi -s context` (new)
  - if stdin is not a tty, read prompt from stdin
  - if both stdin and arg prompt provided, concatenate them
