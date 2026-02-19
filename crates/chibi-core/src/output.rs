use crate::context::TranscriptEntry;
use std::io;

/// Semantic events emitted on the command path (non-streaming).
///
/// Core emits all variants unconditionally; clients decide which to display
/// and how to format them. Verbose-tier events are shown only when the client
/// has verbose mode enabled.
#[derive(Debug, Clone)]
pub enum CommandEvent {
    /// Expired contexts auto-destroyed on startup (verbose-tier).
    AutoDestroyed { count: usize },
    /// Old cache entries removed on startup (verbose-tier).
    CacheCleanup { removed: usize, max_age_days: u64 },
    /// System prompt saved for a context (verbose-tier).
    SystemPromptSet { context: String },
    /// Username saved for a context (verbose-tier).
    UsernameSaved { username: String, context: String },
    /// No inbox messages for context (verbose-tier).
    InboxEmpty { context: String },
    /// Inbox messages being processed (verbose-tier).
    InboxProcessing { count: usize, context: String },
    /// All inboxes empty (verbose-tier).
    AllInboxesEmpty,
    /// Processed N context inboxes (verbose-tier).
    InboxesProcessed { count: usize },
    /// Context loaded with N tools (verbose-tier).
    ContextLoaded { tool_count: usize },
    /// MCP bridge tools loaded successfully (verbose-tier).
    McpToolsLoaded { count: usize },
    /// MCP bridge unavailable at load time (verbose-tier).
    McpBridgeUnavailable { reason: String },
    /// Summary of all tools available after load (verbose-tier).
    LoadSummary {
        builtin_count: usize,
        builtin_names: Vec<String>,
        plugin_count: usize,
        plugin_names: Vec<String>,
    },
    /// LLM-based compaction started (verbose-tier).
    CompactionStarted { context: String, message_count: usize },
    /// LLM-based compaction completed (verbose-tier).
    CompactionComplete { context: String, archived: usize, remaining: usize },
    /// Rolling compaction: LLM selected N messages to archive (verbose-tier).
    RollingCompactionDecision { archived: usize },
    /// Rolling compaction fallback: dropping oldest N% (verbose-tier).
    RollingCompactionFallback { drop_percentage: f64 },
    /// Rolling compaction completed (verbose-tier).
    RollingCompactionComplete { archived: usize, remaining: usize },
    /// No compaction prompt found — using default (verbose-tier).
    CompactionNoPrompt,
}

/// Abstraction over how command results and diagnostics are presented.
///
/// chibi-cli implements this with OutputHandler (text to stdout/stderr, interactive TTY).
/// chibi-json implements this with JsonOutputSink (JSONL to stdout/stderr, auto-approve).
pub trait OutputSink {
    /// Emit a result string (the primary output of a command).
    fn emit_result(&self, content: &str);

    /// Emit a typed command-path event. Clients filter and format as appropriate.
    fn emit_event(&self, event: CommandEvent);

    /// Emit a blank line.
    fn newline(&self);

    /// Emit a transcript entry for display.
    ///
    /// Each sink formats entries appropriately for its output medium:
    /// CLI renders human-readable text, JSON emits structured JSONL.
    fn emit_entry(&self, entry: &TranscriptEntry) -> io::Result<()>;

    /// Prompt the user for confirmation. Returns true if confirmed.
    /// Programmatic sinks (e.g. chibi-json) should auto-approve (return true).
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

/// A no-op output sink for call sites that don't need command-path output.
pub(crate) struct NoopSink;

impl OutputSink for NoopSink {
    fn emit_result(&self, _: &str) {}
    fn emit_event(&self, _: CommandEvent) {}
    fn newline(&self) {}
    fn emit_entry(&self, _: &TranscriptEntry) -> io::Result<()> {
        Ok(())
    }
    fn confirm(&self, _: &str) -> bool {
        false
    }
}
