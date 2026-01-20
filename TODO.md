### context lockfiles ✓ DONE
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

### recursion ✓ DONE
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

### local.toml per-context overrides ✓ DONE
- exactly what it sounds like
- features like context-specific username and model (see below) are put here
- values in this file override the corresponding values in the main config.toml
- hearbeat can not be overridden here
- see separate config-plan.md for tentative implementation plan

### Per-context models ✓ DONE
- allow setting model per-context, the presets are just aliases. use local.toml
- not setting the model = use the default model (mandatory 'model' variable in config.toml)

### JSON transcripts ✓ DONE
- let's make the full transcripts be JSON, and let them include all details
- let's still make txt transcripts because appending is quick
- every message/event needs a unique identifier (to avoid duplication issues for one)
  - i imagine messages are a type of event, as are all tool calls and compaction events etc
- messages should have from/to/content fields.
  - 'to' decides which context receives the message.
  - 'from' decides what the "[SPEAKER]" will be reported as in transcripts
  - 'content' is just the message itself
  - we don't need to enforce 'from' fields in any way. ACAB!
  - should the 'from' field, or a fourth field, indicate if the from entity is a chibi or a human?
    - chibis should normally put the name of their context in the from field, as that's
      how to refer to each chibi
- the JSON transcript will be the "source of truth" while the txt transcript is for the convenience of humans

### Username ✓ DONE (part of inter-context communication)
- setting in config.toml, default to 'user'
- can be overridden by local.toml in the context's directory
- command line option -u/--username to set it per-context
- command line option -U/--temp-username to set it for just this invocation
- the system prompt should make the LLM aware what the user is called,
  and that there may be other speakers than the user

### Inter-context communication
- a feature of the rust code? or a tool? i lean tool if it's solvable
- how to inject the messages? inbox/outbox files that the rust code checks and passes on? something else?
- contexts need a way to distinguish who's speaking
  - this will be reflected in the transcripts
- let tools create context. a tool could ensure that a context is always created at startup if it doesn't exist.
  - this happens by the tool signalling the rust app that a context should be created
  - the tool can also specify the system prompt to assign to the context
- external tool called "coffee-table" that creates "coffee-table" context
  - coffee-table is an inter-context communication bus that provides a fika space
  - coffee-table itself has a system prompt to push discussions forward iff needed
    (and to stay out of the way if things are going smoothly)
  - the coffe-table transcript is the full transcript of all the fika attendants
    - this is because sending a message to the coffee-table means that it ends
      up in the coffee table's context, which is checked by all chibis
      who are interested in that conversation
  - an "attendant" is simply an agent that uses the tool to read the state of other contexts
    to read the context of the coffee-table

### Agentic Workflow Prompts
- write more example prompts for agentic workflows
- idea: workflows could be tools that use hooks to inject bootstrap material/prompts/howtos etc
- coffee-table is one such example. it injects a message in the system prompt, explaining its existence
  and purpose, to that all chibis know where to read and send messages to participate in
  inter-chibi communication
- this is elegant because if one removes the coffee-table tool from the tools folder,
  the feature disappears (although the context remains, but new chibis won't know about it)
  - also, if in the future the tool for reading other context's state, and the tool
    for sending messages (which must be created), are rewritten to support remote locations,
    the chibi swarm becomes distributed (extra bonus is the spawn subagent tool can also spawn remotely)

see also ./additional-features-plan.md

# for later

### Tandem Goals + Tandem Workflow
- instead of one agent for complex goals, spawn several agents with adjacent goals
- original task split into different perspectives:
  - "implement X with a rich feature set"
  - "implement X quickly"
  - "implement X with rigorous security"
- agents have instructions on cooperative work + coffee-table discussion

### Reasoning tokens
- this is something we need to research and make use of:
  https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#enable-reasoning-with-default-config
