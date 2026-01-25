//! Application state management.
//!
//! This module manages all persistent state for chibi:
//! - Context files and directories
//! - Configuration loading and resolution
//! - Transcript and inbox operations

pub mod jsonl;

use jsonl::read_jsonl_file;

use crate::config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig};
use crate::context::{
    Context, ContextEntry, ContextMeta, ContextState, ENTRY_TYPE_ARCHIVAL, ENTRY_TYPE_COMPACTION,
    ENTRY_TYPE_CONTEXT_CREATED, ENTRY_TYPE_MESSAGE, ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT,
    EntryMetadata, Message, TranscriptEntry, is_valid_context_name, now_timestamp,
    validate_context_name,
};
use crate::partition::{PartitionManager, StorageConfig};
use dirs_next::home_dir;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub models_config: ModelsConfig,
    pub state: ContextState,
    pub chibi_dir: PathBuf,
    pub state_path: PathBuf,
    pub contexts_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub plugins_dir: PathBuf,
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
            current_context: "default".to_string(),
            previous_context: None,
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
                    current_context: "default".to_string(),
                    previous_context: None,
                }
            })
        } else {
            ContextState {
                contexts: Vec::new(),
                current_context: "default".to_string(),
                previous_context: None,
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
    /// 3. Validate current_context and previous_context references
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
                        // Load created_at from context_meta.json if available
                        let created_at = self
                            .load_context_meta(&name)
                            .map(|m| m.created_at)
                            .unwrap_or_else(|_| now_timestamp());

                        self.state
                            .contexts
                            .push(ContextEntry::with_created_at(name, created_at));
                        modified = true;
                    }
                }
            }
        }

        // Phase 3: Validate current_context reference
        let current_exists = self
            .state
            .contexts
            .iter()
            .any(|e| e.name == self.state.current_context);
        if !current_exists {
            // Fall back to first available context or "default"
            self.state.current_context = self
                .state
                .contexts
                .first()
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "default".to_string());
            modified = true;
        }

        // Phase 4: Validate previous_context reference
        if let Some(ref prev) = self.state.previous_context {
            let prev_exists = self.state.contexts.iter().any(|e| &e.name == prev);
            if !prev_exists {
                self.state.previous_context = None;
                modified = true;
            }
        }

        // Sort by name for consistent ordering
        self.state.contexts.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(modified)
    }

    /// Update the last_activity_at timestamp for a context
    #[allow(dead_code)]
    pub fn touch_context(&mut self, name: &str) -> io::Result<bool> {
        self.touch_context_with_destroy_settings(name, None, None)
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
    /// Only destroys non-current contexts.
    /// Returns the list of destroyed context names.
    pub fn auto_destroy_expired_contexts(&mut self, verbose: bool) -> io::Result<Vec<String>> {
        let current = self.state.current_context.clone();
        let mut destroyed = Vec::new();

        // Collect contexts to destroy (excluding current)
        let to_destroy: Vec<String> = self
            .state
            .contexts
            .iter()
            .filter(|e| e.name != current && e.should_auto_destroy())
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
            // Clear previous_context if it was destroyed
            if self.state.previous_context.as_ref() == Some(&name) {
                self.state.previous_context = None;
            }
            destroyed.push(name);
        }

        Ok(destroyed)
    }

    pub fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir.join(name)
    }

    pub fn context_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.jsonl")
    }

    /// Path to the old context.json format (for migration)
    fn context_file_old(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.json")
    }

    /// Path to context metadata file
    pub fn context_meta_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context_meta.json")
    }

    /// Path to human-readable transcript (transcript.md)
    pub fn transcript_md_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.md")
    }

    /// Alias for transcript_md_file (backwards compatibility)
    pub fn transcript_file(&self, name: &str) -> PathBuf {
        self.transcript_md_file(name)
    }

    /// Path to JSONL transcript (transcript.jsonl) - legacy location
    fn transcript_jsonl_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.jsonl")
    }

    /// Path to partitioned transcript directory
    pub fn transcript_dir(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript")
    }

    /// Path to dirty context marker file (.dirty)
    /// A "dirty" context has a stale prefix (anchor + system prompt) that needs rebuilding.
    /// A "clean" context has a valid prefix that caches well.
    pub fn dirty_marker_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join(".dirty")
    }

    pub fn summary_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("summary.md")
    }

    /// Path to tool cache directory for a context
    pub fn tool_cache_dir(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("tool_cache")
    }

    /// Path to a cached tool output file (kept for potential future use)
    #[allow(dead_code)]
    pub fn cache_file(&self, name: &str, cache_id: &str) -> PathBuf {
        self.tool_cache_dir(name)
            .join(format!("{}.cache", cache_id))
    }

    /// Path to cache metadata file (kept for potential future use)
    #[allow(dead_code)]
    pub fn cache_meta_file(&self, name: &str, cache_id: &str) -> PathBuf {
        self.tool_cache_dir(name)
            .join(format!("{}.meta.json", cache_id))
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

    /// Compute SHA256 hash of system prompt content
    pub fn compute_system_prompt_hash(&self, content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
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
        // Check if prefix is invalidated and rebuild if needed
        if self.is_context_dirty(name) {
            self.rebuild_context_from_transcript(name)?;
            self.mark_context_clean(name)?;
        }

        let entries = self.read_context_entries(name)?;
        let meta = self.load_context_meta(name)?;

        // Load summary from separate file
        let summary_path = self.summary_file(name);
        let summary = if summary_path.exists() {
            fs::read_to_string(&summary_path)?
        } else {
            String::new()
        };

        // Convert entries to messages for backwards compatibility with existing code
        let messages = self.entries_to_messages(&entries);

        // Get updated_at from the last entry timestamp, or created_at if empty
        let updated_at = entries
            .last()
            .map(|e| e.timestamp)
            .unwrap_or(meta.created_at);

        Ok(Context {
            name: name.to_string(),
            messages,
            created_at: meta.created_at,
            updated_at,
            summary,
        })
    }

    /// Rebuild context.jsonl from transcript.jsonl
    /// This creates a fresh context.jsonl with:
    /// - [0] anchor entry (context_created or latest compaction/archival from transcript)
    /// - [1] system_prompt entry with current content and hash
    /// - [2..] entries from transcript since the anchor
    pub fn rebuild_context_from_transcript(&self, name: &str) -> io::Result<()> {
        use crate::context::{
            ENTRY_TYPE_ARCHIVAL, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_CONTEXT_CREATED,
            ENTRY_TYPE_SYSTEM_PROMPT, EntryMetadata,
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
                let meta = self.load_context_meta(name).unwrap_or_default();
                let anchor = TranscriptEntry {
                    id: Uuid::new_v4().to_string(),
                    timestamp: meta.created_at,
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

        // Load current system prompt and compute hash
        let system_prompt_content = self.load_system_prompt_for(name)?;
        let system_prompt_hash = self.compute_system_prompt_hash(&system_prompt_content);

        let system_prompt_entry = TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: "system".to_string(),
            to: name.to_string(),
            content: system_prompt_content,
            entry_type: ENTRY_TYPE_SYSTEM_PROMPT.to_string(),
            metadata: Some(EntryMetadata {
                summary: None,
                hash: Some(system_prompt_hash),
                transcript_anchor_id: Some(anchor_entry.id.clone()),
            }),
        };

        // Build the complete context entries: anchor + system_prompt + conversation entries
        let mut context_entries = vec![anchor_entry, system_prompt_entry];
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

        // Save metadata
        let meta = ContextMeta {
            created_at: old_context.created_at,
        };
        self.save_context_meta(name, &meta)?;

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
            fs::write(&summary_path, &old_context.summary)?;
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

    /// Write entries to context.jsonl (full rewrite)
    pub fn write_context_entries(&self, name: &str, entries: &[TranscriptEntry]) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        let mut writer = BufWriter::new(file);
        for entry in entries {
            let json = serde_json::to_string(entry)
                .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
            writeln!(writer, "{}", json)?;
        }
        Ok(())
    }

    /// Append a single entry to context.jsonl
    pub fn append_context_entry(&self, name: &str, entry: &TranscriptEntry) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
        writeln!(file, "{}", json)?;
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

    /// Save context metadata
    fn save_context_meta(&self, name: &str, meta: &ContextMeta) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_meta_file(name);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        serde_json::to_writer_pretty(BufWriter::new(file), meta)
            .map_err(|e| io::Error::other(format!("Failed to save context meta: {}", e)))
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

        // Save metadata (created_at)
        let meta = ContextMeta {
            created_at: context.created_at,
        };
        self.save_context_meta(&context.name, &meta)?;

        // For brand new contexts, write context_created anchor to transcript.
        // Don't mark dirty - new contexts don't need a rebuild since they're being
        // created fresh. The anchor is just for transcript history.
        if is_new_context {
            let anchor = self.create_context_created_anchor(&context.name);
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
            fs::write(&summary_path, &context.summary)?;
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
    pub fn append_to_transcript(
        &self,
        context_name: &str,
        entry: &TranscriptEntry,
    ) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        self.migrate_transcript_if_needed(context_name)?;
        let transcript_dir = self.transcript_dir(context_name);
        let storage_config = self.get_storage_config(context_name)?;
        let mut pm = PartitionManager::load_with_config(&transcript_dir, storage_config)?;
        pm.append_entry(entry)?;

        // Check if rotation is needed after append
        pm.rotate_if_needed()?;
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

    /// Append an entry to both transcript and context for the current context
    pub fn append_to_current_transcript_and_context(
        &self,
        entry: &TranscriptEntry,
    ) -> io::Result<()> {
        self.append_to_transcript_and_context(&self.state.current_context, entry)
    }

    pub fn get_current_context(&self) -> io::Result<Context> {
        self.load_context(&self.state.current_context).or_else(|e| {
            if e.kind() == ErrorKind::NotFound {
                // Return empty context if it doesn't exist yet
                Ok(Context::new(self.state.current_context.clone()))
            } else {
                Err(e)
            }
        })
    }

    pub fn save_current_context(&self, context: &Context) -> io::Result<()> {
        self.save_context(context)?;

        // Ensure the context is tracked in state.
        // Important: Read state from disk to avoid persisting transient context switches.
        // The in-memory state may have a transient current_context that shouldn't be saved.
        if !self.state.contexts.iter().any(|e| e.name == context.name) {
            let disk_state = if self.state_path.exists() {
                let content = fs::read_to_string(&self.state_path)?;
                serde_json::from_str(&content).unwrap_or_else(|_| self.state.clone())
            } else {
                self.state.clone()
            };
            let mut new_state = disk_state;
            if !new_state.contexts.iter().any(|e| e.name == context.name) {
                new_state.contexts.push(ContextEntry::with_created_at(
                    context.name.clone(),
                    context.created_at,
                ));
            }
            new_state.save(&self.state_path)?;
        }

        Ok(())
    }

    pub fn add_message(&self, context: &mut Context, role: String, content: String) {
        context.messages.push(Message::new(role, content));
        context.updated_at = now_timestamp();
    }

    pub fn clear_context(&self) -> io::Result<()> {
        let context = self.get_current_context()?;

        // Don't clear if already empty
        if context.messages.is_empty() {
            return Ok(());
        }

        // Append to transcript.md before clearing (for human-readable archival)
        self.append_to_transcript_md(&context)?;

        // Write archival anchor to transcript.jsonl
        let archival_anchor = self.create_archival_anchor(&context.name);
        self.append_to_transcript(&context.name, &archival_anchor)?;

        // Mark context dirty so it rebuilds with new anchor on next load
        self.mark_context_dirty(&context.name)?;

        // Create fresh context (preserving nothing - full clear)
        let new_context = Context::new(self.state.current_context.clone());

        self.save_current_context(&new_context)?;
        Ok(())
    }

    /// Destroy a context and its directory.
    /// If destroying the current context, switches to the previous context first
    /// (or "default" if no previous context exists).
    /// Returns the name of the context that was switched to if a switch occurred.
    pub fn destroy_context(&mut self, name: &str) -> io::Result<Option<String>> {
        let dir = self.context_dir(name);
        if !dir.exists() {
            return Ok(None);
        }

        let mut new_state = self.state.clone();
        let switched_to: Option<String>;

        // If destroying the current context, switch to another first
        if self.state.current_context == name {
            // Determine the fallback context
            let fallback = self
                .state
                .previous_context
                .as_ref()
                .filter(|prev| *prev != name && self.context_dir(prev).exists())
                .cloned()
                .unwrap_or_else(|| "default".to_string());

            new_state.current_context = fallback.clone();
            new_state.previous_context = None; // Clear previous after using it
            self.state.current_context = fallback.clone();
            self.state.previous_context = None;
            switched_to = Some(fallback);
        } else {
            // If destroying a context that was the previous context, clear it
            if self.state.previous_context.as_ref() == Some(&name.to_string()) {
                new_state.previous_context = None;
                self.state.previous_context = None;
            }
            switched_to = None;
        }

        // Remove the directory
        fs::remove_dir_all(&dir)?;

        // Update state
        new_state.contexts.retain(|e| e.name != name);
        new_state.save(&self.state_path)?;

        Ok(switched_to)
    }

    pub fn rename_context(&self, old_name: &str, new_name: &str) -> io::Result<()> {
        validate_context_name(new_name)?;

        let old_dir = self.context_dir(old_name);
        let new_dir = self.context_dir(new_name);

        if !old_dir.exists() {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Context '{}' does not exist", old_name),
            ));
        }

        if new_dir.exists() {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("Context '{}' already exists", new_name),
            ));
        }

        // Rename the directory (context name is derived from directory, no file updates needed)
        fs::rename(&old_dir, &new_dir)?;

        // Update state
        let mut new_state = self.state.clone();
        if new_state.current_context == old_name {
            new_state.current_context = new_name.to_string();
        }
        // Preserve created_at from old entry
        let created_at = new_state
            .contexts
            .iter()
            .find(|e| e.name == old_name)
            .map(|e| e.created_at)
            .unwrap_or_else(now_timestamp);
        new_state.contexts.retain(|e| e.name != old_name);
        if !new_state.contexts.iter().any(|e| e.name == new_name) {
            new_state
                .contexts
                .push(ContextEntry::with_created_at(new_name, created_at));
        }
        new_state.save(&self.state_path)?;

        Ok(())
    }

    pub fn list_contexts(&self) -> Vec<String> {
        // state.json is the single source of truth (synced with filesystem on startup)
        self.state.contexts.iter().map(|e| e.name.clone()).collect()
    }

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

    pub fn load_prompt(&self, name: &str) -> io::Result<String> {
        let prompt_path = self.prompts_dir.join(format!("{}.md", name));
        if prompt_path.exists() {
            fs::read_to_string(&prompt_path)
        } else {
            Ok(String::new())
        }
    }

    /// Get the path to a context's system prompt file
    pub fn context_prompt_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("system_prompt.md")
    }

    /// Load the system prompt for the current context.
    /// Returns context-specific prompt if it exists, otherwise falls back to default.
    pub fn load_system_prompt(&self) -> io::Result<String> {
        let context_prompt_path = self.context_prompt_file(&self.state.current_context);
        if context_prompt_path.exists() {
            fs::read_to_string(&context_prompt_path)
        } else {
            // Fall back to default prompt
            self.load_prompt("chibi")
        }
    }

    /// Load the system prompt for a specific context.
    pub fn load_system_prompt_for(&self, context_name: &str) -> io::Result<String> {
        let context_prompt_path = self.context_prompt_file(context_name);
        if context_prompt_path.exists() {
            fs::read_to_string(&context_prompt_path)
        } else {
            // Fall back to default prompt
            self.load_prompt("chibi")
        }
    }

    /// Set a custom system prompt for a specific context.
    /// This also logs a system_prompt_changed event to transcript and invalidates the prefix.
    pub fn set_system_prompt_for(&self, context_name: &str, content: &str) -> io::Result<()> {
        use crate::context::{ENTRY_TYPE_SYSTEM_PROMPT_CHANGED, EntryMetadata};

        self.ensure_context_dir(context_name)?;
        let prompt_path = self.context_prompt_file(context_name);

        // Check if content actually changed
        let old_content = if prompt_path.exists() {
            fs::read_to_string(&prompt_path).ok()
        } else {
            None
        };

        fs::write(&prompt_path, content)?;

        // Only log change and invalidate if content actually changed
        if old_content.as_deref() != Some(content) {
            // Log system_prompt_changed event to transcript
            let hash = self.compute_system_prompt_hash(content);
            let entry = TranscriptEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now_timestamp(),
                from: "system".to_string(),
                to: context_name.to_string(),
                content: "System prompt updated".to_string(),
                entry_type: ENTRY_TYPE_SYSTEM_PROMPT_CHANGED.to_string(),
                metadata: Some(EntryMetadata {
                    summary: None,
                    hash: Some(hash),
                    transcript_anchor_id: None,
                }),
            };
            self.append_to_transcript(context_name, &entry)?;

            // Invalidate prefix so context.jsonl will be rebuilt on next load
            self.mark_context_dirty(context_name)?;
        }

        Ok(())
    }

    /// Load the reflection content from ~/.chibi/prompts/reflection.md
    pub fn load_reflection(&self) -> io::Result<String> {
        self.load_reflection_prompt()
    }

    /// Load todos for a specific context (alias for load_todos)
    pub fn load_todos_for(&self, context_name: &str) -> io::Result<String> {
        self.load_todos(context_name)
    }

    /// Load goals for a specific context (alias for load_goals)
    pub fn load_goals_for(&self, context_name: &str) -> io::Result<String> {
        self.load_goals(context_name)
    }

    /// Clear a context by name (archive its history)
    ///
    /// This mirrors `clear_context()` behavior but operates on any named context,
    /// not just the current one. Both functions:
    /// - Archive messages to transcript.md
    /// - Write an archival anchor to transcript.jsonl
    /// - Mark the context as dirty for rebuild
    /// - Create a fresh context
    pub fn clear_context_by_name(&self, context_name: &str) -> io::Result<()> {
        let context = self.load_context(context_name)?;

        // Don't clear if already empty
        if context.messages.is_empty() {
            return Ok(());
        }

        // Append to transcript.md before clearing (for human-readable archival)
        self.ensure_context_dir(context_name)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.transcript_file(context_name))?;

        for msg in &context.messages {
            if msg.role == "system" {
                continue;
            }
            writeln!(file, "[{}]: {}\n", msg.role.to_uppercase(), msg.content)?;
        }

        // Write archival anchor to transcript.jsonl (like clear_context does)
        let archival_anchor = self.create_archival_anchor(context_name);
        self.append_to_transcript(context_name, &archival_anchor)?;

        // Mark context dirty so it rebuilds with new anchor on next load
        self.mark_context_dirty(context_name)?;

        // Create fresh context
        let new_context = Context::new(context_name);

        self.save_context(&new_context)?;

        // Ensure the context is tracked in state (like clear_context does via save_current_context)
        if !self
            .state
            .contexts
            .iter()
            .any(|e| e.name == new_context.name)
        {
            let mut new_state = self.state.clone();
            new_state.contexts.push(ContextEntry::with_created_at(
                new_context.name.clone(),
                new_context.created_at,
            ));
            new_state.save(&self.state_path)?;
        }

        Ok(())
    }

    pub fn should_auto_compact(&self, context: &Context, resolved_config: &ResolvedConfig) -> bool {
        if !resolved_config.auto_compact {
            return false;
        }
        let tokens = self.calculate_token_count(&context.messages);
        let usage_percent = (tokens as f32 / resolved_config.context_window_limit as f32) * 100.0;
        usage_percent >= resolved_config.auto_compact_threshold
    }

    /// Load the reflection prompt from ~/.chibi/prompts/reflection.md
    /// Returns empty string if the file doesn't exist
    pub fn load_reflection_prompt(&self) -> io::Result<String> {
        let reflection_path = self.prompts_dir.join("reflection.md");
        if reflection_path.exists() {
            fs::read_to_string(&reflection_path)
        } else {
            Ok(String::new())
        }
    }

    // --- Todos and Goals file helpers ---

    /// Get the path to a context's todos file
    pub fn todos_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("todos.md")
    }

    /// Get the path to a context's goals file
    pub fn goals_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("goals.md")
    }

    /// Load todos for a context (returns empty string if file doesn't exist)
    pub fn load_todos(&self, context_name: &str) -> io::Result<String> {
        let path = self.todos_file(context_name);
        if path.exists() {
            fs::read_to_string(&path)
        } else {
            Ok(String::new())
        }
    }

    /// Load goals for a context (returns empty string if file doesn't exist)
    pub fn load_goals(&self, context_name: &str) -> io::Result<String> {
        let path = self.goals_file(context_name);
        if path.exists() {
            fs::read_to_string(&path)
        } else {
            Ok(String::new())
        }
    }

    /// Save todos for a context
    pub fn save_todos(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        fs::write(self.todos_file(context_name), content)
    }

    /// Save goals for a context
    pub fn save_goals(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        fs::write(self.goals_file(context_name), content)
    }

    /// Load todos for current context
    pub fn load_current_todos(&self) -> io::Result<String> {
        self.load_todos(&self.state.current_context)
    }

    /// Load goals for current context
    pub fn load_current_goals(&self) -> io::Result<String> {
        self.load_goals(&self.state.current_context)
    }

    /// Save todos for current context
    pub fn save_current_todos(&self, content: &str) -> io::Result<()> {
        self.save_todos(&self.state.current_context, content)
    }

    /// Save goals for current context
    pub fn save_current_goals(&self, content: &str) -> io::Result<()> {
        self.save_goals(&self.state.current_context, content)
    }

    /// Get the path to a context's local config file
    pub fn local_config_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("local.toml")
    }

    /// Load local config for a context (returns default if doesn't exist)
    pub fn load_local_config(&self, context_name: &str) -> io::Result<LocalConfig> {
        let path = self.local_config_file(context_name);
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse local.toml: {}", e),
                )
            })
        } else {
            Ok(LocalConfig::default())
        }
    }

    /// Save local config for a context
    pub fn save_local_config(
        &self,
        context_name: &str,
        local_config: &LocalConfig,
    ) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let path = self.local_config_file(context_name);
        let content = toml::to_string_pretty(local_config)
            .map_err(|e| io::Error::other(format!("Failed to serialize local.toml: {}", e)))?;
        fs::write(&path, content)
    }

    /// Resolve model name using models.toml aliases
    /// If the model is an alias defined in models.toml, return the full model name
    /// Otherwise return the original model name
    pub fn resolve_model_name(&self, model: &str) -> String {
        if self.models_config.models.contains_key(model) {
            // The model name itself is a key in models.toml, use it as-is
            // (models.toml maps alias -> metadata, not alias -> full name)
            model.to_string()
        } else {
            model.to_string()
        }
    }

    /// Resolve the full configuration, applying overrides in order:
    /// 1. CLI flags (passed as parameters)
    /// 2. Context-local config (local.toml)
    /// 3. Global config (config.toml)
    /// 4. Models.toml (for model expansion)
    /// 5. Defaults
    pub fn resolve_config(
        &self,
        cli_username: Option<&str>,
        cli_temp_username: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        let local = self.load_local_config(&self.state.current_context)?;

        // Start with defaults, then merge global config
        let mut api_params = ApiParams::defaults();
        api_params = api_params.merge_with(&self.config.api);

        // Start with global config values
        let mut resolved = ResolvedConfig {
            api_key: self.config.api_key.clone(),
            model: self.config.model.clone(),
            context_window_limit: self.config.context_window_limit,
            warn_threshold_percent: self.config.warn_threshold_percent,
            base_url: self.config.base_url.clone(),
            auto_compact: self.config.auto_compact,
            auto_compact_threshold: self.config.auto_compact_threshold,
            max_recursion_depth: self.config.max_recursion_depth,
            username: self.config.username.clone(),
            reflection_enabled: self.config.reflection_enabled,
            tool_output_cache_threshold: self.config.tool_output_cache_threshold,
            tool_cache_max_age_days: self.config.tool_cache_max_age_days,
            auto_cleanup_cache: self.config.auto_cleanup_cache,
            tool_cache_preview_chars: self.config.tool_cache_preview_chars,
            file_tools_allowed_paths: self.config.file_tools_allowed_paths.clone(),
            api: api_params,
        };

        // Apply local config overrides
        if let Some(ref api_key) = local.api_key {
            resolved.api_key = api_key.clone();
        }
        if let Some(ref model) = local.model {
            resolved.model = model.clone();
        }
        if let Some(ref base_url) = local.base_url {
            resolved.base_url = base_url.clone();
        }
        if let Some(context_window_limit) = local.context_window_limit {
            resolved.context_window_limit = context_window_limit;
        }
        if let Some(warn_threshold_percent) = local.warn_threshold_percent {
            resolved.warn_threshold_percent = warn_threshold_percent;
        }
        if let Some(auto_compact) = local.auto_compact {
            resolved.auto_compact = auto_compact;
        }
        if let Some(auto_compact_threshold) = local.auto_compact_threshold {
            resolved.auto_compact_threshold = auto_compact_threshold;
        }
        if let Some(max_recursion_depth) = local.max_recursion_depth {
            resolved.max_recursion_depth = max_recursion_depth;
        }
        if let Some(ref username) = local.username {
            resolved.username = username.clone();
        }
        if let Some(reflection_enabled) = local.reflection_enabled {
            resolved.reflection_enabled = reflection_enabled;
        }
        if let Some(tool_output_cache_threshold) = local.tool_output_cache_threshold {
            resolved.tool_output_cache_threshold = tool_output_cache_threshold;
        }
        if let Some(tool_cache_max_age_days) = local.tool_cache_max_age_days {
            resolved.tool_cache_max_age_days = tool_cache_max_age_days;
        }
        if let Some(auto_cleanup_cache) = local.auto_cleanup_cache {
            resolved.auto_cleanup_cache = auto_cleanup_cache;
        }
        if let Some(tool_cache_preview_chars) = local.tool_cache_preview_chars {
            resolved.tool_cache_preview_chars = tool_cache_preview_chars;
        }
        if let Some(ref file_tools_allowed_paths) = local.file_tools_allowed_paths {
            resolved.file_tools_allowed_paths = file_tools_allowed_paths.clone();
        }

        // Apply context-level API params (Layer 3)
        if let Some(ref local_api) = local.api {
            resolved.api = resolved.api.merge_with(local_api);
        }

        // Apply CLI overrides (highest priority)
        // Note: -u (persistent) should have been saved to local.toml before calling this
        // -U (temp) overrides for this invocation only
        if let Some(username) = cli_temp_username {
            resolved.username = username.to_string();
        } else if let Some(username) = cli_username {
            resolved.username = username.to_string();
        }

        // Resolve model name and potentially override context window + API params
        resolved.model = self.resolve_model_name(&resolved.model);
        if let Some(model_meta) = self.models_config.models.get(&resolved.model) {
            // Apply model-level API params (Layer 2 - after global, before context)
            // Note: We merge model params before context params because context should override model
            // But we do this after context-level override for the rest of config, so we need to
            // re-merge context params on top
            let model_api = resolved.api.merge_with(&model_meta.api);
            // Re-apply context-level API params on top of model params
            resolved.api = if let Some(ref local_api) = local.api {
                model_api.merge_with(local_api)
            } else {
                model_api
            };

            if let Some(context_window) = model_meta.context_window {
                resolved.context_window_limit = context_window;
            }
        }

        Ok(resolved)
    }

    /// Read all entries from the context file (context.jsonl) - unified with transcript
    pub fn read_jsonl_transcript(&self, context_name: &str) -> io::Result<Vec<TranscriptEntry>> {
        self.read_context_entries(context_name)
    }

    // === Entry Creation ===
    // These methods create transcript entries using the builder pattern.
    // Each method encapsulates context-specific logic (from/to derivation).

    /// Create a transcript entry for a user message
    pub fn create_user_message_entry(&self, content: &str, username: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from(username)
            .to(&self.state.current_context)
            .content(content)
            .entry_type(ENTRY_TYPE_MESSAGE)
            .build()
    }

    /// Create a transcript entry for an assistant message
    pub fn create_assistant_message_entry(&self, content: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from(&self.state.current_context)
            .to("user")
            .content(content)
            .entry_type(ENTRY_TYPE_MESSAGE)
            .build()
    }

    /// Create a transcript entry for a tool call
    pub fn create_tool_call_entry(&self, tool_name: &str, arguments: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from(&self.state.current_context)
            .to(tool_name)
            .content(arguments)
            .entry_type(ENTRY_TYPE_TOOL_CALL)
            .build()
    }

    /// Create a transcript entry for a tool result
    pub fn create_tool_result_entry(&self, tool_name: &str, result: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from(tool_name)
            .to(&self.state.current_context)
            .content(result)
            .entry_type(ENTRY_TYPE_TOOL_RESULT)
            .build()
    }

    // === Anchor Entry Creation ===

    /// Create a context_created anchor entry
    pub fn create_context_created_anchor(&self, context_name: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from("system")
            .to(context_name)
            .content("Context created")
            .entry_type(ENTRY_TYPE_CONTEXT_CREATED)
            .build()
    }

    /// Create a compaction anchor entry with summary
    pub fn create_compaction_anchor(&self, context_name: &str, summary: &str) -> TranscriptEntry {
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
    pub fn create_archival_anchor(&self, context_name: &str) -> TranscriptEntry {
        TranscriptEntry::builder()
            .from("system")
            .to(context_name)
            .content("Context archived/cleared")
            .entry_type(ENTRY_TYPE_ARCHIVAL)
            .build()
    }

    // === Compaction Finalization ===

    /// Finalize a compaction operation by writing the anchor to transcript and marking dirty.
    /// This is the common final step for all compaction operations (rolling, manual, by-name).
    pub fn finalize_compaction(&self, context_name: &str, summary: &str) -> io::Result<()> {
        let compaction_anchor = self.create_compaction_anchor(context_name, summary);
        self.append_to_transcript(context_name, &compaction_anchor)?;
        self.mark_context_dirty(context_name)?;
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
    fn test_get_current_context_creates_default() {
        let (app, _temp) = create_test_app();
        let context = app.get_current_context().unwrap();
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
        let mut context = app.get_current_context().unwrap();

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

        // Set it as current
        app.state.current_context = "old-name".to_string();

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
        let (app, _temp) = create_test_app();
        let result = app.rename_context("nonexistent", "new-name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_rename_to_existing_context() {
        let (app, _temp) = create_test_app();

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

        // Make sure we're not on this context
        app.state.current_context = "default".to_string();

        // Destroy
        let result = app.destroy_context("to-destroy").unwrap();
        assert!(result.is_none()); // No switch occurred
        assert!(!app.context_dir("to-destroy").exists());
    }

    #[test]
    fn test_destroy_current_context_switches_to_previous() {
        let (mut app, _temp) = create_test_app();

        // Create two contexts
        let ctx1 = Context::new("context-one");
        let ctx2 = Context::new("context-two");
        app.save_context(&ctx1).unwrap();
        app.save_context(&ctx2).unwrap();

        // Set up state: current is context-two, previous is context-one
        app.state.current_context = "context-two".to_string();
        app.state.previous_context = Some("context-one".to_string());

        // Destroy current context
        let result = app.destroy_context("context-two").unwrap();
        assert_eq!(result, Some("context-one".to_string()));
        assert_eq!(app.state.current_context, "context-one");
        assert_eq!(app.state.previous_context, None);
        assert!(!app.context_dir("context-two").exists());
    }

    #[test]
    fn test_destroy_current_context_falls_back_to_default() {
        let (mut app, _temp) = create_test_app();

        // Create a context
        let ctx = Context::new("my-context");
        app.save_context(&ctx).unwrap();

        // Set up state: current is my-context, no previous
        app.state.current_context = "my-context".to_string();
        app.state.previous_context = None;

        // Destroy current context
        let result = app.destroy_context("my-context").unwrap();
        assert_eq!(result, Some("default".to_string()));
        assert_eq!(app.state.current_context, "default");
        assert!(!app.context_dir("my-context").exists());
    }

    #[test]
    fn test_destroy_current_context_skips_deleted_previous() {
        let (mut app, _temp) = create_test_app();

        // Create contexts
        let ctx1 = Context::new("context-one");
        let ctx2 = Context::new("context-two");
        app.save_context(&ctx1).unwrap();
        app.save_context(&ctx2).unwrap();

        // Delete context-one's directory manually (simulate it being gone)
        fs::remove_dir_all(app.context_dir("context-one")).unwrap();

        // Set up state: current is context-two, previous points to non-existent context
        app.state.current_context = "context-two".to_string();
        app.state.previous_context = Some("context-one".to_string());

        // Destroy current context - should fall back to default since previous doesn't exist
        let result = app.destroy_context("context-two").unwrap();
        assert_eq!(result, Some("default".to_string()));
        assert_eq!(app.state.current_context, "default");
    }

    #[test]
    fn test_destroy_nonexistent_context() {
        let (mut app, _temp) = create_test_app();
        let result = app.destroy_context("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_destroy_clears_previous_context_reference() {
        let (mut app, _temp) = create_test_app();

        // Create two contexts
        let ctx1 = Context::new("context-one");
        let ctx2 = Context::new("context-two");
        app.save_context(&ctx1).unwrap();
        app.save_context(&ctx2).unwrap();

        // Set up state: current is context-one, previous is context-two
        app.state.current_context = "context-one".to_string();
        app.state.previous_context = Some("context-two".to_string());

        // Destroy context-two (which is the previous context)
        let result = app.destroy_context("context-two").unwrap();
        assert!(result.is_none()); // No switch occurred
        assert_eq!(app.state.previous_context, None); // Previous should be cleared
        assert!(!app.context_dir("context-two").exists());
    }

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
        let entries = app.load_and_clear_current_inbox().unwrap();
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

        let entries = app.load_and_clear_current_inbox().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "Message 1");
        assert_eq!(entries[1].content, "Message 2");

        // Should be cleared
        let entries_after = app.load_and_clear_current_inbox().unwrap();
        assert!(entries_after.is_empty());
    }

    // === System prompt tests ===

    #[test]
    fn test_set_and_load_system_prompt() {
        let (app, _temp) = create_test_app();

        app.set_system_prompt_for(&app.state.current_context, "You are a helpful assistant.")
            .unwrap();
        let loaded = app.load_system_prompt().unwrap();
        assert_eq!(loaded, "You are a helpful assistant.");
    }

    #[test]
    fn test_system_prompt_fallback() {
        let (app, _temp) = create_test_app();

        // Write default prompt
        fs::write(app.prompts_dir.join("chibi.md"), "Default prompt").unwrap();

        // No context-specific prompt, should fall back
        let loaded = app.load_system_prompt().unwrap();
        assert_eq!(loaded, "Default prompt");
    }

    // === Config resolution tests ===

    #[test]
    fn test_resolve_config_defaults() {
        let (app, _temp) = create_test_app();
        let resolved = app.resolve_config(None, None).unwrap();

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

        let resolved = app.resolve_config(None, None).unwrap();
        assert_eq!(resolved.model, "local-model");
        assert_eq!(resolved.username, "localuser");
        assert!(resolved.auto_compact);
    }

    #[test]
    fn test_resolve_config_cli_override() {
        let (app, _temp) = create_test_app();

        // Set local config
        let local = LocalConfig {
            username: Some("localuser".to_string()),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        // CLI temp username should override local
        let resolved = app.resolve_config(None, Some("cliuser")).unwrap();
        assert_eq!(resolved.username, "cliuser");
    }

    #[test]
    fn test_resolve_config_api_params_global_defaults() {
        let (app, _temp) = create_test_app();
        let resolved = app.resolve_config(None, None).unwrap();

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

        let resolved = app.resolve_config(None, None).unwrap();

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

        let resolved = app.resolve_config(None, None).unwrap();

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

        let resolved = app.resolve_config(None, None).unwrap();

        // Context should override model
        assert_eq!(resolved.api.temperature, Some(0.9));
        // Model value should be preserved when context doesn't override
        assert_eq!(resolved.api.max_tokens, Some(1000));
    }

    #[test]
    fn test_resolve_config_cli_persistent_username() {
        let (app, _temp) = create_test_app();

        // CLI persistent username (simulates -u flag)
        let resolved = app.resolve_config(Some("persistentuser"), None).unwrap();
        assert_eq!(resolved.username, "persistentuser");
    }

    #[test]
    fn test_resolve_config_cli_temp_username_over_persistent() {
        let (app, _temp) = create_test_app();

        // Temp username should override persistent
        let resolved = app
            .resolve_config(Some("persistentuser"), Some("tempuser"))
            .unwrap();
        assert_eq!(resolved.username, "tempuser");
    }

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
            storage: StorageConfig::default(),
        };
        app.save_local_config("default", &local).unwrap();

        let resolved = app.resolve_config(None, None).unwrap();

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

    // === Transcript entry creation tests ===

    #[test]
    fn test_create_user_message_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_user_message_entry("Hello", "alice");

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
        let entry = app.create_assistant_message_entry("Hi there!");

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "user");
        assert_eq!(entry.content, "Hi there!");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_MESSAGE);
    }

    #[test]
    fn test_create_tool_call_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_tool_call_entry("web_search", r#"{"query": "rust"}"#);

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "web_search");
        assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_TOOL_CALL);
    }

    #[test]
    fn test_create_tool_result_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_tool_result_entry("web_search", "Search results...");

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
        app.state.contexts.push(ContextEntry::new("context-one"));
        app.state.contexts.push(ContextEntry::new("context-two"));
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
    fn test_save_context_adds_to_state_contexts() {
        // Verify that saving a new context adds it to state.json
        let (app, _temp) = create_test_app();

        assert!(!app.state.contexts.iter().any(|e| e.name == "new-context"));

        let ctx = Context::new("new-context");
        app.save_current_context(&ctx).unwrap();

        // Need to reload state to see the change
        // Actually save_current_context should add it - let's verify
        // by checking the state file directly
        let state_content = fs::read_to_string(&app.state_path).unwrap();
        assert!(
            state_content.contains("new-context"),
            "New context should be added to state.json"
        );
    }

    #[test]
    fn test_save_current_context_preserves_disk_current_context() {
        // Verify that save_current_context doesn't persist in-memory current_context
        // This is critical for transient context (-C) support
        let (mut app, _temp) = create_test_app();

        // Save initial state with "default" as current_context
        app.save().unwrap();

        // Simulate transient context switch (in-memory only)
        app.state.current_context = "transient-ctx".to_string();

        // Create and save a new context while "transient-ctx" is in memory
        let ctx = Context::new("new-context");
        app.save_current_context(&ctx).unwrap();

        // Verify the state file still has original current_context, not the transient one
        let state_content = fs::read_to_string(&app.state_path).unwrap();
        let saved_state: ContextState = serde_json::from_str(&state_content).unwrap();

        assert_eq!(
            saved_state.current_context, "default",
            "Transient current_context should not be persisted to state.json"
        );
        assert!(
            saved_state.contexts.iter().any(|e| e.name == "new-context"),
            "New context should still be added to contexts list"
        );
    }

    // === clear_context_by_name archival anchor tests (Bug #2) ===

    #[test]
    fn test_clear_context_by_name_writes_archival_anchor() {
        // BUG: clear_context_by_name should write an archival anchor to transcript.jsonl
        // like clear_context does, but currently it doesn't
        let (app, _temp) = create_test_app();

        // Create a context with some messages
        let mut ctx = Context::new("test-context");
        ctx.messages.push(Message::new("user", "Hello"));
        ctx.messages.push(Message::new("assistant", "Hi there"));
        app.save_context(&ctx).unwrap();

        // Create transcript.jsonl with the messages
        let user_entry = app.create_user_message_entry("Hello", "testuser");
        let asst_entry = app.create_assistant_message_entry("Hi there");
        app.append_to_transcript("test-context", &user_entry)
            .unwrap();
        app.append_to_transcript("test-context", &asst_entry)
            .unwrap();

        // Clear the context by name
        app.clear_context_by_name("test-context").unwrap();

        // Read transcript and check for archival anchor
        let entries = app.read_transcript_entries("test-context").unwrap();
        let has_archival = entries
            .iter()
            .any(|e| e.entry_type == crate::context::ENTRY_TYPE_ARCHIVAL);

        assert!(
            has_archival,
            "clear_context_by_name should write an archival anchor to transcript"
        );
    }

    #[test]
    fn test_clear_context_by_name_marks_dirty() {
        // clear_context_by_name should mark the context as dirty for rebuild
        let (app, _temp) = create_test_app();

        // Create a context with messages
        let mut ctx = Context::new("test-context");
        ctx.messages.push(Message::new("user", "Hello"));
        app.save_context(&ctx).unwrap();

        // Clear should mark dirty
        app.clear_context_by_name("test-context").unwrap();

        assert!(
            app.is_context_dirty("test-context"),
            "clear_context_by_name should mark context as dirty"
        );
    }

    // === Touch context tests ===

    #[test]
    fn test_touch_context_updates_last_activity() {
        let (mut app, _temp) = create_test_app();

        // Add an entry to state.contexts manually (save_context doesn't do this)
        let entry = ContextEntry::new("test-context");
        let initial_activity = entry.last_activity_at;
        app.state.contexts.push(entry);

        // Create the context directory
        let ctx = Context::new("test-context");
        app.save_context(&ctx).unwrap();

        // Touch the context
        let result = app.touch_context("test-context").unwrap();
        assert!(result);

        // Check that last_activity_at was updated
        let entry = app
            .state
            .contexts
            .iter()
            .find(|e| e.name == "test-context")
            .unwrap();
        assert!(entry.last_activity_at >= initial_activity);
    }

    #[test]
    fn test_touch_context_nonexistent_returns_false() {
        let (mut app, _temp) = create_test_app();
        let result = app.touch_context("nonexistent").unwrap();
        assert!(!result);
    }

    // === Auto-destroy tests ===

    #[test]
    fn test_auto_destroy_expired_contexts_by_timestamp() {
        let (mut app, _temp) = create_test_app();

        // Create a context to be destroyed
        let ctx = Context::new("to-destroy");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with destroy_at in the past
        let mut entry = ContextEntry::new("to-destroy");
        entry.destroy_at = 1; // Way in the past
        app.state.contexts.push(entry);

        // Also set the current context to something else
        app.state.current_context = "default".to_string();

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
        let mut entry = ContextEntry::new("to-destroy");
        entry.last_activity_at = 1; // Way in the past
        entry.destroy_after_seconds_inactive = 60; // 1 minute
        app.state.contexts.push(entry);

        // Also set the current context to something else
        app.state.current_context = "default".to_string();

        // Run auto-destroy
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert_eq!(destroyed, vec!["to-destroy".to_string()]);
        assert!(!app.context_dir("to-destroy").exists());
    }

    #[test]
    fn test_auto_destroy_skips_current_context() {
        let (mut app, _temp) = create_test_app();

        // Create a context to be destroyed
        let ctx = Context::new("current-context");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with destroy_at in the past
        let mut entry = ContextEntry::new("current-context");
        entry.destroy_at = 1; // Way in the past
        app.state.contexts.push(entry);

        // Set this as the current context
        app.state.current_context = "current-context".to_string();

        // Run auto-destroy - should NOT destroy the current context
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert!(destroyed.is_empty());
        assert!(app.context_dir("current-context").exists());
    }

    #[test]
    fn test_auto_destroy_clears_previous_context_reference() {
        let (mut app, _temp) = create_test_app();

        // Create contexts
        let ctx1 = Context::new("context-one");
        let ctx2 = Context::new("context-two");
        app.save_context(&ctx1).unwrap();
        app.save_context(&ctx2).unwrap();

        // Add entries to state.contexts
        app.state.contexts.push(ContextEntry::new("context-one"));
        let mut entry2 = ContextEntry::new("context-two");
        entry2.destroy_at = 1; // Way in the past
        app.state.contexts.push(entry2);

        // Set up state: current is context-one, previous is context-two
        app.state.current_context = "context-one".to_string();
        app.state.previous_context = Some("context-two".to_string());

        // Run auto-destroy
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert_eq!(destroyed, vec!["context-two".to_string()]);
        assert_eq!(app.state.previous_context, None); // Should be cleared
    }

    #[test]
    fn test_auto_destroy_respects_disabled_settings() {
        let (mut app, _temp) = create_test_app();

        // Create a context
        let ctx = Context::new("keep-context");
        app.save_context(&ctx).unwrap();

        // Add entry to state.contexts with settings that should NOT trigger destroy
        let mut entry = ContextEntry::new("keep-context");
        entry.last_activity_at = 1; // Way in the past
        entry.destroy_after_seconds_inactive = 0; // Disabled
        entry.destroy_at = 0; // Disabled
        app.state.contexts.push(entry);

        // Also set the current context to something else
        app.state.current_context = "default".to_string();

        // Run auto-destroy - should NOT destroy since both are disabled
        let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
        assert!(destroyed.is_empty());
        assert!(app.context_dir("keep-context").exists());
    }
}
