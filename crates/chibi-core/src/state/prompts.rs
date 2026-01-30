//! System prompt and content file handling for AppState.
//!
//! Methods for loading/saving system prompts, todos, goals, and reflection content.

use crate::context::{ENTRY_TYPE_SYSTEM_PROMPT_CHANGED, TranscriptEntry, now_timestamp};
use std::fs;
use std::io;
use uuid::Uuid;

use super::{AppState, StatePaths};

impl AppState {
    /// Load a named prompt from ~/.chibi/prompts/<name>.md
    pub fn load_prompt(&self, name: &str) -> io::Result<String> {
        let prompt_path = self.prompts_dir.join(format!("{}.md", name));
        if prompt_path.exists() {
            fs::read_to_string(&prompt_path)
        } else {
            Ok(String::new())
        }
    }

    // NOTE: load_system_prompt() (no args) was removed in stateless-core refactor.
    // Use load_system_prompt_for(context_name) instead.

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
    /// This also logs a system_prompt_changed event (with full content) to transcript,
    /// updates the mtime in context_meta, and invalidates the prefix.
    pub fn set_system_prompt_for(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let prompt_path = self.context_prompt_file(context_name);

        // Check if content actually changed
        let old_content = if prompt_path.exists() {
            fs::read_to_string(&prompt_path).ok()
        } else {
            None
        };

        crate::safe_io::atomic_write_text(&prompt_path, content)?;

        // Only log change and invalidate if content actually changed
        if old_content.as_deref() != Some(content) {
            // Log system_prompt_changed event to transcript with full raw prompt content
            let entry = TranscriptEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now_timestamp(),
                from: "system".to_string(),
                to: context_name.to_string(),
                content: content.to_string(), // Full raw prompt for history
                entry_type: ENTRY_TYPE_SYSTEM_PROMPT_CHANGED.to_string(),
                metadata: None,
            };
            self.append_to_transcript(context_name, &entry)?;

            // Update mtime in context_meta
            self.update_system_prompt_mtime(context_name)?;

            // Invalidate prefix so context.jsonl will be rebuilt on next load
            self.mark_context_dirty(context_name)?;
        }

        Ok(())
    }

    /// Update system_prompt_md_mtime in context_meta to current file mtime
    fn update_system_prompt_mtime(&self, context_name: &str) -> io::Result<()> {
        let prompt_path = self.context_prompt_file(context_name);
        let mtime = self.get_file_mtime(&prompt_path);

        let mut meta = self.load_context_meta(context_name).unwrap_or_default();
        meta.system_prompt_md_mtime = mtime;
        self.save_context_meta(context_name, &meta)
    }

    /// Get file mtime as unix timestamp, or None if file doesn't exist
    fn get_file_mtime(&self, path: &std::path::Path) -> Option<u64> {
        if path.exists() {
            fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else {
            None
        }
    }

    /// Check if system_prompt.md was modified externally since last tracked mtime.
    /// If so, log the new prompt to transcript, update mtime, and mark context dirty.
    pub(crate) fn check_system_prompt_mtime_change(&self, context_name: &str) -> io::Result<()> {
        let prompt_path = self.context_prompt_file(context_name);
        let current_mtime = self.get_file_mtime(&prompt_path);

        // Load stored mtime from context_meta
        let meta = self.load_context_meta(context_name).unwrap_or_default();
        let stored_mtime = meta.system_prompt_md_mtime;

        // If mtimes differ (including None -> Some or Some -> None), prompt changed
        if current_mtime != stored_mtime {
            // Only log if the file actually exists (has content to log)
            if prompt_path.exists() {
                let content = fs::read_to_string(&prompt_path)?;

                // Log system_prompt_changed event to transcript with full raw prompt
                let entry = TranscriptEntry {
                    id: Uuid::new_v4().to_string(),
                    timestamp: now_timestamp(),
                    from: "system".to_string(),
                    to: context_name.to_string(),
                    content, // Full raw prompt for history
                    entry_type: ENTRY_TYPE_SYSTEM_PROMPT_CHANGED.to_string(),
                    metadata: None,
                };
                self.append_to_transcript(context_name, &entry)?;
            }

            // Update mtime in context_meta
            self.update_system_prompt_mtime(context_name)?;

            // Mark context dirty so it rebuilds
            self.mark_context_dirty(context_name)?;
        }

        Ok(())
    }

    /// Store the combined system prompt (with hook injections) in context_meta.
    /// This allows reconstructing the full API request from context.jsonl + context_meta.json.
    pub fn save_combined_system_prompt(
        &self,
        context_name: &str,
        combined_prompt: &str,
    ) -> io::Result<()> {
        let mut meta = self.load_context_meta(context_name).unwrap_or_default();
        meta.last_combined_prompt = Some(combined_prompt.to_string());
        self.save_context_meta(context_name, &meta)
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

    /// Save todos for a context (atomic write)
    pub fn save_todos(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        crate::safe_io::atomic_write_text(&self.todos_file(context_name), content)
    }

    /// Save goals for a context (atomic write)
    pub fn save_goals(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        crate::safe_io::atomic_write_text(&self.goals_file(context_name), content)
    }

    // NOTE: load_current_todos, load_current_goals, save_current_todos, save_current_goals
    // were removed in the stateless-core refactor. Use the parameterized versions:
    // - load_todos(context_name) / save_todos(context_name, content)
    // - load_goals(context_name) / save_goals(context_name, content)
}
