### more tests
- test CLI parameter paradigm
- tests for the example tools

### separate tools repo
- eventually

### workflow definition tools
- use hooks to inject string into system prompt, making chibis aware they exist
- tool call that returns system prompts appropriate for sub-agent roles
- tool call that returns workflow description/instructions
- this is very SKILL.md-like ngl

### Tandem Goals + Tandem Workflow
- instead of one agent for complex goals, spawn several agents with adjacent goals
- original task split into different perspectives:
  - "implement X with a rich feature set"
  - "implement X quickly"
  - "implement X with rigorous security"
- agents have instructions on cooperative work + coffee-table discussion
- this looks like a workflow definition tool

### Reasoning tokens
- this is something we need to research and make use of:
  https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#enable-reasoning-with-default-config

### -l option
- current behaviour: -l 4 outputs the last 4 exchanges, -l 0 outputs all exchanges (in current context)
- add this behaviour: -l -5 outputs the *first* five exchanges (of the current context)
- -L/--list-all works like -l/--list but operates on _the full_ transcript (both current and stored combined)

### compaction changes
- current behaviour: the rolling compaction currently drops the earliest half of the current state
- new behaviour: the LLM is tasked with deciding how much to drop (based on goals)
- optional fallback behaviour preserved+improved:
  - config.toml setting for the percentage of history to compact
- implementation option: instead of internal implementation, tool+hooks
   that override the default percentage implementation as per the above?
