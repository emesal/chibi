//! Output handling for the CLI.
//!
//! This module provides `OutputHandler` which handles outputting
//! text results and diagnostics for the terminal.
//! Implements `OutputSink` directly — all output goes through trait methods.

use chibi_core::OutputSink;
use chibi_core::context::TranscriptEntry;
use chibi_core::output::CommandEvent;
use std::io::{self, IsTerminal, Write};

/// CLI output handler — text to stdout, diagnostics to stderr.
///
/// Implements `OutputSink` directly; all output goes through trait methods.
/// Always operates in text mode — JSON output belongs to chibi-json.
#[derive(Default)]
pub struct OutputHandler {
    verbose: bool,
}

impl OutputHandler {
    /// Create a new output handler.
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl OutputSink for OutputHandler {
    fn emit_result(&self, content: &str) {
        println!("{}", content);
    }

    fn emit_event(&self, event: CommandEvent) {
        if !self.verbose {
            return;
        }
        let text = match &event {
            CommandEvent::AutoDestroyed { count } => {
                format!("[Auto-destroyed {} expired context(s)]", count)
            }
            CommandEvent::CacheCleanup {
                removed,
                max_age_days,
            } => format!(
                "[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                removed,
                max_age_days + 1
            ),
            CommandEvent::SystemPromptSet { context } => {
                format!("[System prompt set for context '{}']", context)
            }
            CommandEvent::UsernameSaved { username, context } => {
                format!("[Username '{}' saved to context '{}']", username, context)
            }
            CommandEvent::InboxEmpty { context } => {
                format!("[No messages in inbox for '{}']", context)
            }
            CommandEvent::InboxProcessing { count, context } => format!(
                "[Processing {} message(s) from inbox for '{}']",
                count, context
            ),
            CommandEvent::AllInboxesEmpty => "[No messages in any inbox.]".to_string(),
            CommandEvent::InboxesProcessed { count } => {
                format!("[Processed inboxes for {} context(s).]", count)
            }
            CommandEvent::McpToolsLoaded { count } => format!("[MCP: {} tools loaded]", count),
            CommandEvent::McpBridgeUnavailable { reason } => {
                format!("[MCP: bridge unavailable: {}]", reason)
            }
            CommandEvent::CompactionStarted {
                context,
                message_count,
            } => format!("[Compacting '{}': {} messages]", context, message_count),
            CommandEvent::CompactionComplete {
                context,
                archived,
                remaining,
            } => format!(
                "[Compaction complete '{}': {} archived, {} remaining]",
                context, archived, remaining
            ),
            CommandEvent::RollingCompactionDecision { archived } => {
                format!("[Rolling compaction: LLM selected {} messages to archive]", archived)
            }
            CommandEvent::RollingCompactionFallback { drop_percentage } => format!(
                "[Rolling compaction: LLM decision failed, falling back to dropping oldest {}%]",
                drop_percentage
            ),
            CommandEvent::RollingCompactionComplete { archived, remaining } => format!(
                "[Rolling compaction complete: {} archived, {} remaining]",
                archived, remaining
            ),
            CommandEvent::CompactionNoPrompt => {
                "[No compaction prompt found — using default]".to_string()
            }
            CommandEvent::LoadSummary {
                builtin_count,
                builtin_names,
                plugin_count,
                plugin_names,
            } => {
                let mut lines = format!(
                    "[Built-in ({}): {}]",
                    builtin_count,
                    builtin_names.join(", ")
                );
                if *plugin_count == 0 {
                    lines.push_str("\n[No plugins loaded]");
                } else {
                    lines.push_str(&format!(
                        "\n[Plugins ({}): {}]",
                        plugin_count,
                        plugin_names.join(", ")
                    ));
                }
                lines
            }
        };
        eprintln!("{}", text);
    }

    fn newline(&self) {
        println!();
    }

    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()> {
        use chibi_core::context;

        match entry.entry_type.as_str() {
            context::ENTRY_TYPE_MESSAGE => {
                self.emit_result(&format!("[{}]", entry.from.to_uppercase()));
                self.emit_markdown(&entry.content)?;
                self.newline();
            }
            context::ENTRY_TYPE_TOOL_CALL => {
                if self.verbose {
                    self.emit_result(&format!("[TOOL CALL: {}]\n{}\n", entry.to, entry.content));
                } else {
                    let args_preview = if entry.content.len() > 60 {
                        format!("{}...", &entry.content[..60])
                    } else {
                        entry.content.clone()
                    };
                    self.emit_result(&format!("[TOOL: {}] {}", entry.to, args_preview));
                }
            }
            context::ENTRY_TYPE_TOOL_RESULT => {
                if self.verbose {
                    self.emit_result(&format!(
                        "[TOOL RESULT: {}]\n{}\n",
                        entry.from, entry.content
                    ));
                } else {
                    let size = entry.content.len();
                    let size_str = if size > 1024 {
                        format!("{:.1}kb", size as f64 / 1024.0)
                    } else {
                        format!("{}b", size)
                    };
                    self.emit_result(&format!("  -> {}", size_str));
                }
            }
            "compaction" => {
                if self.verbose {
                    self.emit_result(&format!("[COMPACTION]: {}\n", entry.content));
                }
            }
            _ => {
                if self.verbose {
                    self.emit_result(&format!(
                        "[{}]: {}\n",
                        entry.entry_type.to_uppercase(),
                        entry.content
                    ));
                }
            }
        }
        Ok(())
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
    fn test_emit_entry_message() {
        let handler = OutputHandler::new(false);
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
        // Should not panic — formats as human-readable text
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_call_compact() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "assistant".to_string(),
            to: "shell_exec".to_string(),
            content: r#"{"command":"ls"}"#.to_string(),
            entry_type: "tool_call".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Compact mode: shows tool name + truncated args
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_call_verbose() {
        let handler = OutputHandler::new(true);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "assistant".to_string(),
            to: "shell_exec".to_string(),
            content: r#"{"command":"ls -la"}"#.to_string(),
            entry_type: "tool_call".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Verbose mode: shows full content
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_tool_result_compact() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "shell_exec".to_string(),
            to: "assistant".to_string(),
            content: "file1.rs\nfile2.rs\n".to_string(),
            entry_type: "tool_result".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Compact mode: shows size only
        handler.emit_entry(&entry).unwrap();
    }

    #[test]
    fn test_emit_entry_skips_compaction_when_not_verbose() {
        let handler = OutputHandler::new(false);
        let entry = TranscriptEntry {
            id: "test-id".to_string(),
            timestamp: 12345,
            from: "system".to_string(),
            to: "system".to_string(),
            content: "compacted 10 entries".to_string(),
            entry_type: "compaction".to_string(),
            metadata: None,
            tool_call_id: None,
        };
        // Non-verbose: compaction entries are silently skipped
        handler.emit_entry(&entry).unwrap();
    }

    // ── emit_event tests ─────────────────────────────────────────────────────

    #[test]
    fn test_emit_event_verbose_false_suppresses_output() {
        // All CommandEvent variants are verbose-tier; non-verbose handler must suppress.
        // We can't capture stderr in unit tests, but we verify no panic occurs
        // and the verbose guard is respected.
        let handler = OutputHandler::new(false);
        handler.emit_event(CommandEvent::AutoDestroyed { count: 3 });
        handler.emit_event(CommandEvent::AllInboxesEmpty);
        handler.emit_event(CommandEvent::McpBridgeUnavailable {
            reason: "timeout".to_string(),
        });
    }

    #[test]
    fn test_emit_event_verbose_true_does_not_panic() {
        // Exercise every variant to catch format string regressions.
        let handler = OutputHandler::new(true);
        handler.emit_event(CommandEvent::AutoDestroyed { count: 0 });
        handler.emit_event(CommandEvent::CacheCleanup {
            removed: 5,
            max_age_days: 6,
        });
        handler.emit_event(CommandEvent::SystemPromptSet {
            context: "work".to_string(),
        });
        handler.emit_event(CommandEvent::UsernameSaved {
            username: "alice".to_string(),
            context: "work".to_string(),
        });
        handler.emit_event(CommandEvent::InboxEmpty {
            context: "work".to_string(),
        });
        handler.emit_event(CommandEvent::InboxProcessing {
            count: 2,
            context: "work".to_string(),
        });
        handler.emit_event(CommandEvent::AllInboxesEmpty);
        handler.emit_event(CommandEvent::InboxesProcessed { count: 3 });
        handler.emit_event(CommandEvent::McpToolsLoaded { count: 7 });
        handler.emit_event(CommandEvent::McpBridgeUnavailable {
            reason: "connection refused".to_string(),
        });
        // LoadSummary: no plugins
        handler.emit_event(CommandEvent::LoadSummary {
            builtin_count: 4,
            builtin_names: vec!["a".to_string(), "b".to_string()],
            plugin_count: 0,
            plugin_names: vec![],
        });
        // LoadSummary: with plugins
        handler.emit_event(CommandEvent::LoadSummary {
            builtin_count: 2,
            builtin_names: vec!["a".to_string()],
            plugin_count: 3,
            plugin_names: vec!["p1".to_string(), "p2".to_string(), "p3".to_string()],
        });
        // Compaction events
        handler.emit_event(CommandEvent::CompactionStarted {
            context: "default".to_string(),
            message_count: 42,
        });
        handler.emit_event(CommandEvent::CompactionComplete {
            context: "default".to_string(),
            archived: 40,
            remaining: 2,
        });
        handler.emit_event(CommandEvent::RollingCompactionDecision { archived: 10 });
        handler.emit_event(CommandEvent::RollingCompactionFallback {
            drop_percentage: 30.0,
        });
        handler.emit_event(CommandEvent::RollingCompactionComplete {
            archived: 8,
            remaining: 12,
        });
        handler.emit_event(CommandEvent::CompactionNoPrompt);
    }
}
