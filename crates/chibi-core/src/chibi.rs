//! High-level facade for embedding chibi.
//!
//! The `Chibi` struct provides a clean, high-level API for embedding chibi
//! in other applications. It wraps `AppState` and tool loading, providing
//! a simpler interface for common operations.
//!
//! # Example
//!
//! ```no_run
//! // Requires ~/.chibi directory with config.toml and models.toml.
//! use chibi_core::{Chibi, CollectingSink, ResolvedConfig};
//! use chibi_core::api::PromptOptions;
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     let mut chibi = Chibi::load()?;
//!
//!     // Get the config for the current context
//!     let config = chibi.resolve_config(None, None)?;
//!
//!     // Create options and a sink to collect the response
//!     let options = PromptOptions::new(false, false, false, &[], false);
//!     let mut sink = CollectingSink::new();
//!
//!     // Send a prompt to the default context
//!     chibi.send_prompt_streaming("default", "Hello!", &config, &options, &mut sink).await?;
//!
//!     println!("Response: {}", sink.text);
//!     Ok(())
//! }
//! ```

use std::io;
use std::path::Path;

use crate::api::sink::ResponseSink;
use crate::api::{PromptOptions, send_prompt};
use crate::config::ResolvedConfig;
use crate::context::{Context, ContextEntry};
use crate::state::AppState;
use crate::tools::{self, Tool};

use std::path::PathBuf;

/// Options for loading a Chibi instance.
///
/// Use `Default::default()` for standard behavior, or customize as needed.
///
/// # Example
///
/// ```
/// use chibi_core::LoadOptions;
/// use std::path::PathBuf;
///
/// // Default options use ~/.chibi or CHIBI_HOME
/// let default_opts = LoadOptions::default();
/// assert!(!default_opts.verbose);
/// assert!(default_opts.home.is_none());
///
/// // Custom options with verbose output and custom home
/// let custom_opts = LoadOptions {
///     verbose: true,
///     home: Some(PathBuf::from("/custom/chibi")),
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Print diagnostic info about tool loading to stderr.
    pub verbose: bool,
    /// Override the chibi home directory.
    /// If `None`, uses `CHIBI_HOME` env var or `~/.chibi`.
    pub home: Option<PathBuf>,
}

/// High-level facade for chibi embedding.
///
/// Provides a clean API for common chibi operations without exposing
/// internal implementation details. Wraps `AppState` and manages tool loading.
pub struct Chibi {
    /// The underlying application state.
    pub app: AppState,
    /// Loaded tools (plugins from ~/.chibi/plugins/).
    pub tools: Vec<Tool>,
}

impl Chibi {
    /// Load chibi from default home directory.
    ///
    /// Convenience method equivalent to `load_with_options(LoadOptions::default())`.
    /// Kept as a shortcut for library users who don't need custom options.
    ///
    /// Uses the following precedence for the chibi directory:
    /// 1. `CHIBI_HOME` environment variable
    /// 2. `~/.chibi` default
    pub fn load() -> io::Result<Self> {
        Self::load_with_options(LoadOptions::default())
    }

    /// Load chibi from a specific home directory.
    ///
    /// Convenience method for `load_with_options(LoadOptions { home: Some(...), ..Default::default() })`.
    /// Kept as a shortcut for library users who only need to override the home directory.
    ///
    /// This overrides both `CHIBI_HOME` and the default `~/.chibi`.
    pub fn from_home(home: &Path) -> io::Result<Self> {
        Self::load_with_options(LoadOptions {
            home: Some(home.to_path_buf()),
            ..Default::default()
        })
    }

    /// Load chibi with custom options.
    ///
    /// This is the most flexible way to load chibi, allowing control over
    /// both the home directory and verbose output.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // Requires a chibi home directory with config.toml and models.toml.
    /// use chibi_core::{Chibi, LoadOptions};
    ///
    /// let chibi = Chibi::load_with_options(LoadOptions {
    ///     verbose: true,
    ///     home: Some("/custom/path".into()),
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn load_with_options(options: LoadOptions) -> io::Result<Self> {
        let app = AppState::load(options.home)?;
        let tools = tools::load_tools(&app.plugins_dir, options.verbose)?;
        Ok(Self { app, tools })
    }

    /// Send a prompt with streaming output via a ResponseSink.
    ///
    /// This is the primary method for sending prompts to the LLM. The sink
    /// receives streaming events as they occur, allowing for real-time output
    /// or collection for later processing.
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context to use for this prompt
    /// * `prompt` - The user's prompt text
    /// * `config` - Resolved configuration for this request
    /// * `options` - Options controlling prompt execution behavior
    /// * `sink` - A sink to receive response events
    ///
    /// # Example
    ///
    /// ```no_run
    /// // Requires ~/.chibi directory and valid API key in config.
    /// # use chibi_core::{Chibi, CollectingSink};
    /// # use chibi_core::api::PromptOptions;
    /// # async fn example() -> std::io::Result<()> {
    /// # let chibi = Chibi::load()?;
    /// # let config = chibi.resolve_config(None, None)?;
    /// # let options = PromptOptions::new(false, false, false, &[], false);
    /// let mut sink = CollectingSink::new();
    /// chibi.send_prompt_streaming("default", "Hello", &config, &options, &mut sink).await?;
    /// println!("Got response: {}", sink.text);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_prompt_streaming<S: ResponseSink>(
        &self,
        context_name: &str,
        prompt: &str,
        config: &ResolvedConfig,
        options: &PromptOptions<'_>,
        sink: &mut S,
    ) -> io::Result<()> {
        send_prompt(
            &self.app,
            context_name,
            prompt.to_string(),
            &self.tools,
            config,
            options,
            sink,
        )
        .await
    }

    /// Execute a tool by name with the given arguments.
    ///
    /// Tries built-in tools first, then falls back to loaded plugins.
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context to use for file tools
    /// * `name` - The tool name
    /// * `args` - JSON arguments for the tool
    ///
    /// # Returns
    ///
    /// The tool's output as a string, or an error if the tool wasn't found
    /// or execution failed.
    pub fn execute_tool(
        &self,
        context_name: &str,
        name: &str,
        args: serde_json::Value,
    ) -> io::Result<String> {
        // Try built-in tools first
        if let Some(result) = tools::execute_builtin_tool(&self.app, context_name, name, &args) {
            return result;
        }

        // Try file tools
        if tools::is_file_tool(name) {
            let config = self.app.resolve_config(context_name, None, None)?;
            if let Some(result) =
                tools::execute_file_tool(&self.app, context_name, name, &args, &config)
            {
                return result;
            }
        }

        // Try plugins
        if let Some(tool) = tools::find_tool(&self.tools, name) {
            return tools::execute_tool(tool, &args, false);
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Tool '{}' not found", name),
        ))
    }

    // NOTE: The following methods were removed in the stateless-core refactor:
    // - switch_context() - now handled by CLI Session
    // - swap_with_previous() - now handled by CLI Session
    // - current_context() - use get_or_create_context(name) on app
    // - current_context_name() - CLI owns session state now
    //
    // See CLI Session for context navigation, and use parameterized methods on app.

    /// List all available context names.
    pub fn list_contexts(&self) -> Vec<String> {
        self.app.list_contexts()
    }

    /// List all available contexts with metadata.
    pub fn list_context_entries(&self) -> &[ContextEntry] {
        &self.app.state.contexts
    }

    /// Resolve configuration for the current context.
    ///
    /// Combines global config, context-local config, and optional runtime overrides.
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context to resolve config for
    /// * `persistent_username` - Username override to persist in local config
    /// * `transient_username` - Username override for this session only
    pub fn resolve_config(
        &self,
        context_name: &str,
        persistent_username: Option<&str>,
        transient_username: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        self.app
            .resolve_config(context_name, persistent_username, transient_username)
    }

    /// Save state (current context, context list) to disk.
    pub fn save(&self) -> io::Result<()> {
        self.app.save()
    }

    /// Get the chibi home directory path.
    pub fn home_dir(&self) -> &Path {
        &self.app.chibi_dir
    }

    /// Get the number of loaded tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Execute a hook on all tools that registered for it.
    ///
    /// This is the facade method for hook execution, allowing embedders to
    /// trigger hooks without accessing `self.tools` directly.
    ///
    /// # Arguments
    ///
    /// * `hook` - The hook point to execute
    /// * `data` - JSON data to pass to the hook
    /// * `verbose` - Whether to print diagnostic info to stderr
    ///
    /// # Returns
    ///
    /// A vector of (tool_name, result) for tools that returned non-empty output.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // Requires ~/.chibi directory with plugins that register hooks.
    /// use serde_json::json;
    /// use chibi_core::{Chibi, HookPoint};
    ///
    /// # fn example() -> std::io::Result<()> {
    /// let chibi = Chibi::load()?;
    /// let hook_data = json!({
    ///     "context": chibi.current_context_name(),
    /// });
    /// let results = chibi.execute_hook(HookPoint::OnStart, &hook_data, false)?;
    /// for (tool_name, result) in results {
    ///     println!("{}: {:?}", tool_name, result);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn execute_hook(
        &self,
        hook: tools::HookPoint,
        data: &serde_json::Value,
        verbose: bool,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        tools::execute_hook(&self.tools, hook, data, verbose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Most tests require a real chibi directory structure.
    // These are basic sanity tests.

    #[test]
    fn test_chibi_facade_exists() {
        // Basic compile test - if this compiles, the facade is properly defined
        fn _takes_chibi(_c: Chibi) {}
    }

    #[test]
    fn test_chibi_is_send() {
        // Chibi should not be Send due to RefCell in AppState
        // This is expected - it's a single-threaded facade
        fn _assert_send<T: Send>() {}
        // Intentionally not calling _assert_send::<Chibi>() - it won't compile
    }

    #[test]
    fn test_load_options_default() {
        let opts = LoadOptions::default();
        assert!(!opts.verbose);
        assert!(opts.home.is_none());
    }

    #[test]
    fn test_load_options_with_verbose() {
        let opts = LoadOptions {
            verbose: true,
            ..Default::default()
        };
        assert!(opts.verbose);
    }

    #[test]
    fn test_load_options_with_home() {
        let opts = LoadOptions {
            home: Some(PathBuf::from("/tmp/test-chibi")),
            ..Default::default()
        };
        assert_eq!(opts.home, Some(PathBuf::from("/tmp/test-chibi")));
    }
}
