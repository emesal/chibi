//! Output handling for the CLI.
//!
//! This module provides `OutputHandler` which handles outputting
//! text results and diagnostics for the terminal.
//! Implements `OutputSink` directly — all output goes through trait methods.

use chibi_core::OutputSink;
use chibi_core::context::TranscriptEntry;
use std::io::{self, IsTerminal, Write};

/// CLI output handler — text to stdout, diagnostics to stderr.
///
/// Implements `OutputSink` directly; all output goes through trait methods.
/// Always operates in text mode — JSON output belongs to chibi-json.
pub struct OutputHandler;

impl Default for OutputHandler {
    fn default() -> Self {
        Self
    }
}

impl OutputHandler {
    /// Create a new output handler.
    pub fn new() -> Self {
        Self
    }
}

impl OutputSink for OutputHandler {
    fn emit_result(&self, content: &str) {
        println!("{}", content);
    }

    fn diagnostic(&self, message: &str, verbose: bool) {
        if verbose {
            eprintln!("{}", message);
        }
    }

    fn diagnostic_always(&self, message: &str) {
        eprintln!("{}", message);
    }

    fn newline(&self) {
        println!();
    }

    fn emit_entry(&self, _entry: &TranscriptEntry) -> io::Result<()> {
        // Text mode: transcript entries are handled by the response sink,
        // not emitted directly. No-op.
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        false
    }

    fn confirm(&self, prompt: &str) -> bool {
        let stdin = io::stdin();
        if !stdin.is_terminal() {
            return false;
        }

        eprint!("{} [y/N] ", prompt);
        io::stderr().flush().ok();

        let mut input = String::new();
        if stdin.read_line(&mut input).is_err() {
            return false;
        }

        matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
    }

    fn emit_markdown(&self, content: &str) -> io::Result<()> {
        self.emit_result(content);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_text_mode() {
        let handler = OutputHandler::new();
        assert!(!handler.is_json_mode());
    }

    #[test]
    fn test_emit_entry_noop() {
        let handler = OutputHandler::new();
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
        // Should not panic (no-op in text mode)
        let _ = handler.emit_entry(&entry);
    }

    #[test]
    fn test_diagnostic_verbose_false() {
        let handler = OutputHandler::new();
        // Should not panic (no output when verbose is false)
        handler.diagnostic("Test message", false);
    }

    #[test]
    fn test_diagnostic_verbose_true() {
        let handler = OutputHandler::new();
        // Should not panic
        handler.diagnostic("Test message", true);
    }
}
