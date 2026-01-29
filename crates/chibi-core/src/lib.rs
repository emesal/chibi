//! chibi-core: Embeddable core library for chibi
//!
//! Provides context management, transcript storage, plugins, hooks, and API client.

pub mod api;
pub mod cache;
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

// Re-export commonly used types
pub use api::{CollectingSink, ResponseEvent, ResponseSink};
pub use config::{ApiParams, Config, LocalConfig, ModelsConfig, ResolvedConfig, ToolsConfig};
pub use context::{Context, Message, TranscriptEntry};
pub use input::{ChibiInput, Command, Flags, Inspectable};
pub use partition::StorageConfig;
