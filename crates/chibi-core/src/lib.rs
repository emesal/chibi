//! chibi-core: Embeddable core library for chibi
//!
//! Provides context management, transcript storage, plugins, hooks, and API client.
//!
//! # Quick Start
//!
//! For most embedding use cases, use the [`Chibi`] facade:
//!
//! ```ignore
//! use chibi_core::{Chibi, CollectingSink};
//! use chibi_core::api::PromptOptions;
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     let chibi = Chibi::load()?;
//!     let config = chibi.resolve_config(None, None)?;
//!     let options = PromptOptions::new(false, false, false, &[], false);
//!     let mut sink = CollectingSink::new();
//!
//!     chibi.send_prompt_streaming("Hello!", &config, &options, &mut sink).await?;
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
mod inbox;
pub mod input;
pub mod jsonl;
pub mod llm;
pub mod lock;
pub mod partition;
pub mod safe_io;
pub mod state;
pub mod tools;

// Re-export the facade
pub use chibi::{Chibi, LoadOptions};

// Re-export commonly used types
pub use api::{CollectingSink, PromptOptions, ResponseEvent, ResponseSink};
pub use config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig, ToolsConfig};
pub use context::{Context, ContextEntry, Message, TranscriptEntry};
pub use input::{ChibiInput, Command, Flags, Inspectable};
pub use partition::StorageConfig;
pub use state::AppState;
pub use tools::Tool;
