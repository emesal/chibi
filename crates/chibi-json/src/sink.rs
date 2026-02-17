use chibi_core::api::sink::{ResponseEvent, ResponseSink};
use std::io::{self, Write};

/// JSONL response sink for chibi-json.
///
/// Emits complete transcript entries and diagnostics as JSONL.
/// No streaming partial text — programmatic consumers want complete records.
pub struct JsonResponseSink;

impl JsonResponseSink {
    pub fn new() -> Self {
        Self
    }
}

impl ResponseSink for JsonResponseSink {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(_) => {
                // Text chunks are not emitted — the complete response arrives
                // via TranscriptEntry, which is the authoritative record.
            }
            ResponseEvent::Reasoning(_) => {
                // Reasoning not emitted in JSON mode
            }
            ResponseEvent::TranscriptEntry(entry) => {
                let json = serde_json::to_string(&entry)?;
                println!("{}", json);
                io::stdout().flush()?;
            }
            ResponseEvent::Finished => {}
            ResponseEvent::Diagnostic {
                message,
                verbose_only,
            } => {
                // Always emit diagnostics in JSON mode — programmatic consumers can filter
                // on the `verbose_only` field. Silently dropping them loses information.
                let json = serde_json::json!({
                    "type": "diagnostic",
                    "content": message,
                    "verbose_only": verbose_only,
                });
                eprintln!("{}", json);
            }
            ResponseEvent::ToolStart { name, summary } => {
                let json = serde_json::json!({
                    "type": "tool_start",
                    "name": name,
                    "summary": summary,
                });
                eprintln!("{}", json);
            }
            ResponseEvent::ToolResult {
                name,
                result,
                cached,
            } => {
                let json = serde_json::json!({
                    "type": "tool_result",
                    "name": name,
                    "result": result,
                    "cached": cached,
                });
                eprintln!("{}", json);
            }
            ResponseEvent::Newline | ResponseEvent::StartResponse => {}
        }
        Ok(())
    }
}
