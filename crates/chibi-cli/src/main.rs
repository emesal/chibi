// chibi-cli: CLI frontend for chibi
// Argument parsing, markdown rendering, TTY handling

mod cli;
mod config;
mod image_cache;
mod json_input;
mod markdown;
mod output;

// Re-export key types for use by other modules
pub use cli::{parse, Cli, InspectableExt, PluginInvocation};
pub use config::{
    default_markdown_style, ConfigImageRenderMode, ImageAlignment, ImageConfig, ImageConfigOverride,
    MarkdownStyle, ResolvedConfig,
};
pub use json_input::from_str as parse_json_input;
pub use markdown::{MarkdownConfig, MarkdownStream};
pub use output::OutputHandler;

fn main() {
    println!("chibi-cli stub - Phase 6 complete, modules moved");
}
