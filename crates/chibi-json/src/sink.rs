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
            ResponseEvent::HookDebug { hook, message } => {
                eprintln!("{}", serde_json::json!({
                    "type": "hook_debug",
                    "hook": hook,
                    "message": message,
                }));
            }
            ResponseEvent::FuelStatus { remaining, total, event } => {
                use chibi_core::api::sink::FuelEvent;
                let event_str = match &event {
                    FuelEvent::EnteringTurn => "entering_turn",
                    FuelEvent::AfterToolBatch => "after_tool_batch",
                    FuelEvent::AfterContinuation { .. } => "after_continuation",
                    FuelEvent::EmptyResponse => "empty_response",
                };
                let mut j = serde_json::json!({
                    "type": "fuel_status",
                    "remaining": remaining,
                    "total": total,
                    "event": event_str,
                });
                if let FuelEvent::AfterContinuation { prompt_preview } = event {
                    j["prompt_preview"] = serde_json::json!(prompt_preview);
                }
                eprintln!("{}", j);
            }
            ResponseEvent::FuelExhausted { total } => {
                eprintln!("{}", serde_json::json!({
                    "type": "fuel_exhausted",
                    "total": total,
                }));
            }
            ResponseEvent::ContextWarning { tokens_remaining } => {
                eprintln!("{}", serde_json::json!({
                    "type": "context_warning",
                    "tokens_remaining": tokens_remaining,
                }));
            }
            ResponseEvent::ToolDiagnostic { tool, message } => {
                eprintln!("{}", serde_json::json!({
                    "type": "tool_diagnostic",
                    "tool": tool,
                    "message": message,
                }));
            }
            ResponseEvent::InboxInjected { count } => {
                eprintln!("{}", serde_json::json!({
                    "type": "inbox_injected",
                    "count": count,
                }));
            }
        }
        Ok(())
    }
}
