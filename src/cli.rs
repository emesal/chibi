use std::io::{self, ErrorKind};

/// Direct plugin invocation from CLI
#[derive(Debug)]
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

#[derive(Debug)]
pub struct Cli {
    // Context operations (lowercase = current, uppercase = specified)
    pub switch_context: Option<String>,       // -c / --switch-context
    pub transient_context: Option<String>,    // -C / --transient-context
    pub list_current_context: bool,           // -l / --list-current-context
    pub list_contexts: bool,                  // -L / --list-contexts
    pub delete_current_context: bool,         // -d / --delete-current-context
    pub delete_context: Option<String>,       // -D / --delete-context
    pub archive_current_history: bool,        // -a / --archive-current-history
    pub archive_history: Option<String>,      // -A / --archive-history
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

impl Cli {
    pub fn parse() -> io::Result<Self> {
        let args: Vec<String> = std::env::args().collect();
        Self::parse_from(&args)
    }

    /// Try to extract an attached argument from a short flag.
    /// E.g., "-Dname" returns Some("name"), "-D" returns None.
    fn extract_attached_arg(arg: &str, flag_char: char) -> Option<String> {
        if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
            if arg.chars().nth(1) == Some(flag_char) {
                return Some(arg[2..].to_string());
            }
        }
        None
    }

    /// Parse a single-arg flag that supports attached args (-Dname or -D name)
    fn parse_single_arg_flag(
        args: &[String],
        i: &mut usize,
        short: char,
        long: &str,
    ) -> io::Result<Option<String>> {
        let arg = &args[*i];

        // Check for attached short form first (e.g., -Dname)
        if let Some(attached) = Self::extract_attached_arg(arg, short) {
            *i += 1;
            return Ok(Some(attached));
        }

        // Check for separated short form (-D name) or long form (--delete-context name)
        let short_flag = format!("-{}", short);
        if arg == &short_flag || arg == long {
            if *i + 1 >= args.len() {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("{} requires an argument", arg),
                ));
            }
            let value = args[*i + 1].clone();
            *i += 2;
            return Ok(Some(value));
        }

        Ok(None)
    }

    /// Parse CLI arguments from a slice (testable version)
    pub fn parse_from(args: &[String]) -> io::Result<Self> {
        let mut cli = Cli {
            switch_context: None,
            transient_context: None,
            list_current_context: false,
            list_contexts: false,
            delete_current_context: false,
            delete_context: None,
            archive_current_history: false,
            archive_history: None,
            compact_current_context: false,
            compact_context: None,
            rename_current_context: None,
            rename_context: None,
            show_current_log: None,
            show_log: None,
            inspect_current: None,
            inspect: None,
            set_current_system_prompt: None,
            set_system_prompt: None,
            set_username: None,
            transient_username: None,
            plugin: None,
            call_tool: None,
            verbose: false,
            no_chibi: false,
            force_chibi: false,
            prompt: Vec::new(),
        };

        let mut i = 1;
        let mut is_prompt = false;

        while i < args.len() {
            let arg = &args[i];

            if is_prompt {
                cli.prompt.push(arg.clone());
                i += 1;
                continue;
            }

            if arg == "--" {
                is_prompt = true;
                i += 1;
                continue;
            }

            // -c / --switch-context <NAME>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'c', "--switch-context")? {
                cli.switch_context = Some(val);
                continue;
            }

            // -C / --transient-context <NAME>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'C', "--transient-context")? {
                cli.transient_context = Some(val);
                continue;
            }

            // -l / --list-current-context
            if arg == "-l" || arg == "--list-current-context" {
                cli.list_current_context = true;
                i += 1;
                continue;
            }

            // -L / --list-contexts
            if arg == "-L" || arg == "--list-contexts" {
                cli.list_contexts = true;
                i += 1;
                continue;
            }

            // -d / --delete-current-context
            if arg == "-d" || arg == "--delete-current-context" {
                cli.delete_current_context = true;
                i += 1;
                continue;
            }

            // -D / --delete-context <CONTEXT>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'D', "--delete-context")? {
                cli.delete_context = Some(val);
                continue;
            }

            // -a / --archive-current-history
            if arg == "-a" || arg == "--archive-current-history" {
                cli.archive_current_history = true;
                i += 1;
                continue;
            }

            // -A / --archive-history <CONTEXT>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'A', "--archive-history")? {
                cli.archive_history = Some(val);
                continue;
            }

            // -z / --compact-current-context
            if arg == "-z" || arg == "--compact-current-context" {
                cli.compact_current_context = true;
                i += 1;
                continue;
            }

            // -Z / --compact-context <CONTEXT>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'Z', "--compact-context")? {
                cli.compact_context = Some(val);
                continue;
            }

            // -r / --rename-current-context <NEW>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'r', "--rename-current-context")? {
                cli.rename_current_context = Some(val);
                continue;
            }

            // -R / --rename-context <OLD> <NEW>
            if arg == "-R" || arg == "--rename-context" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires two arguments: <OLD> <NEW>", arg),
                    ));
                }
                cli.rename_context = Some((args[i + 1].clone(), args[i + 2].clone()));
                i += 3;
                continue;
            }

            // -g / --show-current-log <N>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'g', "--show-current-log")? {
                let n: isize = val.parse().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", val))
                })?;
                cli.show_current_log = Some(n);
                continue;
            }

            // -G / --show-log <CTX> <N>
            if arg == "-G" || arg == "--show-log" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires two arguments: <CONTEXT> <N>", arg),
                    ));
                }
                let ctx = args[i + 1].clone();
                let n: isize = args[i + 2].parse().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", args[i + 2]))
                })?;
                cli.show_log = Some((ctx, n));
                i += 3;
                continue;
            }

            // -n / --inspect-current <THING>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'n', "--inspect-current")? {
                let thing = Inspectable::from_str(&val).ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("Unknown inspectable: {}. Valid options: {:?}", val, Inspectable::all_names()),
                    )
                })?;
                cli.inspect_current = Some(thing);
                continue;
            }

            // -N / --inspect <CTX> <THING>
            if arg == "-N" || arg == "--inspect" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires two arguments: <CONTEXT> <THING>", arg),
                    ));
                }
                let ctx = args[i + 1].clone();
                let thing_str = &args[i + 2];
                let thing = Inspectable::from_str(thing_str).ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("Unknown inspectable: {}. Valid options: {:?}", thing_str, Inspectable::all_names()),
                    )
                })?;
                cli.inspect = Some((ctx, thing));
                i += 3;
                continue;
            }

            // -y / --set-current-system-prompt <PROMPT>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'y', "--set-current-system-prompt")? {
                cli.set_current_system_prompt = Some(val);
                continue;
            }

            // -Y / --set-system-prompt <CTX> <PROMPT>
            if arg == "-Y" || arg == "--set-system-prompt" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires two arguments: <CONTEXT> <PROMPT>", arg),
                    ));
                }
                cli.set_system_prompt = Some((args[i + 1].clone(), args[i + 2].clone()));
                i += 3;
                continue;
            }

            // -u / --set-username <NAME>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'u', "--set-username")? {
                cli.set_username = Some(val);
                continue;
            }

            // -U / --transient-username <NAME>
            if let Some(val) = Self::parse_single_arg_flag(args, &mut i, 'U', "--transient-username")? {
                cli.transient_username = Some(val);
                continue;
            }

            // -p / --plugin <NAME> [ARGS...]
            if arg == "-p" || arg == "--plugin" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires a plugin name", arg),
                    ));
                }
                let name = args[i + 1].clone();
                let plugin_args: Vec<String> = args[i + 2..].to_vec();
                cli.plugin = Some(PluginInvocation { name, args: plugin_args });
                break;
            }

            // -P / --call-tool <TOOL> [ARGS...]
            if arg == "-P" || arg == "--call-tool" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires a tool name", arg),
                    ));
                }
                let name = args[i + 1].clone();
                let tool_args: Vec<String> = args[i + 2..].to_vec();
                cli.call_tool = Some(PluginInvocation { name, args: tool_args });
                break;
            }

            // -v / --verbose
            if arg == "-v" || arg == "--verbose" {
                cli.verbose = true;
                i += 1;
                continue;
            }

            // -x / --no-chibi
            if arg == "-x" || arg == "--no-chibi" {
                cli.no_chibi = true;
                i += 1;
                continue;
            }

            // -X / --force-chibi
            if arg == "-X" || arg == "--force-chibi" {
                cli.force_chibi = true;
                i += 1;
                continue;
            }

            // -h / --help
            if arg == "-h" || arg == "--help" {
                Self::print_help();
                std::process::exit(0);
            }

            // --version (no short form)
            if arg == "--version" {
                println!("chibi {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }

            // Check if it starts with a dash (unknown option)
            if arg.starts_with('-') {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unknown option: {}", arg),
                ));
            }

            // This is the start of the prompt
            is_prompt = true;
            cli.prompt.push(arg.clone());
            i += 1;
        }

        // Compute implied no_chibi based on flags
        // These operations imply --no-chibi (output-producing or other-context ops)
        let implies_no_chibi = cli.list_current_context
            || cli.list_contexts
            || cli.delete_current_context
            || cli.delete_context.is_some()
            || cli.archive_history.is_some()
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

        // Validate: can't use both -x and -X
        if cli.no_chibi && cli.force_chibi {
            // force_chibi wins - it's explicitly requesting LLM invocation
            cli.no_chibi = false;
        }

        Ok(cli)
    }

    /// Check if this CLI invocation should invoke the LLM
    pub fn should_invoke_llm(&self) -> bool {
        !self.no_chibi
    }

    pub fn print_help() {
        println!("chibi - A CLI tool for chatting with AI via OpenRouter");
        println!();
        println!("Usage:");
        println!("  chibi [OPTIONS] [PROMPT]");
        println!();
        println!("Context Operations (lowercase = current, UPPERCASE = specified):");
        println!("  -c, --switch-context <NAME>     Switch to context (persistent)");
        println!("                                  Use 'new' for auto-generated name");
        println!("                                  Use 'new:prefix' for prefixed name");
        println!("  -C, --transient-context <NAME>  Use context for this invocation only");
        println!("  -l, --list-current-context      Show current context info");
        println!("  -L, --list-contexts             List all contexts");
        println!("  -d, --delete-current-context    Delete current context");
        println!("  -D, --delete-context <CTX>      Delete specified context");
        println!("  -a, --archive-current-history   Archive current context history");
        println!("  -A, --archive-history <CTX>     Archive specified context history");
        println!("  -z, --compact-current-context   Compact current context");
        println!("  -Z, --compact-context <CTX>     Compact specified context");
        println!("  -r, --rename-current-context <NEW>    Rename current context");
        println!("  -R, --rename-context <OLD> <NEW>      Rename specified context");
        println!("  -g, --show-current-log <N>      Show last N log entries (current)");
        println!("  -G, --show-log <CTX> <N>        Show last N log entries (specified)");
        println!("  -n, --inspect-current <THING>   Inspect current context");
        println!("  -N, --inspect <CTX> <THING>     Inspect specified context");
        println!("  -y, --set-current-system-prompt <PROMPT>  Set system prompt (current)");
        println!("  -Y, --set-system-prompt <CTX> <PROMPT>    Set system prompt (specified)");
        println!();
        println!("Inspectable things: system_prompt, reflection, todos, goals, list");
        println!();
        println!("Username Options:");
        println!("  -u, --set-username <NAME>       Set username (persists to local.toml)");
        println!("  -U, --transient-username <NAME> Set username for this invocation only");
        println!();
        println!("Plugin/Tool Options:");
        println!("  -p, --plugin <NAME> [ARGS...]   Run a plugin directly");
        println!("  -P, --call-tool <TOOL> [ARGS...] Call a tool directly");
        println!();
        println!("Control Flags:");
        println!("  -v, --verbose                   Show extra info (tools loaded, etc.)");
        println!("  -x, --no-chibi                  Don't invoke the LLM");
        println!("  -X, --force-chibi               Force LLM invocation (overrides implied -x)");
        println!("  -h, --help                      Show this help");
        println!("      --version                   Show version");
        println!();
        println!("Flag Behavior:");
        println!("  Some flags imply --no-chibi (operations that produce output or");
        println!("  operate on other contexts). Use -X to override and invoke LLM after.");
        println!();
        println!("  Implied --no-chibi: -l, -L, -d, -D, -A, -Z, -R, -g, -G, -n, -N, -Y, -p, -P");
        println!("  Combinable with prompt: -c, -C, -a, -z, -r, -y, -u, -U, -v");
        println!();
        println!("Prompt Input:");
        println!("  Arguments after options are joined as the prompt.");
        println!("  Use -- to force remaining args as prompt (e.g., chibi -- -starts-with-dash)");
        println!("  No arguments: read from stdin (end with . on empty line)");
        println!("  Piped input: echo 'text' | chibi");
        println!();
        println!("Examples:");
        println!("  chibi What is Rust?             Send prompt to LLM");
        println!("  chibi -c coding write code      Switch context, then send prompt");
        println!("  chibi -L                        List all contexts");
        println!("  chibi -l                        Show current context info");
        println!("  chibi -Dold                     Delete 'old' context (attached arg)");
        println!("  chibi -D old                    Delete 'old' context (separated arg)");
        println!("  chibi -n system_prompt          Inspect current system prompt");
        println!("  chibi -g 10                     Show last 10 log entries");
        println!("  chibi -x -c test                Switch context without LLM");
        println!("  chibi -X -L                     List contexts then invoke LLM");
        println!("  chibi -a hello                  Archive history, then send prompt");
    }
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

    // === Basic flag tests ===

    #[test]
    fn test_no_args() {
        let cli = Cli::parse_from(&args("")).unwrap();
        assert!(cli.prompt.is_empty());
        assert!(!cli.verbose);
        assert!(!cli.list_contexts);
    }

    #[test]
    fn test_simple_prompt() {
        let cli = Cli::parse_from(&args("hello world")).unwrap();
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Context switch tests ===

    #[test]
    fn test_switch_context_short() {
        let cli = Cli::parse_from(&args("-c coding")).unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
        assert!(!cli.no_chibi); // combinable, not implied
    }

    #[test]
    fn test_switch_context_long() {
        let cli = Cli::parse_from(&args("--switch-context coding")).unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
    }

    #[test]
    fn test_switch_context_attached() {
        let cli = Cli::parse_from(&args("-ccoding")).unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
    }

    #[test]
    fn test_switch_context_with_prompt() {
        let cli = Cli::parse_from(&args("-c coding hello world")).unwrap();
        assert_eq!(cli.switch_context, Some("coding".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Transient context tests ===

    #[test]
    fn test_transient_context_short() {
        let cli = Cli::parse_from(&args("-C temp")).unwrap();
        assert_eq!(cli.transient_context, Some("temp".to_string()));
    }

    #[test]
    fn test_transient_context_long() {
        let cli = Cli::parse_from(&args("--transient-context temp")).unwrap();
        assert_eq!(cli.transient_context, Some("temp".to_string()));
    }

    #[test]
    fn test_transient_context_with_prompt() {
        let cli = Cli::parse_from(&args("-C agent run task")).unwrap();
        assert_eq!(cli.transient_context, Some("agent".to_string()));
        assert_eq!(cli.prompt, vec!["run", "task"]);
    }

    // === List tests ===

    #[test]
    fn test_list_current_context_short() {
        let cli = Cli::parse_from(&args("-l")).unwrap();
        assert!(cli.list_current_context);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_list_current_context_long() {
        let cli = Cli::parse_from(&args("--list-current-context")).unwrap();
        assert!(cli.list_current_context);
    }

    #[test]
    fn test_list_contexts_short() {
        let cli = Cli::parse_from(&args("-L")).unwrap();
        assert!(cli.list_contexts);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_list_contexts_long() {
        let cli = Cli::parse_from(&args("--list-contexts")).unwrap();
        assert!(cli.list_contexts);
    }

    // === Delete tests ===

    #[test]
    fn test_delete_current_context_short() {
        let cli = Cli::parse_from(&args("-d")).unwrap();
        assert!(cli.delete_current_context);
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_delete_current_context_long() {
        let cli = Cli::parse_from(&args("--delete-current-context")).unwrap();
        assert!(cli.delete_current_context);
    }

    #[test]
    fn test_delete_context_short() {
        let cli = Cli::parse_from(&args("-D old-context")).unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_delete_context_attached() {
        let cli = Cli::parse_from(&args("-Dold-context")).unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
    }

    #[test]
    fn test_delete_context_long() {
        let cli = Cli::parse_from(&args("--delete-context old-context")).unwrap();
        assert_eq!(cli.delete_context, Some("old-context".to_string()));
    }

    // === Archive tests ===

    #[test]
    fn test_archive_current_history_short() {
        let cli = Cli::parse_from(&args("-a")).unwrap();
        assert!(cli.archive_current_history);
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_archive_current_history_with_prompt() {
        let cli = Cli::parse_from(&args("-a hello")).unwrap();
        assert!(cli.archive_current_history);
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn test_archive_history_short() {
        let cli = Cli::parse_from(&args("-A other")).unwrap();
        assert_eq!(cli.archive_history, Some("other".to_string()));
        assert!(cli.no_chibi); // implied (operates on other context)
    }

    #[test]
    fn test_archive_history_attached() {
        let cli = Cli::parse_from(&args("-Aother")).unwrap();
        assert_eq!(cli.archive_history, Some("other".to_string()));
    }

    // === Compact tests ===

    #[test]
    fn test_compact_current_context_short() {
        let cli = Cli::parse_from(&args("-z")).unwrap();
        assert!(cli.compact_current_context);
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_compact_context_short() {
        let cli = Cli::parse_from(&args("-Z other")).unwrap();
        assert_eq!(cli.compact_context, Some("other".to_string()));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_compact_context_attached() {
        let cli = Cli::parse_from(&args("-Zother")).unwrap();
        assert_eq!(cli.compact_context, Some("other".to_string()));
    }

    // === Rename tests ===

    #[test]
    fn test_rename_current_context_short() {
        let cli = Cli::parse_from(&args("-r newname")).unwrap();
        assert_eq!(cli.rename_current_context, Some("newname".to_string()));
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_rename_current_context_attached() {
        let cli = Cli::parse_from(&args("-rnewname")).unwrap();
        assert_eq!(cli.rename_current_context, Some("newname".to_string()));
    }

    #[test]
    fn test_rename_context_short() {
        let cli = Cli::parse_from(&args("-R old new")).unwrap();
        assert_eq!(cli.rename_context, Some(("old".to_string(), "new".to_string())));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_rename_context_long() {
        let cli = Cli::parse_from(&args("--rename-context old new")).unwrap();
        assert_eq!(cli.rename_context, Some(("old".to_string(), "new".to_string())));
    }

    // === Log/history tests ===

    #[test]
    fn test_show_current_log_short() {
        let cli = Cli::parse_from(&args("-g 10")).unwrap();
        assert_eq!(cli.show_current_log, Some(10));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_show_current_log_attached() {
        let cli = Cli::parse_from(&args("-g10")).unwrap();
        assert_eq!(cli.show_current_log, Some(10));
    }

    #[test]
    fn test_show_current_log_negative() {
        let cli = Cli::parse_from(&args("-g -5")).unwrap();
        assert_eq!(cli.show_current_log, Some(-5));
    }

    #[test]
    fn test_show_log_short() {
        let cli = Cli::parse_from(&args("-G other 10")).unwrap();
        assert_eq!(cli.show_log, Some(("other".to_string(), 10)));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_show_log_long() {
        let cli = Cli::parse_from(&args("--show-log other 10")).unwrap();
        assert_eq!(cli.show_log, Some(("other".to_string(), 10)));
    }

    // === Inspect tests ===

    #[test]
    fn test_inspect_current_system_prompt() {
        let cli = Cli::parse_from(&args("-n system_prompt")).unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::SystemPrompt));
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_inspect_current_reflection() {
        let cli = Cli::parse_from(&args("-n reflection")).unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Reflection));
    }

    #[test]
    fn test_inspect_current_todos() {
        let cli = Cli::parse_from(&args("-n todos")).unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Todos));
    }

    #[test]
    fn test_inspect_current_goals() {
        let cli = Cli::parse_from(&args("-n goals")).unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::Goals));
    }

    #[test]
    fn test_inspect_current_list() {
        let cli = Cli::parse_from(&args("-n list")).unwrap();
        assert_eq!(cli.inspect_current, Some(Inspectable::List));
    }

    #[test]
    fn test_inspect_current_invalid() {
        let result = Cli::parse_from(&args("-n invalid"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown inspectable"));
    }

    #[test]
    fn test_inspect_other() {
        let cli = Cli::parse_from(&args("-N other todos")).unwrap();
        assert_eq!(cli.inspect, Some(("other".to_string(), Inspectable::Todos)));
        assert!(cli.no_chibi); // implied
    }

    // === Set system prompt tests ===

    #[test]
    fn test_set_current_system_prompt_short() {
        let cli = Cli::parse_from(&args("-y prompt.md")).unwrap();
        assert_eq!(cli.set_current_system_prompt, Some("prompt.md".to_string()));
        assert!(!cli.no_chibi); // combinable
    }

    #[test]
    fn test_set_system_prompt_short() {
        let cli = Cli::parse_from(&args("-Y other prompt.md")).unwrap();
        assert_eq!(cli.set_system_prompt, Some(("other".to_string(), "prompt.md".to_string())));
        assert!(cli.no_chibi); // implied (other context)
    }

    // === Username tests ===

    #[test]
    fn test_set_username_short() {
        let cli = Cli::parse_from(&args("-u alice")).unwrap();
        assert_eq!(cli.set_username, Some("alice".to_string()));
    }

    #[test]
    fn test_set_username_attached() {
        let cli = Cli::parse_from(&args("-ualice")).unwrap();
        assert_eq!(cli.set_username, Some("alice".to_string()));
    }

    #[test]
    fn test_transient_username_short() {
        let cli = Cli::parse_from(&args("-U bob")).unwrap();
        assert_eq!(cli.transient_username, Some("bob".to_string()));
    }

    // === Plugin/tool tests ===

    #[test]
    fn test_plugin_short() {
        let cli = Cli::parse_from(&args("-p myplugin")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert!(invocation.args.is_empty());
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_plugin_with_args() {
        let cli = Cli::parse_from(&args("-p myplugin list --all")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["list", "--all"]);
    }

    #[test]
    fn test_call_tool_short() {
        let cli = Cli::parse_from(&args("-P update_todos")).unwrap();
        let invocation = cli.call_tool.unwrap();
        assert_eq!(invocation.name, "update_todos");
        assert!(invocation.args.is_empty());
        assert!(cli.no_chibi); // implied
    }

    #[test]
    fn test_call_tool_with_args() {
        let cli = Cli::parse_from(&args("-P update_todos arg1 arg2")).unwrap();
        let invocation = cli.call_tool.unwrap();
        assert_eq!(invocation.name, "update_todos");
        assert_eq!(invocation.args, vec!["arg1", "arg2"]);
    }

    // === Verbose and control flags ===

    #[test]
    fn test_verbose_short() {
        let cli = Cli::parse_from(&args("-v")).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_verbose_long() {
        let cli = Cli::parse_from(&args("--verbose")).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_no_chibi_explicit() {
        let cli = Cli::parse_from(&args("-x")).unwrap();
        assert!(cli.no_chibi);
    }

    #[test]
    fn test_force_chibi_short() {
        let cli = Cli::parse_from(&args("-X")).unwrap();
        assert!(cli.force_chibi);
        assert!(!cli.no_chibi);
    }

    #[test]
    fn test_force_chibi_overrides_implied() {
        // -L implies no_chibi, but -X overrides it
        let cli = Cli::parse_from(&args("-X -L")).unwrap();
        assert!(cli.list_contexts);
        assert!(cli.force_chibi);
        assert!(!cli.no_chibi); // force_chibi wins
    }

    #[test]
    fn test_force_chibi_overrides_explicit_no_chibi() {
        let cli = Cli::parse_from(&args("-x -X")).unwrap();
        assert!(!cli.no_chibi); // force_chibi wins
        assert!(cli.force_chibi);
    }

    // === Version ===

    #[test]
    fn test_unknown_short_v_flag() {
        // -V is no longer valid
        let result = Cli::parse_from(&args("-V"));
        assert!(result.is_err());
    }

    // === Double dash handling ===

    #[test]
    fn test_double_dash_forces_prompt() {
        let cli = Cli::parse_from(&args("-- -this -looks -like -flags")).unwrap();
        assert_eq!(cli.prompt, vec!["-this", "-looks", "-like", "-flags"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn test_prompt_after_options() {
        let cli = Cli::parse_from(&args("-v hello world")).unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    // === Error cases ===

    #[test]
    fn test_switch_context_missing_arg() {
        let result = Cli::parse_from(&args("-c"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("requires an argument"));
    }

    #[test]
    fn test_rename_context_missing_args() {
        let result = Cli::parse_from(&args("-R old"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("requires two arguments"));
    }

    #[test]
    fn test_show_current_log_invalid_number() {
        let result = Cli::parse_from(&args("-g abc"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid number"));
    }

    #[test]
    fn test_unknown_option() {
        let result = Cli::parse_from(&args("--unknown"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown option"));
    }

    // === should_invoke_llm tests ===

    #[test]
    fn test_should_invoke_llm_default() {
        let cli = Cli::parse_from(&args("hello")).unwrap();
        assert!(cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_explicit_no() {
        let cli = Cli::parse_from(&args("-x hello")).unwrap();
        assert!(!cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_implied_no() {
        let cli = Cli::parse_from(&args("-L")).unwrap();
        assert!(!cli.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_force_yes() {
        let cli = Cli::parse_from(&args("-X -L")).unwrap();
        assert!(cli.should_invoke_llm());
    }

    // === New context syntax ===

    #[test]
    fn test_switch_context_new() {
        let cli = Cli::parse_from(&args("-c new")).unwrap();
        assert_eq!(cli.switch_context, Some("new".to_string()));
    }

    #[test]
    fn test_switch_context_new_with_prefix() {
        let cli = Cli::parse_from(&args("-c new:myproject")).unwrap();
        assert_eq!(cli.switch_context, Some("new:myproject".to_string()));
    }

    // === Complex combinations ===

    #[test]
    fn test_multiple_non_exclusive_options() {
        let cli = Cli::parse_from(&args("-v -U test hello world")).unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.transient_username, Some("test".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    #[test]
    fn test_transient_context_with_verbose_and_prompt() {
        let cli = Cli::parse_from(&args("-C agent -v run task")).unwrap();
        assert_eq!(cli.transient_context, Some("agent".to_string()));
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["run", "task"]);
    }

    // === Prompt parsing behavior ===

    #[test]
    fn cli_bare_word_is_prompt() {
        let cli = Cli::parse_from(&args("hello")).unwrap();
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn cli_multiple_words_are_prompt() {
        let cli = Cli::parse_from(&args("explain this error")).unwrap();
        assert_eq!(cli.prompt, vec!["explain", "this", "error"]);
    }

    #[test]
    fn cli_quoted_prompt_style() {
        let a = vec![
            "chibi".to_string(),
            "add a users table to this schema".to_string(),
        ];
        let cli = Cli::parse_from(&a).unwrap();
        assert_eq!(cli.prompt, vec!["add a users table to this schema"]);
    }

    #[test]
    fn cli_no_subcommand_pattern() {
        let cli = Cli::parse_from(&args("list")).unwrap();
        assert_eq!(cli.prompt, vec!["list"]);
        assert!(!cli.list_contexts);
    }

    #[test]
    fn cli_help_word_is_prompt() {
        let cli = Cli::parse_from(&args("help me understand rust")).unwrap();
        assert_eq!(cli.prompt, vec!["help", "me", "understand", "rust"]);
    }

    #[test]
    fn cli_version_word_is_prompt() {
        let cli = Cli::parse_from(&args("version")).unwrap();
        assert_eq!(cli.prompt, vec!["version"]);
    }

    #[test]
    fn cli_flags_after_prompt_are_prompt() {
        let cli = Cli::parse_from(&args("hello -v world")).unwrap();
        assert_eq!(cli.prompt, vec!["hello", "-v", "world"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn cli_empty_prompt_allowed() {
        let cli = Cli::parse_from(&args("")).unwrap();
        assert!(cli.prompt.is_empty());
    }

    #[test]
    fn cli_options_only_no_prompt() {
        let cli = Cli::parse_from(&args("-v")).unwrap();
        assert!(cli.verbose);
        assert!(cli.prompt.is_empty());
    }

    // === Plugin captures everything after ===

    #[test]
    fn test_plugin_captures_trailing_flags() {
        let cli = Cli::parse_from(&args("-p myplugin -l --verbose")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["-l", "--verbose"]);
    }

    #[test]
    fn test_plugin_verbose_before() {
        let cli = Cli::parse_from(&args("-v -p myplugin arg1")).unwrap();
        assert!(cli.verbose);
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["arg1"]);
    }
}
