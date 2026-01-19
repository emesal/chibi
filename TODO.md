* rolling context window with LLM-guided stripping of the oldest bits
  - the LLM decides what to drop based on todos and goals (see below)
  - the LLM is then tasked to integrate the dropped bits into the summary (see below)
* a summary of the stripped chat history is maintained in the context state
* agentic workflow
  - todos as part of context state (this round?)
  - goals as part of context state (between rounds?)
  - recurse switch that allows the llm to respond without losing control
* sub-agents:
  - a chibi wrapper tool? simpler
  - or do we need the rust app itself to handle this? more complex
  - let's use the wrapper approach at first. the sub-agents can be made aware via their JSON parameters which other contexts they might need to relate too.
  - another example tool can be created which access the various state files of a named context -- READ ONLY
* if chibi is run with the -s parameter's value being "new", a new context is created (the name 'new' is thus reserved and never used). the context name can just be the current date and time YYYYMMDD-HHMMSS with an extra -N (a number) if other contexts already exist with that name.
* chibi should be possible to run in these ways:
  $ chibi -s context "prompt here"
  $ echo "prompt here" | chibi -s context
