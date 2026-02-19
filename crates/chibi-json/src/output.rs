use chibi_core::OutputSink;
use chibi_core::context::TranscriptEntry;
use chibi_core::output::CommandEvent;
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
            CommandEvent::ContextLoaded { tool_count } => {
                serde_json::json!({"type": "context_loaded", "tool_count": tool_count})
            }
            CommandEvent::McpToolsLoaded { count } => {
                serde_json::json!({"type": "mcp_tools_loaded", "count": count})
            }
            CommandEvent::McpBridgeUnavailable { reason } => {
                serde_json::json!({"type": "mcp_bridge_unavailable", "reason": reason})
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
}
