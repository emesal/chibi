//! Inbox operations for inter-context messaging.
//!
//! This module extends `AppState` with methods for managing context inboxes,
//! which enable asynchronous communication between contexts.

use crate::context::{InboxEntry, now_timestamp};
use crate::safe_io::FileLock;
use crate::state::AppState;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use uuid::Uuid;

impl AppState {
    /// Get the path to a context's inbox file
    pub fn inbox_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("inbox.jsonl")
    }

    /// Get the path to a context's inbox lock file
    pub fn inbox_lock_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join(".inbox.lock")
    }

    /// Send a message to another context's inbox
    pub fn send_inbox_message(&self, to_context: &str, message: &str) -> io::Result<()> {
        let entry = InboxEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: self.state.current_context.clone(),
            to: to_context.to_string(),
            content: message.to_string(),
        };
        self.append_to_inbox(to_context, &entry)
    }

    /// Append a message to a context's inbox with exclusive locking
    pub fn append_to_inbox(&self, context_name: &str, entry: &InboxEntry) -> io::Result<()> {
        // Ensure context directory exists
        let context_dir = self.context_dir(context_name);
        std::fs::create_dir_all(&context_dir)?;

        let lock_path = self.inbox_lock_file(context_name);
        let inbox_path = self.inbox_file(context_name);

        // Acquire RAII lock - released automatically on drop
        let _lock = FileLock::acquire(&lock_path)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inbox_path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize error: {}", e)))?;
        writeln!(file, "{}", json)?;

        Ok(())
    }

    /// Load and clear a context's inbox atomically.
    ///
    /// This is the parameterized version - use this when you know the context name.
    pub fn load_and_clear_inbox(&self, context_name: &str) -> io::Result<Vec<InboxEntry>> {
        let lock_path = self.inbox_lock_file(context_name);
        let inbox_path = self.inbox_file(context_name);

        // If inbox doesn't exist, return empty
        if !inbox_path.exists() {
            return Ok(Vec::new());
        }

        // Acquire RAII lock - released automatically on drop
        let _lock = FileLock::acquire(&lock_path)?;

        // Read all entries
        let file = File::open(&inbox_path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<InboxEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => eprintln!("[Warning: Failed to parse inbox entry: {}]", e),
            }
        }

        // Clear the inbox by truncating the file
        // If truncation fails, warn but still return entries (avoid silent data loss)
        if !entries.is_empty()
            && let Err(e) = File::create(&inbox_path)
        {
            eprintln!(
                "[Warning: Failed to clear inbox after reading {} message(s): {}. Messages may be re-delivered.]",
                entries.len(),
                e
            );
        }

        Ok(entries)
    }

    /// Load and clear the current context's inbox atomically.
    ///
    /// Convenience wrapper around `load_and_clear_inbox` using `state.current_context`.
    pub fn load_and_clear_current_inbox(&self) -> io::Result<Vec<InboxEntry>> {
        self.load_and_clear_inbox(&self.state.current_context)
    }
}
