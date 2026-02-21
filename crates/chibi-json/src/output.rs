use chibi_core::OutputSink;
use chibi_core::context::TranscriptEntry;
use chibi_core::output::CommandEvent;
use std::io::{self, Write};

/// Map `io::ErrorKind` to a stable coarse-grained error code string.
fn error_code(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::InvalidData => "invalid_data",
        io::ErrorKind::AlreadyExists => "already_exists",
        _ => "internal_error",
    }
}

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

    fn emit_event(&self, event: CommandEvent) {
        let json = match event {
            CommandEvent::AutoDestroyed { count } => {
                serde_json::json!({"type": "auto_destroyed", "count": count})
            }
            CommandEvent::CacheCleanup {
                removed,
                max_age_days,
            } => serde_json::json!({"type": "cache_cleanup", "removed": removed,
                                   "max_age_days": max_age_days}),
            CommandEvent::SystemPromptSet { context } => {
                serde_json::json!({"type": "system_prompt_set", "context": context})
            }
            CommandEvent::UsernameSaved { username, context } => {
                serde_json::json!({"type": "username_saved", "username": username,
                                   "context": context})
            }
            CommandEvent::ModelSet { model, context } => {
                serde_json::json!({"type": "model_set", "model": model, "context": context})
            }
            CommandEvent::InboxEmpty { context } => {
                serde_json::json!({"type": "inbox_empty", "context": context})
            }
            CommandEvent::InboxProcessing { count, context } => {
                serde_json::json!({"type": "inbox_processing", "count": count,
                                   "context": context})
            }
            CommandEvent::AllInboxesEmpty => serde_json::json!({"type": "all_inboxes_empty"}),
            CommandEvent::InboxesProcessed { count } => {
                serde_json::json!({"type": "inboxes_processed", "count": count})
            }
            CommandEvent::McpToolsLoaded { count } => {
                serde_json::json!({"type": "mcp_tools_loaded", "count": count})
            }
            CommandEvent::McpBridgeUnavailable { reason } => {
                serde_json::json!({"type": "mcp_bridge_unavailable", "reason": reason})
            }
            CommandEvent::CompactionStarted {
                context,
                message_count,
            } => serde_json::json!({"type": "compaction_started", "context": context,
                                   "message_count": message_count}),
            CommandEvent::CompactionComplete {
                context,
                archived,
                remaining,
            } => serde_json::json!({"type": "compaction_complete", "context": context,
                                   "archived": archived, "remaining": remaining}),
            CommandEvent::RollingCompactionDecision { archived } => {
                serde_json::json!({"type": "rolling_compaction_decision", "archived": archived})
            }
            CommandEvent::RollingCompactionFallback { drop_percentage } => {
                serde_json::json!({"type": "rolling_compaction_fallback",
                                   "drop_percentage": drop_percentage})
            }
            CommandEvent::RollingCompactionComplete {
                archived,
                remaining,
            } => {
                serde_json::json!({"type": "rolling_compaction_complete",
                                   "archived": archived, "remaining": remaining})
            }
            CommandEvent::CompactionNoPrompt => {
                serde_json::json!({"type": "compaction_no_prompt"})
            }
            CommandEvent::LoadSummary {
                builtin_count,
                builtin_names,
                plugin_count,
                plugin_names,
            } => serde_json::json!({"type": "load_summary", "builtin_count": builtin_count,
                                   "builtin_names": builtin_names, "plugin_count": plugin_count,
                                   "plugin_names": plugin_names}),
        };
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

    fn confirm(&self, _prompt: &str) -> bool {
        true // trust mode -- programmatic callers have already decided
    }

    fn emit_markdown(&self, content: &str) -> io::Result<()> {
        self.emit_result(content);
        Ok(())
    }

    fn emit_done(&self, result: &io::Result<()>) {
        let json = match result {
            Ok(()) => serde_json::json!({"type": "done", "ok": true}),
            Err(e) => serde_json::json!({
                "type": "done",
                "ok": false,
                "code": error_code(e),
                "message": e.to_string(),
            }),
        };
        eprintln!("{}", json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn error_code_not_found() {
        let e = io::Error::new(io::ErrorKind::NotFound, "x");
        assert_eq!(error_code(&e), "not_found");
    }

    #[test]
    fn error_code_invalid_input() {
        let e = io::Error::new(io::ErrorKind::InvalidInput, "x");
        assert_eq!(error_code(&e), "invalid_input");
    }

    #[test]
    fn error_code_permission_denied() {
        let e = io::Error::new(io::ErrorKind::PermissionDenied, "x");
        assert_eq!(error_code(&e), "permission_denied");
    }

    #[test]
    fn error_code_invalid_data() {
        let e = io::Error::new(io::ErrorKind::InvalidData, "x");
        assert_eq!(error_code(&e), "invalid_data");
    }

    #[test]
    fn error_code_already_exists() {
        let e = io::Error::new(io::ErrorKind::AlreadyExists, "x");
        assert_eq!(error_code(&e), "already_exists");
    }

    #[test]
    fn error_code_fallback() {
        let e = io::Error::new(io::ErrorKind::BrokenPipe, "x");
        assert_eq!(error_code(&e), "internal_error");
    }
}
