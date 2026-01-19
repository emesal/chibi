* maximum recursion depth for continue_processing
* example tool for spawning sub-agents
* use environment variables for tool parameters
  - chibi parses the binary and populates the env vars
  - the json parameter are still passed along
* keep summary as a separate file in the context directory
* use flat text files or md for context files
* consider using an external tool for accessing state of other contexts
  - pro: easy to disable the functionality
* plugin hooks
  - tools may register to be called by the binary at hook points
