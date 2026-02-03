//! Response sink abstraction for decoupling API from presentation.
//!
//! This module defines the `ResponseSink` trait that allows the API module
//! to emit events without knowing about specific presentation implementations
//! (like terminal markdown rendering or JSON output).

use crate::context::TranscriptEntry;
use std::io;

/// Events emitted during prompt processing.
///
/// These events represent the various outputs that occur during an API
/// interaction, from text streaming to tool execution status.
#[derive(Debug, Clone)]
pub enum ResponseEvent<'a> {
    /// A chunk of text content from the streaming response.
    TextChunk(&'a str),

    /// A diagnostic message (typically shown in verbose mode or always).
    Diagnostic {
        message: String,
        /// If true, only show when verbose mode is enabled.
        verbose_only: bool,
    },

    /// A transcript entry to be logged/displayed.
    TranscriptEntry(TranscriptEntry),

    /// A tool has started execution.
    ToolStart { name: String },

    /// A tool has completed execution.
    ToolResult {
        name: String,
        result: String,
        cached: bool,
    },

    /// The response stream has finished.
    Finished,

    /// A newline should be emitted (typically after response completion).
    Newline,

    /// A new response is starting (used to reset state between agentic loop iterations).
    StartResponse,
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

    /// Returns true if the sink is in JSON output mode.
    ///
    /// When in JSON mode, text chunks should typically not be streamed
    /// to the terminal, as the output will be formatted as JSON instead.
    fn is_json_mode(&self) -> bool {
        false
    }
}

/// A sink that collects responses for programmatic use.
///
/// Useful for testing or when you need to capture the response
/// without any terminal output.
#[derive(Debug, Default)]
pub struct CollectingSink {
    /// Accumulated text content from the response.
    pub text: String,
    /// Transcript entries emitted during the interaction.
    pub entries: Vec<TranscriptEntry>,
    /// Diagnostic messages emitted.
    pub diagnostics: Vec<String>,
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
            ResponseEvent::TranscriptEntry(entry) => {
                self.entries.push(entry);
            }
            ResponseEvent::Diagnostic { message, .. } => {
                self.diagnostics.push(message);
            }
            ResponseEvent::Finished | ResponseEvent::Newline | ResponseEvent::StartResponse => {}
            ResponseEvent::ToolStart { .. } | ResponseEvent::ToolResult { .. } => {}
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
    fn test_collecting_sink_diagnostics() {
        let mut sink = CollectingSink::new();
        sink.handle(ResponseEvent::Diagnostic {
            message: "test message".to_string(),
            verbose_only: true,
        })
        .unwrap();
        assert_eq!(sink.diagnostics.len(), 1);
        assert_eq!(sink.diagnostics[0], "test message");
    }

    #[test]
    fn test_is_json_mode_default() {
        let sink = CollectingSink::new();
        assert!(!sink.is_json_mode());
    }
}
