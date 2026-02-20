//! chibi-core: Embeddable core library for chibi
//!
//! Provides context management, transcript storage, plugins, hooks, and API client.
//!
//! # Quick Start
//!
//! For most embedding use cases, use the [`Chibi`] facade:
//!
//! ```no_run
//! // Requires ~/.chibi directory with config.toml and models.toml.
//! // See chibi documentation for setup instructions.
//! use chibi_core::{Chibi, CollectingSink};
//! use chibi_core::api::PromptOptions;
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     let chibi = Chibi::load()?;
//!     let config = chibi.resolve_config("default", None)?;
//!     let options = PromptOptions::new(false, &[], false);
//!     let mut sink = CollectingSink::new();
//!
//!     chibi.send_prompt_streaming("default", "Hello!", &config, &options, &mut sink).await?;
//!     println!("Response: {}", sink.text);
//!     Ok(())
//! }
//! ```
//!
//! For lower-level access, use the individual modules directly.

pub mod agents_md;
pub mod api;
mod chibi;
pub mod config;
pub mod context;
pub mod execution;
pub mod gateway;
mod inbox;
pub mod index;
pub mod input;
pub mod json_ext;
pub mod jsonl;
pub mod lock;
pub mod model_info;
pub mod output;
pub mod partition;
pub mod safe_io;
pub mod state;
pub mod tools;
pub mod vcs;
pub mod vfs;
pub mod vfs_cache;

/// System prompt used when processing inbox messages via -b/-B flags.
pub const INBOX_CHECK_PROMPT: &str = "[System: You have received new message(s) above. Review and take appropriate action now — you may not be reactivated soon, so handle anything urgent immediately.]";

// Re-export the facade
pub use chibi::{Chibi, LoadOptions, PermissionHandler, project_chibi_dir, project_index_db_path};

// Re-export commonly used types
pub use api::{CollectingSink, PromptOptions, ResponseEvent, ResponseSink};
pub use config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig, ToolsConfig};
pub use context::{Context, ContextEntry, TranscriptEntry};
pub use execution::{CommandEffect, execute_command};
pub use input::{Command, ExecutionFlags, Inspectable};
pub use output::{CommandEvent, OutputSink};
pub use partition::StorageConfig;
pub use state::{AppState, StatePaths};
pub use tools::{HookPoint, SpawnOptions, Tool, spawn_agent};

/// Returns ratatoskr's package version.
pub fn ratatoskr_version() -> &'static str {
    ratatoskr::PKG_VERSION
}

/// Shared test helpers for integration-style tests across chibi-core modules.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::Chibi;
    use crate::config::{ApiParams, Config, ToolsConfig, VfsConfig};
    use crate::partition::StorageConfig;
    use crate::state::AppState;
    use tempfile::TempDir;

    /// Build a minimal `Chibi` instance backed by a temporary directory.
    ///
    /// Returns `(Chibi, TempDir)` — the `TempDir` must outlive `Chibi`.
    pub(crate) fn create_test_chibi() -> (Chibi, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: Some("test-key".to_string()),
            model: Some("test-model".to_string()),
            context_window_limit: Some(8000),
            warn_threshold_percent: 75.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            reflection_enabled: true,
            reflection_character_limit: 10000,
            fuel: 15,
            fuel_empty_response_cost: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
            fallback_tool: "call_user".to_string(),
            tools: ToolsConfig::default(),
            vfs: VfsConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
        };
        let app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();
        let root = temp_dir.path().to_path_buf();
        let chibi = Chibi::for_test(app, root);
        (chibi, temp_dir)
    }
}
