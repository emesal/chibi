use crate::context::TranscriptEntry;
use std::io;

/// Abstraction over how command results and diagnostics are presented.
///
/// chibi-cli implements this with OutputHandler (text to stdout/stderr, interactive TTY).
/// chibi-json implements this with JsonOutputSink (JSONL to stdout/stderr, auto-approve).
pub trait OutputSink {
    /// Emit a result string (the primary output of a command).
    fn emit_result(&self, content: &str);

    /// Emit a diagnostic message. Only shown when `verbose` is true.
    fn diagnostic(&self, message: &str, verbose: bool);

    /// Emit a diagnostic message unconditionally.
    fn diagnostic_always(&self, message: &str);

    /// Emit a blank line.
    fn newline(&self);

    /// Emit a transcript entry (for JSON-mode structured output).
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;

    /// Whether this sink operates in JSON mode (affects downstream formatting).
    fn is_json_mode(&self) -> bool;

    /// Prompt the user for confirmation. Returns true if confirmed.
    /// JSON-mode implementations should auto-approve (return true).
    fn confirm(&self, prompt: &str) -> bool;

    /// Emit content that may contain markdown.
    ///
    /// CLI renders this via streamdown; JSON emits raw text.
    /// The default implementation falls back to `emit_result()`.
    fn emit_markdown(&self, content: &str) -> io::Result<()> {
        self.emit_result(content);
        Ok(())
    }
}
