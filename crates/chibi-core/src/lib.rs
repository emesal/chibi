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
//!     let options = PromptOptions::new(false, false, &[], false);
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
pub mod vfs_cache;
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
pub use output::OutputSink;
pub use partition::StorageConfig;
pub use state::{AppState, StatePaths};
pub use tools::{HookPoint, SpawnOptions, Tool, spawn_agent};

/// Returns ratatoskr's package version.
pub fn ratatoskr_version() -> &'static str {
    ratatoskr::PKG_VERSION
}
