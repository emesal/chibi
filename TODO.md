* rolling context window with LLM-guided stripping of the oldest bits
* a summary of the stripped chat history is maintained in the context state
* agentic workflow
  - todos as part of state (this round?)
  - goals as part of state (between rounds?)
  - recurse switch that allows the llm to respond without losing control
* sub-agents:
  - a chibi wrapper tool? simpler
  - or do we need the rust app itself to handle this? more complex
