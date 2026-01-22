//! CLI argument parsing with clap.
//!
//! This module handles parsing command-line arguments and converting them
//! to the unified `ChibiInput` format.

use crate::input::{ChibiInput, Command, ContextSelection, Flags, UsernameOverride};
use clap::Parser;
use std::io::{self, BufRead, ErrorKind, IsTerminal};

/// Direct plugin invocation from CLI
#[derive(Debug, Clone)]
pub struct PluginInvocation {
    pub name: String,
    pub args: Vec<String>,
}

/// Inspectable things via -n/-N
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    #[arg(
        short = 'c',
        long = "switch-context",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    pub switch_context: Option<String>,

    /// Use context for this invocation only
    #[arg(
        short = 'C',
        long = "transient-context",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
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
    #[arg(
        short = 'D',
        long = "delete-context",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub delete_context: Option<String>,

    /// Archive current context (clear history, save to transcript)
    #[arg(short = 'a', long = "archive-current-context")]
    pub archive_current_context: bool,

    /// Archive specified context
    #[arg(
        short = 'A',
        long = "archive-context",
        value_name = "CTX",
        allow_hyphen_values = true
    )]
    pub archive_context: Option<String>,

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

    /// Set username for this invocation only
    #[arg(
        short = 'U',
        long = "transient-username",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
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

    /// Convert to ChibiInput format
    pub fn to_input(&self) -> io::Result<ChibiInput> {
        // Validate inspect values
        let inspect_current = if let Some(ref s) = self.inspect_current {
            Some(Inspectable::from_str(s).ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "Unknown inspectable: {}. Valid options: {:?}",
                        s,
                        Inspectable::all_names()
                    ),
                )
            })?)
        } else {
            None
        };

        let inspect = if let Some(ref v) = self.inspect {
            if v.len() >= 2 {
                Some((
                    v[0].clone(),
                    Inspectable::from_str(&v[1]).ok_or_else(|| {
                        io::Error::new(
                            ErrorKind::InvalidInput,
                            format!(
                                "Unknown inspectable: {}. Valid options: {:?}",
                                v[1],
                                Inspectable::all_names()
                            ),
                        )
                    })?,
                ))
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

        // Parse rename_context tuple
        let rename_context = self.rename_context.as_ref().and_then(|v| {
            if v.len() >= 2 {
                Some((v[0].clone(), v[1].clone()))
            } else {
                None
            }
        });

        // Parse set_system_prompt tuple
        let set_system_prompt = self.set_system_prompt.as_ref().and_then(|v| {
            if v.len() >= 2 {
                Some((v[0].clone(), v[1].clone()))
            } else {
                None
            }
        });

        // Parse plugin invocation
        let plugin = self.plugin.as_ref().map(|v| PluginInvocation {
            name: v.first().cloned().unwrap_or_default(),
            args: v.get(1..).unwrap_or(&[]).to_vec(),
        });

        // Parse call_tool invocation
        let call_tool = self.call_tool.as_ref().map(|v| PluginInvocation {
            name: v.first().cloned().unwrap_or_default(),
            args: v.get(1..).unwrap_or(&[]).to_vec(),
        });

        // Compute implied no_chibi based on flags
        let implies_no_chibi = self.list_current_context
            || self.list_contexts
            || self.delete_current_context
            || self.delete_context.is_some()
            || self.archive_context.is_some()
            || self.compact_context.is_some()
            || rename_context.is_some()
            || self.show_current_log.is_some()
            || show_log.is_some()
            || inspect_current.is_some()
            || inspect.is_some()
            || set_system_prompt.is_some()
            || plugin.is_some()
            || call_tool.is_some();

        let mut no_chibi = self.no_chibi || implies_no_chibi;
        if self.force_chibi {
            no_chibi = false;
        }

        // Determine context selection
        let context = if let Some(ref name) = self.transient_context {
            ContextSelection::Transient { name: name.clone() }
        } else if let Some(ref name) = self.switch_context {
            ContextSelection::Switch {
                name: name.clone(),
                persistent: true,
            }
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
        let command = if !self.prompt.is_empty() && !no_chibi {
            Command::SendPrompt {
                prompt: self.prompt.join(" "),
            }
        } else if self.list_contexts {
            Command::ListContexts
        } else if self.list_current_context {
            Command::ListCurrentContext
        } else if self.delete_current_context {
            Command::DeleteContext { name: None }
        } else if let Some(ref name) = self.delete_context {
            Command::DeleteContext {
                name: Some(name.clone()),
            }
        } else if self.archive_current_context {
            Command::ArchiveContext { name: None }
        } else if let Some(ref name) = self.archive_context {
            Command::ArchiveContext {
                name: Some(name.clone()),
            }
        } else if self.compact_current_context {
            Command::CompactContext { name: None }
        } else if let Some(ref name) = self.compact_context {
            Command::CompactContext {
                name: Some(name.clone()),
            }
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
        } else {
            Command::NoOp
        };

        let flags = Flags {
            verbose: self.verbose,
            json_output: self.json_output,
            no_chibi,
        };

        Ok(ChibiInput {
            command,
            flags,
            context,
            username_override,
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
    const ATTACHED_FLAGS: &[char] = &['c', 'C', 'D', 'A', 'Z', 'r', 'g', 'n', 'y', 'u', 'U'];

    let mut result = Vec::new();

    for arg in args {
        // Check if this is a short flag with attached value (e.g., -Dname)
        if arg.len() > 2
            && arg.starts_with('-')
            && !arg.starts_with("--")
            && arg
                .chars()
                .nth(1)
                .map_or(false, |c| ATTACHED_FLAGS.contains(&c))
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
        println!("chibi {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    let mut input = cli.to_input()?;

    // Handle stdin prompt reading (CLI-specific behavior)
    // This happens when there's no command that produces output and we might need
    // to read from stdin or interactive input
    let should_read_prompt = !input.flags.no_chibi && matches!(input.command, Command::NoOp);

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

    /// Helper to create args from a command string
    fn args(s: &str) -> Vec<String> {
        std::iter::once("chibi".to_string())
            .chain(s.split_whitespace().map(|s| s.to_string()))
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
        assert!(!input.flags.no_chibi); // combinable, not implied
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
    fn test_switch_context_with_prompt() {
        let input = parse_input("-c coding hello world").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "coding")
        );
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "hello world")
        );
    }

    // === Transient context tests ===

    #[test]
    fn test_transient_context_short() {
        let input = parse_input("-C temp").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Transient { ref name } if name == "temp")
        );
    }

    #[test]
    fn test_transient_context_with_prompt() {
        let input = parse_input("-C agent run task").unwrap();
        assert!(
            matches!(input.context, ContextSelection::Transient { ref name } if name == "agent")
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
        assert!(input.flags.no_chibi); // implied
    }

    #[test]
    fn test_list_contexts_short() {
        let input = parse_input("-L").unwrap();
        assert!(matches!(input.command, Command::ListContexts));
        assert!(input.flags.no_chibi); // implied
    }

    // === Delete tests ===

    #[test]
    fn test_delete_current_context_short() {
        let input = parse_input("-d").unwrap();
        assert!(matches!(
            input.command,
            Command::DeleteContext { name: None }
        ));
        assert!(input.flags.no_chibi);
    }

    #[test]
    fn test_delete_context_short() {
        let input = parse_input("-D old-context").unwrap();
        assert!(
            matches!(input.command, Command::DeleteContext { ref name } if *name == Some("old-context".to_string()))
        );
        assert!(input.flags.no_chibi);
    }

    #[test]
    fn test_delete_context_attached() {
        let input = parse_input("-Dold-context").unwrap();
        assert!(
            matches!(input.command, Command::DeleteContext { ref name } if *name == Some("old-context".to_string()))
        );
    }

    // === Archive tests ===

    #[test]
    fn test_archive_current_context_short() {
        let input = parse_input("-a").unwrap();
        assert!(matches!(
            input.command,
            Command::ArchiveContext { name: None }
        ));
        assert!(!input.flags.no_chibi); // combinable
    }

    #[test]
    fn test_archive_context_short() {
        let input = parse_input("-A other").unwrap();
        assert!(
            matches!(input.command, Command::ArchiveContext { ref name } if *name == Some("other".to_string()))
        );
        assert!(input.flags.no_chibi);
    }

    // === Compact tests ===

    #[test]
    fn test_compact_current_context_short() {
        let input = parse_input("-z").unwrap();
        assert!(matches!(
            input.command,
            Command::CompactContext { name: None }
        ));
        assert!(!input.flags.no_chibi); // combinable
    }

    #[test]
    fn test_compact_context_short() {
        let input = parse_input("-Z other").unwrap();
        assert!(
            matches!(input.command, Command::CompactContext { ref name } if *name == Some("other".to_string()))
        );
        assert!(input.flags.no_chibi);
    }

    // === Rename tests ===

    #[test]
    fn test_rename_current_context_short() {
        let input = parse_input("-r newname").unwrap();
        assert!(
            matches!(input.command, Command::RenameContext { old: None, ref new } if new == "newname")
        );
        assert!(!input.flags.no_chibi); // combinable
    }

    #[test]
    fn test_rename_context_short() {
        let input = parse_input("-R old new").unwrap();
        assert!(
            matches!(input.command, Command::RenameContext { ref old, ref new } if *old == Some("old".to_string()) && new == "new")
        );
        assert!(input.flags.no_chibi);
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
        assert!(input.flags.no_chibi);
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
        assert!(input.flags.no_chibi);
    }

    // === Inspect tests ===

    #[test]
    fn test_inspect_current_system_prompt() {
        let input = parse_input("-n system_prompt").unwrap();
        assert!(
            matches!(input.command, Command::Inspect { context: None, ref thing } if *thing == Inspectable::SystemPrompt)
        );
        assert!(input.flags.no_chibi);
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
        assert!(input.flags.no_chibi);
    }

    // === Set system prompt tests ===

    #[test]
    fn test_set_current_system_prompt_short() {
        let input = parse_input("-y prompt.md").unwrap();
        assert!(
            matches!(input.command, Command::SetSystemPrompt { context: None, ref prompt } if prompt == "prompt.md")
        );
        assert!(!input.flags.no_chibi); // combinable
    }

    #[test]
    fn test_set_system_prompt_short() {
        let input = parse_input("-Y other prompt.md").unwrap();
        assert!(
            matches!(input.command, Command::SetSystemPrompt { ref context, ref prompt }
            if *context == Some("other".to_string()) && prompt == "prompt.md")
        );
        assert!(input.flags.no_chibi);
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
    fn test_transient_username_short() {
        let input = parse_input("-U bob").unwrap();
        assert!(
            matches!(input.username_override, Some(UsernameOverride::Transient(ref u)) if u == "bob")
        );
    }

    // === Plugin/tool tests ===

    #[test]
    fn test_plugin_short() {
        let input = parse_input("-p myplugin").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args.is_empty())
        );
        assert!(input.flags.no_chibi);
    }

    #[test]
    fn test_plugin_with_args() {
        let input = parse_input("-p myplugin list --all").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["list", "--all"])
        );
    }

    #[test]
    fn test_call_tool_short() {
        let input = parse_input("-P update_todos").unwrap();
        assert!(
            matches!(input.command, Command::CallTool { ref name, ref args }
            if name == "update_todos" && args.is_empty())
        );
        assert!(input.flags.no_chibi);
    }

    // === Verbose and control flags ===

    #[test]
    fn test_verbose_short() {
        let input = parse_input("-v").unwrap();
        assert!(input.flags.verbose);
    }

    #[test]
    fn test_no_chibi_explicit() {
        let input = parse_input("-x").unwrap();
        assert!(input.flags.no_chibi);
    }

    #[test]
    fn test_force_chibi_short() {
        let input = parse_input("-X").unwrap();
        // force_chibi is handled during parsing, not stored in flags
        assert!(!input.flags.no_chibi);
    }

    #[test]
    fn test_force_chibi_overrides_implied() {
        let input = parse_input("-X -L").unwrap();
        assert!(matches!(input.command, Command::ListContexts));
        // force_chibi overrides the implied no_chibi from -L
        assert!(!input.flags.no_chibi);
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
    fn test_plugin_captures_trailing_flags() {
        let input = parse_input("-p myplugin -l --verbose").unwrap();
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["-l", "--verbose"])
        );
    }

    #[test]
    fn test_plugin_verbose_before() {
        let input = parse_input("-v -p myplugin arg1").unwrap();
        assert!(input.flags.verbose);
        assert!(
            matches!(input.command, Command::RunPlugin { ref name, ref args }
            if name == "myplugin" && args == &["arg1"])
        );
    }
}
