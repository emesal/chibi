# Chibi TODO

## Completed
- [x] maximum recursion depth for continue_processing (config: max_recursion_depth, default 15)
- [x] use environment variables for tool parameters (documented limitation)
- [x] keep summary as a separate file in the context directory (summary.md)
- [x] use flat text files or md for context files (kept context.json, added summary.md)
- [x] consider using an external tool for accessing state of other contexts (read_context tool)
- [x] consider using an external tool for recursing (agent tool combines spawn + continue)
- [x] example tool for spawning sub-agents (agent tool in examples/tools/)
- [x] added -S/--sub-context flag for running in a context without changing global state

## Deferred

### Plugin Hooks
- tools may register to be called by the binary at hook points
- potential hook points: pre/post message, pre/post tool call, on context switch, on compaction
- consider: can hooks modify state or just observe?

### Agentic Workflow Prompts
- write more example prompts for agentic workflows
- default prompts unless specified in context dir
- idea: workflows with prompts could be tools that use hooks to inject bootstrap material

### local.toml per-context overrides
- exactly what it sounds like
- features like username and model (see below) are put here instead of in their own files

### Inter-context communication
- a feature of the rust code? or a tool?
- contexts need a way to distinguish who's speaking
  - this will be reflected in the transcripts
- username
  - setting in config.toml, default is 'user'
  - command line option -u/--username to set it per-context
  - command line option -U/--temp-username to set it for just this invocation
- if a tool, how to inject the messages? inbox/outbox files the rust code checks and passes on?
- messages should have FROM/TO/CONTENT fields.
  - TO decides recipient context.
  - FROM decides what the [SPEAKER] will be reported as
  - CONTENT is just the message itself
  - we don't need to enforce FROM fields in any way. ACAB!
- let tools create contexts. if deleted: autogenerates at tool startup
- external tool that creates coffee-table context
  - uses inter-context communication bus to provide a fikarast space
  - coffee-table itself has a system prompt to push discussions forward iff needed
  - coffe-table transcript is the full transcript of fika attendants

### Tandem Goals + Tandem Workflow
- instead of one agent for complex goals, spawn several agents with adjacent goals
- original task split into different perspectives:
  - "implement X with a rich feature set"
  - "implement X quickly"
  - "implement X with rigorous security"
- agents have instructions on cooperative work + roundtable discussion
- needs more experimentation to develop the methodology
- running any number of agents in parallel (or cooperative? but rust is good at this)

### Per-context models
- allow setting multiple named presets in config.toml containing model names
  - does toml have arrays?
- allow setting the model name per-context, or using any of the presets
- not setting the model = use the default model
- model name is stored in a flat text file in context dir
