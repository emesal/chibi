use chibi_core::api::sink::{ResponseEvent, ResponseSink};
use std::io::{self, Write};

/// JSONL response sink for chibi-json.
///
/// Accumulates text chunks and emits complete transcript entries as JSONL.
/// No streaming partial text -- programmatic consumers want complete records.
pub struct JsonResponseSink {
    accumulated_text: String,
}

impl JsonResponseSink {
    pub fn new() -> Self {
        Self {
            accumulated_text: String::new(),
        }
    }
}

impl ResponseSink for JsonResponseSink {
    fn handle(&mut self, event: ResponseEvent<'_>) -> io::Result<()> {
        match event {
            ResponseEvent::TextChunk(text) => {
                self.accumulated_text.push_str(text);
            }
            ResponseEvent::Reasoning(_) => {
                // Reasoning not emitted in JSON mode
            }
            ResponseEvent::TranscriptEntry(entry) => {
                let json = serde_json::to_string(&entry)?;
                println!("{}", json);
                io::stdout().flush()?;
            }
            ResponseEvent::Finished => {
                self.accumulated_text.clear();
            }
            ResponseEvent::Diagnostic {
                message,
                verbose_only,
            } => {
                if !verbose_only {
                    let json = serde_json::json!({"type": "diagnostic", "content": message});
                    eprintln!("{}", json);
                }
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

    fn is_json_mode(&self) -> bool {
        true
    }
}
