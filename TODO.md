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
- this is something we need to research and make use of:
  https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#enable-reasoning-with-default-config