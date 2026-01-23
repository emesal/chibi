//! Transcript entry creation helpers.
//!
//! Factory methods for creating different types of transcript entries.
//! These are extracted for potential future use in migrating AppState methods.

#![allow(dead_code)]

use crate::context::{
    ENTRY_TYPE_ARCHIVAL, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_CONTEXT_CREATED, ENTRY_TYPE_MESSAGE,
    ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT, EntryMetadata, TranscriptEntry,
};

/// Create a user message entry
pub fn create_user_message_entry(
    current_context: &str,
    content: &str,
    username: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(username)
        .to(current_context)
        .content(content)
        .entry_type(ENTRY_TYPE_MESSAGE)
        .build()
}

/// Create an assistant message entry
pub fn create_assistant_message_entry(current_context: &str, content: &str) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(current_context)
        .to("user")
        .content(content)
        .entry_type(ENTRY_TYPE_MESSAGE)
        .build()
}

/// Create a tool call entry
pub fn create_tool_call_entry(
    current_context: &str,
    tool_name: &str,
    arguments: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(current_context)
        .to(tool_name)
        .content(arguments)
        .entry_type(ENTRY_TYPE_TOOL_CALL)
        .build()
}

/// Create a tool result entry
pub fn create_tool_result_entry(
    current_context: &str,
    tool_name: &str,
    result: &str,
) -> TranscriptEntry {
    TranscriptEntry::builder()
        .from(tool_name)
        .to(current_context)
        .content(result)
        .entry_type(ENTRY_TYPE_TOOL_RESULT)
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
            hash: None,
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
