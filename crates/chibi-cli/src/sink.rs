//! CLI-specific response sink for terminal output.
//!
//! This module implements `ResponseSink` for the CLI, handling markdown
//! rendering and diagnostic output.

use chibi_core::api::sink::{ResponseEvent, ResponseSink};
use std::io;

use chibi_core::OutputSink;

use crate::markdown::{MarkdownConfig, MarkdownStream};

/// CLI-specific response sink for terminal output.
///
/// Connects the core API's event system to CLI presentation concerns:
/// - Renders text chunks through `MarkdownStream` when enabled
/// - Handles diagnostic messages through `OutputHandler`
/// - Manages stream lifecycle (finish, newlines)
pub struct CliResponseSink<'a> {
    output: &'a dyn OutputSink,
    markdown: Option<MarkdownStream>,
    markdown_config: Option<MarkdownConfig>,
    verbose: bool,
    /// Whether to display tool call diagnostics (independent of verbose)
    show_tool_calls: bool,
    /// Whether to display thinking/reasoning content
    show_thinking: bool,
    /// Whether we're currently inside a reasoning/thinking block
    in_reasoning: bool,
}

impl<'a> CliResponseSink<'a> {
    /// Create a new CLI response sink.
    ///
    /// # Arguments
    /// * `output` - The output handler for diagnostic and JSON output
    /// * `markdown_config` - Optional config for creating markdown streams
    /// * `verbose` - Whether verbose diagnostics should be shown
    /// * `show_tool_calls` - Whether tool call start/result messages are shown
    /// * `show_thinking` - Whether thinking/reasoning content is shown
    pub fn new(
        output: &'a dyn OutputSink,
        markdown_config: Option<MarkdownConfig>,
        verbose: bool,
        show_tool_calls: bool,
        show_thinking: bool,
    ) -> Self {
        let markdown = markdown_config
            .as_ref()
            .map(|cfg| MarkdownStream::new(cfg.clone()));
        Self {
            output,
            markdown,
            markdown_config,
            verbose,
            show_tool_calls,
            show_thinking,
            in_reasoning: false,
        }
    }
}

impl CliResponseSink<'_> {
    /// Close an open reasoning/thinking block by emitting the closing tag.
    fn close_reasoning(&mut self) -> io::Result<()> {
        if let Some(md) = &mut self.markdown {
            md.write_chunk("\n</think>\n")?;
        }
        self.in_reasoning = false;
        Ok(())
    }
}

impl ResponseSink for CliResponseSink<'_> {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(chunk) => {
                if self.in_reasoning {
                    self.close_reasoning()?;
                }
                if let Some(md) = &mut self.markdown {
                    md.write_chunk(chunk)?;
                }
            }
            ResponseEvent::Reasoning(chunk) => {
                if self.show_thinking
                    && let Some(md) = &mut self.markdown
                {
                    if !self.in_reasoning {
                        md.write_chunk("<think>\n")?;
                        self.in_reasoning = true;
                    }
                    md.write_chunk(chunk)?;
                }
            }
            ResponseEvent::TranscriptEntry(_entry) => {
                // CLI displays content via streaming events (TextChunk, ToolStart, ToolResult).
                // TranscriptEntry is for structured consumers (JSON mode, show_log).
            }
            ResponseEvent::Finished => {
                if self.in_reasoning {
                    self.close_reasoning()?;
                }
                if let Some(mut md) = self.markdown.take() {
                    md.finish()?;
                }
            }
            ResponseEvent::Newline => {
                self.output.newline();
            }
            ResponseEvent::ToolStart { name, summary } => {
                let msg = match summary {
                    Some(s) => format!("\n[Tool: {}] {}", name, s),
                    None => format!("\n[Tool: {}]", name),
                };
                self.output.diagnostic(&msg, self.show_tool_calls);
            }
            ResponseEvent::ToolResult { name, cached, .. } => {
                if cached {
                    self.output
                        .diagnostic(&format!("\n[Tool {} (cached)]", name), self.show_tool_calls);
                }
            }
            ResponseEvent::StartResponse => {
                // Create a fresh markdown stream for the new response
                self.markdown = self
                    .markdown_config
                    .as_ref()
                    .map(|cfg| MarkdownStream::new(cfg.clone()));
                self.in_reasoning = false;
            }
            ResponseEvent::HookDebug { message, .. } => {
                if self.verbose {
                    self.output.diagnostic_always(&message);
                }
            }
            ResponseEvent::FuelStatus { remaining, total, event } => {
                if self.verbose {
                    use chibi_core::api::sink::FuelEvent;
                    let msg = match event {
                        FuelEvent::EnteringTurn =>
                            format!("[fuel: {}/{} entering turn]", remaining, total),
                        FuelEvent::AfterToolBatch =>
                            format!("[fuel: {}/{} after tool batch]", remaining, total),
                        FuelEvent::AfterContinuation { prompt_preview } =>
                            format!("[continuing (fuel: {}/{}): {}]", remaining, total, prompt_preview),
                        FuelEvent::EmptyResponse =>
                            format!("[empty response, fuel: {}/{}]", remaining, total),
                    };
                    self.output.diagnostic_always(&msg);
                }
            }
            ResponseEvent::FuelExhausted { total } => {
                self.output.diagnostic_always(
                    &format!("[fuel exhausted (0/{}), returning control to user]", total),
                );
            }
            ResponseEvent::ContextWarning { tokens_remaining } => {
                if self.verbose {
                    self.output.diagnostic_always(
                        &format!("[Context window warning: {} tokens remaining]", tokens_remaining),
                    );
                }
            }
            ResponseEvent::ToolDiagnostic { message, .. } => {
                if self.verbose {
                    self.output.diagnostic_always(&message);
                }
            }
            ResponseEvent::InboxInjected { count } => {
                if self.verbose {
                    self.output.diagnostic_always(
                        &format!("[Inbox: {} message(s) injected]", count),
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputHandler;
    use chibi_core::context::TranscriptEntry;

    #[test]
    fn test_handle_text_chunk_no_markdown() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        // Should not panic when no markdown stream is present
        sink.handle(ResponseEvent::TextChunk("test")).unwrap();
    }

    #[test]
    fn test_handle_finished_no_markdown() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        // Should not panic when no markdown stream is present
        sink.handle(ResponseEvent::Finished).unwrap();
    }

    #[test]
    fn test_handle_transcript_entry() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "user".to_string(),
            to: "assistant".to_string(),
            content: "Hello".to_string(),
            entry_type: "message".to_string(),
            metadata: None,
            tool_call_id: None,
        };

        // TranscriptEntry is a no-op in CLI sink (content displayed via streaming events)
        sink.handle(ResponseEvent::TranscriptEntry(entry)).unwrap();
    }

    #[test]
    fn test_handle_fuel_status_verbose_false() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        // Verbose is false — FuelStatus should be suppressed (no panic)
        use chibi_core::api::sink::FuelEvent;
        sink.handle(ResponseEvent::FuelStatus {
            remaining: 3,
            total: 10,
            event: FuelEvent::EnteringTurn,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_fuel_exhausted_always_shown() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        // FuelExhausted is always shown regardless of verbose
        sink.handle(ResponseEvent::FuelExhausted { total: 10 }).unwrap();
    }

    #[test]
    fn test_handle_newline() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

        // Should not panic
        sink.handle(ResponseEvent::Newline).unwrap();
    }

    #[test]
    fn test_handle_tool_start() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, true, true, false);

        // Should not panic
        sink.handle(ResponseEvent::ToolStart {
            name: "test_tool".to_string(),
            summary: None,
        })
        .unwrap();
    }

    #[test]
    fn test_handle_tool_result_cached() {
        let output = OutputHandler::new(false);
        let mut sink = CliResponseSink::new(&output, None, true, true, false);

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
        let mut sink = CliResponseSink::new(&output, None, true, true, false);

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
        let mut sink = CliResponseSink::new(&output, None, false, true, false);

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
        let mut sink = CliResponseSink::new(&output, Some(config), false, true, false);

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
