//! CLI argument parsing with clap.
//!
//! This module handles parsing command-line arguments and converting them
//! to the unified `ChibiInput` format.

use chibi_core::input::{Command, DebugKey, ExecutionFlags, Inspectable};

use crate::input::{ChibiInput, ContextSelection, UsernameOverride};
use clap::Parser;
use std::io::{self, BufRead, ErrorKind, IsTerminal};

use crate::config::ResolvedConfig;

/// Direct plugin invocation from CLI
#[derive(Debug, Clone)]
pub struct PluginInvocation {
    pub name: String,
    pub args: Vec<String>,
}

/// Extension methods for Inspectable that depend on CLI config.
pub trait InspectableExt {
    fn from_str_cli(s: &str) -> Option<Inspectable>;
    fn all_names_cli() -> Vec<&'static str>;
}

impl InspectableExt for Inspectable {
    fn from_str_cli(s: &str) -> Option<Inspectable> {
        match s {
            // File-based items
            "system_prompt" | "prompt" => Some(Inspectable::SystemPrompt),
            "reflection" => Some(Inspectable::Reflection),
            "todos" => Some(Inspectable::Todos),
            "goals" => Some(Inspectable::Goals),
            // Global items
            "home" => Some(Inspectable::Home),
            "list" => Some(Inspectable::List),
            // Check if it's a valid config field path
            other => {
                if ResolvedConfig::list_fields().contains(&other) {
                    Some(Inspectable::ConfigField(other.to_string()))
                } else {
                    None
                }
            }
        }
    }

    fn all_names_cli() -> Vec<&'static str> {
        let mut names = vec![
            "system_prompt",
            "reflection",
            "todos",
            "goals",
            "home",
            "list",
        ];
        names.extend(ResolvedConfig::list_fields());
        names
    }
}

/// chibi - A CLI tool for chatting with AI via OpenRouter
#[derive(Parser, Debug)]
#[command(
    name = "chibi",
    version,
    about = "A CLI tool for chatting with AI via OpenRouter",
    after_help = CLI_AFTER_HELP,
    disable_help_flag = true,
    disable_version_flag = true
)]
pub struct Cli {
    // === Context operations ===
    /// Switch to context (persistent). Use 'new' for auto-generated name, 'new:prefix' for prefixed
    #[arg(
        short = 'c',
        long = "switch-context",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    pub switch_context: Option<String>,

    /// Use context for this invocation only (ephemeral, does not update session)
    #[arg(
        short = 'C',
        long = "ephemeral-context",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    pub ephemeral_context: Option<String>,

    /// Show current context info
    #[arg(short = 'l', long = "list-current-context")]
    pub list_current_context: bool,

    /// List all contexts
    #[arg(short = 'L', long = "list-contexts")]
    pub list_contexts: bool,

    /// Destroy current context (permanently removes all data)
    #[arg(short = 'd', long = "destroy-current-context")]
    pub destroy_current_context: bool,

    /// Destroy specified context (permanently removes all data)
    #[arg(
        short = 'D',
        long = "destroy-context",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub destroy_context: Option<String>,

    /// Archive current context history (save to transcript)
    #[arg(short = 'a', long = "archive-current-history")]
    pub archive_current_history: bool,

    /// Archive specified context's history
    #[arg(
        short = 'A',
        long = "archive-history",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub archive_history: Option<String>,

    /// Clear the tool output cache for current context
    #[arg(long = "clear-cache")]
    pub clear_cache: bool,

    /// Clear the tool output cache for specified context
    #[arg(
        long = "clear-cache-for",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub clear_cache_for: Option<String>,

    /// Cleanup old cache entries across all contexts (removes entries older than configured max_age)
    #[arg(long = "cleanup-cache")]
    pub cleanup_cache: bool,

    /// Check all context inboxes and process any messages
    #[arg(short = 'b', long = "check-all-inboxes")]
    pub check_all_inboxes: bool,

    /// Check inbox for specified context and process any messages
    #[arg(
        short = 'B',
        long = "check-inbox-for",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub check_inbox_for: Option<String>,

    /// Compact current context (summarize and clear)
    #[arg(short = 'z', long = "compact-current-context")]
    pub compact_current_context: bool,

    /// Compact specified context
    #[arg(
        short = 'Z',
        long = "compact-context",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub compact_context: Option<String>,

    /// Rename current context
    #[arg(
        short = 'r',
        long = "rename-current-context",
        value_name = "NEW",
        allow_hyphen_values = true
    )]
    pub rename_current_context: Option<String>,

    /// Rename specified context (requires OLD and NEW args)
    #[arg(short = 'R', long = "rename-context", value_names = ["OLD", "NEW"], num_args = 2, allow_hyphen_values = true)]
    pub rename_context: Option<Vec<String>>,

    /// Show last N log entries (current context). Use negative for first N
    #[arg(
        short = 'g',
        long = "show-current-log",
        value_name = "N",
        allow_hyphen_values = true
    )]
    pub show_current_log: Option<isize>,

    /// Show last N log entries (requires CTX and N)
    #[arg(short = 'G', long = "show-log", value_names = ["CTX", "N"], num_args = 2, allow_hyphen_values = true)]
    pub show_log: Option<Vec<String>>,

    /// Inspect current context (system_prompt, reflection, todos, goals, list)
    #[arg(short = 'n', long = "inspect-current", value_name = "THING")]
    pub inspect_current: Option<String>,

    /// Inspect specified context (requires CTX and THING)
    #[arg(short = 'N', long = "inspect", value_names = ["CTX", "THING"], num_args = 2)]
    pub inspect: Option<Vec<String>>,

    /// Set system prompt for current context (file path or content)
    #[arg(
        short = 'y',
        long = "set-current-system-prompt",
        value_name = "PROMPT",
        allow_hyphen_values = true
    )]
    pub set_current_system_prompt: Option<String>,

    /// Set system prompt for specified context (requires CTX and PROMPT)
    #[arg(short = 'Y', long = "set-system-prompt", value_names = ["CTX", "PROMPT"], num_args = 2, allow_hyphen_values = true)]
    pub set_system_prompt: Option<Vec<String>>,

    // === Username options ===
    /// Set username (persists to local.toml)
    #[arg(
        short = 'u',
        long = "set-username",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    pub set_username: Option<String>,

    /// Set username for this invocation only (ephemeral)
    #[arg(
        short = 'U',
        long = "ephemeral-username",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    pub ephemeral_username: Option<String>,

    // === Plugin/tool options ===
    /// Run a plugin directly (-p NAME "ARGS")
    /// For args with spaces or special chars: -p myplugin "arg1 'quoted arg' arg2"
    /// For no args: -p myplugin ""
    #[arg(short = 'p', long = "plugin", value_names = ["NAME", "ARGS"], num_args = 2, allow_hyphen_values = true)]
    pub plugin: Option<Vec<String>>,

    /// Call a tool directly (-P TOOL JSON_ARGS)
    /// Example: -P send_message '{"to":"foo","content":"hi"}'
    /// For no args: -P mytool '{}'
    #[arg(short = 'P', long = "call-tool", value_names = ["TOOL", "JSON"], num_args = 2, allow_hyphen_values = true)]
    pub call_tool: Option<Vec<String>>,

    // === Model metadata ===
    /// Show model metadata in TOML format (settable fields only)
    #[arg(
        long = "model-metadata",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub model_metadata: Option<String>,

    /// Show full model metadata in TOML format (with pricing, capabilities, parameter ranges)
    #[arg(
        long = "model-metadata-full",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub model_metadata_full: Option<String>,

    // === Model setting ===
    /// Set model for current context (persists to local.toml)
    #[arg(
        short = 'm',
        long = "set-model",
        value_name = "MODEL",
        allow_hyphen_values = true
    )]
    pub set_model: Option<String>,

    /// Set model for specified context (requires CTX and MODEL)
    #[arg(
        short = 'M',
        long = "set-model-for-context",
        value_names = ["CTX", "MODEL"],
        num_args = 2,
        allow_hyphen_values = true
    )]
    pub set_model_for_context: Option<Vec<String>>,

    // === Control flags ===
    /// Show extra info (tools loaded, etc.)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Hide tool call display (verbose overrides this)
    #[arg(long = "hide-tool-calls")]
    pub hide_tool_calls: bool,

    /// Show thinking/reasoning content (verbose overrides this)
    #[arg(long = "show-thinking")]
    pub show_thinking: bool,

    /// Omit tools from API requests (pure text mode)
    #[arg(long = "no-tool-calls")]
    pub no_tool_calls: bool,

    /// Trust mode: auto-approve all permission checks (for automation/piping)
    #[arg(short = 't', long = "trust")]
    pub trust: bool,

    /// Force handoff to user (-x)
    #[arg(short = 'x', long = "force-call-user")]
    pub force_call_user: bool,

    /// Force handoff to agent (-X)
    #[arg(short = 'X', long = "force-call-agent")]
    pub force_call_agent: bool,

    /// Disable markdown rendering (raw output)
    #[arg(long = "raw")]
    pub raw: bool,

    /// Override a config value for this invocation (repeatable, KEY=VALUE)
    #[arg(short = 's', long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,

    // === Debug options ===
    /// Enable debug features (request-log, response-meta, all)
    #[arg(long = "debug", value_name = "KEY")]
    pub debug: Option<String>,

    /// Auto-destroy this context at a Unix timestamp
    #[arg(long = "destroy-at", value_name = "TIMESTAMP")]
    pub destroy_at: Option<u64>,

    /// Auto-destroy this context after N seconds of inactivity
    #[arg(long = "destroy-after-inactive", value_name = "SECS")]
    pub destroy_after_inactive: Option<u64>,

    // === Directory override ===
    /// Override chibi home directory (default: ~/.chibi, or CHIBI_HOME env var)
    #[arg(long = "home", value_name = "PATH")]
    pub home: Option<String>,

    /// Override project root directory (default: cwd, or CHIBI_PROJECT_ROOT env var)
    #[arg(long = "project-root", value_name = "PATH")]
    pub project_root: Option<String>,

    // === Help and version ===
    /// Show help
    #[arg(short = 'h', long = "help")]
    pub help: bool,

    /// Show version
    #[arg(long = "version")]
    pub version: bool,

    // === Positional: prompt ===
    /// The prompt to send (all remaining arguments)
    /// Note: Use -- before prompts that start with - (e.g., chibi -- -starts-with-dash)
    #[arg(trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

const CLI_AFTER_HELP: &str = r#"EXAMPLES:
  chibi What is Rust?             Send prompt to LLM
  chibi -c coding write code      Switch context, then send prompt
  chibi -L                        List all contexts
  chibi -l                        Show current context info
  chibi -Dold                     Destroy 'old' context (attached arg)
  chibi -D old                    Destroy 'old' context (separated arg)
  chibi -n system_prompt          Inspect current system prompt
  chibi -g 10                     Show last 10 log entries
  chibi -x -c test                Switch context without LLM
  chibi -X -L                     List contexts then invoke LLM
  chibi -a hello                  Archive history, then send prompt
  chibi -b                        Check all inboxes, process any messages
  chibi -B work                   Check inbox for 'work' context only
  chibi -p myplugin "arg1 arg2"   Run plugin with args (shell-style split)
  chibi -P mytool '{}'            Call tool with empty JSON args
  chibi -P send '{"to":"x"}'      Call tool with JSON args

FLAG BEHAVIOR:
  Some flags imply --no-chibi (operations that produce output or
  operate on other contexts). Use -X to override and invoke LLM after.

  Implied --no-chibi: -l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -M, -p, -P, --model-metadata, --model-metadata-full
  Combinable with prompt: -c, -C, -a, -z, -r, -m, -y, -u, -U, -v

PROMPT INPUT:
  Arguments after options are joined as the prompt.
  Use -- to force remaining args as prompt (e.g., chibi -- -starts-with-dash)
  No arguments: read from stdin (end with . on empty line)
  Piped input: echo 'text' | chibi"#;

/// Helper for current/specific context command dispatch.
/// Checks the bool (current context) and Option (specific context) flags,
/// returning Some(name) if either is set, where name is None for current context.
fn check_context_pair(current: bool, specific: &Option<String>) -> Option<Option<String>> {
    if current {
        Some(None)
    } else {
        specific.as_ref().map(|name| Some(name.clone()))
    }
}

/// Extract a string pair from an Option<Vec<String>>.
/// Returns Some((first, second)) if vec has at least 2 elements.
fn extract_string_pair(v: &Option<Vec<String>>) -> Option<(String, String)> {
    v.as_ref()
        .filter(|v| v.len() >= 2)
        .map(|v| (v[0].clone(), v[1].clone()))
}

/// Parse an Inspectable from a string, returning an io::Error on failure.
fn parse_inspectable(s: &str) -> io::Result<Inspectable> {
    Inspectable::from_str_cli(s).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Unknown inspectable: {}. Valid options: {:?}",
                s,
                Inspectable::all_names_cli()
            ),
        )
    })
}

impl Cli {
    /// Parse CLI arguments from environment
    pub fn parse_args() -> io::Result<Self> {
        // We need custom parsing to handle attached args like -Dname
        let args: Vec<String> = std::env::args().collect();
        Self::parse_from_args(&args)
    }

    /// Parse CLI arguments from a slice (testable version)
    pub fn parse_from_args(args: &[String]) -> io::Result<Self> {
        // Pre-process args to expand attached short forms like -Dname to -D name
        let expanded = expand_attached_args(args);

        match Self::try_parse_from(&expanded) {
            Ok(cli) => Ok(cli),
            Err(e) => {
                // Convert clap error to io::Error
                let msg = e.to_string();
                if msg.contains("unexpected argument") || msg.contains("invalid value") {
                    Err(io::Error::new(ErrorKind::InvalidInput, msg))
                } else {
                    Err(io::Error::other(msg))
                }
            }
        }
    }

    /// Convert to ChibiInput format
    pub fn to_input(&self) -> io::Result<ChibiInput> {
        // Validate inspect values
        let inspect_current = self
            .inspect_current
            .as_ref()
            .map(|s| parse_inspectable(s))
            .transpose()?;

        let inspect = if let Some(ref v) = self.inspect {
            if v.len() >= 2 {
                Some((v[0].clone(), parse_inspectable(&v[1])?))
            } else {
                None
            }
        } else {
            None
        };

        // Validate show_log number
        let show_log = if let Some(ref v) = self.show_log {
            if v.len() >= 2 {
                let n = v[1].parse::<isize>().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", v[1]))
                })?;
                Some((v[0].clone(), n))
            } else {
                None
            }
        } else {
            None
        };

        // Parse string pair tuples
        let rename_context = extract_string_pair(&self.rename_context);
        let set_system_prompt = extract_string_pair(&self.set_system_prompt);

        // Parse plugin invocation with shell-style arg splitting
        let plugin = if let Some(v) = &self.plugin {
            let name = v.first().cloned().unwrap_or_default();
            let args = if let Some(args_str) = v.get(1) {
                if args_str.is_empty() {
                    vec![]
                } else {
                    shlex::split(args_str).ok_or_else(|| {
                        io::Error::new(
                            ErrorKind::InvalidInput,
                            format!("Invalid shell syntax in plugin args: {}", args_str),
                        )
                    })?
                }
            } else {
                vec![]
            };
            Some(PluginInvocation { name, args })
        } else {
            None
        };

        // Parse call_tool invocation (takes JSON directly)
        let call_tool = self.call_tool.as_ref().map(|v| PluginInvocation {
            name: v.first().cloned().unwrap_or_default(),
            args: v.get(1).cloned().into_iter().collect(),
        });

        // Parse debug keys (comma-separated), separating CLI-only keys from core keys.
        // CLI-only keys (md=<file>, force-markdown) are extracted here and never
        // enter ExecutionFlags — core doesn't need to know about them.
        let debug_segments: Vec<&str> = self
            .debug
            .as_deref()
            .map(|s| s.split(',').map(str::trim).collect())
            .unwrap_or_default();
        let md_file = debug_segments.iter().find_map(|s| {
            s.strip_prefix("md=")
                .filter(|p| !p.is_empty())
                .map(String::from)
        });
        let force_markdown = debug_segments
            .iter()
            .any(|s| *s == "force-markdown" || *s == "force_markdown");
        let debug_keys: Vec<DebugKey> = debug_segments
            .iter()
            .filter(|s| !s.starts_with("md=") && **s != "force-markdown" && **s != "force_markdown")
            .filter_map(|s| DebugKey::parse(s))
            .collect();
        let debug_implies_force_call_user = md_file.is_some();

        // Compute implied force_call_user based on flags
        let implies_force_call_user = self.list_current_context
            || self.list_contexts
            || self.destroy_current_context
            || self.destroy_context.is_some()
            || self.archive_history.is_some()
            || self.compact_context.is_some()
            || rename_context.is_some()
            || self.show_current_log.is_some()
            || show_log.is_some()
            || inspect_current.is_some()
            || inspect.is_some()
            || set_system_prompt.is_some()
            || plugin.is_some()
            || call_tool.is_some()
            || debug_implies_force_call_user
            || self.model_metadata.is_some()
            || self.model_metadata_full.is_some()
            || self.set_model_for_context.is_some();

        let mut force_call_user = self.force_call_user || implies_force_call_user;
        if self.force_call_agent {
            force_call_user = false;
        }

        // Validate: -x with prompt is an error
        if self.force_call_user && !self.prompt.is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "-x (force-call-user) is incompatible with a prompt",
            ));
        }

        // Determine context selection
        let context = if let Some(ref name) = self.ephemeral_context {
            ContextSelection::Ephemeral { name: name.clone() }
        } else if let Some(ref name) = self.switch_context {
            ContextSelection::Switch {
                name: name.clone(),
                persistent: true,
            }
        } else {
            ContextSelection::Current
        };

        // Determine username override
        let username_override = if let Some(ref name) = self.ephemeral_username {
            Some(UsernameOverride::Ephemeral(name.clone()))
        } else {
            self.set_username
                .as_ref()
                .map(|name| UsernameOverride::Persistent(name.clone()))
        };

        // Determine command
        // First check prompt (highest priority when not force_call_user)
        let command = if !self.prompt.is_empty() && !force_call_user {
            Command::SendPrompt {
                prompt: self.prompt.join(" "),
            }
        // Simple standalone commands
        } else if self.list_contexts {
            Command::ListContexts
        } else if self.list_current_context {
            Command::ListCurrentContext
        } else if self.cleanup_cache {
            Command::CleanupCache
        // Current/specific context pairs (data-driven dispatch)
        } else if let Some(name) =
            check_context_pair(self.destroy_current_context, &self.destroy_context)
        {
            Command::DestroyContext { name }
        } else if let Some(name) =
            check_context_pair(self.archive_current_history, &self.archive_history)
        {
            Command::ArchiveHistory { name }
        } else if let Some(name) = check_context_pair(self.clear_cache, &self.clear_cache_for) {
            Command::ClearCache { name }
        } else if let Some(name) =
            check_context_pair(self.compact_current_context, &self.compact_context)
        {
            Command::CompactContext { name }
        // Complex cases that need pre-parsed values
        } else if let Some(ref new_name) = self.rename_current_context {
            Command::RenameContext {
                old: None,
                new: new_name.clone(),
            }
        } else if let Some((ref old, ref new)) = rename_context {
            Command::RenameContext {
                old: Some(old.clone()),
                new: new.clone(),
            }
        } else if let Some(count) = self.show_current_log {
            Command::ShowLog {
                context: None,
                count,
            }
        } else if let Some((ref ctx, count)) = show_log {
            Command::ShowLog {
                context: Some(ctx.clone()),
                count,
            }
        } else if let Some(ref thing) = inspect_current {
            Command::Inspect {
                context: None,
                thing: thing.clone(),
            }
        } else if let Some((ref ctx, ref thing)) = inspect {
            Command::Inspect {
                context: Some(ctx.clone()),
                thing: thing.clone(),
            }
        } else if let Some(ref prompt_val) = self.set_current_system_prompt {
            Command::SetSystemPrompt {
                context: None,
                prompt: prompt_val.clone(),
            }
        } else if let Some((ref ctx, ref prompt_val)) = set_system_prompt {
            Command::SetSystemPrompt {
                context: Some(ctx.clone()),
                prompt: prompt_val.clone(),
            }
        } else if let Some(ref invocation) = plugin {
            Command::RunPlugin {
                name: invocation.name.clone(),
                args: invocation.args.clone(),
            }
        } else if let Some(ref invocation) = call_tool {
            Command::CallTool {
                name: invocation.name.clone(),
                args: invocation.args.clone(),
            }
        } else if let Some(ref model) = self.model_metadata {
            Command::ModelMetadata {
                model: model.clone(),
                full: false,
            }
        } else if let Some(ref model) = self.model_metadata_full {
            Command::ModelMetadata {
                model: model.clone(),
                full: true,
            }
        } else if let Some(ref model) = self.set_model {
            Command::SetModel {
                context: None,
                model: model.clone(),
            }
        } else if let Some(ref v) = self.set_model_for_context {
            if v.len() >= 2 {
                Command::SetModel {
                    context: Some(v[0].clone()),
                    model: v[1].clone(),
                }
            } else {
                Command::NoOp
            }
        } else if self.check_all_inboxes {
            Command::CheckAllInboxes
        } else if let Some(ref ctx) = self.check_inbox_for {
            Command::CheckInbox {
                context: ctx.clone(),
            }
        } else {
            Command::NoOp
        };

        let flags = ExecutionFlags {
            force_call_user,
            force_call_agent: self.force_call_agent,
            debug: debug_keys,
            destroy_at: self.destroy_at,
            destroy_after_seconds_inactive: self.destroy_after_inactive,
        };

        // Parse -s/--set KEY=VALUE pairs
        let mut config_overrides: Vec<(String, String)> = self
            .set
            .iter()
            .map(|s| {
                let (k, v) = s.split_once('=').ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("--set value must be KEY=VALUE, got: {}", s),
                    )
                })?;
                Ok((k.to_string(), v.to_string()))
            })
            .collect::<io::Result<_>>()?;

        // Boolean CLI flags → config overrides (only when true)
        if self.no_tool_calls {
            config_overrides.push(("no_tool_calls".to_string(), "true".to_string()));
        }

        Ok(ChibiInput {
            command,
            flags,
            context,
            username_override,
            raw: self.raw,
            md_file,
            force_markdown,
            config_overrides,
            verbose_flag: self.verbose,
            hide_tool_calls_flag: self.hide_tool_calls,
            show_thinking_flag: self.show_thinking,
        })
    }

    /// Print help message
    pub fn print_help() {
        use clap::CommandFactory;
        let mut cmd = Self::command();
        let _ = cmd.print_help();
    }
}

/// Expand attached short args like -Dname to -D name
/// This preserves backward compatibility with the old hand-rolled parser
fn expand_attached_args(args: &[String]) -> Vec<String> {
    // Flags that take a value and can have it attached
    const ATTACHED_FLAGS: &[char] = &[
        'c', 'C', 'D', 'A', 'Z', 'r', 'g', 'n', 'y', 'u', 'U', 'm', 'M',
    ];

    let mut result = Vec::new();

    for arg in args {
        // Check if this is a short flag with attached value (e.g., -Dname)
        // Use char_indices for safe UTF-8 handling
        if arg.starts_with('-') && !arg.starts_with("--") {
            let mut chars = arg.char_indices().skip(1); // Skip the leading '-'
            if let Some((_, flag_char)) = chars.next()
                && ATTACHED_FLAGS.contains(&flag_char)
                && let Some((value_start, _)) = chars.next()
            {
                // Split into -X and value
                let flag = format!("-{}", flag_char);
                let value = arg[value_start..].to_string();
                result.push(flag);
                result.push(value);
                continue;
            }
        }
        result.push(arg.clone());
    }

    result
}

/// Read prompt interactively from terminal (dot on empty line terminates)
fn read_prompt_interactive() -> io::Result<String> {
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut buffer = String::new();
    let mut prompt = String::new();
    let mut first = true;

    loop {
        buffer.clear();
        let bytes_read = stdin_lock.read_line(&mut buffer)?;

        // EOF (Ctrl+D)
        if bytes_read == 0 {
            break;
        }

        // Remove trailing newline
        if buffer.ends_with('\n') {
            buffer.pop();
            if buffer.ends_with('\r') {
                buffer.pop();
            }
        }

        // Check for termination: a single dot on a line
        if buffer.trim() == "." {
            break;
        }

        if !first {
            prompt.push(' ');
        }
        prompt.push_str(&buffer);
        first = false;
    }

    Ok(prompt)
}

/// Read prompt from piped stdin (reads until EOF)
fn read_prompt_from_pipe() -> io::Result<String> {
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    // Read all remaining lines
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        input.push('\n');
        input.push_str(&line?);
    }

    Ok(input.trim().to_string())
}

/// Parse CLI arguments and return unified ChibiInput
pub fn parse() -> io::Result<ChibiInput> {
    let cli = Cli::parse_args()?;

    // Handle help and version early
    if cli.help {
        Cli::print_help();
        std::process::exit(0);
    }
    if cli.version {
        let branch = option_env!("VERGEN_GIT_BRANCH").unwrap_or("unknown");
        let is_main = branch == "main";
        if is_main {
            println!("chibi v{}", env!("CARGO_PKG_VERSION"));
        } else {
            let sha = option_env!("VERGEN_GIT_SHA")
                .map(|s| &s[..7.min(s.len())])
                .unwrap_or("unknown");
            let date = env!("CHIBI_BUILD_DATE");
            println!(
                "chibi v{}-{} ({} {})",
                env!("CARGO_PKG_VERSION"),
                branch,
                sha,
                date
            );
        }
        println!("ratatoskr {}", chibi_core::ratatoskr_version());
        std::process::exit(0);
    }

    let mut input = cli.to_input()?;

    // Handle stdin prompt reading (CLI-specific behavior)
    // This happens when there's no command that produces output and we might need
    // to read from stdin or interactive input
    let should_read_prompt = !input.flags.force_call_user && matches!(input.command, Command::NoOp);

    if should_read_prompt {
        let stdin_is_pipe = !io::stdin().is_terminal();
        let arg_prompt = if cli.prompt.is_empty() {
            None
        } else {
            Some(cli.prompt.join(" "))
        };

        let prompt = match (stdin_is_pipe, arg_prompt) {
            // Piped input + arg prompt: concatenate
            (true, Some(arg)) => {
                let piped = read_prompt_from_pipe()?;
                if piped.is_empty() {
                    arg
                } else {
                    format!("{}\n\n{}", arg, piped)
                }
            }
            // Piped input only
            (true, None) => read_prompt_from_pipe()?,
            // Arg prompt only
            (false, Some(arg)) => arg,
            // Interactive: read from terminal
            (false, None) => read_prompt_interactive()?,
        };

        if !prompt.trim().is_empty() {
            input.command = Command::SendPrompt { prompt };
        }
    }

    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create args from a command string (shell-style parsing)
    fn args(s: &str) -> Vec<String> {
        std::iter::once("chibi".to_string())
            .chain(shlex::split(s).unwrap_or_default())
            .collect()
    }

    /// Parse and return Cli for testing
    fn parse_cli(s: &str) -> io::Result<Cli> {
        Cli::parse_from_args(&args(s))
    }

    /// Parse and return ChibiInput for testing
    fn parse_input(s: &str) -> io::Result<ChibiInput> {
        let cli = Cli::parse_from_args(&args(s))?;
        cli.to_input()
    }

    // === Basic flag tests ===

    #[test]
    fn test_no_args() {
        let cli = parse_cli("").unwrap();
        assert!(cli.prompt.is_empty());
        assert!(!cli.verbose);
        assert!(!cli.list_contexts);
    }

    #[test]
    fn test_simple_prompt() {
        let cli = parse_cli("hello world").unwrap();
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Context switch tests ===

    #[test]
    fn test_switch_context_short() {
        let input = parse_input("-c coding").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, persistent: true } if name == "coding")
        );
        assert!(!input.flags.force_call_user); // combinable, not implied
    }

    #[test]
    fn test_switch_context_long() {
        let input = parse_input("--switch-context coding").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "coding")
        );
    }

    #[test]
    fn test_switch_context_attached() {
        let input = parse_input("-ccoding").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "coding")
        );
    }

    #[test]
    fn test_switch_context_attached_dash_works() {
        // After removing allow_hyphen_values from prompt, -xc- now works correctly
        let input = parse_input("-xc-").unwrap();
        assert!(matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "-"));
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_switch_context_dash_with_space_works() {
        // With a space also works
        let input = parse_input("-xc -").unwrap();
        assert!(matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "-"));
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_prompt_with_dash_requires_double_dash() {
        // Prompts starting with - should use -- separator (use a letter not claimed by any flag)
        let result = parse_cli("-qwerty");
        assert!(result.is_err());

        // With --, it works
        let input = parse_input("-- -starts-with-dash").unwrap();
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "-starts-with-dash")
        );
    }

    #[test]
    fn test_switch_context_with_prompt() {
        let input = parse_input("-c coding hello world").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "coding")
        );
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "hello world")
        );
    }

    // === Ephemeral context tests ===

    #[test]
    fn test_ephemeral_context_short() {
        let input = parse_input("-C temp").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Ephemeral { ref name } if name == "temp")
        );
    }

    #[test]
    fn test_ephemeral_context_with_prompt() {
        let input = parse_input("-C agent run task").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Ephemeral { ref name } if name == "agent")
        );
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "run task")
        );
    }

    // === List tests ===

    #[test]
    fn test_list_current_context_short() {
        let input = parse_input("-l").unwrap();
        assert!(matches!(input.command, Command::ListCurrentContext));
        assert!(input.flags.force_call_user); // implied
    }

    #[test]
    fn test_list_contexts_short() {
        let input = parse_input("-L").unwrap();
        assert!(matches!(input.command, Command::ListContexts));
        assert!(input.flags.force_call_user); // implied
    }

    // === Destroy tests ===

    #[test]
    fn test_destroy_current_context_short() {
        let input = parse_input("-d").unwrap();
        assert!(matches!(
            input.command,
            Command::DestroyContext { name: None }
        ));
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_destroy_context_short() {
        let input = parse_input("-D old-context").unwrap();
        assert!(
            matches!(input.command, Command::DestroyContext { ref name } if *name == Some("old-context".to_string()))
        );
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_destroy_context_attached() {
        let input = parse_input("-Dold-context").unwrap();
        assert!(
            matches!(input.command, Command::DestroyContext { ref name } if *name == Some("old-context".to_string()))
        );
    }

    // === Archive tests ===

    #[test]
    fn test_archive_current_history_short() {
        let input = parse_input("-a").unwrap();
        assert!(matches!(
            input.command,
            Command::ArchiveHistory { name: None }
        ));
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_archive_history_short() {
        let input = parse_input("-A other").unwrap();
        assert!(
            matches!(input.command, Command::ArchiveHistory { ref name } if *name == Some("other".to_string()))
        );
        assert!(input.flags.force_call_user);
    }

    // === Compact tests ===

    #[test]
    fn test_compact_current_context_short() {
        let input = parse_input("-z").unwrap();
        assert!(matches!(
            input.command,
            Command::CompactContext { name: None }
        ));
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_compact_context_short() {
        let input = parse_input("-Z other").unwrap();
        assert!(
            matches!(input.command, Command::CompactContext { ref name } if *name == Some("other".to_string()))
        );
        assert!(input.flags.force_call_user);
    }

    // === Inbox check tests ===

    #[test]
    fn test_check_all_inboxes_short() {
        let input = parse_input("-b").unwrap();
        assert!(matches!(input.command, Command::CheckAllInboxes));
        assert!(!input.flags.force_call_user); // will invoke LLM if inbox has messages
    }

    #[test]
    fn test_check_all_inboxes_long() {
        let input = parse_input("--check-all-inboxes").unwrap();
        assert!(matches!(input.command, Command::CheckAllInboxes));
    }

    #[test]
    fn test_check_inbox_for_short() {
        let input = parse_input("-B work").unwrap();
        assert!(matches!(input.command, Command::CheckInbox { ref context } if context == "work"));
        assert!(!input.flags.force_call_user);
    }

    #[test]
    fn test_check_inbox_for_long() {
        let input = parse_input("--check-inbox-for work").unwrap();
        assert!(matches!(input.command, Command::CheckInbox { ref context } if context == "work"));
    }

    #[test]
    fn test_check_inbox_for_with_hyphen_context() {
        let input = parse_input("-B my-context").unwrap();
        assert!(
            matches!(input.command, Command::CheckInbox { ref context } if context == "my-context")
        );
    }

    // === Rename tests ===

    #[test]
    fn test_rename_current_context_short() {
        let input = parse_input("-r newname").unwrap();
        assert!(
            matches!(input.command, Command::RenameContext { old: None, ref new } if new == "newname")
        );
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_rename_context_short() {
        let input = parse_input("-R old new").unwrap();
        assert!(
            matches!(input.command, Command::RenameContext { ref old, ref new } if *old == Some("old".to_string()) && new == "new")
        );
        assert!(input.flags.force_call_user);
    }

    // === Log/history tests ===

    #[test]
    fn test_show_current_log_short() {
        let input = parse_input("-g 10").unwrap();
        assert!(matches!(
            input.command,
            Command::ShowLog {
                context: None,
                count: 10
            }
        ));
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_show_current_log_negative() {
        let input = parse_input("-g -5").unwrap();
        assert!(matches!(
            input.command,
            Command::ShowLog {
                context: None,
                count: -5
            }
        ));
    }

    #[test]
    fn test_show_log_short() {
        let input = parse_input("-G other 10").unwrap();
        assert!(
            matches!(input.command, Command::ShowLog { ref context, count: 10 } if *context == Some("other".to_string()))
        );
        assert!(input.flags.force_call_user);
    }

    // === Inspect tests ===

    #[test]
    fn test_inspect_current_system_prompt() {
        let input = parse_input("-n system_prompt").unwrap();
        assert!(
            matches!(input.command, Command::Inspect { context: None, ref thing } if *thing == Inspectable::SystemPrompt)
        );
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_inspect_current_invalid() {
        let result = parse_input("-n invalid");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown inspectable")
        );
    }

    #[test]
    fn test_inspect_other() {
        let input = parse_input("-N other todos").unwrap();
        assert!(
            matches!(input.command, Command::Inspect { ref context, ref thing }
            if *context == Some("other".to_string()) && *thing == Inspectable::Todos)
        );
        assert!(input.flags.force_call_user);
    }

    // === Set system prompt tests ===

    #[test]
    fn test_set_current_system_prompt_short() {
        let input = parse_input("-y prompt.md").unwrap();
        assert!(
            matches!(input.command, Command::SetSystemPrompt { context: None, ref prompt } if prompt == "prompt.md")
        );
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_set_system_prompt_short() {
        let input = parse_input("-Y other prompt.md").unwrap();
        assert!(
            matches!(input.command, Command::SetSystemPrompt { ref context, ref prompt }
            if *context == Some("other".to_string()) && prompt == "prompt.md")
        );
        assert!(input.flags.force_call_user);
    }

    // === Username tests ===

    #[test]
    fn test_set_username_short() {
        let input = parse_input("-u alice").unwrap();
        assert!(
            matches!(input.username_override, Some(UsernameOverride::Persistent(ref u)) if u == "alice")
        );
    }

    #[test]
    fn test_ephemeral_username_short() {
        let input = parse_input("-U bob").unwrap();
        assert!(
            matches!(input.username_override, Some(UsernameOverride::Ephemeral(ref u)) if u == "bob")
        );
    }

    // === Plugin/tool tests ===

    #[test]
    fn test_plugin_short() {
        // -p now requires exactly 2 args: NAME and ARGS (use empty string for no args)
        let input = parse_input("-p myplugin \"\"").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args.is_empty())
        );
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_plugin_with_args() {
        // Args are now passed as a single quoted string, split using shell rules
        let input = parse_input("-p myplugin \"list --all\"").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["list", "--all"])
        );
    }

    #[test]
    fn test_call_tool_short() {
        // -P now requires exactly 2 args: TOOL and JSON (use {} for no args)
        let input = parse_input("-P update_todos \"{}\"").unwrap();
        assert!(
            matches!(input.command, Command::CallTool { ref name, ref args }
            if name == "update_todos" && args == &["{}"])
        );
        assert!(input.flags.force_call_user);
    }

    // === Verbose and control flags ===

    #[test]
    fn test_verbose_short() {
        let input = parse_input("-v").unwrap();
        assert!(input.verbose_flag);
    }

    #[test]
    fn test_force_call_user_explicit() {
        let input = parse_input("-x").unwrap();
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_force_call_agent_short() {
        let input = parse_input("-X").unwrap();
        // force_call_agent is stored in flags
        assert!(input.flags.force_call_agent);
        assert!(!input.flags.force_call_user);
    }

    #[test]
    fn test_force_call_agent_overrides_implied() {
        let input = parse_input("-X -L").unwrap();
        assert!(matches!(input.command, Command::ListContexts));
        // force_call_agent overrides the implied force_call_user from -L
        assert!(!input.flags.force_call_user);
        assert!(input.flags.force_call_agent);
    }

    // === Double dash handling ===

    #[test]
    fn test_double_dash_forces_prompt() {
        let cli = parse_cli("-- -this -looks -like -flags").unwrap();
        assert_eq!(cli.prompt, vec!["-this", "-looks", "-like", "-flags"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn test_prompt_after_options() {
        let cli = parse_cli("-v hello world").unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Error cases ===

    #[test]
    fn test_switch_context_missing_arg() {
        let result = parse_cli("-c");
        assert!(result.is_err());
    }

    #[test]
    fn test_rename_context_missing_args() {
        let result = parse_cli("-R old");
        assert!(result.is_err());
    }

    #[test]
    fn test_show_current_log_invalid_number() {
        let result = parse_input("-g abc");
        assert!(result.is_err());
    }

    // === New context syntax ===

    #[test]
    fn test_switch_context_new() {
        let input = parse_input("-c new").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "new")
        );
    }

    #[test]
    fn test_switch_context_new_with_prefix() {
        let input = parse_input("-c new:myproject").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "new:myproject")
        );
    }

    // === Prompt parsing behavior ===

    #[test]
    fn cli_bare_word_is_prompt() {
        let cli = parse_cli("hello").unwrap();
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn cli_multiple_words_are_prompt() {
        let cli = parse_cli("explain this error").unwrap();
        assert_eq!(cli.prompt, vec!["explain", "this", "error"]);
    }

    #[test]
    fn cli_quoted_prompt_style() {
        let a = vec![
            "chibi".to_string(),
            "add a users table to this schema".to_string(),
        ];
        let cli = Cli::parse_from_args(&a).unwrap();
        assert_eq!(cli.prompt, vec!["add a users table to this schema"]);
    }

    #[test]
    fn cli_empty_prompt_allowed() {
        let cli = parse_cli("").unwrap();
        assert!(cli.prompt.is_empty());
    }

    #[test]
    fn cli_options_only_no_prompt() {
        let cli = parse_cli("-v").unwrap();
        assert!(cli.verbose);
        assert!(cli.prompt.is_empty());
    }

    // === Plugin captures everything after ===

    #[test]
    fn test_plugin_no_longer_captures_trailing_flags() {
        // With the new 2-arg format, flags after -p are parsed normally
        let input = parse_input("-p myplugin '-l --verbose' -v").unwrap();
        assert!(input.verbose_flag); // -v is now parsed as verbose flag
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["-l", "--verbose"])
        );
    }

    #[test]
    fn test_plugin_verbose_before() {
        let input = parse_input("-v -p myplugin \"arg1\"").unwrap();
        assert!(input.verbose_flag);
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["arg1"])
        );
    }

    #[test]
    fn test_plugin_with_quoted_args() {
        // Shell-style quoting within the args string
        let input = parse_input("-p myplugin \"arg1 'with spaces' arg2\"").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["arg1", "with spaces", "arg2"])
        );
    }

    #[test]
    fn test_plugin_verbose_after() {
        // Flags can now come after -p since it only takes 2 args
        let input = parse_input("-p myplugin \"arg1\" -v").unwrap();
        assert!(input.verbose_flag);
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["arg1"])
        );
    }

    #[test]
    fn test_call_tool_with_json() {
        let input = parse_input("-P send_message '{\"to\":\"foo\",\"content\":\"hi\"}'").unwrap();
        assert!(
            matches!(input.command, Command::CallTool { ref name, ref args }
            if name == "send_message" && args == &["{\"to\":\"foo\",\"content\":\"hi\"}"])
        );
    }

    #[test]
    fn test_call_tool_with_flags_after() {
        // Flags can now come after -P since it only takes 2 args
        let input = parse_input("-P mytool '{}' -v -C ephemeral").unwrap();
        assert!(input.verbose_flag);
        assert!(
            matches!(input.context, ContextSelection::Ephemeral { ref name } if name == "ephemeral")
        );
        assert!(
            matches!(input.command, Command::CallTool { ref name, ref args }
            if name == "mytool" && args == &["{}"])
        );
    }

    // === Attached arg expansion tests (expand_attached_args) ===

    #[test]
    fn test_expand_attached_args_basic() {
        let args = vec!["chibi".to_string(), "-Dmycontext".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "mycontext"]);
    }

    #[test]
    fn test_expand_attached_args_already_separated() {
        let args = vec![
            "chibi".to_string(),
            "-D".to_string(),
            "mycontext".to_string(),
        ];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "mycontext"]);
    }

    #[test]
    fn test_expand_attached_args_flag_only() {
        // Just -D without value - should not expand
        let args = vec!["chibi".to_string(), "-D".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D"]);
    }

    #[test]
    fn test_expand_attached_args_with_utf8_value() {
        // UTF-8 characters in the value should be preserved
        let args = vec!["chibi".to_string(), "-Dкириллица".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "кириллица"]);
    }

    #[test]
    fn test_expand_attached_args_with_emoji_value() {
        // Emoji in the value
        let args = vec!["chibi".to_string(), "-Dtest🎉emoji".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "test🎉emoji"]);
    }

    #[test]
    fn test_expand_attached_args_with_cjk_value() {
        // CJK characters in the value
        let args = vec!["chibi".to_string(), "-D日本語".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "日本語"]);
    }

    #[test]
    fn test_expand_attached_args_dash_as_value() {
        // -D- should expand to -D and -
        let args = vec!["chibi".to_string(), "-D-".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "-"]);
    }

    #[test]
    fn test_expand_attached_args_multiple_dashes_as_value() {
        // -D-- should expand to -D and --
        let args = vec!["chibi".to_string(), "-D--".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-D", "--"]);
    }

    #[test]
    fn test_expand_attached_args_preserves_long_flags() {
        // Long flags should not be expanded
        let args = vec![
            "chibi".to_string(),
            "--delete-context".to_string(),
            "mycontext".to_string(),
        ];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "--delete-context", "mycontext"]);
    }

    #[test]
    fn test_expand_attached_args_non_expandable_flag() {
        // -v is not in the ATTACHED_FLAGS list, so -vfoo should stay as-is
        let args = vec!["chibi".to_string(), "-vfoo".to_string()];
        let expanded = super::expand_attached_args(&args);
        // -v is not expandable, so it stays as -vfoo
        assert_eq!(expanded, vec!["chibi", "-vfoo"]);
    }

    #[test]
    fn test_expand_attached_args_multiple_flags() {
        let args = vec![
            "chibi".to_string(),
            "-v".to_string(),
            "-Dold".to_string(),
            "-cmycontext".to_string(),
        ];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(
            expanded,
            vec!["chibi", "-v", "-D", "old", "-c", "mycontext"]
        );
    }

    #[test]
    fn test_expand_attached_args_empty_after_flag() {
        // Edge case: what if flag char is at end? Should not panic
        let args = vec!["chibi".to_string(), "-c".to_string()];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-c"]);
    }

    #[test]
    fn test_expand_attached_args_preserves_positional() {
        // Positional args should be preserved
        let args = vec![
            "chibi".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "hello", "world"]);
    }

    #[test]
    fn test_expand_attached_args_mixed_with_positional() {
        let args = vec![
            "chibi".to_string(),
            "-cmyctx".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ];
        let expanded = super::expand_attached_args(&args);
        assert_eq!(expanded, vec!["chibi", "-c", "myctx", "hello", "world"]);
    }

    // === Inspectable tests ===

    #[test]
    fn test_inspectable_from_str_all_variants() {
        assert_eq!(
            Inspectable::from_str_cli("system_prompt"),
            Some(Inspectable::SystemPrompt)
        );
        assert_eq!(
            Inspectable::from_str_cli("prompt"),
            Some(Inspectable::SystemPrompt)
        ); // alias
        assert_eq!(
            Inspectable::from_str_cli("reflection"),
            Some(Inspectable::Reflection)
        );
        assert_eq!(Inspectable::from_str_cli("todos"), Some(Inspectable::Todos));
        assert_eq!(Inspectable::from_str_cli("goals"), Some(Inspectable::Goals));
        assert_eq!(Inspectable::from_str_cli("list"), Some(Inspectable::List));
        assert_eq!(Inspectable::from_str_cli("unknown"), None);
        assert_eq!(Inspectable::from_str_cli(""), None);
    }

    #[test]
    fn test_inspectable_all_names() {
        let names = Inspectable::all_names_cli();
        assert!(names.contains(&"system_prompt"));
        assert!(names.contains(&"reflection"));
        assert!(names.contains(&"todos"));
        assert!(names.contains(&"goals"));
        assert!(names.contains(&"list"));
    }

    // === ConfigField inspect tests (issue #18) ===

    #[test]
    fn test_inspectable_config_field_model() {
        assert_eq!(
            Inspectable::from_str_cli("model"),
            Some(Inspectable::ConfigField("model".to_string()))
        );
    }

    #[test]
    fn test_inspectable_config_field_username() {
        assert_eq!(
            Inspectable::from_str_cli("username"),
            Some(Inspectable::ConfigField("username".to_string()))
        );
    }

    #[test]
    fn test_inspectable_config_field_api_temperature() {
        assert_eq!(
            Inspectable::from_str_cli("api.temperature"),
            Some(Inspectable::ConfigField("api.temperature".to_string()))
        );
    }

    #[test]
    fn test_inspectable_config_field_api_reasoning_effort() {
        assert_eq!(
            Inspectable::from_str_cli("api.reasoning.effort"),
            Some(Inspectable::ConfigField("api.reasoning.effort".to_string()))
        );
    }

    #[test]
    fn test_inspectable_config_field_context_window_limit() {
        assert_eq!(
            Inspectable::from_str_cli("context_window_limit"),
            Some(Inspectable::ConfigField("context_window_limit".to_string()))
        );
    }

    #[test]
    fn test_inspectable_all_names_includes_config_fields() {
        let names = Inspectable::all_names_cli();
        // Should include config fields
        assert!(names.contains(&"model"));
        assert!(names.contains(&"username"));
        assert!(names.contains(&"api.temperature"));
        assert!(names.contains(&"api.reasoning.effort"));
    }

    #[test]
    fn test_inspectable_invalid_config_path() {
        // Invalid paths should return None
        assert_eq!(Inspectable::from_str_cli("api.nonexistent"), None);
        assert_eq!(Inspectable::from_str_cli("foo.bar.baz"), None);
    }

    // === Debug flag tests ===

    #[test]
    fn test_debug_md_implies_force_call_user() {
        let input = parse_input("--debug md=README.md").unwrap();
        assert!(input.flags.force_call_user); // should imply -x
        assert_eq!(input.md_file.as_deref(), Some("README.md"));
        // md= should not leak into core debug keys
        assert!(input.flags.debug.is_empty());
    }

    #[test]
    fn test_debug_md_can_be_overridden_with_force_call_agent() {
        let input = parse_input("-X --debug md=README.md").unwrap();
        assert!(!input.flags.force_call_user); // -X should override
        assert!(input.flags.force_call_agent);
        assert_eq!(input.md_file.as_deref(), Some("README.md"));
    }

    #[test]
    fn test_debug_request_log_does_not_imply_force_call_user() {
        let input = parse_input("--debug request-log").unwrap();
        assert!(!input.flags.force_call_user); // should NOT imply -x
        assert!(
            input
                .flags
                .debug
                .iter()
                .any(|k| matches!(k, DebugKey::RequestLog))
        );
    }

    #[test]
    fn test_destroy_at_flag() {
        let input = parse_input("--destroy-at 1234567890").unwrap();
        assert_eq!(input.flags.destroy_at, Some(1234567890));
        assert_eq!(input.flags.destroy_after_seconds_inactive, None);
    }

    #[test]
    fn test_destroy_after_inactive_flag() {
        let input = parse_input("--destroy-after-inactive 60").unwrap();
        assert_eq!(input.flags.destroy_after_seconds_inactive, Some(60));
        assert_eq!(input.flags.destroy_at, None);
    }

    #[test]
    fn test_destroy_flags_absent_by_default() {
        let input = parse_input("-l").unwrap();
        assert_eq!(input.flags.destroy_at, None);
        assert_eq!(input.flags.destroy_after_seconds_inactive, None);
    }

    #[test]
    fn test_destroy_at_combinable_with_context_switch() {
        let input = parse_input("--destroy-after-inactive 1 -c test-ctx -l").unwrap();
        assert_eq!(input.flags.destroy_after_seconds_inactive, Some(1));
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "test-ctx")
        );
    }

    // === Model metadata tests ===

    #[test]
    fn test_model_metadata_long() {
        let input = parse_input("--model-metadata anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: false } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_model_metadata_full_long() {
        let input = parse_input("--model-metadata-full anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: true } if model == "anthropic/claude-sonnet-4")
        );
    }

    // Verify old -m/-M no longer accepted for model-metadata
    #[test]
    fn test_model_metadata_long_only() {
        let input = parse_input("--model-metadata anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: false } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_model_metadata_full_long_only() {
        let input = parse_input("--model-metadata-full anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::ModelMetadata { ref model, full: true } if model == "anthropic/claude-sonnet-4")
        );
    }

    // === Set model tests ===

    #[test]
    fn test_set_model_short() {
        let input = parse_input("-m anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
        assert!(!input.flags.force_call_user); // combinable
    }

    #[test]
    fn test_set_model_long() {
        let input = parse_input("--set-model anthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_set_model_attached() {
        let input = parse_input("-manthropic/claude-sonnet-4").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { context: None, ref model } if model == "anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_set_model_for_context_short() {
        let input = parse_input("-M myctx openai/gpt-4o").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { ref context, ref model }
                if *context == Some("myctx".to_string()) && model == "openai/gpt-4o")
        );
        assert!(input.flags.force_call_user); // implies --no-chibi
    }

    #[test]
    fn test_set_model_for_context_long() {
        let input = parse_input("--set-model-for-context myctx openai/gpt-4o").unwrap();
        assert!(
            matches!(input.command, Command::SetModel { ref context, ref model }
                if *context == Some("myctx".to_string()) && model == "openai/gpt-4o")
        );
        assert!(input.flags.force_call_user);
    }

    #[test]
    fn test_debug_comma_separated() {
        let input = parse_input("--debug request-log,force-markdown").unwrap();
        // force-markdown is CLI-only, extracted into its own field
        assert_eq!(input.flags.debug.len(), 1);
        assert!(
            input
                .flags
                .debug
                .iter()
                .any(|k| matches!(k, DebugKey::RequestLog))
        );
        assert!(input.force_markdown);
    }

    // === -s/--set config override tests ===

    #[test]
    fn test_set_single_override() {
        let input = parse_input("-s fuel=50").unwrap();
        assert_eq!(
            input.config_overrides,
            vec![("fuel".to_string(), "50".to_string())]
        );
    }

    #[test]
    fn test_set_multiple_overrides() {
        let input = parse_input("-s fuel=50 -s model=gpt-4").unwrap();
        assert_eq!(input.config_overrides.len(), 2);
        assert_eq!(
            input.config_overrides[0],
            ("fuel".to_string(), "50".to_string())
        );
        assert_eq!(
            input.config_overrides[1],
            ("model".to_string(), "gpt-4".to_string())
        );
    }

    #[test]
    fn test_set_long_form() {
        let input = parse_input("--set fuel=50").unwrap();
        assert_eq!(
            input.config_overrides,
            vec![("fuel".to_string(), "50".to_string())]
        );
    }

    #[test]
    fn test_set_value_with_equals() {
        // values can contain '=' (split on first only)
        let input = parse_input("-s api.stop=a=b").unwrap();
        assert_eq!(
            input.config_overrides,
            vec![("api.stop".to_string(), "a=b".to_string())]
        );
    }

    #[test]
    fn test_set_missing_equals_errors() {
        let result = parse_input("-s fuelonly");
        assert!(result.is_err());
    }

    #[test]
    fn test_set_with_prompt() {
        let input = parse_input("-s fuel=5 -- hello world").unwrap();
        assert_eq!(
            input.config_overrides,
            vec![("fuel".to_string(), "5".to_string())]
        );
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "hello world")
        );
    }
}
