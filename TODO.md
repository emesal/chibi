### context lockfiles
- each chibi uses a lockfile for its context
- lockfiles are touched every 30 seconds while the chibi is running
  - the heartbeat can be changed in config.toml
  - done by a background thread. since it's light it run for the duration
  - after 1.5x the heartbeat duration, the lockfile is considered stale
  - lockfiles contain the PID of the process holding the lock
  - if no process has that PID, the lockfile is stale
- in the context list, locked contexts are appended with [active]
- in the context list, contexts with stale locks are appended with [stale]
- stale lockfiles are... deleted? when a new chibi wants to acquire the context?
  - yes, on acquisition and not before
- lockfiles are acquired immediately upon the chibi starting, and kept until it exists
- lockfiles prevent *writes* but does not prohibit reads

### recursion
- recursion can be implemented as a noop tool
  - all tools return control to the LLM for another turn; tool calls are really just recursion
    with context modification. a noop tool skips the context modification.
- get rid of the internal tool for recursion
- also rip out the recursion code from the external agent tool (renamed to sub-agent)
- the recurse tool should be called "recurse"
- we could still keep the "note to self" feature, if it serves any purpose?
- the next iteration of the agent has access to the chat history and knows
    what it did before calling the recurse tool
  - the "note to self" could be described to the LLM as where it should explain
    why it is recursing, and reiterating the task.
  - we should check that it's possible to catch calls to the recurse tool with a hook
- rename the agent tool to sub-agent

### local.toml per-context overrides
- exactly what it sounds like
- features like context-specific username and model (see below) are put here
- values in this file override the corresponding values in the main config.toml
- hearbeat can not be overridden here

### JSON transcripts
- let's make the full transcripts be JSON, and let them include all details
- let's still make txt transcripts because appending is quick
- every message/event needs a unique identifier (to avoid duplication issues for one)
- messages should have from/to/content fields.
  - to decides recipient context.
  - from decides what the "[SPEAKER]" will be reported as in transcripts
  - content is just the message itself
  - we don't need to enforce FROM fields in any way. ACAB!
  - should the from-field or a fourth field indicate if the from entity is a chibi, a human or the system?
    - the system should just be called SYSTEM. the system prompt should indicate this.

### Inter-context communication
- a feature of the rust code? or a tool? i lean tool if it's solvable
- how to inject the messages? inbox/outbox files that the rust code checks and passes on? something else?
- contexts need a way to distinguish who's speaking
  - this will be reflected in the transcripts
- username
  - setting in config.toml, default to 'user'
  - can be overridden by local.toml in the context's directory
  - command line option -u/--username to set it per-context
  - command line option -U/--temp-username to set it for just this invocation
  - the system prompt should make the LLM aware what the user is called,
    and that there may be other speakers than the user
- let tools create context. a tool could ensure that a context is always created at startup if it doesn't exist.
- external tool that creates "coffee-table" context
  - coffee-table is an inter-context communication bus that provides a fika space
  - coffee-table itself has a system prompt to push discussions forward iff needed
    (and to stay out of the way if things are going smoothly)
  - the coffe-table transcript is the full transcript of all the fika attendants
    - this is because sending a message to the coffee-table means that it ends
      up in the coffee table's context, which is checked by all chibis
      who are interested in that conversation

### Agentic Workflow Prompts
- write more example prompts for agentic workflows
- idea: workflows could be tools that use hooks to inject bootstrap material/prompts/howtos etc
- coffee-table is one example?

### Tandem Goals + Tandem Workflow
- instead of one agent for complex goals, spawn several agents with adjacent goals
- original task split into different perspectives:
  - "implement X with a rich feature set"
  - "implement X quickly"
  - "implement X with rigorous security"
- agents have instructions on cooperative work + coffee-table discussion

### Per-context models
- allow setting multiple named presets in config.toml containing model names
  - ie 'model[quick] = "model_name"'
  - but does toml have arrays?
- there are no mandatory presets. it's up to the user. the example config can include a bunch though
- allow setting model per-context, the presets are just aliases. use local.toml
- not setting the model = use the default model (mandatory 'model' variable in config.toml)

### Reasoning tokens
- this is something we need to research and make use of:
  https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#enable-reasoning-with-default-config
