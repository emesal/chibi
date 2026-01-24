//! Path helper methods for context directories and files.
//!
//! All methods return paths relative to the chibi directory structure.
//! This trait is extracted for potential future use in migrating AppState path methods.

#![allow(dead_code)]

use std::path::PathBuf;

/// Extension trait for path helpers that require AppState fields.
/// This trait is implemented on AppState in mod.rs.
pub trait PathHelpers {
    fn contexts_dir(&self) -> &PathBuf;

    /// Get the directory for a specific context
    fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir().join(name)
    }

    /// Get the path to context.jsonl (the LLM context window)
    fn context_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.jsonl")
    }

    /// Path to the old context.json format (for migration)
    fn context_file_old(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.json")
    }

    /// Path to context metadata file
    fn context_meta_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context_meta.json")
    }

    /// Path to human-readable transcript (transcript.md)
    fn transcript_md_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.md")
    }

    /// Alias for transcript_md_file (backwards compatibility)
    fn transcript_file(&self, name: &str) -> PathBuf {
        self.transcript_md_file(name)
    }

    /// Path to JSONL transcript (transcript.jsonl) - the authoritative log
    fn transcript_jsonl_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.jsonl")
    }

    /// Path to dirty context marker file (.dirty)
    fn dirty_marker_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join(".dirty")
    }

    /// Path to summary file
    fn summary_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("summary.md")
    }

    /// Path to context-specific system prompt
    fn context_prompt_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("system_prompt.md")
    }

    /// Path to context todos file
    fn todos_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("todos.md")
    }

    /// Path to context goals file
    fn goals_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("goals.md")
    }

    /// Path to context local config file
    fn local_config_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("local.toml")
    }

    /// Path to context inbox file
    fn inbox_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("inbox.jsonl")
    }
}
