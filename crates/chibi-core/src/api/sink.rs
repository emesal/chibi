//! Response sink abstraction for decoupling API from presentation.
//!
//! This module defines the `ResponseSink` trait that allows the API module
//! to emit events without knowing about specific presentation implementations
//! (like terminal markdown rendering or JSON output).

use crate::context::TranscriptEntry;
use std::io;

/// Describes which fuel-related moment triggered a [`ResponseEvent::FuelStatus`] event.
#[derive(Debug, Clone)]
pub enum FuelEvent {
    /// Emitted at the start of each agentic turn.
    EnteringTurn,
    /// Emitted after a batch of tool calls completes.
    AfterToolBatch,
    /// Emitted when the agentic loop continues with a follow-up prompt.
    AfterContinuation { prompt_preview: String },
    /// Emitted when the model returns an empty response.
    EmptyResponse,
}

/// Events emitted during prompt processing.
///
/// These events represent the various outputs that occur during an API
/// interaction, from text streaming to tool execution status. Core emits all
/// variants unconditionally; clients filter and format as appropriate.
#[derive(Debug, Clone)]
pub enum ResponseEvent<'a> {
    /// A chunk of text content from the streaming response.
    TextChunk(&'a str),

    /// A chunk of thinking/reasoning content from the streaming response.
    Reasoning(&'a str),

    /// A transcript entry to be logged/displayed.
    TranscriptEntry(TranscriptEntry),

    /// A tool has started execution.
    ToolStart { name: String, summary: Option<String> },

    /// A tool has completed execution.
    ToolResult { name: String, result: String, cached: bool },

    /// The response stream has finished.
    Finished,

    /// A newline should be emitted (typically after response completion).
    Newline,

    /// A new response is starting (used to reset state between agentic loop iterations).
    StartResponse,

    /// Hook filter/modification/override debug info (verbose-tier in CLI).
    HookDebug { hook: String, message: String },

    /// Fuel budget status update (verbose-tier in CLI).
    FuelStatus { remaining: usize, total: usize, event: FuelEvent },

    /// Fuel budget exhausted — always shown in CLI.
    FuelExhausted { total: usize },

    /// Context window nearing limit (verbose-tier in CLI).
    ContextWarning { tokens_remaining: usize },

    /// Per-tool diagnostic message (verbose-tier in CLI).
    ToolDiagnostic { tool: String, message: String },

    /// Inbox messages injected into prompt (verbose-tier in CLI).
    InboxInjected { count: usize },
}

/// Trait for handling response events during prompt processing.
///
/// Implementations of this trait handle the presentation layer concerns
/// (terminal output, JSON formatting, etc.) while the core API remains
/// agnostic to how events are displayed.
///
/// # Example
///
/// ```
/// use chibi_core::api::sink::{ResponseSink, ResponseEvent};
/// use std::io;
///
/// struct MySink {
///     text: String,
/// }
///
/// impl ResponseSink for MySink {
///     fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
///         if let ResponseEvent::TextChunk(chunk) = event {
///             self.text.push_str(chunk);
///         }
///         Ok(())
///     }
/// }
///
/// let mut sink = MySink { text: String::new() };
/// sink.handle(ResponseEvent::TextChunk("Hello")).unwrap();
/// assert_eq!(sink.text, "Hello");
/// ```
pub trait ResponseSink {
    /// Handle a response event.
    ///
    /// Called for each event during prompt processing. Implementations
    /// should handle the event appropriately for their output medium.
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()>;
}

/// A sink that collects responses for programmatic use.
///
/// Useful for testing or when you need to capture the response
/// without any terminal output.
#[derive(Debug, Default)]
pub struct CollectingSink {
    /// Accumulated text content from the response.
    pub text: String,
    /// Accumulated reasoning/thinking content from the response.
    pub reasoning: String,
    /// Transcript entries emitted during the interaction.
    pub entries: Vec<TranscriptEntry>,
}

impl CollectingSink {
    /// Create a new collecting sink.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ResponseSink for CollectingSink {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(chunk) => {
                self.text.push_str(chunk);
            }
            ResponseEvent::Reasoning(chunk) => {
                self.reasoning.push_str(chunk);
            }
            ResponseEvent::TranscriptEntry(entry) => {
                self.entries.push(entry);
            }
            ResponseEvent::Finished
            | ResponseEvent::Newline
            | ResponseEvent::StartResponse
            | ResponseEvent::ToolStart { .. }
            | ResponseEvent::ToolResult { .. }
            | ResponseEvent::HookDebug { .. }
            | ResponseEvent::FuelStatus { .. }
            | ResponseEvent::FuelExhausted { .. }
            | ResponseEvent::ContextWarning { .. }
            | ResponseEvent::ToolDiagnostic { .. }
            | ResponseEvent::InboxInjected { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collecting_sink_text() {
        let mut sink = CollectingSink::new();
        sink.handle(ResponseEvent::TextChunk("Hello ")).unwrap();
        sink.handle(ResponseEvent::TextChunk("World")).unwrap();
        assert_eq!(sink.text, "Hello World");
    }

    #[test]
    fn test_collecting_sink_ignores_fuel_exhausted() {
        let mut sink = CollectingSink::new();
        sink.handle(ResponseEvent::FuelExhausted { total: 10 }).unwrap();
        assert_eq!(sink.text, "");
        assert_eq!(sink.reasoning, "");
        assert!(sink.entries.is_empty());
    }
}
