//! LLM types used during streaming response accumulation.

/// Accumulated tool call data during streaming
#[derive(Default)]
pub struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
