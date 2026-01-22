//! Output handling for normal and JSON modes.
//!
//! This module provides `OutputHandler` which handles outputting
//! transcript entries and diagnostics in either plain text or JSONL format.

use crate::context::{TranscriptEntry, now_timestamp};
use std::io::{self, Write};
use uuid::Uuid;

/// Handles output in normal or JSON mode.
///
/// In JSON mode (`--json-output`), all output is JSONL:
/// - stdout: TranscriptEntry records (user messages, tool calls, tool results, assistant responses)
/// - stderr: Diagnostic entries (type: "diagnostic")
///
/// In normal mode:
/// - stdout: Streamed assistant response text
/// - stderr: Diagnostic messages (verbose output)
pub struct OutputHandler {
    json_mode: bool,
}

impl OutputHandler {
    /// Create a new output handler.
    pub fn new(json_mode: bool) -> Self {
        Self { json_mode }
    }

    /// Check if we're in JSON mode.
    pub fn is_json_mode(&self) -> bool {
        self.json_mode
    }

    /// Emit a transcript entry to stdout (JSONL in JSON mode, nothing in normal mode).
    ///
    /// In JSON mode, this outputs the entry as a JSON line.
    /// In normal mode, this is a no-op (the caller handles streaming).
    pub fn emit(&self, entry: &TranscriptEntry) -> io::Result<()> {
        if self.json_mode {
            let json = serde_json::to_string(entry)
                .map_err(|e| io::Error::other(format!("Failed to serialize entry: {}", e)))?;
            println!("{}", json);
            io::stdout().flush()?;
        }
        Ok(())
    }

    /// Emit a diagnostic message to stderr.
    ///
    /// In JSON mode, this outputs a JSONL entry with type "diagnostic".
    /// In normal mode, this outputs plain text if verbose is enabled.
    pub fn diagnostic(&self, message: &str, verbose: bool) {
        if !verbose {
            return;
        }

        if self.json_mode {
            let entry = TranscriptEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now_timestamp(),
                from: "system".to_string(),
                to: "user".to_string(),
                content: message.to_string(),
                entry_type: "diagnostic".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&entry) {
                eprintln!("{}", json);
            }
        } else {
            eprintln!("{}", message);
        }
    }

    /// Emit a diagnostic message unconditionally (ignores verbose flag).
    ///
    /// Use this for errors or critical information that should always be shown.
    pub fn diagnostic_always(&self, message: &str) {
        if self.json_mode {
            let entry = TranscriptEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now_timestamp(),
                from: "system".to_string(),
                to: "user".to_string(),
                content: message.to_string(),
                entry_type: "diagnostic".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&entry) {
                eprintln!("{}", json);
            }
        } else {
            eprintln!("{}", message);
        }
    }

    /// Print an empty line (normal mode only).
    pub fn newline(&self) {
        if !self.json_mode {
            println!();
        }
    }

    /// Emit a result entry for operations like list_contexts, delete, etc.
    ///
    /// In JSON mode, this outputs a JSONL entry with the result.
    /// In normal mode, this just prints the content.
    pub fn emit_result(&self, content: &str) {
        if self.json_mode {
            let entry = TranscriptEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now_timestamp(),
                from: "system".to_string(),
                to: "user".to_string(),
                content: content.to_string(),
                entry_type: "result".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&entry) {
                println!("{}", json);
            }
        } else {
            println!("{}", content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_normal_mode() {
        let handler = OutputHandler::new(false);
        assert!(!handler.is_json_mode());
    }

    #[test]
    fn test_new_json_mode() {
        let handler = OutputHandler::new(true);
        assert!(handler.is_json_mode());
    }

    #[test]
    fn test_emit_json_mode() {
        let handler = OutputHandler::new(true);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "user".to_string(),
            to: "assistant".to_string(),
            content: "Hello".to_string(),
            entry_type: "message".to_string(),
        };
        // Should not panic
        let _ = handler.emit(&entry);
    }

    #[test]
    fn test_emit_normal_mode() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "user".to_string(),
            to: "assistant".to_string(),
            content: "Hello".to_string(),
            entry_type: "message".to_string(),
        };
        // Should not panic (no-op in normal mode)
        let _ = handler.emit(&entry);
    }

    #[test]
    fn test_diagnostic_verbose_false() {
        let handler = OutputHandler::new(false);
        // Should not panic (no output when verbose is false)
        handler.diagnostic("Test message", false);
    }

    #[test]
    fn test_diagnostic_verbose_true() {
        let handler = OutputHandler::new(false);
        // Should not panic
        handler.diagnostic("Test message", true);
    }
}
