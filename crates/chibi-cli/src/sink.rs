//! CLI-specific response sink for terminal output.
//!
//! This module implements `ResponseSink` for the CLI, handling markdown
//! rendering, diagnostic output, and JSON mode.

use chibi_core::api::sink::{ResponseEvent, ResponseSink};
use std::io;

use crate::markdown::{MarkdownConfig, MarkdownStream};
use crate::output::OutputHandler;

/// CLI-specific response sink for terminal output.
///
/// Connects the core API's event system to CLI presentation concerns:
/// - Renders text chunks through `MarkdownStream` when enabled
/// - Handles diagnostic messages through `OutputHandler`
/// - Emits transcript entries in JSON mode
/// - Manages stream lifecycle (finish, newlines)
pub struct CliResponseSink<'a> {
    output: &'a OutputHandler,
    markdown: Option<MarkdownStream>,
    markdown_config: Option<MarkdownConfig>,
    verbose: bool,
}

impl<'a> CliResponseSink<'a> {
    /// Create a new CLI response sink.
    ///
    /// # Arguments
    /// * `output` - The output handler for diagnostic and JSON output
    /// * `markdown_config` - Optional config for creating markdown streams
    /// * `verbose` - Whether verbose diagnostics should be shown
    pub fn new(
        output: &'a OutputHandler,
        markdown_config: Option<MarkdownConfig>,
        verbose: bool,
    ) -> Self {
        let markdown = markdown_config
            .as_ref()
            .map(|cfg| MarkdownStream::new(cfg.clone()));
        Self {
            output,
            markdown,
            markdown_config,
            verbose,
        }
    }
}

impl ResponseSink for CliResponseSink<'_> {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(chunk) => {
                if let Some(md) = &mut self.markdown {
                    md.write_chunk(chunk)?;
                }
            }
            ResponseEvent::Diagnostic {
                message,
                verbose_only,
            } => {
                if verbose_only {
                    self.output.diagnostic(&message, self.verbose);
                } else {
                    self.output.diagnostic_always(&message);
                }
            }
            ResponseEvent::TranscriptEntry(entry) => {
                self.output.emit(&entry)?;
            }
            ResponseEvent::Finished => {
                if let Some(mut md) = self.markdown.take() {
                    md.finish()?;
                }
            }
            ResponseEvent::Newline => {
                self.output.newline();
            }
            ResponseEvent::ToolStart { name } => {
                self.output
                    .diagnostic(&format!("[Tool: {}]", name), self.verbose);
            }
            ResponseEvent::ToolResult { name, cached, .. } => {
                if cached {
                    self.output
                        .diagnostic(&format!("[Tool {} (cached)]", name), self.verbose);
                }
            }
            ResponseEvent::StartResponse => {
                // Create a fresh markdown stream for the new response
                self.markdown = self
                    .markdown_config
                    .as_ref()
                    .map(|cfg| MarkdownStream::new(cfg.clone()));
            }
        }
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        self.output.is_json_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chibi_core::context::TranscriptEntry;

    #[test]
    fn test_is_json_mode_normal() {
        let output = OutputHandler::new(false);
        let sink = CliResponseSink::new(&output, None, false);
        assert!(!sink.is_json_mode());
    }

    #[test]
    fn test_is_json_mode_json() {
        let output = OutputHandler::new(true);
        let sink = CliResponseSink::new(&output, None, false);
        assert!(sink.is_json_mode());
    }

    #[test]
    fn test_handle_text_chunk_no_markdown() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic when no markdown stream is present
        sink.handle(ResponseEvent::TextChunk("test")).unwrap();
    }

    #[test]
    fn test_handle_finished_no_markdown() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic when no markdown stream is present
        sink.handle(ResponseEvent::Finished).unwrap();
    }

    #[test]
    fn test_handle_transcript_entry_json_mode() {
        let output = OutputHandler::new(true);
        let mut sink = CliResponseSink::new(&output, None, false);

        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "user".to_string(),
            to: "assistant".to_string(),
            content: "Hello".to_string(),
            entry_type: "message".to_string(),
            metadata: None,
        };

        // Should not panic
        sink.handle(ResponseEvent::TranscriptEntry(entry)).unwrap();
    }

    #[test]
    fn test_handle_diagnostic_verbose_false() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic (verbose is false, message should be suppressed)
        sink.handle(ResponseEvent::Diagnostic {
            message: "test".to_string(),
            verbose_only: true,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_diagnostic_always() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic (verbose_only: false means always show)
        sink.handle(ResponseEvent::Diagnostic {
            message: "error".to_string(),
            verbose_only: false,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_newline() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic
        sink.handle(ResponseEvent::Newline).unwrap();
    }

    #[test]
    fn test_handle_tool_start() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, true);

        // Should not panic
        sink.handle(ResponseEvent::ToolStart {
            name: "test_tool".to_string(),
        })
        .unwrap();
    }

    #[test]
    fn test_handle_tool_result_cached() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, true);

        // Should not panic
        sink.handle(ResponseEvent::ToolResult {
            name: "test_tool".to_string(),
            result: "result".to_string(),
            cached: true,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_tool_result_not_cached() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, true);

        // Should not panic (non-cached results don't print extra message)
        sink.handle(ResponseEvent::ToolResult {
            name: "test_tool".to_string(),
            result: "result".to_string(),
            cached: false,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_start_response_no_config() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false);

        // Should not panic when no markdown config is present
        sink.handle(ResponseEvent::StartResponse).unwrap();
        assert!(sink.markdown.is_none());
    }

    #[test]
    fn test_start_response_recreates_markdown_stream() {
        use crate::config::{ImageConfig, default_markdown_style};

        let output = OutputHandler::new(false);
        let config = MarkdownConfig {
            render_markdown: true,
            force_render: true, // Force render even when not a TTY (for tests)
            image: ImageConfig::default(),
            image_cache_dir: None,
            markdown_style: default_markdown_style(),
        };
        let mut sink = CliResponseSink::new(&output, Some(config), false);

        // Initially has a markdown stream
        assert!(sink.markdown.is_some());

        // Simulate finishing a response (consumes the stream)
        sink.handle(ResponseEvent::Finished).unwrap();
        assert!(sink.markdown.is_none());

        // StartResponse should recreate the stream
        sink.handle(ResponseEvent::StartResponse).unwrap();
        assert!(sink.markdown.is_some());
    }
}
