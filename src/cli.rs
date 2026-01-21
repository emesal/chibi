//! CLI argument parsing with clap.
//!
//! This module handles parsing command-line arguments and converting them
//! to the unified `ChibiInput` format.

use crate::input::{ChibiInput, Command, ContextSelection, Flags, PartialRuntimeConfig, UsernameOverride};
use clap::Parser;
use std::io::{self, ErrorKind};

/// Direct plugin invocation from CLI
#[derive(Debug, Clone)]
pub struct PluginInvocation {
    pub name: String,
    pub args: Vec<String>,
}

/// Inspectable things via -n/-N
#[derive(Debug, Clone, PartialEq)]
pub enum Inspectable {
    SystemPrompt,
    Reflection,
    Todos,
    Goals,
    List, // Lists all inspectable items
}

impl Inspectable {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "system_prompt" | "prompt" => Some(Inspectable::SystemPrompt),
            "reflection" => Some(Inspectable::Reflection),
            "todos" => Some(Inspectable::Todos),
            "goals" => Some(Inspectable::Goals),
            "list" => Some(Inspectable::List),
            _ => None,
        }
    }

    pub fn all_names() -> &'static [&'static str] {
        &["system_prompt", "reflection", "todos", "goals", "list"]
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
    #[arg(short = 'c', long = "switch-context", value_name = "NAME", allow_hyphen_values = true)]
    pub switch_context: Option<String>,

    /// Use context for this invocation only
    #[arg(short = 'C', long = "transient-context", value_name = "NAME", allow_hyphen_values = true)]
    pub transient_context: Option<String>,

    /// Show current context info
    #[arg(short = 'l', long = "list-current-context")]
    pub list_current_context: bool,

    /// List all contexts
    #[arg(short = 'L', long = "list-contexts")]
    pub list_contexts: bool,

    /// Delete current context
    #[arg(short = 'd', long = "delete-current-context")]
    pub delete_current_context: bool,

    /// Delete specified context
    #[arg(short = 'D', long = "delete-context", value_name = "CTX", allow_hyphen_values = true)]
    pub delete_context: Option<String>,

    /// Archive current context (clear history, save to transcript)
    #[arg(short = 'a', long = "archive-current-context")]
    pub archive_current_context: bool,

    /// Archive specified context
    #[arg(short = 'A', long = "archive-context", value_name = "CTX", allow_hyphen_values = true)]
    pub archive_context: Option<String>,

    /// Compact current context (summarize and clear)
    #[arg(short = 'z', long = "compact-current-context")]
    pub compact_current_context: bool,

    /// Compact specified context
    #[arg(short = 'Z', long = "compact-context", value_name = "CTX", allow_hyphen_values = true)]
    pub compact_context: Option<String>,

    /// Rename current context
    #[arg(short = 'r', long = "rename-current-context", value_name = "NEW", allow_hyphen_values = true)]
    pub rename_current_context: Option<String>,

    /// Rename specified context (requires OLD and NEW args)
    #[arg(short = 'R', long = "rename-context", value_names = ["OLD", "NEW"], num_args = 2, allow_hyphen_values = true)]
    pub rename_context: Option<Vec<String>>,

    /// Show last N log entries (current context). Use negative for first N
    #[arg(short = 'g', long = "show-current-log", value_name = "N", allow_hyphen_values = true)]
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
    #[arg(short = 'y', long = "set-current-system-prompt", value_name = "PROMPT", allow_hyphen_values = true)]
    pub set_current_system_prompt: Option<String>,

    /// Set system prompt for specified context (requires CTX and PROMPT)
    #[arg(short = 'Y', long = "set-system-prompt", value_names = ["CTX", "PROMPT"], num_args = 2, allow_hyphen_values = true)]
    pub set_system_prompt: Option<Vec<String>>,

    // === Username options ===

    /// Set username (persists to local.toml)
    #[arg(short = 'u', long = "set-username", value_name = "NAME", allow_hyphen_values = true)]
    pub set_username: Option<String>,

    /// Set username for this invocation only
    #[arg(short = 'U', long = "transient-username", value_name = "NAME", allow_hyphen_values = true)]
    pub transient_username: Option<String>,

    // === Plugin/tool options ===
    // Note: These are handled specially because they consume all remaining args

    /// Run a plugin directly (-p NAME [ARGS...])
    #[arg(short = 'p', long = "plugin", value_name = "NAME", num_args = 1.., allow_hyphen_values = true)]
    pub plugin: Option<Vec<String>>,

    /// Call a tool directly (-P TOOL [ARGS...])
    #[arg(short = 'P', long = "call-tool", value_name = "TOOL", num_args = 1.., allow_hyphen_values = true)]
    pub call_tool: Option<Vec<String>>,

    // === Control flags ===

    /// Show extra info (tools loaded, etc.)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Don't invoke the LLM
    #[arg(short = 'x', long = "no-chibi")]
    pub no_chibi: bool,

    /// Force LLM invocation (overrides implied -x)
    #[arg(short = 'X', long = "force-chibi")]
    pub force_chibi: bool,

    // === JSON modes ===

    /// Read input as JSON from stdin (exclusive with config flags)
    #[arg(long = "json-config")]
    pub json_config: bool,

    /// Output in JSONL format
    #[arg(long = "json-output")]
    pub json_output: bool,

    // === Help and version ===

    /// Show help
    #[arg(short = 'h', long = "help")]
    pub help: bool,

    /// Show version
    #[arg(long = "version")]
    pub version: bool,

    // === Positional: prompt ===
    /// The prompt to send (all remaining arguments)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}

const CLI_AFTER_HELP: &str = r#"EXAMPLES:
  chibi What is Rust?             Send prompt to LLM
  chibi -c coding write code      Switch context, then send prompt
  chibi -L                        List all contexts
  chibi -l                        Show current context info
  chibi -Dold                     Delete 'old' context (attached arg)
  chibi -D old                    Delete 'old' context (separated arg)
  chibi -n system_prompt          Inspect current system prompt
  chibi -g 10                     Show last 10 log entries
  chibi -x -c test                Switch context without LLM
  chibi -X -L                     List contexts then invoke LLM
  chibi -a hello                  Archive context, then send prompt

FLAG BEHAVIOR:
  Some flags imply --no-chibi (operations that produce output or
  operate on other contexts). Use -X to override and invoke LLM after.

  Implied --no-chibi: -l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -p, -P
  Combinable with prompt: -c, -C, -a, -z, -r, -y, -u, -U, -v

PROMPT INPUT:
  Arguments after options are joined as the prompt.
  Use -- to force remaining args as prompt (e.g., chibi -- -starts-with-dash)
  No arguments: read from stdin (end with . on empty line)
  Piped input: echo 'text' | chibi"#;

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
                    Err(io::Error::new(ErrorKind::Other, msg))
                }
            }
        }
    }

    /// Convert parsed CLI to the old Cli format for backward compatibility
    /// This method returns a struct matching the old interface
    pub fn to_legacy(&self) -> io::Result<LegacyCli> {
        let mut cli = LegacyCli {
            switch_context: self.switch_context.clone(),
            transient_context: self.transient_context.clone(),
            list_current_context: self.list_current_context,
            list_contexts: self.list_contexts,
            delete_current_context: self.delete_current_context,
            delete_context: self.delete_context.clone(),
            archive_current_context: self.archive_current_context,
            archive_context: self.archive_context.clone(),
            compact_current_context: self.compact_current_context,
            compact_context: self.compact_context.clone(),
            rename_current_context: self.rename_current_context.clone(),
            rename_context: self.rename_context.as_ref().map(|v| {
                if v.len() >= 2 {
                    (v[0].clone(), v[1].clone())
                } else {
                    (String::new(), String::new())
                }
            }),
            show_current_log: self.show_current_log,
            show_log: self.show_log.as_ref().and_then(|v| {
                if v.len() >= 2 {
                    v[1].parse::<isize>().ok().map(|n| (v[0].clone(), n))
                } else {
                    None
                }
            }),
            inspect_current: match &self.inspect_current {
                Some(s) => Inspectable::from_str(s).ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("Unknown inspectable: {}. Valid options: {:?}", s, Inspectable::all_names()),
                    )
                }).ok(),
                None => None,
            },
            inspect: match &self.inspect {
                Some(v) if v.len() >= 2 => {
                    Inspectable::from_str(&v[1]).ok_or_else(|| {
                        io::Error::new(
                            ErrorKind::InvalidInput,
                            format!("Unknown inspectable: {}. Valid options: {:?}", v[1], Inspectable::all_names()),
                        )
                    }).ok().map(|thing| (v[0].clone(), thing))
                }
                _ => None,
            },
            set_current_system_prompt: self.set_current_system_prompt.clone(),
            set_system_prompt: self.set_system_prompt.as_ref().map(|v| {
                if v.len() >= 2 {
                    (v[0].clone(), v[1].clone())
                } else {
                    (String::new(), String::new())
                }
            }),
            set_username: self.set_username.clone(),
            transient_username: self.transient_username.clone(),
            plugin: self.plugin.as_ref().map(|v| {
                PluginInvocation {
                    name: v.first().cloned().unwrap_or_default(),
                    args: v.get(1..).unwrap_or(&[]).to_vec(),
                }
            }),
            call_tool: self.call_tool.as_ref().map(|v| {
                PluginInvocation {
                    name: v.first().cloned().unwrap_or_default(),
                    args: v.get(1..).unwrap_or(&[]).to_vec(),
                }
            }),
            verbose: self.verbose,
            no_chibi: self.no_chibi,
            force_chibi: self.force_chibi,
            prompt: self.prompt.clone(),
        };

        // Validate inspect values now
        if let Some(s) = &self.inspect_current {
            if Inspectable::from_str(s).is_none() {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unknown inspectable: {}. Valid options: {:?}", s, Inspectable::all_names()),
                ));
            }
        }
        if let Some(v) = &self.inspect {
            if v.len() >= 2 && Inspectable::from_str(&v[1]).is_none() {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unknown inspectable: {}. Valid options: {:?}", v[1], Inspectable::all_names()),
                ));
            }
        }
        if let Some(v) = &self.show_log {
            if v.len() >= 2 {
                v[1].parse::<isize>().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", v[1]))
                })?;
            }
        }

        // Compute implied no_chibi based on flags
        let implies_no_chibi = cli.list_current_context
            || cli.list_contexts
            || cli.delete_current_context
            || cli.delete_context.is_some()
            || cli.archive_context.is_some()
            || cli.compact_context.is_some()
            || cli.rename_context.is_some()
            || cli.show_current_log.is_some()
            || cli.show_log.is_some()
            || cli.inspect_current.is_some()
            || cli.inspect.is_some()
            || cli.set_system_prompt.is_some()
            || cli.plugin.is_some()
            || cli.call_tool.is_some();

        if implies_no_chibi && !cli.force_chibi {
            cli.no_chibi = true;
        }

        // force_chibi wins over no_chibi
        if cli.force_chibi {
            cli.no_chibi = false;
        }

        Ok(cli)
    }

    /// Convert to the new ChibiInput format
    pub fn to_input(&self) -> io::Result<ChibiInput> {
        let legacy = self.to_legacy()?;
        legacy.to_input()
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
    const ATTACHED_FLAGS: &[char] = &['c', 'C', 'D', 'A', 'Z', 'r', 'g', 'n', 'y', 'u', 'U'];

    let mut result = Vec::new();

    for arg in args {
        // Check if this is a short flag with attached value (e.g., -Dname)
        if arg.len() > 2
            && arg.starts_with('-')
            && !arg.starts_with("--")
            && arg.chars().nth(1).map_or(false, |c| ATTACHED_FLAGS.contains(&c))
        {
            // Split into -X and value
            let flag = format!("-{}", arg.chars().nth(1).unwrap());
            let value = arg[2..].to_string();
            result.push(flag);
            result.push(value);
        } else {
            result.push(arg.clone());
        }
    }

    result
}

/// Legacy CLI struct for backward compatibility with existing code
#[derive(Debug)]
pub struct LegacyCli {
    // Context operations (lowercase = current, uppercase = specified)
    pub switch_context: Option<String>,       // -c / --switch-context
    pub transient_context: Option<String>,    // -C / --transient-context
    pub list_current_context: bool,           // -l / --list-current-context
    pub list_contexts: bool,                  // -L / --list-contexts
    pub delete_current_context: bool,         // -d / --delete-current-context
    pub delete_context: Option<String>,       // -D / --delete-context
    pub archive_current_context: bool,        // -a / --archive-current-context
    pub archive_context: Option<String>,      // -A / --archive-context
    pub compact_current_context: bool,        // -z / --compact-current-context
    pub compact_context: Option<String>,      // -Z / --compact-context
    pub rename_current_context: Option<String>,        // -r / --rename-current-context <NEW>
    pub rename_context: Option<(String, String)>,      // -R / --rename-context <OLD> <NEW>
    pub show_current_log: Option<isize>,      // -g / --show-current-log <N>
    pub show_log: Option<(String, isize)>,    // -G / --show-log <CTX> <N>
    pub inspect_current: Option<Inspectable>, // -n / --inspect-current <THING>
    pub inspect: Option<(String, Inspectable)>, // -N / --inspect <CTX> <THING>
    pub set_current_system_prompt: Option<String>, // -y / --set-current-system-prompt
    pub set_system_prompt: Option<(String, String)>, // -Y / --set-system-prompt <CTX> <PROMPT>

    // Username options
    pub set_username: Option<String>,         // -u / --set-username
    pub transient_username: Option<String>,   // -U / --transient-username

    // Plugin/tool options
    pub plugin: Option<PluginInvocation>,     // -p / --plugin
    pub call_tool: Option<PluginInvocation>,  // -P / --call-tool

    // Control flags
    pub verbose: bool,                        // -v / --verbose
    pub no_chibi: bool,                       // -x / --no-chibi
    pub force_chibi: bool,                    // -X / --force-chibi

    // The prompt to send
    pub prompt: Vec<String>,
}

impl LegacyCli {
    /// Check if this CLI invocation should invoke the LLM
    pub fn should_invoke_llm(&self) -> bool {
        !self.no_chibi
    }

    /// Convert to ChibiInput
    pub fn to_input(&self) -> io::Result<ChibiInput> {
        // Determine context selection
        let context = if let Some(ref name) = self.transient_context {
            ContextSelection::Transient { name: name.clone() }
        } else if let Some(ref name) = self.switch_context {
            ContextSelection::Switch { name: name.clone(), persistent: true }
        } else {
            ContextSelection::Current
        };

        // Determine username override
        let username_override = if let Some(ref name) = self.transient_username {
            Some(UsernameOverride::Transient(name.clone()))
        } else if let Some(ref name) = self.set_username {
            Some(UsernameOverride::Persistent(name.clone()))
        } else {
            None
        };

        // Determine command
        let command = if !self.prompt.is_empty() && self.should_invoke_llm() {
            Command::SendPrompt { prompt: self.prompt.join(" ") }
        } else if self.list_contexts {
            Command::ListContexts
        } else if self.list_current_context {
            Command::ListCurrentContext
        } else if self.delete_current_context {
            Command::DeleteContext { name: None }
        } else if let Some(ref name) = self.delete_context {
            Command::DeleteContext { name: Some(name.clone()) }
        } else if self.archive_current_context {
            Command::ArchiveContext { name: None }
        } else if let Some(ref name) = self.archive_context {
            Command::ArchiveContext { name: Some(name.clone()) }
        } else if self.compact_current_context {
            Command::CompactContext { name: None }
        } else if let Some(ref name) = self.compact_context {
            Command::CompactContext { name: Some(name.clone()) }
        } else if let Some(ref new_name) = self.rename_current_context {
            Command::RenameContext { old: None, new: new_name.clone() }
        } else if let Some((ref old, ref new)) = self.rename_context {
            Command::RenameContext { old: Some(old.clone()), new: new.clone() }
        } else if let Some(count) = self.show_current_log {
            Command::ShowLog { context: None, count }
        } else if let Some((ref ctx, count)) = self.show_log {
            Command::ShowLog { context: Some(ctx.clone()), count }
        } else if let Some(ref thing) = self.inspect_current {
            Command::Inspect { context: None, thing: thing.clone() }
        } else if let Some((ref ctx, ref thing)) = self.inspect {
            Command::Inspect { context: Some(ctx.clone()), thing: thing.clone() }
        } else if let Some(ref prompt) = self.set_current_system_prompt {
            Command::SetSystemPrompt { context: None, prompt: prompt.clone() }
        } else if let Some((ref ctx, ref prompt)) = self.set_system_prompt {
            Command::SetSystemPrompt { context: Some(ctx.clone()), prompt: prompt.clone() }
        } else if let Some(ref invocation) = self.plugin {
            Command::RunPlugin { name: invocation.name.clone(), args: invocation.args.clone() }
        } else if let Some(ref invocation) = self.call_tool {
            Command::CallTool { name: invocation.name.clone(), args: invocation.args.clone() }
        } else {
            Command::NoOp
        };

        let flags = Flags {
            verbose: self.verbose,
            json_output: false, // Not set via legacy CLI
            no_chibi: self.no_chibi,
            force_chibi: self.force_chibi,
        };

        Ok(ChibiInput {
            config: PartialRuntimeConfig::default(),
            command,
            flags,
            context,
            username_override,
        })
    }
}

// === Backward compatibility: keep the old parse() function signature ===

/// Parse CLI arguments (backward compatibility wrapper)
pub fn parse() -> io::Result<LegacyCli> {
    let cli = Cli::parse_args()?;

    // Handle help and version early
    if cli.help {
        Cli::print_help();
        std::process::exit(0);
    }
    if cli.version {
        println!("chibi {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    cli.to_legacy()
}

/// Backward compatibility type alias
pub type OldCli = LegacyCli;

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create args from a command string
    fn args(s: &str) -> Vec<String> {
        std::iter::once("chibi".to_string())
            .chain(s.split_whitespace().map(|s| s.to_string()))
            .collect()
    }

    /// Parse and return legacy CLI for testing
    fn parse_legacy(s: &str) -> io::Result<LegacyCli> {
        let cli = Cli::parse_from_args(&args(s))?;
        cli.to_legacy()
    }

    // === Basic flag tests ===

    #[test]
    fn test_no_args() {
        let cli = parse_legacy("").unwrap();
        assert!(cli.prompt.is_empty());
        assert!(!cli.verbose);
        assert!(!cli.list_contexts);
    }

    #[test]
    fn test_simple_prompt() {
        let cli = parse_legacy("hello world").unwrap();
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Context switch tests ===

    #[test]
    fn test_switch_context_short() {
        let cli = parse_legacy("-c coding").unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
        assert!(!cli.no_chibi); // combinable, not implied
    }

    #[test]
    fn test_switch_context_long() {
        let cli = parse_legacy("--switch-context coding").unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
    }

    #[test]
    fn test_switch_context_attached() {
        let cli = parse_legacy("-ccoding").unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
    }

    #[test]
    fn test_switch_context_with_prompt() {
        let cli = parse_legacy("-c coding hello world").unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Transient context tests ===

    #[test]
    fn test_transient_context_short() {
        let cli = parse_legacy("-C temp").unwrap();
        assert_eq!(cli.transient_context, Some("temp".to_string()));
    }

    #[test]
    fn test_transient_context_long() {
        let cli = parse_legacy("--transient-context temp").unwrap();
        assert_eq!(cli.transient_context, Some("temp".to_string()));
    }

    #[test]
    fn test_transient_context_with_prompt() {
        let cli = parse_legacy("-C agent run task").unwrap();
        assert_eq!(cli.transient_context, Some("agent".to_string()));
        assert_eq!(cli.prompt, vec!["run", "task"]);
    }

    // === List tests ===

    #[test]
    fn test_list_current_context_short() {
        let cli = parse_legacy("-l").unwrap();
        assert!(cli.list_current_context);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_list_current_context_long() {
        let cli = parse_legacy("--list-current-context").unwrap();
        assert!(cli.list_current_context);
    }

    #[test]
    fn test_list_contexts_short() {
        let cli = parse_legacy("-L").unwrap();
        assert!(cli.list_contexts);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_list_contexts_long() {
        let cli = parse_legacy("--list-contexts").unwrap();
        assert!(cli.list_contexts);
    }

    // === Delete tests ===

    #[test]
    fn test_delete_current_context_short() {
        let cli = parse_legacy("-d").unwrap();
        assert!(cli.delete_current_context);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_delete_current_context_long() {
        let cli = parse_legacy("--delete-current-context").unwrap();
        assert!(cli.delete_current_context);
    }

    #[test]
    fn test_delete_context_short() {
        let cli = parse_legacy("-D old-context").unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_delete_context_attached() {
        let cli = parse_legacy("-Dold-context").unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
    }

    #[test]
    fn test_delete_context_long() {
        let cli = parse_legacy("--delete-context old-context").unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
    }

    // === Archive tests ===

    #[test]
    fn test_archive_current_context_short() {
        let cli = parse_legacy("-a").unwrap();
        assert!(cli.archive_current_context);
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_archive_current_context_with_prompt() {
        let cli = parse_legacy("-a hello").unwrap();
        assert!(cli.archive_current_context);
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn test_archive_context_short() {
        let cli = parse_legacy("-A other").unwrap();
        assert_eq!(cli.archive_context, Some("other".to_string()));
        assert!(cli.no_chibi); // implied (operates on other context)
    }

    #[test]
    fn test_archive_context_attached() {
        let cli = parse_legacy("-Aother").unwrap();
        assert_eq!(cli.archive_context, Some("other".to_string()));
    }

    // === Compact tests ===

    #[test]
    fn test_compact_current_context_short() {
        let cli = parse_legacy("-z").unwrap();
        assert!(cli.compact_current_context);
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_compact_context_short() {
        let cli = parse_legacy("-Z other").unwrap();
        assert_eq!(cli.compact_context, Some("other".to_string()));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_compact_context_attached() {
        let cli = parse_legacy("-Zother").unwrap();
        assert_eq!(cli.compact_context, Some("other".to_string()));
    }

    // === Rename tests ===

    #[test]
    fn test_rename_current_context_short() {
        let cli = parse_legacy("-r newname").unwrap();
        assert_eq!(cli.rename_current_context, Some("newname".to_string()));
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_rename_current_context_attached() {
        let cli = parse_legacy("-rnewname").unwrap();
        assert_eq!(cli.rename_current_context, Some("newname".to_string()));
    }

    #[test]
    fn test_rename_context_short() {
        let cli = parse_legacy("-R old new").unwrap();
        assert_eq!(cli.rename_context, Some(("old".to_string(), "new".to_string())));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_rename_context_long() {
        let cli = parse_legacy("--rename-context old new").unwrap();
        assert_eq!(cli.rename_context, Some(("old".to_string(), "new".to_string())));
    }

    // === Log/history tests ===

    #[test]
    fn test_show_current_log_short() {
        let cli = parse_legacy("-g 10").unwrap();
        assert_eq!(cli.show_current_log, Some(10));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_show_current_log_attached() {
        let cli = parse_legacy("-g10").unwrap();
        assert_eq!(cli.show_current_log, Some(10));
    }

    #[test]
    fn test_show_current_log_negative() {
        let cli = parse_legacy("-g -5").unwrap();
        assert_eq!(cli.show_current_log, Some(-5));
    }

    #[test]
    fn test_show_log_short() {
        let cli = parse_legacy("-G other 10").unwrap();
        assert_eq!(cli.show_log, Some(("other".to_string(), 10)));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_show_log_long() {
        let cli = parse_legacy("--show-log other 10").unwrap();
        assert_eq!(cli.show_log, Some(("other".to_string(), 10)));
    }

    // === Inspect tests ===

    #[test]
    fn test_inspect_current_system_prompt() {
        let cli = parse_legacy("-n system_prompt").unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::SystemPrompt));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_inspect_current_reflection() {
        let cli = parse_legacy("-n reflection").unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Reflection));
    }

    #[test]
    fn test_inspect_current_todos() {
        let cli = parse_legacy("-n todos").unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Todos));
    }

    #[test]
    fn test_inspect_current_goals() {
        let cli = parse_legacy("-n goals").unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Goals));
    }

    #[test]
    fn test_inspect_current_list() {
        let cli = parse_legacy("-n list").unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::List));
    }

    #[test]
    fn test_inspect_current_invalid() {
        let result = parse_legacy("-n invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown inspectable"));
    }

    #[test]
    fn test_inspect_other() {
        let cli = parse_legacy("-N other todos").unwrap();
        assert_eq!(cli.inspect, Some(("other".to_string(), Inspectable::Todos)));
        assert!(cli.no_chibi); // implied
    }

    // === Set system prompt tests ===

    #[test]
    fn test_set_current_system_prompt_short() {
        let cli = parse_legacy("-y prompt.md").unwrap();
        assert_eq!(cli.set_current_system_prompt, Some("prompt.md".to_string()));
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_set_system_prompt_short() {
        let cli = parse_legacy("-Y other prompt.md").unwrap();
        assert_eq!(cli.set_system_prompt, Some(("other".to_string(), "prompt.md".to_string())));
        assert!(cli.no_chibi); // implied (other context)
    }

    // === Username tests ===

    #[test]
    fn test_set_username_short() {
        let cli = parse_legacy("-u alice").unwrap();
        assert_eq!(cli.set_username, Some("alice".to_string()));
    }

    #[test]
    fn test_set_username_attached() {
        let cli = parse_legacy("-ualice").unwrap();
        assert_eq!(cli.set_username, Some("alice".to_string()));
    }

    #[test]
    fn test_transient_username_short() {
        let cli = parse_legacy("-U bob").unwrap();
        assert_eq!(cli.transient_username, Some("bob".to_string()));
    }

    // === Plugin/tool tests ===

    #[test]
    fn test_plugin_short() {
        let cli = parse_legacy("-p myplugin").unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert!(invocation.args.is_empty());
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_plugin_with_args() {
        let cli = parse_legacy("-p myplugin list --all").unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["list", "--all"]);
    }

    #[test]
    fn test_call_tool_short() {
        let cli = parse_legacy("-P update_todos").unwrap();
        let invocation = cli.call_tool.unwrap();
        assert_eq!(invocation.name, "update_todos");
        assert!(invocation.args.is_empty());
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_call_tool_with_args() {
        let cli = parse_legacy("-P update_todos arg1 arg2").unwrap();
        let invocation = cli.call_tool.unwrap();
        assert_eq!(invocation.name, "update_todos");
        assert_eq!(invocation.args, vec!["arg1", "arg2"]);
    }

    // === Verbose and control flags ===

    #[test]
    fn test_verbose_short() {
        let cli = parse_legacy("-v").unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_verbose_long() {
        let cli = parse_legacy("--verbose").unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_no_chibi_explicit() {
        let cli = parse_legacy("-x").unwrap();
        assert!(cli.no_chibi);
    }

    #[test]
    fn test_force_chibi_short() {
        let cli = parse_legacy("-X").unwrap();
        assert!(cli.force_chibi);
        assert!(!cli.no_chibi);
    }

    #[test]
    fn test_force_chibi_overrides_implied() {
        // -L implies no_chibi, but -X overrides it
        let cli = parse_legacy("-X -L").unwrap();
        assert!(cli.list_contexts);
        assert!(cli.force_chibi);
        assert!(!cli.no_chibi); // force_chibi wins
    }

    #[test]
    fn test_force_chibi_overrides_explicit_no_chibi() {
        let cli = parse_legacy("-x -X").unwrap();
        assert!(!cli.no_chibi); // force_chibi wins
        assert!(cli.force_chibi);
    }

    // === Version ===

    #[test]
    fn test_unknown_short_v_flag_is_prompt() {
        // -V is treated as positional prompt (no longer an error)
        // This is more permissive - users can ask "what does -V mean?"
        let cli = parse_legacy("-V").unwrap();
        assert_eq!(cli.prompt, vec!["-V"]);
    }

    // === Double dash handling ===

    #[test]
    fn test_double_dash_forces_prompt() {
        let cli = parse_legacy("-- -this -looks -like -flags").unwrap();
        assert_eq!(cli.prompt, vec!["-this", "-looks", "-like", "-flags"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn test_prompt_after_options() {
        let cli = parse_legacy("-v hello world").unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Error cases ===

    #[test]
    fn test_switch_context_missing_arg() {
        let result = parse_legacy("-c");
        assert!(result.is_err());
    }

    #[test]
    fn test_rename_context_missing_args() {
        let result = parse_legacy("-R old");
        assert!(result.is_err());
    }

    #[test]
    fn test_show_current_log_invalid_number() {
        let result = parse_legacy("-g abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_long_option_is_prompt() {
        // Unknown long options are treated as positional prompt
        // This is more permissive - users can ask "what does --unknown mean?"
        let cli = parse_legacy("--unknown").unwrap();
        assert_eq!(cli.prompt, vec!["--unknown"]);
    }

    // === should_invoke_llm tests ===

    #[test]
    fn test_should_invoke_llm_default() {
        let cli = parse_legacy("hello").unwrap();
        assert!(cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_explicit_no() {
        let cli = parse_legacy("-x hello").unwrap();
        assert!(!cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_implied_no() {
        let cli = parse_legacy("-L").unwrap();
        assert!(!cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_force_yes() {
        let cli = parse_legacy("-X -L").unwrap();
        assert!(cli.should_invoke_llm());
    }

    // === New context syntax ===

    #[test]
    fn test_switch_context_new() {
        let cli = parse_legacy("-c new").unwrap();
        assert_eq!(cli.switch_context, Some("new".to_string()));
    }

    #[test]
    fn test_switch_context_new_with_prefix() {
        let cli = parse_legacy("-c new:myproject").unwrap();
        assert_eq!(cli.switch_context, Some("new:myproject".to_string()));
    }

    // === Complex combinations ===

    #[test]
    fn test_multiple_non_exclusive_options() {
        let cli = parse_legacy("-v -U test hello world").unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.transient_username, Some("test".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    #[test]
    fn test_transient_context_with_verbose_and_prompt() {
        let cli = parse_legacy("-C agent -v run task").unwrap();
        assert_eq!(cli.transient_context, Some("agent".to_string()));
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["run", "task"]);
    }

    // === Prompt parsing behavior ===

    #[test]
    fn cli_bare_word_is_prompt() {
        let cli = parse_legacy("hello").unwrap();
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn cli_multiple_words_are_prompt() {
        let cli = parse_legacy("explain this error").unwrap();
        assert_eq!(cli.prompt, vec!["explain", "this", "error"]);
    }

    #[test]
    fn cli_quoted_prompt_style() {
        let a = vec![
            "chibi".to_string(),
            "add a users table to this schema".to_string(),
        ];
        let cli = Cli::parse_from_args(&a).unwrap().to_legacy().unwrap();
        assert_eq!(cli.prompt, vec!["add a users table to this schema"]);
    }

    #[test]
    fn cli_no_subcommand_pattern() {
        let cli = parse_legacy("list").unwrap();
        assert_eq!(cli.prompt, vec!["list"]);
        assert!(!cli.list_contexts);
    }

    #[test]
    fn cli_help_word_is_prompt() {
        let cli = parse_legacy("help me understand rust").unwrap();
        assert_eq!(cli.prompt, vec!["help", "me", "understand", "rust"]);
    }

    #[test]
    fn cli_version_word_is_prompt() {
        let cli = parse_legacy("version").unwrap();
        assert_eq!(cli.prompt, vec!["version"]);
    }

    #[test]
    fn cli_flags_after_prompt_are_prompt() {
        let cli = parse_legacy("hello -v world").unwrap();
        assert_eq!(cli.prompt, vec!["hello", "-v", "world"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn cli_empty_prompt_allowed() {
        let cli = parse_legacy("").unwrap();
        assert!(cli.prompt.is_empty());
    }

    #[test]
    fn cli_options_only_no_prompt() {
        let cli = parse_legacy("-v").unwrap();
        assert!(cli.verbose);
        assert!(cli.prompt.is_empty());
    }

    // === Plugin captures everything after ===

    #[test]
    fn test_plugin_captures_trailing_flags() {
        let cli = parse_legacy("-p myplugin -l --verbose").unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["-l", "--verbose"]);
    }

    #[test]
    fn test_plugin_verbose_before() {
        let cli = parse_legacy("-v -p myplugin arg1").unwrap();
        assert!(cli.verbose);
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["arg1"]);
    }
}
