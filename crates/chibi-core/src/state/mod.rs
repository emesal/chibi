//! Application state management.
//!
//! This module manages all persistent state for chibi:
//! - Context files and directories
//! - Configuration loading and resolution
//! - Transcript and inbox operations

mod config_resolution;
mod context_ops;
mod entries;
mod paths;
mod prompts;

pub use entries::{
    create_archival_anchor, create_assistant_message_entry, create_compaction_anchor,
    create_context_created_anchor, create_tool_call_entry, create_tool_result_entry,
    create_user_message_entry,
};
pub use paths::StatePaths;

use crate::jsonl::read_jsonl_file;

use crate::config::{Config, ModelsConfig, ResolvedConfig};
// Imports needed for tests (will be moved with tests to tests.rs in step 7)
#[cfg(test)]
use crate::config::{ApiParams, LocalConfig};
// Note: ImageConfig, MarkdownStyle removed - these are CLI presentation concerns
use crate::context::{
    Context, ContextEntry, ContextMeta, ContextState, Message, TranscriptEntry,
    is_valid_context_name, now_timestamp,
};
use crate::partition::{ActiveState, PartitionManager, StorageConfig};
use dirs_next::home_dir;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, ErrorKind, Write};
use std::path::PathBuf;
use uuid::Uuid;

pub struct AppState {
    pub config: Config,
    pub models_config: ModelsConfig,
    pub state: ContextState,
    pub chibi_dir: PathBuf,
    pub state_path: PathBuf,
    pub contexts_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub plugins_dir: PathBuf,
    /// Cache of active partition state per context, avoiding repeated file scans.
    /// Uses interior mutability since caching is a side effect that doesn't change
    /// logical state.
    active_state_cache: RefCell<HashMap<String, ActiveState>>,
}

impl StatePaths for AppState {
    fn contexts_dir(&self) -> &PathBuf {
        &self.contexts_dir
    }
}

impl AppState {
    /// Create AppState from a custom directory (for testing)
    #[cfg(test)]
    pub fn from_dir(chibi_dir: PathBuf, config: Config) -> io::Result<Self> {
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        let plugins_dir = chibi_dir.join("plugins");
        let state_path = chibi_dir.join("state.json");

        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        fs::create_dir_all(&plugins_dir)?;

        let state = ContextState {
            contexts: Vec::new(),
        };

        Ok(AppState {
            config,
            models_config: ModelsConfig::default(),
            state,
            chibi_dir,
            state_path,
            contexts_dir,
            prompts_dir,
            plugins_dir,
            active_state_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Load AppState with optional home directory override.
    ///
    /// Precedence for chibi directory:
    /// 1. `home_override` parameter (from --home CLI flag)
    /// 2. `CHIBI_HOME` environment variable
    /// 3. `~/.chibi` default
    pub fn load(home_override: Option<PathBuf>) -> io::Result<Self> {
        let chibi_dir = if let Some(path) = home_override {
            path
        } else if let Ok(chibi_home) = std::env::var("CHIBI_HOME") {
            PathBuf::from(chibi_home)
        } else {
            let home = home_dir()
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Home directory not found"))?;
            home.join(".chibi")
        };
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        let plugins_dir = chibi_dir.join("plugins");

        // Create directories if they don't exist
        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        fs::create_dir_all(&plugins_dir)?;

        let config_path = chibi_dir.join("config.toml");
        let models_path = chibi_dir.join("models.toml");
        let state_path = chibi_dir.join("state.json");

        let config: Config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse config: {}", e),
                )
            })?
        } else {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!(
                    "Config file not found at {}. Please create config.toml with api_key, model, context_window_limit, and warn_threshold_percent",
                    config_path.display()
                ),
            ));
        };

        // Load models.toml (optional)
        let models_config: ModelsConfig = if models_path.exists() {
            let content = fs::read_to_string(&models_path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse models.toml: {}", e),
                )
            })?
        } else {
            ModelsConfig::default()
        };

        let state = if state_path.exists() {
            let file = File::open(&state_path)?;
            serde_json::from_reader(BufReader::new(file)).unwrap_or_else(|e| {
                eprintln!("[WARN] State file corrupted, resetting to defaults: {}", e);
                ContextState {
                    contexts: Vec::new(),
                }
            })
        } else {
            ContextState {
                contexts: Vec::new(),
            }
        };

        let mut app = AppState {
            config,
            models_config,
            state,
            chibi_dir,
            state_path,
            contexts_dir,
            prompts_dir,
            plugins_dir,
            active_state_cache: RefCell::new(HashMap::new()),
        };

        // Sync state with filesystem (handles stale entries and orphan directories)
        if app.sync_state_with_filesystem()? {
            app.save()?;
        }

        Ok(app)
    }

    pub fn save(&self) -> io::Result<()> {
        self.state.save(&self.state_path)
    }

    /// Synchronize state.json with filesystem reality.
    /// Called during startup after reading state.json.
    ///
    /// Operations:
    /// 1. Remove entries whose directories no longer exist
    /// 2. Register orphan directories (exist on disk but not in state)
    ///
    /// Note: Session state (current/previous context) is now managed by CLI.
    ///
    /// Returns true if state was modified (needs saving)
    pub fn sync_state_with_filesystem(&mut self) -> io::Result<bool> {
        use std::collections::HashSet;

        let mut modified = false;
        let contexts_dir = self.contexts_dir.clone();

        // Phase 1: Remove stale entries (directory doesn't exist)
        let original_count = self.state.contexts.len();
        self.state
            .contexts
            .retain(|entry| contexts_dir.join(&entry.name).is_dir());
        if self.state.contexts.len() != original_count {
            modified = true;
        }

        // Phase 2: Discover orphan directories
        let known_names: HashSet<_> = self.state.contexts.iter().map(|e| e.name.clone()).collect();

        if let Ok(entries) = fs::read_dir(&contexts_dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !known_names.contains(&name) && is_valid_context_name(&name) {
                        // Orphaned context directory - use current timestamp
                        // (state.json is the single source of truth for created_at)
                        self.state
                            .contexts
                            .push(ContextEntry::with_created_at(name, now_timestamp()));
                        modified = true;
                    }
                }
            }
        }

        // Sort by name for consistent ordering
        self.state.contexts.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(modified)
    }

    /// Update the last_activity_at timestamp for a context and optionally set destroy settings.
    /// The destroy settings are only set via --debug flags for testing purposes.
    pub fn touch_context_with_destroy_settings(
        &mut self,
        name: &str,
        destroy_at: Option<u64>,
        destroy_after_seconds_inactive: Option<u64>,
    ) -> io::Result<bool> {
        if let Some(entry) = self.state.contexts.iter_mut().find(|e| e.name == name) {
            entry.touch();
            if let Some(ts) = destroy_at {
                entry.destroy_at = ts;
            }
            if let Some(secs) = destroy_after_seconds_inactive {
                entry.destroy_after_seconds_inactive = secs;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Auto-destroy contexts that have expired based on their settings.
    /// Returns the list of destroyed context names.
    ///
    /// Note: This now destroys ALL expired contexts. The CLI is responsible for
    /// checking if the session's current context was destroyed and handling it.
    pub fn auto_destroy_expired_contexts(&mut self, verbose: bool) -> io::Result<Vec<String>> {
        let mut destroyed = Vec::new();

        // Collect contexts to destroy
        let to_destroy: Vec<String> = self
            .state
            .contexts
            .iter()
            .filter(|e| e.should_auto_destroy())
            .map(|e| e.name.clone())
            .collect();

        // Destroy each one
        for name in to_destroy {
            if verbose {
                eprintln!("[DEBUG] Auto-destroying expired context: {}", name);
            }
            // Remove directory
            let dir = self.context_dir(&name);
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
            // Remove from state
            self.state.contexts.retain(|e| e.name != name);
            destroyed.push(name);
        }

        Ok(destroyed)
    }

    pub fn ensure_context_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.context_dir(name);
        fs::create_dir_all(&dir)
    }

    /// Ensure tool cache directory exists (kept for potential future use)
    #[allow(dead_code)]
    pub fn ensure_tool_cache_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.tool_cache_dir(name);
        fs::create_dir_all(&dir)
    }

    /// Clear the tool cache for a context
    pub fn clear_tool_cache(&self, name: &str) -> io::Result<()> {
        let cache_dir = self.tool_cache_dir(name);
        crate::cache::clear_cache(&cache_dir)
    }

    /// Cleanup old cache entries for a context (based on max age)
    pub fn cleanup_tool_cache(&self, name: &str, max_age_days: u64) -> io::Result<usize> {
        let cache_dir = self.tool_cache_dir(name);
        crate::cache::cleanup_old_cache(&cache_dir, max_age_days)
    }

    /// Cleanup old cache entries for all contexts
    pub fn cleanup_all_tool_caches(&self, max_age_days: u64) -> io::Result<usize> {
        let mut total_removed = 0;
        for context_entry in &self.state.contexts {
            let removed = self.cleanup_tool_cache(&context_entry.name, max_age_days)?;
            total_removed += removed;
        }
        Ok(total_removed)
    }

    // === Context Dirty/Clean State ===
    //
    // A context is "clean" when its context.jsonl prefix (anchor + system prompt entries)
    // matches the current state and caches well. A context becomes "dirty" when something
    // changes that requires rebuilding the prefix (e.g., system prompt change, compaction).
    // The .dirty marker file indicates a dirty context that needs rebuilding on next load.

    /// Check if the context is dirty (needs prefix rebuild)
    pub fn is_context_dirty(&self, name: &str) -> bool {
        self.dirty_marker_file(name).exists()
    }

    /// Mark the context as dirty (triggers rebuild on next load)
    pub fn mark_context_dirty(&self, name: &str) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let marker_path = self.dirty_marker_file(name);
        fs::write(&marker_path, "")
    }

    /// Mark the context as clean (remove dirty marker after rebuild)
    pub fn mark_context_clean(&self, name: &str) -> io::Result<()> {
        let marker_path = self.dirty_marker_file(name);
        if marker_path.exists() {
            fs::remove_file(&marker_path)?;
        }
        Ok(())
    }

    pub fn load_context(&self, name: &str) -> io::Result<Context> {
        let context_dir = self.context_dir(name);
        let manifest_path = context_dir.join("manifest.json");
        let active_path = context_dir.join("active.jsonl");
        let context_jsonl = self.context_file(name);
        let context_json_old = self.context_file_old(name);

        // Try to load from partitioned format first (manifest.json or active.jsonl)
        if manifest_path.exists() || active_path.exists() {
            return self.load_context_from_jsonl(name);
        }

        // Try to load from legacy JSONL format (context.jsonl)
        if context_jsonl.exists() {
            return self.load_context_from_jsonl(name);
        }

        // Check if very old format exists and migrate
        if context_json_old.exists() {
            return self.migrate_and_load_context(name);
        }

        // Neither exists - return not found error
        Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Context '{}' not found", name),
        ))
    }

    /// Load context from the new JSONL format
    fn load_context_from_jsonl(&self, name: &str) -> io::Result<Context> {
        // Check if system_prompt.md was modified externally
        self.check_system_prompt_mtime_change(name)?;

        // Check if prefix is invalidated and rebuild if needed
        if self.is_context_dirty(name) {
            self.rebuild_context_from_transcript(name)?;
            self.mark_context_clean(name)?;
        }

        let entries = self.read_context_entries(name)?;

        // Load summary from separate file
        let summary_path = self.summary_file(name);
        let summary = if summary_path.exists() {
            fs::read_to_string(&summary_path)?
        } else {
            String::new()
        };

        // Convert entries to messages for backwards compatibility with existing code
        let messages = self.entries_to_messages(&entries);

        // Get created_at from state.json (single source of truth)
        let created_at = self.get_context_created_at(name);

        // Get updated_at from the last entry timestamp, or created_at if empty
        let updated_at = entries.last().map(|e| e.timestamp).unwrap_or(created_at);

        Ok(Context {
            name: name.to_string(),
            messages,
            created_at,
            updated_at,
            summary,
        })
    }

    /// Rebuild context.jsonl from transcript.jsonl
    /// This creates a fresh context.jsonl with:
    /// - `[0]` anchor entry (context_created or latest compaction/archival from transcript)
    /// - `[1..]` entries from transcript since the anchor
    ///
    /// Note: System prompt is NOT stored in context.jsonl. It lives in system_prompt.md
    /// (source of truth) and context_meta.json (last combined prompt sent to API).
    pub fn rebuild_context_from_transcript(&self, name: &str) -> io::Result<()> {
        use crate::context::{
            ENTRY_TYPE_ARCHIVAL, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_CONTEXT_CREATED,
        };

        // Read transcript entries
        let transcript_entries = self.read_transcript_entries(name)?;

        // Find the most recent anchor in the transcript
        let anchor_index = transcript_entries.iter().rposition(|e| {
            e.entry_type == ENTRY_TYPE_CONTEXT_CREATED
                || e.entry_type == ENTRY_TYPE_COMPACTION
                || e.entry_type == ENTRY_TYPE_ARCHIVAL
        });

        // Determine anchor entry and entries to include
        let (anchor_entry, entries_after_anchor) = match anchor_index {
            Some(idx) => {
                // Use existing anchor from transcript
                let anchor = transcript_entries[idx].clone();
                let entries: Vec<_> = transcript_entries[idx + 1..]
                    .iter()
                    .filter(|e| {
                        // Exclude system_prompt_changed events from context
                        e.entry_type != crate::context::ENTRY_TYPE_SYSTEM_PROMPT_CHANGED
                    })
                    .cloned()
                    .collect();
                (anchor, entries)
            }
            None => {
                // No anchor found, create context_created anchor
                // Use created_at from state.json (single source of truth)
                let created_at = self.get_context_created_at(name);
                let anchor = TranscriptEntry {
                    id: Uuid::new_v4().to_string(),
                    timestamp: created_at,
                    from: "system".to_string(),
                    to: name.to_string(),
                    content: "Context created".to_string(),
                    entry_type: ENTRY_TYPE_CONTEXT_CREATED.to_string(),
                    metadata: None,
                };
                // Include all transcript entries (excluding system_prompt_changed)
                let entries: Vec<_> = transcript_entries
                    .iter()
                    .filter(|e| e.entry_type != crate::context::ENTRY_TYPE_SYSTEM_PROMPT_CHANGED)
                    .cloned()
                    .collect();
                (anchor, entries)
            }
        };

        // Build the complete context entries: anchor + conversation entries
        // (system prompt is stored in context_meta.json, not in context.jsonl)
        let mut context_entries = vec![anchor_entry];
        context_entries.extend(entries_after_anchor);

        // Write to context.jsonl (full rewrite)
        self.write_context_entries(name, &context_entries)?;

        Ok(())
    }

    /// Migrate legacy transcript.jsonl to partitioned transcript/ directory
    fn migrate_transcript_if_needed(&self, name: &str) -> io::Result<()> {
        let legacy_path = self.transcript_jsonl_file(name);
        let transcript_dir = self.transcript_dir(name);
        let manifest_path = transcript_dir.join("manifest.json");

        // Already migrated or no legacy file
        if manifest_path.exists() || !legacy_path.exists() {
            return Ok(());
        }

        // Create transcript directory and move legacy file
        fs::create_dir_all(&transcript_dir)?;
        let active_path = transcript_dir.join("active.jsonl");
        fs::rename(&legacy_path, &active_path)?;

        Ok(())
    }

    /// Read all entries from transcript (using partitioned storage)
    pub fn read_transcript_entries(&self, name: &str) -> io::Result<Vec<TranscriptEntry>> {
        self.migrate_transcript_if_needed(name)?;
        let transcript_dir = self.transcript_dir(name);
        let storage_config = self.get_storage_config(name)?;
        let pm = PartitionManager::load_with_config(&transcript_dir, storage_config)?;
        pm.read_all_entries()
    }

    /// Migrate from old context.json format to new context.jsonl format
    fn migrate_and_load_context(&self, name: &str) -> io::Result<Context> {
        let old_path = self.context_file_old(name);
        let file = File::open(&old_path)?;
        let old_context: Context = serde_json::from_reader(BufReader::new(file)).map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("Failed to parse old context '{}': {}", name, e),
            )
        })?;

        // Note: created_at is stored in state.json, not context_meta.json
        // context_meta.json is used for system_prompt_md_mtime and last_combined_prompt

        // Convert messages to entries and save to JSONL
        let entries = self.messages_to_entries(&old_context.messages, name);
        self.write_context_entries(name, &entries)?;

        // Load summary from separate file (keep it there)
        let summary_path = self.summary_file(name);
        let summary = if summary_path.exists() {
            fs::read_to_string(&summary_path)?
        } else {
            old_context.summary.clone()
        };

        // If summary was in old context but not in file, save it
        if !old_context.summary.is_empty() && !summary_path.exists() {
            crate::safe_io::atomic_write_text(&summary_path, &old_context.summary)?;
        }

        // Remove old context.json file
        fs::remove_file(&old_path)?;

        Ok(Context {
            name: name.to_string(),
            messages: old_context.messages,
            created_at: old_context.created_at,
            updated_at: old_context.updated_at,
            summary,
        })
    }

    /// Get the resolved storage configuration for a context
    fn get_storage_config(&self, name: &str) -> io::Result<StorageConfig> {
        let local = self.load_local_config(name)?;
        // Merge local overrides with global config
        Ok(StorageConfig {
            partition_max_entries: local
                .storage
                .partition_max_entries
                .or(self.config.storage.partition_max_entries),
            partition_max_age_seconds: local
                .storage
                .partition_max_age_seconds
                .or(self.config.storage.partition_max_age_seconds),
            partition_max_tokens: local
                .storage
                .partition_max_tokens
                .or(self.config.storage.partition_max_tokens),
            bytes_per_token: local
                .storage
                .bytes_per_token
                .or(self.config.storage.bytes_per_token),
            enable_bloom_filters: local
                .storage
                .enable_bloom_filters
                .or(self.config.storage.enable_bloom_filters),
        })
    }

    /// Read entries from context.jsonl (the LLM's working memory)
    pub fn read_context_entries(&self, name: &str) -> io::Result<Vec<TranscriptEntry>> {
        read_jsonl_file(&self.context_file(name))
    }

    /// Write entries to context.jsonl (full rewrite, atomic)
    pub fn write_context_entries(&self, name: &str, entries: &[TranscriptEntry]) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);

        let mut content = String::new();
        for entry in entries {
            let json = serde_json::to_string(entry)
                .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
            content.push_str(&json);
            content.push('\n');
        }
        crate::safe_io::atomic_write_text(&path, &content)
    }

    /// Append a single entry to context.jsonl
    ///
    /// # Durability
    ///
    /// Writes are flushed to disk via fsync to ensure durability and consistency
    /// with the transcript (which also fsyncs). This prevents context.jsonl from
    /// diverging from transcript.jsonl on crash.
    pub fn append_context_entry(&self, name: &str, entry: &TranscriptEntry) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
        writeln!(file, "{}", json)?;
        file.sync_all()?;
        Ok(())
    }

    /// Load context metadata
    fn load_context_meta(&self, name: &str) -> io::Result<ContextMeta> {
        let path = self.context_meta_file(name);
        if path.exists() {
            let file = File::open(&path)?;
            serde_json::from_reader(BufReader::new(file)).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse context meta: {}", e),
                )
            })
        } else {
            Ok(ContextMeta::default())
        }
    }

    /// Get created_at timestamp for a context from state.json (single source of truth)
    pub fn get_context_created_at(&self, name: &str) -> u64 {
        self.state
            .contexts
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.created_at)
            .unwrap_or_else(now_timestamp)
    }

    /// Save context metadata (atomic write)
    fn save_context_meta(&self, name: &str, meta: &ContextMeta) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_meta_file(name);
        crate::safe_io::atomic_write_json(&path, meta)
    }

    /// Convert transcript entries to messages (for backwards compat)
    fn entries_to_messages(&self, entries: &[TranscriptEntry]) -> Vec<Message> {
        entries
            .iter()
            .filter(|e| e.entry_type == crate::context::ENTRY_TYPE_MESSAGE)
            .map(|e| {
                let role = if e.to == "user" {
                    "assistant".to_string()
                } else {
                    "user".to_string()
                };
                Message {
                    id: e.id.clone(),
                    role,
                    content: e.content.clone(),
                }
            })
            .collect()
    }

    /// Convert messages to transcript entries (for migration)
    fn messages_to_entries(
        &self,
        messages: &[Message],
        context_name: &str,
    ) -> Vec<TranscriptEntry> {
        messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| {
                let (from, to) = if m.role == "assistant" {
                    (context_name.to_string(), "user".to_string())
                } else {
                    ("user".to_string(), context_name.to_string())
                };
                TranscriptEntry {
                    id: m.id.clone(),
                    timestamp: now_timestamp(),
                    from,
                    to,
                    content: m.content.clone(),
                    entry_type: crate::context::ENTRY_TYPE_MESSAGE.to_string(),
                    metadata: None,
                }
            })
            .collect()
    }

    pub fn save_context(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;

        // Check if this is a brand new context (no transcript.jsonl exists yet)
        let transcript_path = self.transcript_jsonl_file(&context.name);
        let is_new_context = !transcript_path.exists();

        // Note: created_at is stored in state.json, not context_meta.json
        // context_meta.json is used for system_prompt_md_mtime and last_combined_prompt

        // For brand new contexts, write context_created anchor to transcript.
        // Don't mark dirty - new contexts don't need a rebuild since they're being
        // created fresh. The anchor is just for transcript history.
        if is_new_context {
            let anchor = create_context_created_anchor(&context.name);
            self.append_to_transcript(&context.name, &anchor)?;

            // Also write the initial messages to transcript so they're preserved
            let entries = self.messages_to_entries(&context.messages, &context.name);
            for entry in &entries {
                self.append_to_transcript(&context.name, entry)?;
            }
        }

        // Convert messages to entries and write to JSONL
        // Note: This is a full rewrite. If context is dirty, the next load will rebuild
        // with proper anchor + system_prompt prefix from transcript.
        let entries = self.messages_to_entries(&context.messages, &context.name);
        self.write_context_entries(&context.name, &entries)?;

        // Save summary to separate file
        let summary_path = self.summary_file(&context.name);
        if !context.summary.is_empty() {
            crate::safe_io::atomic_write_text(&summary_path, &context.summary)?;
        } else if summary_path.exists() {
            // Remove empty summary file
            fs::remove_file(&summary_path)?;
        }

        Ok(())
    }

    /// Append all context messages to human-readable transcript.md
    pub fn append_to_transcript_md(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.transcript_md_file(&context.name))?;

        for msg in &context.messages {
            // Skip system messages to avoid cluttering transcript with boilerplate
            if msg.role == "system" {
                continue;
            }
            writeln!(file, "[{}]: {}\n", msg.role.to_uppercase(), msg.content)?;
        }

        Ok(())
    }

    /// Append a single entry to transcript (the authoritative log, using partitioned storage)
    ///
    /// Uses cached active state when available to avoid repeated file scans.
    /// The cache is updated after each operation and invalidated on context clear/compaction.
    pub fn append_to_transcript(
        &self,
        context_name: &str,
        entry: &TranscriptEntry,
    ) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        self.migrate_transcript_if_needed(context_name)?;
        let transcript_dir = self.transcript_dir(context_name);
        let storage_config = self.get_storage_config(context_name)?;

        // Get cached state if available
        let cached_state = self.active_state_cache.borrow().get(context_name).cloned();

        let mut pm = PartitionManager::load_with_cached_state(
            &transcript_dir,
            storage_config,
            cached_state,
        )?;
        pm.append_entry(entry)?;

        // Check if rotation is needed after append
        let rotated = pm.rotate_if_needed()?;

        // Update cache with new state (if rotated, state was reset)
        if rotated {
            // After rotation, remove from cache so next load re-scans the empty active partition
            self.active_state_cache.borrow_mut().remove(context_name);
        } else {
            // Update cache with the modified state
            self.active_state_cache
                .borrow_mut()
                .insert(context_name.to_string(), pm.active_state());
        }

        Ok(())
    }

    /// Append an entry to both transcript.jsonl and context.jsonl (tandem write)
    /// This is the primary method for recording new events during normal operation.
    pub fn append_to_transcript_and_context(
        &self,
        context_name: &str,
        entry: &TranscriptEntry,
    ) -> io::Result<()> {
        // Write to authoritative transcript first
        self.append_to_transcript(context_name, entry)?;
        // Then append to context (LLM window)
        self.append_context_entry(context_name, entry)?;
        Ok(())
    }

    // NOTE: append_to_current_transcript_and_context and get_current_context were removed
    // in the stateless-core refactor. Use the parameterized versions instead:
    // - append_to_transcript_and_context(context_name, entry)
    // - get_or_create_context(name)

    pub fn calculate_token_count(&self, messages: &[Message]) -> usize {
        // Rough estimation: 4 chars per token on average
        messages
            .iter()
            .map(|m| (m.content.len() + m.role.len()) / 4)
            .sum()
    }

    pub fn should_warn(&self, messages: &[Message]) -> bool {
        let tokens = self.calculate_token_count(messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.warn_threshold_percent
    }

    pub fn remaining_tokens(&self, messages: &[Message]) -> usize {
        let tokens = self.calculate_token_count(messages);
        self.config.context_window_limit.saturating_sub(tokens)
    }



    pub fn should_auto_compact(&self, context: &Context, resolved_config: &ResolvedConfig) -> bool {
        if !resolved_config.auto_compact {
            return false;
        }
        let tokens = self.calculate_token_count(&context.messages);
        let usage_percent = (tokens as f32 / resolved_config.context_window_limit as f32) * 100.0;
        usage_percent >= resolved_config.auto_compact_threshold
    }





    /// Read all entries from the context file (context.jsonl) - unified with transcript
    pub fn read_jsonl_transcript(&self, context_name: &str) -> io::Result<Vec<TranscriptEntry>> {
        self.read_context_entries(context_name)
    }

    // === Compaction Finalization ===

    /// Finalize a compaction operation by writing the anchor to transcript and marking dirty.
    /// This is the common final step for all compaction operations (rolling, manual, by-name).
    pub fn finalize_compaction(&self, context_name: &str, summary: &str) -> io::Result<()> {
        let compaction_anchor = create_compaction_anchor(context_name, summary);
        self.append_to_transcript(context_name, &compaction_anchor)?;
        self.mark_context_dirty(context_name)?;

        // Invalidate active state cache after compaction (context content changed)
        self.active_state_cache.borrow_mut().remove(context_name);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::InboxEntry;
    use tempfile::TempDir;

    /// Create a test AppState with a temporary directory
    fn create_test_app() -> (AppState, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            base_url: "https://test.api/v1".to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            max_recursion_depth: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
        };
        let app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();
        (app, temp_dir)
    }

    // === Path construction tests ===

    #[test]
    fn test_context_dir() {
        let (app, _temp) = create_test_app();
        let dir = app.context_dir("mycontext");
        assert!(dir.ends_with("contexts/mycontext"));
    }

    #[test]
    fn test_context_file() {
        let (app, _temp) = create_test_app();
        let file = app.context_file("mycontext");
        assert!(file.ends_with("contexts/mycontext/context.jsonl"));
    }

    #[test]
    fn test_todos_file() {
        let (app, _temp) = create_test_app();
        let file = app.todos_file("mycontext");
        assert!(file.ends_with("contexts/mycontext/todos.md"));
    }

    #[test]
    fn test_goals_file() {
        let (app, _temp) = create_test_app();
        let file = app.goals_file("mycontext");
        assert!(file.ends_with("contexts/mycontext/goals.md"));
    }

    #[test]
    fn test_inbox_file() {
        let (app, _temp) = create_test_app();
        let file = app.inbox_file("mycontext");
        assert!(file.ends_with("contexts/mycontext/inbox.jsonl"));
    }

    // === Context lifecycle tests ===

    #[test]
    fn test_get_or_create_context_creates_default() {
        let (app, _temp) = create_test_app();
        let context = app.get_or_create_context("default").unwrap();
        assert_eq!(context.name, "default");
        assert!(context.messages.is_empty());
    }

    #[test]
    fn test_save_and_load_context() {
        let (app, _temp) = create_test_app();

        let context = Context {
            name: "test-context".to_string(),
            messages: vec![
                Message::new("user", "Hello"),
                Message::new("assistant", "Hi there!"),
            ],
            created_at: 1234567890,
            updated_at: 1234567891,
            summary: "Test summary".to_string(),
        };

        app.save_context(&context).unwrap();

        let loaded = app.load_context("test-context").unwrap();
        assert_eq!(loaded.name, "test-context");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "Hello");
        assert_eq!(loaded.summary, "Test summary");
    }

    #[test]
    fn test_add_message() {
        let (app, _temp) = create_test_app();
        let mut context = app.get_or_create_context("default").unwrap();

        assert!(context.messages.is_empty());

        app.add_message(&mut context, "user".to_string(), "Test message".to_string());

        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].role, "user");
        assert_eq!(context.messages[0].content, "Test message");
        assert!(context.updated_at > 0);
    }

    #[test]
    fn test_list_contexts_empty() {
        let (app, _temp) = create_test_app();
        let contexts = app.list_contexts();
        assert!(contexts.is_empty());
    }

    #[test]
    fn test_list_contexts_with_contexts() {
        let (mut app, _temp) = create_test_app();

        // Create some contexts
        for name in &["alpha", "beta", "gamma"] {
            let context = Context {
                name: name.to_string(),
                messages: vec![],
                created_at: 0,
                updated_at: 0,
                summary: String::new(),
            };
            app.save_context(&context).unwrap();
        }

        // Sync state with filesystem (discovers new directories)
        app.sync_state_with_filesystem().unwrap();

        let contexts = app.list_contexts();
        assert_eq!(contexts.len(), 3);
        // Should be sorted
        assert_eq!(contexts[0], "alpha");
        assert_eq!(contexts[1], "beta");
        assert_eq!(contexts[2], "gamma");
    }

    #[test]
    fn test_rename_context() {
        let (mut app, _temp) = create_test_app();

        // Create a context
        let context = Context {
            name: "old-name".to_string(),
            messages: vec![Message::new("user", "Hello")],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();

        // Rename
        app.rename_context("old-name", "new-name").unwrap();

        // Verify
        assert!(!app.context_dir("old-name").exists());
        assert!(app.context_dir("new-name").exists());

        let loaded = app.load_context("new-name").unwrap();
        assert_eq!(loaded.name, "new-name");
        assert_eq!(loaded.messages[0].content, "Hello");
    }

    #[test]
    fn test_rename_nonexistent_context() {
        let (mut app, _temp) = create_test_app();
        let result = app.rename_context("nonexistent", "new-name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_rename_to_existing_context() {
        let (mut app, _temp) = create_test_app();

        // Create both contexts
        for name in &["source", "target"] {
            let context = Context {
                name: name.to_string(),
                messages: vec![],
                created_at: 0,
                updated_at: 0,
                summary: String::new(),
            };
            app.save_context(&context).unwrap();
        }

        let result = app.rename_context("source", "target");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_destroy_context() {
        let (mut app, _temp) = create_test_app();

        // Create context to destroy
        let context = Context {
            name: "to-destroy".to_string(),
            messages: vec![],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();

        // Destroy
        let result = app.destroy_context("to-destroy").unwrap();
        assert!(result); // Destroyed successfully
        assert!(!app.context_dir("to-destroy").exists());
    }

    #[test]
    fn test_destroy_nonexistent_context() {
        let (mut app, _temp) = create_test_app();
        let result = app.destroy_context("nonexistent").unwrap();
        assert!(!result); // Nothing to destroy
    }

    // NOTE: Tests for "destroy current context switches to previous" were removed
    // in the stateless-core refactor. Session state (current/previous context)
    // is now managed by the CLI layer, not chibi-core.

    // === Token calculation tests ===

    #[test]
    fn test_calculate_token_count_empty() {
        let (app, _temp) = create_test_app();
        let count = app.calculate_token_count(&[]);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_calculate_token_count() {
        let (app, _temp) = create_test_app();
        let messages = vec![
            Message::new("user", "Hello world!"), // 4+12 = 16 chars / 4 = 4 tokens
        ];
        let count = app.calculate_token_count(&messages);
        assert_eq!(count, 4); // (4 + 12) / 4 = 4
    }

    #[test]
    fn test_remaining_tokens() {
        let (app, _temp) = create_test_app();
        let messages = vec![
            Message::new("user", "x".repeat(4000)), // ~1000 tokens
        ];
        let remaining = app.remaining_tokens(&messages);
        // 8000 - ~1000 = ~7000
        assert!(remaining < 8000);
        assert!(remaining > 6000);
    }

    #[test]
    fn test_should_warn() {
        let (app, _temp) = create_test_app();

        // Small message shouldn't warn
        let small_messages = vec![Message::new("user", "Hello")];
        assert!(!app.should_warn(&small_messages));

        // Large message should warn (above 75% of 8000 = 6000 tokens = ~24000 chars)
        let large_messages = vec![Message::new("user", "x".repeat(30000))];
        assert!(app.should_warn(&large_messages));
    }

    // === Todos/Goals tests ===

    #[test]
    fn test_todos_save_and_load() {
        let (app, _temp) = create_test_app();

        app.save_todos("default", "- [ ] Task 1\n- [x] Task 2")
            .unwrap();
        let loaded = app.load_todos("default").unwrap();
        assert_eq!(loaded, "- [ ] Task 1\n- [x] Task 2");
    }

    #[test]
    fn test_todos_empty_returns_empty_string() {
        let (app, _temp) = create_test_app();
        let loaded = app.load_todos("nonexistent").unwrap();
        assert_eq!(loaded, "");
    }

    #[test]
    fn test_goals_save_and_load() {
        let (app, _temp) = create_test_app();

        app.save_goals("default", "Build something awesome")
            .unwrap();
        let loaded = app.load_goals("default").unwrap();
        assert_eq!(loaded, "Build something awesome");
    }

    // === Local config tests ===

    #[test]
    fn test_local_config_default() {
        let (app, _temp) = create_test_app();
        let local = app.load_local_config("default").unwrap();
        assert!(local.model.is_none());
        assert!(local.username.is_none());
    }

    #[test]
    fn test_local_config_save_and_load() {
        let (app, _temp) = create_test_app();

        let local = LocalConfig {
            model: Some("custom-model".to_string()),
            username: Some("alice".to_string()),
            auto_compact: Some(true),
            ..Default::default()
        };

        app.save_local_config("default", &local).unwrap();
        let loaded = app.load_local_config("default").unwrap();

        assert_eq!(loaded.model, Some("custom-model".to_string()));
        assert_eq!(loaded.username, Some("alice".to_string()));
        assert_eq!(loaded.auto_compact, Some(true));
    }

    // === Inbox tests ===

    #[test]
    fn test_inbox_empty() {
        let (app, _temp) = create_test_app();
        let entries = app.load_and_clear_inbox("default").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_inbox_append_and_load() {
        let (app, _temp) = create_test_app();

        let entry1 = InboxEntry {
            id: "1".to_string(),
            timestamp: 1000,
            from: "sender".to_string(),
            to: "default".to_string(),
            content: "Message 1".to_string(),
        };
        let entry2 = InboxEntry {
            id: "2".to_string(),
            timestamp: 2000,
            from: "sender".to_string(),
            to: "default".to_string(),
            content: "Message 2".to_string(),
        };

        app.append_to_inbox("default", &entry1).unwrap();
        app.append_to_inbox("default", &entry2).unwrap();

        let entries = app.load_and_clear_inbox("default").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "Message 1");
        assert_eq!(entries[1].content, "Message 2");

        // Should be cleared
        let entries_after = app.load_and_clear_inbox("default").unwrap();
        assert!(entries_after.is_empty());
    }

    // === System prompt tests ===

    #[test]
    fn test_set_and_load_system_prompt() {
        let (app, _temp) = create_test_app();

        app.set_system_prompt_for("default", "You are a helpful assistant.")
            .unwrap();
        let loaded = app.load_system_prompt_for("default").unwrap();
        assert_eq!(loaded, "You are a helpful assistant.");
    }

    #[test]
    fn test_system_prompt_fallback() {
        let (app, _temp) = create_test_app();

        // Write default prompt
        fs::write(app.prompts_dir.join("chibi.md"), "Default prompt").unwrap();

        // No context-specific prompt, should fall back
        let loaded = app.load_system_prompt_for("default").unwrap();
        assert_eq!(loaded, "Default prompt");
    }

    // === Config resolution tests ===

    #[test]
    fn test_resolve_config_defaults() {
        let (app, _temp) = create_test_app();
        let resolved = app.resolve_config("default", None).unwrap();

        assert_eq!(resolved.api_key, "test-key");
        assert_eq!(resolved.model, "test-model");
        assert_eq!(resolved.username, "testuser");
    }

    #[test]
    fn test_resolve_config_local_override() {
        let (app, _temp) = create_test_app();

        // Set local config
        let local = LocalConfig {
            model: Some("local-model".to_string()),
            username: Some("localuser".to_string()),
            auto_compact: Some(true),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        let resolved = app.resolve_config("default", None).unwrap();
        assert_eq!(resolved.model, "local-model");
        assert_eq!(resolved.username, "localuser");
        assert!(resolved.auto_compact);
    }

    #[test]
    fn test_resolve_config_username_override() {
        let (app, _temp) = create_test_app();

        // Set local config
        let local = LocalConfig {
            username: Some("localuser".to_string()),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        // Runtime username override should override local
        let resolved = app.resolve_config("default", Some("overrideuser")).unwrap();
        assert_eq!(resolved.username, "overrideuser");
    }

    #[test]
    fn test_resolve_config_api_params_global_defaults() {
        let (app, _temp) = create_test_app();
        let resolved = app.resolve_config("default", None).unwrap();

        // Should have defaults from ApiParams::defaults()
        assert_eq!(resolved.api.prompt_caching, Some(true));
        assert_eq!(resolved.api.parallel_tool_calls, Some(true));
        assert_eq!(
            resolved.api.reasoning.effort,
            Some(crate::config::ReasoningEffort::Medium)
        );
    }

    #[test]
    fn test_resolve_config_api_params_context_override() {
        let (app, _temp) = create_test_app();

        // Set local config with API overrides
        let local = LocalConfig {
            api: Some(ApiParams {
                temperature: Some(0.7),
                max_tokens: Some(2000),
                ..Default::default()
            }),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        let resolved = app.resolve_config("default", None).unwrap();

        // Context-level API params should override
        assert_eq!(resolved.api.temperature, Some(0.7));
        assert_eq!(resolved.api.max_tokens, Some(2000));
        // But defaults should still be present for unset values
        assert_eq!(resolved.api.prompt_caching, Some(true));
    }

    #[test]
    fn test_resolve_config_model_level_api_params() {
        // Create test app with models config
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            base_url: "https://test.api/v1".to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            max_recursion_depth: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
        };

        let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

        // Add model config
        app.models_config.models.insert(
            "test-model".to_string(),
            crate::config::ModelMetadata {
                context_window: Some(16000),
                api: ApiParams {
                    temperature: Some(0.5),
                    reasoning: crate::config::ReasoningConfig {
                        effort: Some(crate::config::ReasoningEffort::High),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        );

        let resolved = app.resolve_config("default", None).unwrap();

        // Model-level params should be applied
        assert_eq!(resolved.api.temperature, Some(0.5));
        assert_eq!(
            resolved.api.reasoning.effort,
            Some(crate::config::ReasoningEffort::High)
        );
        // Model context window should override
        assert_eq!(resolved.context_window_limit, 16000);
    }

    #[test]
    fn test_resolve_config_hierarchy_context_over_model() {
        // Test that context-level API params override model-level
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            base_url: "https://test.api/v1".to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            max_recursion_depth: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
        };

        let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

        // Add model config with temperature
        app.models_config.models.insert(
            "test-model".to_string(),
            crate::config::ModelMetadata {
                context_window: Some(16000),
                api: ApiParams {
                    temperature: Some(0.5),
                    max_tokens: Some(1000),
                    ..Default::default()
                },
            },
        );

        // Set local config that overrides temperature but not max_tokens
        let local = LocalConfig {
            api: Some(ApiParams {
                temperature: Some(0.9), // Override model's 0.5
                ..Default::default()
            }),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        let resolved = app.resolve_config("default", None).unwrap();

        // Context should override model
        assert_eq!(resolved.api.temperature, Some(0.9));
        // Model value should be preserved when context doesn't override
        assert_eq!(resolved.api.max_tokens, Some(1000));
    }

    // NOTE: test_resolve_config_cli_persistent_username and
    // test_resolve_config_cli_temp_username_over_persistent were removed in the
    // stateless-core refactor. The distinction between persistent (-u) and
    // ephemeral (-U) usernames is now handled by the CLI layer, not chibi-core.
    // Core only knows about a single optional username_override parameter.

    #[test]
    fn test_resolve_config_all_local_overrides() {
        let (app, _temp) = create_test_app();

        // Set all local config overrides
        let local = LocalConfig {
            model: Some("local-model".to_string()),
            api_key: Some("local-key".to_string()),
            base_url: Some("https://local.api/v1".to_string()),
            username: Some("localuser".to_string()),
            auto_compact: Some(true),
            auto_compact_threshold: Some(90.0),
            max_recursion_depth: Some(50),
            warn_threshold_percent: Some(85.0),
            context_window_limit: Some(16000),
            reflection_enabled: Some(false),
            tool_output_cache_threshold: None,
            tool_cache_max_age_days: None,
            auto_cleanup_cache: None,
            tool_cache_preview_chars: None,
            file_tools_allowed_paths: None,
            api: None,
            tools: None,
            storage: StorageConfig::default(),
        };
        app.save_local_config("default", &local).unwrap();

        let resolved = app.resolve_config("default", None).unwrap();

        assert_eq!(resolved.model, "local-model");
        assert_eq!(resolved.api_key, "local-key");
        assert_eq!(resolved.base_url, "https://local.api/v1");
        assert_eq!(resolved.username, "localuser");
        assert!(resolved.auto_compact);
        assert!((resolved.auto_compact_threshold - 90.0).abs() < f32::EPSILON);
        assert_eq!(resolved.max_recursion_depth, 50);
        assert!((resolved.warn_threshold_percent - 85.0).abs() < f32::EPSILON);
        assert_eq!(resolved.context_window_limit, 16000);
        assert!(!resolved.reflection_enabled);
    }

    // Note: Image config tests removed - image presentation is handled by CLI layer

    // === Transcript entry creation tests ===

    #[test]
    fn test_create_user_message_entry() {
        let (app, _temp) = create_test_app();
        let entry = create_user_message_entry("default", "Hello", "alice");

        assert!(!entry.id.is_empty());
        assert!(entry.timestamp > 0);
        assert_eq!(entry.from, "alice");
        assert_eq!(entry.to, "default");
        assert_eq!(entry.content, "Hello");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_MESSAGE);
    }

    #[test]
    fn test_create_assistant_message_entry() {
        let (app, _temp) = create_test_app();
        let entry = create_assistant_message_entry("default", "Hi there!");

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "user");
        assert_eq!(entry.content, "Hi there!");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_MESSAGE);
    }

    #[test]
    fn test_create_tool_call_entry() {
        let (app, _temp) = create_test_app();
        let entry = create_tool_call_entry("default", "web_search", r#"{"query": "rust"}"#);

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "web_search");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_TOOL_CALL);
    }

    #[test]
    fn test_create_tool_result_entry() {
        let (app, _temp) = create_test_app();
        let entry = create_tool_result_entry("default", "web_search", "Search results...");

        assert_eq!(entry.from, "web_search");
        assert_eq!(entry.to, "default");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_TOOL_RESULT);
    }

    // === JSONL parsing robustness tests ===

    #[test]
    fn test_jsonl_empty_file() {
        let (app, _temp) = create_test_app();

        // Create empty context.jsonl
        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();
        fs::write(ctx_dir.join("context.jsonl"), "").unwrap();

        // Should return empty vec, not error
        let entries = app.read_context_entries("test-context").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_jsonl_blank_lines() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL with blank lines
        let content = r#"
{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}

{"id":"2","timestamp":1234567891,"from":"ctx","to":"user","content":"hi","entry_type":"message"}

"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "hello");
        assert_eq!(entries[1].content, "hi");
    }

    #[test]
    fn test_jsonl_malformed_entries_skipped() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL with some malformed entries
        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}
not valid json at all
{"id":"2","timestamp":1234567891,"from":"ctx","to":"user","content":"hi","entry_type":"message"}
{"incomplete": true
{"id":"3","timestamp":1234567892,"from":"user","to":"ctx","content":"bye","entry_type":"message"}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        // Should skip malformed entries and return valid ones
        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 3, "Should have 3 valid entries");
        assert_eq!(entries[0].content, "hello");
        assert_eq!(entries[1].content, "hi");
        assert_eq!(entries[2].content, "bye");
    }

    #[test]
    fn test_jsonl_nonexistent_file() {
        let (app, _temp) = create_test_app();

        // Don't create the context directory
        let entries = app.read_context_entries("nonexistent-context").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_jsonl_unicode_content() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL with unicode content
        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"こんにちは 🎉 Привет","entry_type":"message"}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "こんにちは 🎉 Привет");
    }

    #[test]
    fn test_jsonl_with_escaped_content() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL with escaped characters in content
        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"line1\nline2\ttab","entry_type":"message"}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].content.contains('\n'));
        assert!(entries[0].content.contains('\t'));
    }

    #[test]
    fn test_jsonl_missing_optional_fields() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL without optional metadata field
        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].metadata.is_none());
    }

    #[test]
    fn test_jsonl_with_metadata() {
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        // Write JSONL with metadata field
        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message","metadata":{"summary":"test summary"}}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries = app.read_context_entries("test-context").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].metadata.is_some());
        assert_eq!(
            entries[0].metadata.as_ref().unwrap().summary,
            Some("test summary".to_string())
        );
    }

    #[test]
    fn test_jsonl_transcript_vs_context_entries() {
        // read_jsonl_transcript and read_context_entries should behave the same
        let (app, _temp) = create_test_app();

        let ctx_dir = app.context_dir("test-context");
        fs::create_dir_all(&ctx_dir).unwrap();

        let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}"#;
        fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

        let entries1 = app.read_context_entries("test-context").unwrap();
        let entries2 = app.read_jsonl_transcript("test-context").unwrap();

        assert_eq!(entries1.len(), entries2.len());
        assert_eq!(entries1[0].id, entries2[0].id);
    }

    // === State/directory sync tests (Issue #13) ===

    #[test]
    fn test_list_contexts_excludes_manually_deleted_directories() {
        // BUG: When a context directory is manually deleted (rm -r), the context
        // should not appear in list_contexts(). Currently it lingers in state.json.
        let (mut app, _temp) = create_test_app();

        // Create two contexts
        let ctx1 = Context::new("context-one");
        let ctx2 = Context::new("context-two");
        app.save_context(&ctx1).unwrap();
        app.save_context(&ctx2).unwrap();

        // Add them to state.json
        app.state.contexts.push(ContextEntry::with_created_at(
            "context-one",
            now_timestamp(),
        ));
        app.state.contexts.push(ContextEntry::with_created_at(
            "context-two",
            now_timestamp(),
        ));
        app.save().unwrap();

        // Manually delete one context's directory (simulating rm -r)
        fs::remove_dir_all(app.context_dir("context-one")).unwrap();

        // Sync state with filesystem (this happens on startup in real usage)
        app.sync_state_with_filesystem().unwrap();

        // list_contexts should NOT include the deleted context
        let contexts = app.list_contexts();
        assert!(
            !contexts.contains(&"context-one".to_string()),
            "Deleted context should not appear in list_contexts()"
        );
        assert!(contexts.contains(&"context-two".to_string()));
    }

    #[test]
    fn test_list_contexts_only_includes_directories_not_files() {
        // BUG: Files in ~/.chibi/contexts/ should not appear as contexts
        let (mut app, _temp) = create_test_app();

        // Create a real context
        let ctx = Context::new("real-context");
        app.save_context(&ctx).unwrap();

        // Create a stray file in the contexts directory (not a context)
        let stray_file = app.contexts_dir.join("not-a-context.txt");
        fs::write(&stray_file, "stray file content").unwrap();

        // Sync state with filesystem (discovers new directories, ignores files)
        app.sync_state_with_filesystem().unwrap();

        let contexts = app.list_contexts();

        // Should include the real context
        assert!(contexts.contains(&"real-context".to_string()));

        // Should NOT include the stray file
        assert!(
            !contexts.contains(&"not-a-context.txt".to_string()),
            "Stray files should not appear as contexts"
        );
    }

    #[test]
    fn test_save_and_register_context_adds_to_state_contexts() {
        // Verify that save_and_register_context adds new contexts to state.json
        let (app, _temp) = create_test_app();

        // First save initial state so state.json exists
        app.save().unwrap();

        assert!(!app.state.contexts.iter().any(|e| e.name == "new-context"));

        let ctx = Context::new("new-context");
        app.save_and_register_context(&ctx).unwrap();

        // Check the state file directly
        let state_content = fs::read_to_string(&app.state_path).unwrap();
        assert!(
            state_content.contains("new-context"),
            "New context should be added to state.json"
        );
    }

    // NOTE: test_save_and_register_context_preserves_disk_current_context was removed
    // in the stateless-core refactor. current_context is no longer stored in state.json;
    // it's now managed by the CLI Session layer.

    // === Touch context tests ===

    #[test]
    fn test_touch_context_with_destroy_settings_on_new_context() {
        let (mut app, _temp) = create_test_app();

        // Simulate what happens when switching to a new context with debug settings:
        // 1. Context entry is added to state.contexts (our fix)
        app.state.contexts.push(ContextEntry::with_created_at(
            "new-test-context",
            now_timestamp(),
        ));

        // 2. Debug settings are applied via touch_context_with_destroy_settings
        let result = app
            .touch_context_with_destroy_settings("new-test-context", None, Some(60))
            .unwrap();
        assert!(
            result,
            "Should successfully apply debug settings to new context"
        );

        // 3. Verify the destroy settings were actually saved
        let entry = app
            .state
            .contexts
            .iter()
            .find(|e| e.name == "new-test-context")
            .unwrap();
        assert_eq!(entry.destroy_after_seconds_inactive, 60);
        assert_eq!(entry.destroy_at, 0);
        assert!(
            entry.last_activity_at > 0,
            "last_activity_at should be updated by touch"
        );
    }

    // === Auto-destroy tests ===

    #[test]
    fn test_auto_destroy_expired_contexts_by_timestamp() {
        let (mut app, _temp) = create_test_app();

        // Create a context to be destroyed
        let ctx = Context::new("to-destroy");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with destroy_at in the past
        let mut entry = ContextEntry::with_created_at("to-destroy", now_timestamp());
        entry.destroy_at = 1; // Way in the past
        app.state.contexts.push(entry);

        // Run auto-destroy
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert_eq!(destroyed, vec!["to-destroy".to_string()]);
        assert!(!app.context_dir("to-destroy").exists());
    }

    #[test]
    fn test_auto_destroy_expired_contexts_by_inactivity() {
        let (mut app, _temp) = create_test_app();

        // Create a context to be destroyed
        let ctx = Context::new("to-destroy");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with inactivity timeout triggered
        let mut entry = ContextEntry::with_created_at("to-destroy", now_timestamp());
        entry.last_activity_at = 1; // Way in the past
        entry.destroy_after_seconds_inactive = 60; // 1 minute
        app.state.contexts.push(entry);

        // Run auto-destroy
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert_eq!(destroyed, vec!["to-destroy".to_string()]);
        assert!(!app.context_dir("to-destroy").exists());
    }

    // NOTE: test_auto_destroy_skips_current_context was removed in the stateless-core
    // refactor. Core no longer tracks "current context" - that's CLI's responsibility.
    // auto_destroy_expired_contexts now destroys all expired contexts unconditionally.

    // NOTE: test_auto_destroy_clears_previous_context_reference was removed in the
    // stateless-core refactor. previous_context is now CLI session state.

    #[test]
    fn test_auto_destroy_respects_disabled_settings() {
        let (mut app, _temp) = create_test_app();

        // Create a context
        let ctx = Context::new("keep-context");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with settings that should NOT trigger destroy
        let mut entry = ContextEntry::with_created_at("keep-context", now_timestamp());
        entry.last_activity_at = 1; // Way in the past
        entry.destroy_after_seconds_inactive = 0; // Disabled
        entry.destroy_at = 0; // Disabled
        app.state.contexts.push(entry);

        // Run auto-destroy - should NOT destroy since both are disabled
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert!(destroyed.is_empty());
        assert!(app.context_dir("keep-context").exists());
    }

    // === Active state caching tests (Issue #1) ===

    #[test]
    fn test_append_to_transcript_caches_state() {
        let (app, _temp) = create_test_app();

        // Create context (save_context writes a context_created anchor, populating the cache)
        let ctx = Context::new("test-context");
        app.save_context(&ctx).unwrap();
        let count_after_save = app
            .active_state_cache
            .borrow()
            .get("test-context")
            .map(|s| s.entry_count())
            .unwrap_or(0);

        // Append an explicit entry
        let entry = create_user_message_entry("test-context", "Hello", "testuser");
        app.append_to_transcript("test-context", &entry).unwrap();

        // Cache should exist and have incremented
        let cache = app.active_state_cache.borrow();
        assert!(
            cache.contains_key("test-context"),
            "cache should contain entry after append"
        );
        assert_eq!(
            cache.get("test-context").unwrap().entry_count(),
            count_after_save + 1,
            "cache entry_count should increment by 1 after append"
        );
    }

    #[test]
    fn test_append_to_transcript_updates_cache_incrementally() {
        let (app, _temp) = create_test_app();

        // Create context
        let ctx = Context::new("test-context");
        app.save_context(&ctx).unwrap();

        let entry1 = create_user_message_entry("test-context", "Hello", "testuser");
        app.append_to_transcript("test-context", &entry1).unwrap();
        let count_after_first = app
            .active_state_cache
            .borrow()
            .get("test-context")
            .unwrap()
            .entry_count();

        let entry2 = create_user_message_entry("test-context", "World", "testuser");
        app.append_to_transcript("test-context", &entry2).unwrap();

        // Cache should have incremented by exactly 1 from the second append
        let cache = app.active_state_cache.borrow();
        assert_eq!(
            cache.get("test-context").unwrap().entry_count(),
            count_after_first + 1,
            "cache should increment by 1 after each append"
        );
    }

    #[test]
    fn test_destroy_context_invalidates_cache() {
        let (mut app, _temp) = create_test_app();

        // Create context and populate cache
        let ctx = Context::new("test-context");
        app.save_context(&ctx).unwrap();

        let entry = create_user_message_entry("test-context", "Hello", "testuser");
        app.append_to_transcript("test-context", &entry).unwrap();

        // Verify cache has entry
        assert!(app.active_state_cache.borrow().contains_key("test-context"));

        // Destroy context
        app.destroy_context("test-context").unwrap();

        // Cache should be invalidated
        assert!(
            !app.active_state_cache.borrow().contains_key("test-context"),
            "cache should be invalidated after destroy_context"
        );
    }

    #[test]
    fn test_finalize_compaction_invalidates_cache() {
        let (app, _temp) = create_test_app();

        // Create context and populate cache
        let ctx = Context::new("test-context");
        app.save_context(&ctx).unwrap();

        let entry = create_user_message_entry("test-context", "Hello", "testuser");
        app.append_to_transcript("test-context", &entry).unwrap();

        // Verify cache has entry
        assert!(app.active_state_cache.borrow().contains_key("test-context"));

        // Finalize compaction (writes another entry via append_to_transcript,
        // then invalidates cache)
        app.finalize_compaction("test-context", "Test summary")
            .unwrap();

        // Cache should be invalidated
        assert!(
            !app.active_state_cache.borrow().contains_key("test-context"),
            "cache should be invalidated after finalize_compaction"
        );
    }

    #[test]
    fn test_clear_context_invalidates_cache() {
        let (app, _temp) = create_test_app();

        // Create the "default" context with messages so clear_context has something to clear
        let mut ctx = Context::new("default");
        ctx.messages
            .push(Message::new("user".to_string(), "Hello".to_string()));
        app.save_context(&ctx).unwrap();

        // Populate cache explicitly
        let entry = create_user_message_entry("test-context", "Hello", "testuser");
        app.append_to_transcript("default", &entry).unwrap();
        assert!(app.active_state_cache.borrow().contains_key("default"));

        // clear_context writes archival anchor and saves fresh context (both populate
        // cache), then invalidates the cache as the final step
        app.clear_context("default").unwrap();

        // Cache should be absent after clear
        assert!(
            !app.active_state_cache.borrow().contains_key("default"),
            "cache should be invalidated after clear_context"
        );
    }
}
