//! API module for chibi-core.
//!
//! This module provides the core API functionality for interacting with LLMs,
//! decoupled from presentation concerns through the `ResponseSink` trait.

pub mod compact;
pub mod logging;
pub mod request;
pub mod send;
pub mod sink;

pub use compact::{
    compact_context_by_name, compact_context_with_llm, compact_context_with_llm_manual,
    rolling_compact,
};
pub use logging::{log_request_if_enabled, log_response_meta_if_enabled};
pub use request::{PromptOptions, build_request_body, extract_choice_content};
pub use send::send_prompt;
pub use sink::{CollectingSink, ResponseEvent, ResponseSink};
