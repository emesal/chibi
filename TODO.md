### per-context API
- model can already be overridden

### first-class group messaging
- including remotely

### ability to invoke chibi with json instead of CLI parameters
- see separate draft (file link here)

### context.jsonl and transcript.jsonl written simultaneously
- introduces some redundancy but simplifies for later when transcripts are elsewhere
- tool calls aren't in the log, only tool results?

### command line option for inbox sweep
- when invoked, do a sweep of all inboxes, waking any chibis with new messages and letting them handle them

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
- need to research and make use of:
  https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#enable-reasoning-with-default-config
