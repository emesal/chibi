//! Path computation methods for AppState.
//!
//! All path methods are pure computations with no I/O - they just construct
//! PathBuf values based on the context name and chibi directory structure.

use std::path::PathBuf;

/// Trait providing path computation methods for context directories and files.
pub trait StatePaths {
    /// Get the contexts directory root
    fn contexts_dir(&self) -> &PathBuf;

    /// Path to a context's directory
    fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir().join(name)
    }

    /// Path to context.jsonl file
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

    /// Path to JSONL transcript (transcript.jsonl) - legacy location
    fn transcript_jsonl_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.jsonl")
    }

    /// Path to partitioned transcript directory
    fn transcript_dir(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript")
    }

    /// Path to dirty context marker file (.dirty)
    /// A "dirty" context has a stale prefix (anchor + system prompt) that needs rebuilding.
    /// A "clean" context has a valid prefix that caches well.
    fn dirty_marker_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join(".dirty")
    }

    /// Path to summary file
    fn summary_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("summary.md")
    }

    /// Path to tool cache directory for a context
    fn tool_cache_dir(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("tool_cache")
    }

    /// Path to a cached tool output file
    fn cache_file(&self, name: &str, cache_id: &str) -> PathBuf {
        self.tool_cache_dir(name)
            .join(format!("{}.cache", cache_id))
    }

    /// Path to cache metadata file
    fn cache_meta_file(&self, name: &str, cache_id: &str) -> PathBuf {
        self.tool_cache_dir(name)
            .join(format!("{}.meta.json", cache_id))
    }

    /// Get the path to a context's system prompt file
    fn context_prompt_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("system_prompt.md")
    }

    /// Get the path to a context's todos file
    fn todos_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("todos.md")
    }

    /// Get the path to a context's goals file
    fn goals_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("goals.md")
    }

    /// Get the path to a context's local config file
    fn local_config_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("local.toml")
    }
}
