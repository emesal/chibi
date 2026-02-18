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
//!     // Get the config for the default context
//!     let config = chibi.resolve_config("default", None)?;
//!
//!     // Create options and a sink to collect the response
//!     let options = PromptOptions::new(false, false, &[], false);
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
use crate::state::AppState;
use crate::tools::{self, Tool};

use std::path::PathBuf;

/// Permission handler for gated operations (file writes, shell execution).
///
/// Receives hook data as JSON (containing tool_name, path/command, etc.).
/// Returns `Ok(true)` to allow the operation, `Ok(false)` to deny.
///
/// The frontend (e.g. CLI) registers a handler that prompts the user
/// interactively. When no handler is set, operations fail-safe to deny.
pub type PermissionHandler = Box<dyn Fn(&serde_json::Value) -> io::Result<bool>>;

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
/// assert!(default_opts.project_root.is_none());
///
/// // Custom options with verbose output and custom home
/// let custom_opts = LoadOptions {
///     verbose: true,
///     home: Some(PathBuf::from("/custom/chibi")),
///     project_root: Some(PathBuf::from("/my/project")),
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Print diagnostic info about tool loading to stderr.
    pub verbose: bool,
    /// Override the chibi home directory.
    /// If `None`, uses `CHIBI_HOME` env var or `~/.chibi`.
    pub home: Option<PathBuf>,
    /// Override the project root directory.
    /// If `None`, uses `CHIBI_PROJECT_ROOT` env var or current working directory.
    pub project_root: Option<PathBuf>,
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
    /// Project root directory (always resolved, never None).
    pub project_root: PathBuf,
    /// Optional permission handler for gated operations.
    /// If `None`, gated operations fail-safe to deny (unless a plugin approves).
    permission_handler: Option<PermissionHandler>,
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
    /// Project root resolution order: `options.project_root` > `CHIBI_PROJECT_ROOT` env > cwd.
    ///
    /// Sets `CHIBI_PROJECT_ROOT` and `CHIBI_INDEX_DB` environment variables so
    /// plugins and hooks can discover the project root and index database path.
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
    ///     ..Default::default()
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn load_with_options(options: LoadOptions) -> io::Result<Self> {
        let app = AppState::load(options.home)?;
        // CLI flag overrides config setting
        let verbose = options.verbose || app.config.verbose;
        let mut tools = tools::load_tools(&app.plugins_dir, verbose)?;

        // Load MCP bridge tools (non-fatal: bridge may not be configured)
        match tools::mcp::load_mcp_tools(&app.chibi_dir) {
            Ok(mcp_tools) => {
                if verbose && !mcp_tools.is_empty() {
                    eprintln!("[MCP: {} tools loaded]", mcp_tools.len());
                }
                tools.extend(mcp_tools);
            }
            Err(e) => {
                if verbose {
                    eprintln!("[MCP: bridge unavailable: {e}]");
                }
            }
        }

        let project_root = resolve_project_root(options.project_root)?;

        // Expose project root to plugins/hooks via environment variables.
        // SAFETY: Called once during single-threaded initialization, before any
        // plugin/hook child processes are spawned.
        unsafe {
            std::env::set_var("CHIBI_PROJECT_ROOT", &project_root);
            std::env::set_var("CHIBI_INDEX_DB", project_index_db_path(&project_root));
        }

        Ok(Self {
            app,
            tools,
            project_root,
            permission_handler: None,
        })
    }

    /// Set the permission handler for gated operations.
    ///
    /// The handler is called when a gated tool (write_file, file_edit, shell_exec)
    /// is invoked and no plugin has denied the operation. If no handler is set,
    /// operations fail-safe to deny.
    pub fn set_permission_handler(&mut self, handler: PermissionHandler) {
        self.permission_handler = Some(handler);
    }

    /// Initialize the session.
    ///
    /// Executes `OnStart` hooks. Call this once at the start of a session,
    /// before any prompts are sent.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use chibi_core::Chibi;
    ///
    /// # fn example() -> std::io::Result<()> {
    /// let chibi = Chibi::load()?;
    /// chibi.init()?;
    /// // ... use chibi ...
    /// chibi.shutdown()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn init(&self) -> io::Result<Vec<(String, serde_json::Value)>> {
        let hook_data = serde_json::json!({
            "chibi_home": self.app.chibi_dir.to_string_lossy(),
            "project_root": self.project_root.to_string_lossy(),
            "tool_count": self.tools.len(),
        });
        tools::execute_hook(&self.tools, tools::HookPoint::OnStart, &hook_data)
    }

    /// Shutdown the session.
    ///
    /// Executes `OnEnd` hooks. Call this once at the end of a session,
    /// after all prompts are complete.
    pub fn shutdown(&self) -> io::Result<Vec<(String, serde_json::Value)>> {
        let hook_data = serde_json::json!({
            "chibi_home": self.app.chibi_dir.to_string_lossy(),
            "project_root": self.project_root.to_string_lossy(),
            "tool_count": self.tools.len(),
        });
        tools::execute_hook(&self.tools, tools::HookPoint::OnEnd, &hook_data)
    }

    /// Clear a context, executing PreClear/PostClear hooks.
    ///
    /// This wraps `AppState::clear_context` with hook execution.
    pub fn clear_context(&self, context_name: &str) -> io::Result<()> {
        // Get context info for hook data before clearing
        let context = self.app.get_or_create_context(context_name)?;

        let pre_hook_data = serde_json::json!({
            "context_name": context_name,
            "message_count": context.messages.len(),
            "summary": context.summary,
        });
        let _ = tools::execute_hook(&self.tools, tools::HookPoint::PreClear, &pre_hook_data);

        self.app.clear_context(context_name)?;

        let post_hook_data = serde_json::json!({
            "context_name": context_name,
        });
        let _ = tools::execute_hook(&self.tools, tools::HookPoint::PostClear, &post_hook_data);

        Ok(())
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
    /// # let config = chibi.resolve_config("default", None)?;
    /// # let options = PromptOptions::new(false, false, &[], false);
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
            self.permission_handler.as_ref(),
            self.home_dir(),
            &self.project_root,
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
    pub async fn execute_tool(
        &self,
        context_name: &str,
        name: &str,
        args: serde_json::Value,
    ) -> io::Result<String> {
        // Try built-in tools first
        if let Some(result) =
            tools::execute_builtin_tool(&self.app, context_name, name, &args, None)
        {
            return result;
        }

        // Try file tools
        if tools::is_file_tool(name) {
            let mut config = self.app.resolve_config(context_name, None)?;
            tools::ensure_project_root_allowed(&mut config, &self.project_root);
            if let Some(result) = tools::execute_file_tool(
                &self.app,
                context_name,
                name,
                &args,
                &config,
                &self.project_root,
            ) {
                return result;
            }
        }

        // Try coding tools
        if tools::is_coding_tool(name) {
            let project_root = std::env::var("CHIBI_PROJECT_ROOT")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
            if let Some(result) = tools::execute_coding_tool(
                name,
                &args,
                &project_root,
                &self.tools,
                &self.app.vfs,
                context_name,
            )
            .await
            {
                return result;
            }
        }

        // Try agent tools
        if tools::is_agent_tool(name) {
            let config = self.app.resolve_config(context_name, None)?;
            return tools::execute_agent_tool(&config, name, &args, &self.tools).await;
        }

        // Try MCP tools (virtual path mcp://server/tool)
        if let Some(tool) = tools::find_tool(&self.tools, name) {
            if tools::mcp::is_mcp_tool(tool) {
                return tools::mcp::execute_mcp_tool(tool, &args, &self.app.chibi_dir);
            }
            // Regular plugin
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

    /// Resolve configuration for the current context.
    ///
    /// Combines global config, context-local config, and optional runtime override.
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context to resolve config for
    /// * `username_override` - Optional username override for this invocation
    pub fn resolve_config(
        &self,
        context_name: &str,
        username_override: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        self.app.resolve_config(context_name, username_override)
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
}

/// Resolve project root: explicit path > `CHIBI_PROJECT_ROOT` env > VCS root > cwd.
fn resolve_project_root(explicit: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root);
    }
    if let Ok(env_root) = std::env::var("CHIBI_PROJECT_ROOT")
        && !env_root.is_empty()
    {
        return Ok(PathBuf::from(env_root));
    }
    let cwd = std::env::current_dir()?;
    Ok(crate::vcs::detect_project_root(&cwd).unwrap_or(cwd))
}

/// Return the project-local chibi directory (`<project_root>/.chibi/`), creating it if absent.
pub fn project_chibi_dir(root: &Path) -> io::Result<PathBuf> {
    let dir = root.join(".chibi");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// Return the path to the project's codebase index database.
pub fn project_index_db_path(root: &Path) -> PathBuf {
    root.join(".chibi").join("codebase.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatePaths;
    use crate::config::{ApiParams, ToolsConfig, VfsConfig};
    use crate::partition::StorageConfig;
    use tempfile::TempDir;

    // Note: Most tests require a real chibi directory structure.
    // These are basic sanity tests.

    /// create a test chibi instance with a temporary directory.
    /// returns both for lifetime management (tempdir must outlive chibi).
    fn create_test_chibi() -> (Chibi, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::config::Config {
            api_key: Some("test-key".to_string()),
            model: Some("test-model".to_string()),
            context_window_limit: Some(8000),
            warn_threshold_percent: 75.0,
            verbose: false,
            hide_tool_calls: false,
            no_tool_calls: false,
            show_thinking: false,
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
            auto_cleanup_cache: true,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
            fallback_tool: "call_user".to_string(),
            tools: ToolsConfig::default(),
            vfs: VfsConfig::default(),
            url_policy: None,
        };
        let app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();
        let chibi = Chibi {
            project_root: temp_dir.path().to_path_buf(),
            app,
            tools: vec![],
            permission_handler: None,
        };
        (chibi, temp_dir)
    }

    #[test]
    fn test_chibi_facade_exists() {
        // basic compile test - if this compiles, the facade is properly defined
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
        assert!(opts.project_root.is_none());
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

    #[test]
    fn test_load_options_with_project_root() {
        let opts = LoadOptions {
            project_root: Some(PathBuf::from("/my/project")),
            ..Default::default()
        };
        assert_eq!(opts.project_root, Some(PathBuf::from("/my/project")));
    }

    #[test]
    fn test_resolve_project_root_explicit() {
        let root = resolve_project_root(Some(PathBuf::from("/explicit/root"))).unwrap();
        assert_eq!(root, PathBuf::from("/explicit/root"));
    }

    #[test]
    fn test_resolve_project_root_from_env() {
        // SAFETY: Test runs in single-threaded context
        unsafe {
            std::env::set_var("CHIBI_PROJECT_ROOT", "/env/root");
        }
        let root = resolve_project_root(None).unwrap();
        unsafe {
            std::env::remove_var("CHIBI_PROJECT_ROOT");
        }
        assert_eq!(root, PathBuf::from("/env/root"));
    }

    #[test]
    fn test_resolve_project_root_falls_back_to_vcs_or_cwd() {
        // SAFETY: Test runs in single-threaded context
        unsafe {
            std::env::remove_var("CHIBI_PROJECT_ROOT");
        }
        let root = resolve_project_root(None).unwrap();
        let cwd = std::env::current_dir().unwrap();
        // Should find VCS root (if running inside a repo) or fall back to cwd
        let expected = crate::vcs::detect_project_root(&cwd).unwrap_or(cwd);
        assert_eq!(root, expected);
    }

    #[test]
    fn test_project_chibi_dir_creates_directory() {
        let tmp = std::env::temp_dir().join("chibi-test-project-dir");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let dir = project_chibi_dir(&tmp).unwrap();
        assert_eq!(dir, tmp.join(".chibi"));
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_project_index_db_path() {
        let root = PathBuf::from("/my/project");
        assert_eq!(
            project_index_db_path(&root),
            PathBuf::from("/my/project/.chibi/codebase.db")
        );
    }

    // === Chibi struct method tests ===

    #[test]
    fn test_init_no_hooks() {
        let (chibi, _tmp) = create_test_chibi();
        let results = chibi.init().unwrap();
        // no plugins loaded, so no hook results
        assert!(results.is_empty());
    }

    #[test]
    fn test_shutdown_no_hooks() {
        let (chibi, _tmp) = create_test_chibi();
        let results = chibi.shutdown().unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_init_hook_data() {
        // we can't easily test hook *execution* without a real hook plugin,
        // but we verify init() succeeds and returns the right shape.
        // the hook_data json built internally contains chibi_home, project_root,
        // and tool_count — verified indirectly by init() not erroring.
        let (chibi, _tmp) = create_test_chibi();
        let results = chibi.init().unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_clear_context_nonexistent() {
        // clear_context calls get_or_create_context first (creates if missing),
        // then clears it — should succeed as a no-op on fresh context
        let (chibi, _tmp) = create_test_chibi();
        chibi.clear_context("nonexistent").unwrap();
    }

    #[test]
    fn test_clear_context_with_messages() {
        let (chibi, _tmp) = create_test_chibi();
        let ctx_name = "test-ctx";

        // add a message to the context
        let mut context = chibi.app.get_or_create_context(ctx_name).unwrap();
        chibi
            .app
            .add_message(&mut context, "user".to_string(), "hello".to_string());
        assert_eq!(context.messages.len(), 1);
        chibi.app.save_context(&context).unwrap();

        // clear should succeed
        chibi.clear_context(ctx_name).unwrap();

        // verify the context is now empty
        let cleared = chibi.app.get_or_create_context(ctx_name).unwrap();
        assert!(cleared.messages.is_empty());
    }

    #[test]
    fn test_list_contexts_empty() {
        let (chibi, _tmp) = create_test_chibi();
        assert!(chibi.list_contexts().is_empty());
    }

    #[test]
    fn test_list_contexts_after_create() {
        let (mut chibi, _tmp) = create_test_chibi();

        // save contexts to disk then sync in-memory state
        for name in &["alpha", "beta"] {
            let ctx = chibi.app.get_or_create_context(name).unwrap();
            chibi.app.save_context(&ctx).unwrap();
        }
        chibi.app.sync_state_with_filesystem().unwrap();

        let names = chibi.list_contexts();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn test_resolve_config_defaults() {
        let (chibi, _tmp) = create_test_chibi();
        let config = chibi.resolve_config("default", None).unwrap();
        // should reflect the values from create_test_chibi's Config
        assert_eq!(config.model, "test-model");
        assert_eq!(config.username, "testuser");
        assert_eq!(config.fuel, 15);
        assert_eq!(config.context_window_limit, 8000);
        assert!(!config.verbose);
    }

    #[test]
    fn test_resolve_config_with_local_override() {
        let (chibi, _tmp) = create_test_chibi();
        let ctx_name = "override-ctx";

        // ensure the context directory exists
        let ctx = chibi.app.get_or_create_context(ctx_name).unwrap();
        chibi.app.save_context(&ctx).unwrap();

        // write a local.toml that overrides the model
        let local_toml = chibi.app.context_dir(ctx_name).join("local.toml");
        std::fs::write(&local_toml, "model = \"local-model\"\nfuel = 99\n").unwrap();

        let config = chibi.resolve_config(ctx_name, None).unwrap();
        assert_eq!(config.model, "local-model");
        assert_eq!(config.fuel, 99);
        // non-overridden values should still come from global config
        assert_eq!(config.username, "testuser");
    }

    #[test]
    fn test_save_persists_state() {
        let (chibi, tmp) = create_test_chibi();
        chibi.save().unwrap();
        let state_path = tmp.path().join("state.json");
        assert!(state_path.exists());
    }

    #[test]
    fn test_home_dir() {
        let (chibi, tmp) = create_test_chibi();
        assert_eq!(chibi.home_dir(), tmp.path());
    }

    #[test]
    fn test_tool_count_empty() {
        let (chibi, _tmp) = create_test_chibi();
        assert_eq!(chibi.tool_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_tool_not_found() {
        let (chibi, _tmp) = create_test_chibi();
        let result = chibi
            .execute_tool("default", "nonexistent_tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("nonexistent_tool"));
    }

    #[tokio::test]
    async fn test_execute_tool_builtin() {
        let (chibi, _tmp) = create_test_chibi();
        let result = chibi
            .execute_tool(
                "default",
                "update_todos",
                serde_json::json!({"content": "- [ ] write tests"}),
            )
            .await;
        let output = result.unwrap();
        assert!(output.contains("Todos updated"));
    }
}
