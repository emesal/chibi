use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind};
use std::path::Path;
use uuid::Uuid;

/// Generate a new UUID for message IDs (used as serde default)
fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub role: String,
    pub content: String,
}

impl Message {
    /// Create a new message with auto-generated ID
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: generate_uuid(),
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Context {
    pub name: String,
    pub messages: Vec<Message>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Summary of conversation history (loaded from separate file, not serialized)
    #[serde(skip)]
    pub summary: String,
}

impl Context {
    /// Create a new empty context with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            messages: Vec::new(),
            created_at: now_timestamp(),
            updated_at: 0,
            summary: String::new(),
        }
    }
}

/// Metadata entry for a context in state.json
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ContextEntry {
    pub name: String,
    pub created_at: u64,
    /// Last time this context was used (0 = never updated)
    #[serde(default)]
    pub last_activity_at: u64,
    /// Auto-destroy after this many seconds of inactivity (0 = disabled)
    #[serde(default)]
    pub destroy_after_seconds_inactive: u64,
    /// Auto-destroy at this timestamp (0 = disabled)
    #[serde(default)]
    pub destroy_at: u64,
}

impl ContextEntry {
    #[allow(dead_code)]
    pub fn new(name: impl Into<String>) -> Self {
        let now = now_timestamp();
        Self {
            name: name.into(),
            created_at: now,
            last_activity_at: now,
            destroy_after_seconds_inactive: 0,
            destroy_at: 0,
        }
    }

    pub fn with_created_at(name: impl Into<String>, created_at: u64) -> Self {
        Self {
            name: name.into(),
            created_at,
            last_activity_at: 0,
            destroy_after_seconds_inactive: 0,
            destroy_at: 0,
        }
    }

    /// Update last_activity_at to the current time
    pub fn touch(&mut self) {
        self.last_activity_at = now_timestamp();
    }

    /// Check if this context should be auto-destroyed
    pub fn should_auto_destroy(&self) -> bool {
        let now = now_timestamp();

        // Check destroy_at (absolute timestamp)
        if self.destroy_at >= 1 && now > self.destroy_at {
            return true;
        }

        // Check inactivity timeout
        if self.destroy_after_seconds_inactive >= 1 && self.last_activity_at >= 1 {
            let deadline = self.last_activity_at + self.destroy_after_seconds_inactive;
            if now > deadline {
                return true;
            }
        }

        false
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextState {
    pub contexts: Vec<ContextEntry>,
    pub current_context: String,
    #[serde(default)]
    pub previous_context: Option<String>,
}

impl ContextState {
    pub fn switch_context(&mut self, name: String) -> io::Result<()> {
        validate_context_name(&name)?;
        if self.current_context != name {
            self.previous_context = Some(self.current_context.clone());
        }
        self.current_context = name;
        Ok(())
    }

    pub fn save(&self, state_path: &Path) -> io::Result<()> {
        crate::safe_io::atomic_write_json(state_path, self)
    }
}

pub fn is_valid_context_name(name: &str) -> bool {
    // Reject reserved name
    if name == "-" {
        return false;
    }
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn validate_context_name(name: &str) -> io::Result<()> {
    if name == "-" {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Invalid context name '-'. This is a reserved name used to reference the previous context.",
        ));
    }
    if !is_valid_context_name(name) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Invalid context name '{}'. Names must be alphanumeric with dashes and underscores only.",
                name
            ),
        ));
    }
    Ok(())
}

pub fn now_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_secs()
}

// Entry type constants
pub const ENTRY_TYPE_MESSAGE: &str = "message";
pub const ENTRY_TYPE_TOOL_CALL: &str = "tool_call";
pub const ENTRY_TYPE_TOOL_RESULT: &str = "tool_result";

// Anchor entry types (context.jsonl[0])
pub const ENTRY_TYPE_CONTEXT_CREATED: &str = "context_created";
pub const ENTRY_TYPE_COMPACTION: &str = "compaction";
pub const ENTRY_TYPE_ARCHIVAL: &str = "archival";

// Change event (transcript only) - logs full raw prompt content
pub const ENTRY_TYPE_SYSTEM_PROMPT_CHANGED: &str = "system_prompt_changed";

/// Entry for JSONL transcript file (now also context.jsonl)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub id: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub content: String,
    pub entry_type: String,
    /// Optional metadata for anchor entries (summary, hash, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<EntryMetadata>,
}

impl TranscriptEntry {
    /// Create a builder for constructing transcript entries
    pub fn builder() -> TranscriptEntryBuilder {
        TranscriptEntryBuilder::default()
    }
}

/// Builder for creating TranscriptEntry instances with a fluent API.
/// All fields have sensible defaults: auto-generated ID, current timestamp,
/// empty strings for from/to/content, and "message" for entry_type.
#[derive(Default)]
pub struct TranscriptEntryBuilder {
    from: Option<String>,
    to: Option<String>,
    content: Option<String>,
    entry_type: Option<String>,
    metadata: Option<EntryMetadata>,
}

impl TranscriptEntryBuilder {
    /// Set the source/sender of the entry
    pub fn from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// Set the destination/recipient of the entry
    pub fn to(mut self, to: impl Into<String>) -> Self {
        self.to = Some(to.into());
        self
    }

    /// Set the content of the entry
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Set the entry type (e.g., "message", "tool_call", "tool_result")
    pub fn entry_type(mut self, entry_type: impl Into<String>) -> Self {
        self.entry_type = Some(entry_type.into());
        self
    }

    /// Set metadata for the entry
    pub fn metadata(mut self, metadata: EntryMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Build the TranscriptEntry with auto-generated ID and current timestamp
    pub fn build(self) -> TranscriptEntry {
        TranscriptEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: self.from.unwrap_or_default(),
            to: self.to.unwrap_or_default(),
            content: self.content.unwrap_or_default(),
            entry_type: self
                .entry_type
                .unwrap_or_else(|| ENTRY_TYPE_MESSAGE.to_string()),
            metadata: self.metadata,
        }
    }
}

/// Metadata for special entries (anchors, system prompts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMetadata {
    /// Summary content for compaction anchors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Reference to the transcript entry ID this anchor corresponds to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_anchor_id: Option<String>,
}

/// Metadata for context (stored in context_meta.json)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextMeta {
    /// Tracks mtime of system_prompt.md to detect external edits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_md_mtime: Option<u64>,
    /// The full combined prompt last sent to API (raw + hook injections + todos/goals/etc)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_combined_prompt: Option<String>,
}

/// Entry for inbox.jsonl file - messages from other contexts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxEntry {
    pub id: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_context_names() {
        assert!(is_valid_context_name("default"));
        assert!(is_valid_context_name("my-context"));
        assert!(is_valid_context_name("my_context"));
        assert!(is_valid_context_name("MyContext123"));
        assert!(is_valid_context_name("a"));
        assert!(is_valid_context_name("context-with-dashes"));
        assert!(is_valid_context_name("context_with_underscores"));
        assert!(is_valid_context_name("MixedCase-And_123"));
    }

    #[test]
    fn test_invalid_context_names() {
        assert!(!is_valid_context_name(""));
        assert!(!is_valid_context_name("has spaces"));
        assert!(!is_valid_context_name("has.dots"));
        assert!(!is_valid_context_name("has/slash"));
        assert!(!is_valid_context_name("has\\backslash"));
        assert!(!is_valid_context_name("has:colon"));
        assert!(!is_valid_context_name("emoji🎉"));
        assert!(!is_valid_context_name("café"));
        assert!(!is_valid_context_name("日本語"));
    }

    #[test]
    fn test_validate_context_name_ok() {
        assert!(validate_context_name("valid-name").is_ok());
        assert!(validate_context_name("another_valid_123").is_ok());
    }

    #[test]
    fn test_validate_context_name_error() {
        let result = validate_context_name("invalid name");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Invalid context name"));
    }

    #[test]
    fn test_validate_context_name_rejects_dash() {
        let result = validate_context_name("-");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("reserved name"));
        assert!(err.to_string().contains("previous context"));
    }

    #[test]
    fn test_now_timestamp_is_reasonable() {
        let ts = now_timestamp();
        // Should be after Jan 1, 2024 (1704067200)
        assert!(ts > 1704067200);
        // Should be before Jan 1, 2100 (4102444800)
        assert!(ts < 4102444800);
    }

    #[test]
    fn test_context_state_switch_valid() {
        let mut state = ContextState {
            contexts: vec![ContextEntry::new("default")],
            current_context: "default".to_string(),
            previous_context: None,
        };
        assert!(state.switch_context("new-context".to_string()).is_ok());
        assert_eq!(state.current_context, "new-context");
        assert_eq!(state.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_context_state_switch_invalid() {
        let mut state = ContextState {
            contexts: vec![ContextEntry::new("default")],
            current_context: "default".to_string(),
            previous_context: None,
        };
        assert!(state.switch_context("invalid name".to_string()).is_err());
        // Should not have changed
        assert_eq!(state.current_context, "default");
        assert_eq!(state.previous_context, None);
    }

    #[test]
    fn test_context_state_switch_same_context() {
        let mut state = ContextState {
            contexts: vec![ContextEntry::new("default")],
            current_context: "default".to_string(),
            previous_context: Some("old".to_string()),
        };
        // Switching to the same context should not update previous_context
        assert!(state.switch_context("default".to_string()).is_ok());
        assert_eq!(state.current_context, "default");
        assert_eq!(state.previous_context, Some("old".to_string()));
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::new("user", "Hello, world!");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("Hello, world!"));
        assert!(json.contains("id")); // Should have auto-generated id

        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, "user");
        assert!(!parsed.id.is_empty()); // ID should be present
        assert_eq!(parsed.content, "Hello, world!");
    }

    #[test]
    fn test_inbox_entry_serialization() {
        let entry = InboxEntry {
            id: "test-id".to_string(),
            timestamp: 1234567890,
            from: "context-a".to_string(),
            to: "context-b".to_string(),
            content: "Test message".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: InboxEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test-id");
        assert_eq!(parsed.timestamp, 1234567890);
        assert_eq!(parsed.from, "context-a");
        assert_eq!(parsed.to, "context-b");
        assert_eq!(parsed.content, "Test message");
    }

    // === Context name validation edge cases ===

    #[test]
    fn test_context_name_all_dashes() {
        // Names like "--" or "---" are technically valid by current rules
        // (alphanumeric OR dash OR underscore) but might be problematic
        // Question: Should these be valid? They could conflict with CLI flags.
        // Current behavior: valid (all chars are dashes, which are allowed)
        assert!(is_valid_context_name("--"));
        assert!(is_valid_context_name("---"));
    }

    #[test]
    fn test_context_name_all_underscores() {
        // Similar edge case with underscores
        assert!(is_valid_context_name("_"));
        assert!(is_valid_context_name("__"));
        assert!(is_valid_context_name("___"));
    }

    #[test]
    fn test_context_name_leading_dash() {
        // Leading dash could be problematic with CLI parsing
        assert!(is_valid_context_name("-mycontext"));
        // Single dash is reserved for previous context reference
        assert!(!is_valid_context_name("-"));
    }

    #[test]
    fn test_context_name_leading_underscore() {
        assert!(is_valid_context_name("_mycontext"));
        assert!(is_valid_context_name("_hidden"));
    }

    #[test]
    fn test_context_name_numeric_only() {
        // All-numeric names should be valid
        assert!(is_valid_context_name("123"));
        assert!(is_valid_context_name("0"));
        assert!(is_valid_context_name("999999"));
    }

    #[test]
    fn test_context_name_very_long() {
        // Very long names - should be valid but might cause filesystem issues
        let long_name = "a".repeat(255); // Max filename length on most filesystems
        assert!(is_valid_context_name(&long_name));
    }

    #[test]
    fn test_context_name_reserved_words() {
        // "new" is special in the CLI for auto-generating context names
        // but should still be valid as an actual context name
        assert!(is_valid_context_name("new"));
        // "default" is the default context name
        assert!(is_valid_context_name("default"));
    }

    #[test]
    fn test_context_name_with_numbers_at_start() {
        assert!(is_valid_context_name("123abc"));
        assert!(is_valid_context_name("1-context"));
        assert!(is_valid_context_name("2_test"));
    }

    #[test]
    fn test_context_name_mixed_case_preserved() {
        // Verify mixed case names work (filesystem dependent)
        assert!(is_valid_context_name("MyContext"));
        assert!(is_valid_context_name("ALLCAPS"));
        assert!(is_valid_context_name("lowercase"));
    }

    // === TranscriptEntry builder tests ===

    #[test]
    fn test_transcript_entry_builder_defaults() {
        let entry = TranscriptEntry::builder().build();
        assert!(!entry.id.is_empty()); // UUID generated
        assert!(entry.timestamp > 0); // Timestamp set
        assert_eq!(entry.from, ""); // Empty default
        assert_eq!(entry.to, ""); // Empty default
        assert_eq!(entry.content, ""); // Empty default
        assert_eq!(entry.entry_type, ENTRY_TYPE_MESSAGE); // Default type
        assert!(entry.metadata.is_none());
    }

    #[test]
    fn test_transcript_entry_builder_full() {
        let metadata = EntryMetadata {
            summary: Some("test summary".to_string()),
            transcript_anchor_id: None,
        };
        let entry = TranscriptEntry::builder()
            .from("sender")
            .to("receiver")
            .content("hello")
            .entry_type(ENTRY_TYPE_TOOL_CALL)
            .metadata(metadata)
            .build();

        assert_eq!(entry.from, "sender");
        assert_eq!(entry.to, "receiver");
        assert_eq!(entry.content, "hello");
        assert_eq!(entry.entry_type, ENTRY_TYPE_TOOL_CALL);
        assert!(entry.metadata.is_some());
        assert_eq!(
            entry.metadata.unwrap().summary,
            Some("test summary".to_string())
        );
    }

    #[test]
    fn test_transcript_entry_builder() {
        let entry = TranscriptEntry::builder()
            .from("from")
            .to("to")
            .content("content")
            .entry_type("custom_type")
            .build();
        assert_eq!(entry.from, "from");
        assert_eq!(entry.to, "to");
        assert_eq!(entry.content, "content");
        assert_eq!(entry.entry_type, "custom_type");
        assert!(!entry.id.is_empty());
        assert!(entry.timestamp > 0);
    }

    #[test]
    fn test_transcript_entry_builder_with_metadata() {
        let metadata = EntryMetadata {
            summary: Some("summary".to_string()),
            transcript_anchor_id: Some("anchor-id".to_string()),
        };
        let entry = TranscriptEntry::builder()
            .from("from")
            .to("to")
            .content("content")
            .entry_type("type")
            .metadata(metadata)
            .build();
        assert!(entry.metadata.is_some());
        let m = entry.metadata.unwrap();
        assert_eq!(m.summary, Some("summary".to_string()));
        assert_eq!(m.transcript_anchor_id, Some("anchor-id".to_string()));
    }

    // === Context struct tests ===

    #[test]
    fn test_context_new_initializes_correctly() {
        let ctx = Context::new("test-ctx");
        assert_eq!(ctx.name, "test-ctx");
        assert!(ctx.messages.is_empty());
        assert!(ctx.created_at > 0);
        assert_eq!(ctx.updated_at, 0); // Not updated yet
        assert!(ctx.summary.is_empty());
    }

    #[test]
    fn test_context_meta_default() {
        let meta = ContextMeta::default();
        assert!(meta.system_prompt_md_mtime.is_none());
        assert!(meta.last_combined_prompt.is_none());
    }

    // === EntryMetadata tests ===

    #[test]
    fn test_entry_metadata_serialization_skips_none() {
        let metadata = EntryMetadata {
            summary: Some("test".to_string()),
            transcript_anchor_id: None,
        };
        let json = serde_json::to_string(&metadata).unwrap();
        // Should not contain "transcript_anchor_id" key when None
        assert!(json.contains("summary"));
        assert!(!json.contains("transcript_anchor_id"));
    }

    // === ContextEntry auto-destroy tests ===

    #[test]
    fn test_context_entry_new_initializes_all_fields() {
        let entry = ContextEntry::new("test");
        assert_eq!(entry.name, "test");
        assert!(entry.created_at > 0);
        assert!(entry.last_activity_at > 0);
        assert_eq!(entry.destroy_after_seconds_inactive, 0);
        assert_eq!(entry.destroy_at, 0);
    }

    #[test]
    fn test_context_entry_with_created_at_defaults_auto_destroy_fields() {
        let entry = ContextEntry::with_created_at("test", 1234567890);
        assert_eq!(entry.name, "test");
        assert_eq!(entry.created_at, 1234567890);
        assert_eq!(entry.last_activity_at, 0);
        assert_eq!(entry.destroy_after_seconds_inactive, 0);
        assert_eq!(entry.destroy_at, 0);
    }

    #[test]
    fn test_context_entry_touch_updates_last_activity() {
        let mut entry = ContextEntry::with_created_at("test", 1234567890);
        assert_eq!(entry.last_activity_at, 0);
        entry.touch();
        assert!(entry.last_activity_at > 0);
    }

    #[test]
    fn test_context_entry_should_auto_destroy_disabled_by_default() {
        let entry = ContextEntry::new("test");
        // Both destroy_after_seconds_inactive and destroy_at are 0 (disabled)
        assert!(!entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_should_auto_destroy_by_timestamp() {
        let mut entry = ContextEntry::new("test");
        // Set destroy_at to a past timestamp
        entry.destroy_at = 1; // Way in the past
        assert!(entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_should_auto_destroy_by_timestamp_future() {
        let mut entry = ContextEntry::new("test");
        // Set destroy_at to a future timestamp (year 2100)
        entry.destroy_at = 4102444800;
        assert!(!entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_should_auto_destroy_by_inactivity() {
        let mut entry = ContextEntry::new("test");
        // Set last_activity_at to way in the past
        entry.last_activity_at = 1;
        // Set a small inactivity timeout
        entry.destroy_after_seconds_inactive = 60;
        // Now (current time) > last_activity_at + 60 seconds
        assert!(entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_should_not_auto_destroy_if_active() {
        let mut entry = ContextEntry::new("test");
        // Touch to set last_activity_at to now
        entry.touch();
        // Set a large inactivity timeout (1 hour)
        entry.destroy_after_seconds_inactive = 3600;
        // Should not auto-destroy since activity was just now
        assert!(!entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_should_auto_destroy_inactivity_requires_activity() {
        let mut entry = ContextEntry::with_created_at("test", 1);
        // last_activity_at is 0 (never touched)
        entry.destroy_after_seconds_inactive = 60;
        // Should NOT auto-destroy because last_activity_at is 0
        assert!(!entry.should_auto_destroy());
    }

    #[test]
    fn test_context_entry_serde_defaults() {
        // Simulate deserializing an old state.json without the new fields
        let json = r#"{"name":"test","created_at":1234567890}"#;
        let entry: ContextEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "test");
        assert_eq!(entry.created_at, 1234567890);
        assert_eq!(entry.last_activity_at, 0); // Defaults to 0
        assert_eq!(entry.destroy_after_seconds_inactive, 0); // Defaults to 0
        assert_eq!(entry.destroy_at, 0); // Defaults to 0
    }

    #[test]
    fn test_context_entry_serde_round_trip() {
        let mut entry = ContextEntry::new("test");
        entry.destroy_after_seconds_inactive = 3600;
        entry.destroy_at = 1234567890;

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ContextEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, entry.name);
        assert_eq!(parsed.created_at, entry.created_at);
        assert_eq!(parsed.last_activity_at, entry.last_activity_at);
        assert_eq!(
            parsed.destroy_after_seconds_inactive,
            entry.destroy_after_seconds_inactive
        );
        assert_eq!(parsed.destroy_at, entry.destroy_at);
    }

    #[test]
    fn test_context_state_save_atomic() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let state_path = temp_dir.path().join("state.json");

        let state = ContextState {
            contexts: vec![ContextEntry::new("default")],
            current_context: "default".to_string(),
            previous_context: None,
        };

        state.save(&state_path).unwrap();

        // Verify the file exists and is valid JSON
        let content = std::fs::read_to_string(&state_path).unwrap();
        let parsed: ContextState = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.current_context, "default");

        // Verify no temp files left behind (temp files use .tmp.{id} extension)
        let parent = state_path.parent().unwrap();
        let tmp_files: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(tmp_files.is_empty(), "temp files should not remain");
    }

    #[test]
    fn test_context_state_save_concurrent_writes() {
        use std::sync::Arc;
        use std::thread;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let state_path = Arc::new(temp_dir.path().join("state.json"));

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let path = Arc::clone(&state_path);
                thread::spawn(move || {
                    let state = ContextState {
                        contexts: vec![ContextEntry::new(&format!("ctx-{}", i))],
                        current_context: format!("ctx-{}", i),
                        previous_context: None,
                    };
                    state.save(&path).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // The file should be valid JSON (one of the writes won)
        let content = std::fs::read_to_string(&*state_path).unwrap();
        let parsed: ContextState = serde_json::from_str(&content).unwrap();
        assert!(parsed.current_context.starts_with("ctx-"));
    }
}
