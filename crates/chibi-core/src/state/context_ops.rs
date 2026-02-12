//! Context CRUD operations for AppState.
//!
//! Methods for creating, loading, saving, clearing, destroying, renaming, and listing contexts.

use crate::context::{Context, ContextEntry, now_timestamp, validate_context_name};
use std::fs;
use std::io::{self, ErrorKind};

use super::{AppState, StatePaths, create_archival_anchor};

impl AppState {
    /// Get or create a context by name.
    ///
    /// Returns an existing context if found, or creates a new empty one.
    /// This is the parameterized version of `get_current_context`.
    pub fn get_or_create_context(&self, name: &str) -> io::Result<Context> {
        self.load_context(name).or_else(|e| {
            if e.kind() == ErrorKind::NotFound {
                // Return empty context if it doesn't exist yet
                Ok(Context::new(name.to_string()))
            } else {
                Err(e)
            }
        })
    }

    pub fn save_and_register_context(&self, context: &Context) -> io::Result<()> {
        self.save_context(context)?;

        // Ensure the context is tracked in state.
        // Important: Read state from disk to get the authoritative list of contexts,
        // rather than using in-memory state which may be stale.
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
        context.messages.push(serde_json::json!({
            "_id": uuid::Uuid::new_v4().to_string(),
            "role": role,
            "content": content,
        }));
        context.updated_at = now_timestamp();
    }

    /// Clear the current context (archive history).
    ///
    /// # Concurrency
    ///
    /// This function is safe to call without holding a `ContextLock` because:
    /// - Transcript writes use `FileLock` via `append_to_transcript`
    /// - Context saves use atomic writes via `save_context`
    ///
    /// If another process is running `send_prompt`, the transcript operations
    /// will serialize correctly via their respective locks.
    pub fn clear_context(&self, context_name: &str) -> io::Result<()> {
        let context = self.get_or_create_context(context_name)?;

        // Don't clear if already empty
        if context.messages.is_empty() {
            return Ok(());
        }

        // Append to transcript.md before clearing (for human-readable archival)
        self.append_to_transcript_md(&context)?;

        // Write archival anchor to transcript.jsonl
        let archival_anchor = create_archival_anchor(&context.name);
        self.append_to_transcript(&context.name, &archival_anchor)?;

        // Mark context dirty so it rebuilds with new anchor on next load
        self.mark_context_dirty(&context.name)?;

        // Create fresh context (preserving nothing - full clear)
        let new_context = Context::new(context_name.to_string());
        self.save_context(&new_context)?;

        // Invalidate active state cache after all writes are complete,
        // so the next append re-scans fresh state from disk
        self.active_state_cache.borrow_mut().remove(&context.name);
        Ok(())
    }

    /// Destroy a context and its directory.
    ///
    /// Returns `true` if the context existed and was destroyed, `false` if it didn't exist.
    ///
    /// Note: Session state (what was current/previous) is managed by CLI. This method
    /// just deletes the context. Caller is responsible for updating session if needed.
    pub fn destroy_context(&mut self, name: &str) -> io::Result<bool> {
        let dir = self.context_dir(name);
        if !dir.exists() {
            return Ok(false);
        }

        // Remove the directory
        fs::remove_dir_all(&dir)?;

        // Invalidate active state cache for the destroyed context
        self.active_state_cache.borrow_mut().remove(name);

        // Update state - remove from contexts list and save
        self.state.contexts.retain(|e| e.name != name);
        self.state.save(&self.state_path)?;

        Ok(true)
    }

    pub fn rename_context(&mut self, old_name: &str, new_name: &str) -> io::Result<()> {
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

        // Update state: preserve created_at from old entry
        let created_at = self
            .state
            .contexts
            .iter()
            .find(|e| e.name == old_name)
            .map(|e| e.created_at)
            .unwrap_or_else(now_timestamp);

        self.state.contexts.retain(|e| e.name != old_name);
        if !self.state.contexts.iter().any(|e| e.name == new_name) {
            self.state
                .contexts
                .push(ContextEntry::with_created_at(new_name, created_at));
        }
        self.state.save(&self.state_path)?;

        // Note: If CLI's session.current_context was renamed, CLI must update it.
        Ok(())
    }

    pub fn list_contexts(&self) -> Vec<String> {
        // state.json is the single source of truth (synced with filesystem on startup)
        self.state.contexts.iter().map(|e| e.name.clone()).collect()
    }
}
