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
    create_context_created_anchor, create_flow_control_call_entry,
    create_flow_control_result_entry, create_tool_call_entry, create_tool_result_entry,
    create_user_message_entry,
};
pub use paths::StatePaths;

use crate::jsonl::read_jsonl_file;

use crate::config::{Config, ConfigDefaults, ModelsConfig, ResolvedConfig};
// Note: ImageConfig, MarkdownStyle removed - these are CLI presentation concerns
use crate::context::{
    Context, ContextEntry, ContextMeta, ContextState, TranscriptEntry, is_valid_context_name,
    now_timestamp,
};
use crate::partition::{ActiveState, PartitionManager};
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
    /// Shared virtual file system.
    pub vfs: crate::vfs::Vfs,
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

        let vfs_root = chibi_dir.join("vfs");
        if config.vfs.backend != "local" {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "unsupported VFS backend '{}' (only 'local' is built in)",
                    config.vfs.backend
                ),
            ));
        }
        fs::create_dir_all(&vfs_root)?;
        let vfs_backend = crate::vfs::LocalBackend::new(vfs_root);
        let vfs = crate::vfs::Vfs::new(Box::new(vfs_backend));

        Ok(AppState {
            config,
            models_config: ModelsConfig::default(),
            state,
            chibi_dir,
            state_path,
            contexts_dir,
            prompts_dir,
            plugins_dir,
            vfs,
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
            Config::default()
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

        let vfs_root = chibi_dir.join("vfs");
        if config.vfs.backend != "local" {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "unsupported VFS backend '{}' (only 'local' is built in)",
                    config.vfs.backend
                ),
            ));
        }
        fs::create_dir_all(&vfs_root)?;
        let vfs_backend = crate::vfs::LocalBackend::new(vfs_root);
        let vfs = crate::vfs::Vfs::new(Box::new(vfs_backend));

        let mut app = AppState {
            config,
            models_config,
            state,
            chibi_dir,
            state_path,
            contexts_dir,
            prompts_dir,
            plugins_dir,
            vfs,
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
    /// The destroy settings come from `ExecutionFlags.destroy_at` and
    /// `ExecutionFlags.destroy_after_seconds_inactive`.
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
    pub fn auto_destroy_expired_contexts(&mut self) -> io::Result<Vec<String>> {
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

    /// Clear the tool cache for a context (deletes all entries from VFS).
    pub async fn clear_tool_cache(&self, name: &str) -> io::Result<()> {
        let dir_str = format!("/sys/tool_cache/{}", name);
        let dir = crate::vfs::VfsPath::new(&dir_str)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        match self.vfs.delete(crate::vfs::SYSTEM_CALLER, &dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Clean up tool cache entries older than `max_age_days` for a single context.
    /// Returns the number of entries removed.
    pub async fn cleanup_tool_cache(
        &self,
        context_name: &str,
        max_age_days: u64,
    ) -> io::Result<usize> {
        use crate::vfs::VfsEntryKind;

        let dir_str = format!("/sys/tool_cache/{}", context_name);
        let dir = match crate::vfs::VfsPath::new(&dir_str) {
            Ok(p) => p,
            Err(_) => return Ok(0),
        };

        let entries = match self.vfs.list(crate::vfs::SYSTEM_CALLER, &dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };

        let mut removed = 0;
        for entry in entries {
            if entry.kind != VfsEntryKind::File {
                continue;
            }
            let file_path_str = format!("{}/{}", dir_str, entry.name);
            let file_path = match crate::vfs::VfsPath::new(&file_path_str) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let meta = match self
                .vfs
                .metadata(crate::vfs::SYSTEM_CALLER, &file_path)
                .await
            {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(created) = meta.created
                && is_cache_entry_expired(created, max_age_days)
            {
                let _ = self.vfs.delete(crate::vfs::SYSTEM_CALLER, &file_path).await;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Clean up tool cache entries for all contexts. Returns total entries removed.
    pub async fn cleanup_all_tool_caches(&self, max_age_days: u64) -> io::Result<usize> {
        use crate::vfs::VfsEntryKind;

        let root = crate::vfs::VfsPath::new("/sys/tool_cache")
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

        let ctx_dirs = match self.vfs.list(crate::vfs::SYSTEM_CALLER, &root).await {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };

        let mut total = 0;
        for dir in ctx_dirs {
            if dir.kind == VfsEntryKind::Directory {
                total += self.cleanup_tool_cache(&dir.name, max_age_days).await?;
            }
        }
        Ok(total)
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
                    .filter(|e| is_context_entry(e))
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
                    tool_call_id: None,
                };
                // Include all transcript entries that belong in context
                let entries: Vec<_> = transcript_entries
                    .iter()
                    .filter(|e| is_context_entry(e))
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
        let storage_config = self.resolve_config(name, None)?.storage;
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

    /// Convert transcript entries to JSON messages, preserving tool history.
    ///
    /// Walks entries sequentially:
    /// - "message" entries → `{"_id", "role", "content"}`
    /// - "tool_call" entries → grouped into API-turn batches, each emitted as an
    ///   assistant message with `tool_calls[]` followed by individual tool result messages
    /// - Other entry types (compaction, archival, context_created, system_prompt_changed)
    ///   are skipped.
    ///
    /// **Batch grouping:** A batch boundary is determined structurally — a leading run of
    /// consecutive `tool_call` entries (no interleaved results) forms one API-turn batch,
    /// followed by exactly that many `tool_result` entries. This correctly handles:
    /// - `tc tc tr tr` → one batch (parallel tool calls in a single API response)
    /// - `tc tr tc tr` → two batches (sequential single-tool API responses)
    /// - `tc tc tr tr tc tr` → two batches (parallel then sequential)
    ///
    /// Backward compat: old interleaved entries without tool_call_id are paired by
    /// position with synthetic IDs derived from the tool_call entry's own ID.
    fn entries_to_messages(&self, entries: &[TranscriptEntry]) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();
        let mut i = 0;

        while i < entries.len() {
            let entry = &entries[i];

            match entry.entry_type.as_str() {
                crate::context::ENTRY_TYPE_MESSAGE => {
                    let role = if entry.to == "user" {
                        "assistant"
                    } else {
                        "user"
                    };
                    messages.push(serde_json::json!({
                        "_id": entry.id,
                        "role": role,
                        "content": entry.content,
                    }));
                    i += 1;
                }
                crate::context::ENTRY_TYPE_TOOL_CALL => {
                    // Collect the leading run of tool_call entries — these were all
                    // returned in a single API response (parallel tool calls).
                    // Stop at the first non-tool_call entry.
                    let mut tool_calls: Vec<&TranscriptEntry> = Vec::new();
                    while i < entries.len()
                        && entries[i].entry_type == crate::context::ENTRY_TYPE_TOOL_CALL
                    {
                        tool_calls.push(&entries[i]);
                        i += 1;
                    }

                    // Collect exactly tool_calls.len() tool_result entries that follow.
                    // Stop early at the first non-tool_result, preserving any remaining
                    // tool_call entries for the next iteration (next API turn).
                    let mut tool_results: Vec<&TranscriptEntry> = Vec::new();
                    while tool_results.len() < tool_calls.len()
                        && i < entries.len()
                        && entries[i].entry_type == crate::context::ENTRY_TYPE_TOOL_RESULT
                    {
                        tool_results.push(&entries[i]);
                        i += 1;
                    }

                    // Build assistant message with tool_calls array
                    let tool_calls_json: Vec<serde_json::Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            let tc_id = tc
                                .tool_call_id
                                .clone()
                                .unwrap_or_else(|| format!("synth_{}", tc.id));
                            serde_json::json!({
                                "id": tc_id,
                                "type": "function",
                                "function": {
                                    "name": tc.to,
                                    "arguments": tc.content,
                                }
                            })
                        })
                        .collect();

                    messages.push(serde_json::json!({
                        "_id": tool_calls.first().map(|tc| tc.id.as_str()).unwrap_or(""),
                        "role": "assistant",
                        "tool_calls": tool_calls_json,
                    }));

                    // Emit tool result messages, paired by tool_call_id or position
                    for (idx, tr) in tool_results.iter().enumerate() {
                        let tc_id = if let Some(ref id) = tr.tool_call_id {
                            id.clone()
                        } else if idx < tool_calls.len() {
                            // Backward compat: pair by position with synthetic ID
                            tool_calls[idx]
                                .tool_call_id
                                .clone()
                                .unwrap_or_else(|| format!("synth_{}", tool_calls[idx].id))
                        } else {
                            format!("synth_{}", tr.id)
                        };
                        messages.push(serde_json::json!({
                            "_id": tr.id,
                            "role": "tool",
                            "tool_call_id": tc_id,
                            "content": tr.content,
                        }));
                    }
                }
                crate::context::ENTRY_TYPE_TOOL_RESULT => {
                    // Orphaned tool result (shouldn't happen but handle gracefully)
                    let tc_id = entry
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("synth_{}", entry.id));
                    messages.push(serde_json::json!({
                        "_id": entry.id,
                        "role": "tool",
                        "tool_call_id": tc_id,
                        "content": entry.content,
                    }));
                    i += 1;
                }
                _ => {
                    // Skip non-message types (compaction, archival, context_created, etc.)
                    i += 1;
                }
            }
        }

        messages
    }

    /// Convert JSON messages back to transcript entries (for save_context).
    ///
    /// Decomposes JSON values:
    /// - role "user" or "assistant" (no tool_calls) → ENTRY_TYPE_MESSAGE
    /// - role "assistant" with tool_calls → N ENTRY_TYPE_TOOL_CALL entries
    /// - role "tool" → ENTRY_TYPE_TOOL_RESULT entry
    /// - role "system" → skipped
    fn messages_to_entries(
        &self,
        messages: &[serde_json::Value],
        context_name: &str,
    ) -> Vec<TranscriptEntry> {
        let mut entries = Vec::new();

        for m in messages {
            let role = m["role"].as_str().unwrap_or("");
            match role {
                "system" => continue,
                "assistant" => {
                    if let Some(tool_calls) = m["tool_calls"].as_array() {
                        // Assistant message with tool calls → one entry per tool call
                        for tc in tool_calls {
                            let name = tc["function"]["name"].as_str().unwrap_or("unknown");
                            let arguments = tc["function"]["arguments"].as_str().unwrap_or("{}");
                            let tc_id = tc["id"].as_str().unwrap_or("");
                            let mut builder = TranscriptEntry::builder()
                                .from(context_name)
                                .to(name)
                                .content(arguments)
                                .entry_type(crate::context::ENTRY_TYPE_TOOL_CALL);
                            if !tc_id.is_empty() {
                                builder = builder.tool_call_id(tc_id);
                            }
                            entries.push(builder.build());
                        }
                    } else {
                        // Plain assistant message
                        let content = m["content"].as_str().unwrap_or("");
                        entries.push(
                            TranscriptEntry::builder()
                                .from(context_name)
                                .to("user")
                                .content(content)
                                .entry_type(crate::context::ENTRY_TYPE_MESSAGE)
                                .build(),
                        );
                    }
                }
                "tool" => {
                    let tc_id = m["tool_call_id"].as_str().unwrap_or("");
                    let content = m["content"].as_str().unwrap_or("");
                    // Use _id as a hint for tool name if available, otherwise "tool"
                    let tool_name = "tool";
                    let mut builder = TranscriptEntry::builder()
                        .from(tool_name)
                        .to(context_name)
                        .content(content)
                        .entry_type(crate::context::ENTRY_TYPE_TOOL_RESULT);
                    if !tc_id.is_empty() {
                        builder = builder.tool_call_id(tc_id);
                    }
                    entries.push(builder.build());
                }
                _ => {
                    // "user" or unknown → user message
                    let content = m["content"].as_str().unwrap_or("");
                    entries.push(
                        TranscriptEntry::builder()
                            .from("user")
                            .to(context_name)
                            .content(content)
                            .entry_type(crate::context::ENTRY_TYPE_MESSAGE)
                            .build(),
                    );
                }
            }
        }

        entries
    }

    pub fn save_context(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;

        // Check if this is a brand new context (no transcript/manifest.json exists yet)
        let transcript_manifest = self.transcript_dir(&context.name).join("manifest.json");
        let is_new_context = !transcript_manifest.exists();

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
        let storage_config = self.resolve_config(context_name, None)?.storage;

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

    pub fn calculate_token_count(&self, messages: &[serde_json::Value]) -> usize {
        // Rough estimation: serialize each message and divide by 4 chars per token.
        // More accurate than the old version since it includes tool call arguments/results.
        messages
            .iter()
            .map(|m| serde_json::to_string(m).map(|s| s.len() / 4).unwrap_or(0))
            .sum()
    }

    pub fn should_warn(&self, messages: &[serde_json::Value]) -> bool {
        let limit = self
            .config
            .context_window_limit
            .unwrap_or(ConfigDefaults::CONTEXT_WINDOW_LIMIT);
        if limit == 0 {
            return false;
        }
        let tokens = self.calculate_token_count(messages);
        let usage_percent = (tokens as f32 / limit as f32) * 100.0;
        usage_percent >= self.config.warn_threshold_percent
    }

    pub fn remaining_tokens(&self, messages: &[serde_json::Value]) -> usize {
        let limit = self
            .config
            .context_window_limit
            .unwrap_or(ConfigDefaults::CONTEXT_WINDOW_LIMIT);
        if limit == 0 {
            return usize::MAX;
        }
        let tokens = self.calculate_token_count(messages);
        limit.saturating_sub(tokens)
    }

    pub fn should_auto_compact(&self, context: &Context, resolved_config: &ResolvedConfig) -> bool {
        if !resolved_config.auto_compact {
            return false;
        }
        // Skip compaction if context window is unknown (0 = not yet resolved)
        if resolved_config.context_window_limit == 0 {
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

/// Returns true if a transcript entry should be included in context.jsonl.
///
/// Transcript-only entries (never written to context):
/// - `system_prompt_changed` — prompt change events, stored in context_meta.json
/// - `flow_control_call` / `flow_control_result` — chibi plumbing (call_user/call_agent);
///   must not appear in LLM message history
fn is_context_entry(entry: &TranscriptEntry) -> bool {
    !matches!(
        entry.entry_type.as_str(),
        crate::context::ENTRY_TYPE_SYSTEM_PROMPT_CHANGED
            | crate::context::ENTRY_TYPE_FLOW_CONTROL_CALL
            | crate::context::ENTRY_TYPE_FLOW_CONTROL_RESULT
    )
}

/// Check whether a cache entry's creation timestamp is older than `max_age_days`.
///
/// The `+1` offset means `max_age_days=0` tolerates entries less than 1 day old,
/// preventing accidental deletion of entries created during the current session.
pub(crate) fn is_cache_entry_expired(
    created: chrono::DateTime<chrono::Utc>,
    max_age_days: u64,
) -> bool {
    let max_age = chrono::Duration::days((max_age_days + 1) as i64);
    let cutoff = chrono::Utc::now() - max_age;
    created < cutoff
}

#[cfg(test)]
mod tests;
