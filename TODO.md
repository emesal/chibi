* maximum recursion depth for continue_processing
* use environment variables for tool parameters
  - chibi parses the binary and populates the env vars
  - the json parameter are still passed along
* keep summary as a separate file in the context directory
* use flat text files or md for context files
* consider using an external tool for accessing state of other contexts
  - pro: easy to disable the functionality
* consider using an external tool for recursing
  - pro: see above
* example tool for spawning sub-agents
  - pro: mhm guess what
* plugin hooks
  - tools may register to be called by the binary at hook points
* write more example prompts for agentic workflows
  - default prompts unles specified in context dir
