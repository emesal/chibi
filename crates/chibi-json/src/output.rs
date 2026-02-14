use chibi_core::OutputSink;
use chibi_core::context::TranscriptEntry;
use std::io::{self, Write};

/// JSONL output sink for chibi-json.
///
/// Results go to stdout as JSONL, diagnostics go to stderr as JSONL.
/// Confirmation always returns true (trust mode).
pub struct JsonOutputSink;

impl OutputSink for JsonOutputSink {
    fn emit_result(&self, content: &str) {
        let json = serde_json::json!({"type": "result", "content": content});
        println!("{}", json);
    }

    fn diagnostic(&self, message: &str, verbose: bool) {
        if verbose {
            let json = serde_json::json!({"type": "diagnostic", "content": message});
            eprintln!("{}", json);
        }
    }

    fn diagnostic_always(&self, message: &str) {
        let json = serde_json::json!({"type": "diagnostic", "content": message});
        eprintln!("{}", json);
    }

    fn newline(&self) {
        // no-op in JSON mode -- whitespace is meaningless
    }

    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()> {
        let json = serde_json::to_string(entry)?;
        println!("{}", json);
        io::stdout().flush()?;
        Ok(())
    }

    fn is_json_mode(&self) -> bool {
        true
    }

    fn confirm(&self, _prompt: &str) -> bool {
        true // trust mode -- programmatic callers have already decided
    }

    fn emit_markdown(&self, content: &str) -> io::Result<()> {
        self.emit_result(content);
        Ok(())
    }
}
