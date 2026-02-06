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
//!     let options = PromptOptions::new(false, false, false, &[], false);
//!     let mut sink = CollectingSink::new();
//!
//!     chibi.send_prompt_streaming("default", "Hello!", &config, &options, &mut sink).await?;
//!     println!("Response: {}", sink.text);
//!     Ok(())
//! }
//! ```
//!
//! For lower-level access, use the individual modules directly.

pub mod api;
pub mod cache;
mod chibi;
pub mod config;
pub mod context;
pub mod gateway;
mod inbox;
pub mod input;
pub mod json_ext;
pub mod jsonl;
pub mod lock;
pub mod model_info;
pub mod partition;
pub mod safe_io;
pub mod state;
pub mod tools;

/// System prompt used when processing inbox messages via -b/-B flags.
pub const INBOX_CHECK_PROMPT: &str = "[System: You have received new message(s) above. Review and take appropriate action now — you may not be reactivated soon, so handle anything urgent immediately.]";

// Re-export the facade
pub use chibi::{Chibi, LoadOptions};

// Re-export commonly used types
pub use api::{CollectingSink, PromptOptions, ResponseEvent, ResponseSink};
pub use config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig, ToolsConfig};
pub use context::{Context, ContextEntry, Message, TranscriptEntry};
pub use input::{Command, Flags, Inspectable};
pub use partition::StorageConfig;
pub use state::{AppState, StatePaths};
pub use tools::{HookPoint, Tool};

/// Returns ratatoskr's package version.
pub fn ratatoskr_version() -> &'static str {
    ratatoskr::PKG_VERSION
}
