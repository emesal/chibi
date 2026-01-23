use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind};
use std::path::PathBuf;
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextState {
    pub contexts: Vec<String>,
    pub current_context: String,
}

impl ContextState {
    pub fn switch_context(&mut self, name: String) -> io::Result<()> {
        validate_context_name(&name)?;
        self.current_context = name;
        Ok(())
    }

    pub fn save(&self, state_path: &PathBuf) -> io::Result<()> {
        use std::fs::OpenOptions;
        use std::io::BufWriter;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(state_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| io::Error::other(format!("Failed to save state: {}", e)))?;
        Ok(())
    }
}

pub fn is_valid_context_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn validate_context_name(name: &str) -> io::Result<()> {
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

// System prompt entry type (context.jsonl[1])
pub const ENTRY_TYPE_SYSTEM_PROMPT: &str = "system_prompt";

// Change event (transcript only)
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
    /// Create a new transcript entry with auto-generated ID and current timestamp
    #[allow(dead_code)]
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
        entry_type: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: from.into(),
            to: to.into(),
            content: content.into(),
            entry_type: entry_type.into(),
            metadata: None,
        }
    }

    /// Create a new transcript entry with metadata
    #[allow(dead_code)]
    pub fn with_metadata(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
        entry_type: impl Into<String>,
        metadata: EntryMetadata,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: from.into(),
            to: to.into(),
            content: content.into(),
            entry_type: entry_type.into(),
            metadata: Some(metadata),
        }
    }

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
    /// SHA256 hash for system prompt entries
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Reference to the transcript entry ID this anchor corresponds to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_anchor_id: Option<String>,
}

/// Metadata for context (stored in context_meta.json)
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextMeta {
    pub created_at: u64,
}

impl Default for ContextMeta {
    fn default() -> Self {
        Self {
            created_at: now_timestamp(),
        }
    }
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
            contexts: vec!["default".to_string()],
            current_context: "default".to_string(),
        };
        assert!(state.switch_context("new-context".to_string()).is_ok());
        assert_eq!(state.current_context, "new-context");
    }

    #[test]
    fn test_context_state_switch_invalid() {
        let mut state = ContextState {
            contexts: vec!["default".to_string()],
            current_context: "default".to_string(),
        };
        assert!(state.switch_context("invalid name".to_string()).is_err());
        // Should not have changed
        assert_eq!(state.current_context, "default");
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
}
