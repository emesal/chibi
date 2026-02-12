//! Entry creation helpers for transcript entries.
//!
//! These are pure constructors with no I/O - they create TranscriptEntry values
//! using the builder pattern.

use crate::context::{
    ENTRY_TYPE_ARCHIVAL, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_CONTEXT_CREATED, ENTRY_TYPE_MESSAGE,
    ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT, EntryMetadata, TranscriptEntry,
};

/// Create a transcript entry for a user message
pub fn create_user_message_entry(
    context_name: &str,
    content: &str,
    username: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(username)
        .to(context_name)
        .content(content)
        .entry_type(ENTRY_TYPE_MESSAGE)
        .build()
}

/// Create a transcript entry for an assistant message
pub fn create_assistant_message_entry(context_name: &str, content: &str) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(context_name)
        .to("user")
        .content(content)
        .entry_type(ENTRY_TYPE_MESSAGE)
        .build()
}

/// Create a transcript entry for a tool call
pub fn create_tool_call_entry(
    context_name: &str,
    tool_name: &str,
    arguments: &str,
    tool_call_id: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(context_name)
        .to(tool_name)
        .content(arguments)
        .entry_type(ENTRY_TYPE_TOOL_CALL)
        .tool_call_id(tool_call_id)
        .build()
}

/// Create a transcript entry for a tool result
pub fn create_tool_result_entry(
    context_name: &str,
    tool_name: &str,
    result: &str,
    tool_call_id: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(tool_name)
        .to(context_name)
        .content(result)
        .entry_type(ENTRY_TYPE_TOOL_RESULT)
        .tool_call_id(tool_call_id)
        .build()
}

/// Create a context_created anchor entry
pub fn create_context_created_anchor(context_name: &str) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from("system")
        .to(context_name)
        .content("Context created")
        .entry_type(ENTRY_TYPE_CONTEXT_CREATED)
        .build()
}

/// Create a compaction anchor entry with summary
pub fn create_compaction_anchor(context_name: &str, summary: &str) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from("system")
        .to(context_name)
        .content("Context compacted")
        .entry_type(ENTRY_TYPE_COMPACTION)
        .metadata(EntryMetadata {
            summary: Some(summary.to_string()),
            transcript_anchor_id: None,
        })
        .build()
}

/// Create an archival anchor entry
pub fn create_archival_anchor(context_name: &str) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from("system")
        .to(context_name)
        .content("Context archived/cleared")
        .entry_type(ENTRY_TYPE_ARCHIVAL)
        .build()
}
