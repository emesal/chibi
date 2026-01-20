use std::io::{self, ErrorKind};

/// Direct plugin invocation from CLI
#[derive(Debug)]
pub struct PluginInvocation {
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct Cli {
    pub switch: Option<String>,
    pub sub_context: Option<String>,
    pub list: bool,
    pub which: bool,
    pub delete: Option<String>,
    pub clear: bool,
    pub compact: bool,
    pub rename: Option<(String, String)>,
    pub history: bool,
    pub history_all: bool,
    pub num_messages: Option<isize>,
    pub verbose: bool,
    pub show_prompt: bool,
    pub set_prompt: Option<String>,
    pub no_reflection: bool,
    pub username: Option<String>,
    pub temp_username: Option<String>,
    pub plugin: Option<PluginInvocation>,
    pub prompt: Vec<String>,
}

impl Cli {
    pub fn parse() -> io::Result<Self> {
        let args: Vec<String> = std::env::args().collect();
        Self::parse_from(&args)
    }

    /// Parse CLI arguments from a slice (testable version)
    pub fn parse_from(args: &[String]) -> io::Result<Self> {
        let mut switch = None;
        let mut sub_context = None;
        let mut list = false;
        let mut which = false;
        let mut delete = None;
        let mut clear = false;
        let mut compact = false;
        let mut rename = None;
        let mut history = false;
        let mut history_all = false;
        let mut num_messages: Option<isize> = None;
        let mut verbose = false;
        let mut show_prompt = false;
        let mut set_prompt = None;
        let mut no_reflection = false;
        let mut username = None;
        let mut temp_username = None;
        let mut plugin = None;
        let mut prompt = Vec::new();
        let mut i = 1;
        let mut is_prompt = false;

        while i < args.len() {
            let arg = &args[i];

            if is_prompt {
                prompt.push(arg.clone());
                i += 1;
                continue;
            }

            if arg == "--" {
                is_prompt = true;
                i += 1;
                continue;
            }

            if arg == "-s" || arg == "--switch" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                switch = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-S" || arg == "--sub-context" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                sub_context = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-l" || arg == "--list" {
                list = true;
                i += 1;
                continue;
            }

            if arg == "-w" || arg == "--which" {
                which = true;
                i += 1;
                continue;
            }

            if arg == "-d" || arg == "--delete" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                delete = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-C" || arg == "--clear" {
                clear = true;
                i += 1;
                continue;
            }

            if arg == "-c" || arg == "--compact" {
                compact = true;
                i += 1;
                continue;
            }

            if arg == "-r" || arg == "--rename" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires two arguments", arg),
                    ));
                }
                rename = Some((args[i + 1].clone(), args[i + 2].clone()));
                i += 3;
                continue;
            }

            if arg == "-H" || arg == "--history" {
                history = true;
                i += 1;
                continue;
            }

            if arg == "-L" || arg == "--history-all" {
                history_all = true;
                i += 1;
                continue;
            }

            if arg == "-n" || arg == "--num-messages" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                num_messages = Some(args[i + 1].parse().map_err(|_| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("Invalid number: {}", args[i + 1]),
                    )
                })?);
                i += 2;
                continue;
            }

            if arg == "-p" || arg == "--prompt" {
                show_prompt = true;
                i += 1;
                continue;
            }

            if arg == "-e" || arg == "--set-prompt" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                set_prompt = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-v" || arg == "--verbose" {
                verbose = true;
                i += 1;
                continue;
            }

            if arg == "-x" || arg == "--no-reflection" {
                no_reflection = true;
                i += 1;
                continue;
            }

            if arg == "-u" || arg == "--username" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                username = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-U" || arg == "--temp-username" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires an argument", arg),
                    ));
                }
                temp_username = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-P" || arg == "--plugin" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} requires a plugin name", arg),
                    ));
                }
                let name = args[i + 1].clone();
                // Everything after the plugin name becomes plugin args
                let plugin_args: Vec<String> = args[i + 2..].to_vec();
                plugin = Some(PluginInvocation {
                    name,
                    args: plugin_args,
                });
                // We've consumed all remaining args
                break;
            }

            if arg == "-h" || arg == "--help" {
                Self::print_help();
                std::process::exit(0);
            }

            if arg == "-V" || arg == "--version" {
                println!("chibi {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }

            // Check if it starts with a dash
            if arg.starts_with('-') {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unknown option: {}", arg),
                ));
            }

            // This is the start of the prompt
            is_prompt = true;
            prompt.push(arg.clone());
            i += 1;
        }

        // -n implies -H (unless -L is specified)
        if num_messages.is_some() && !history_all {
            history = true;
        }

        // Validate argument combinations
        // These are "exclusive" commands that can't be combined with prompts
        let exclusive_commands = [
            switch.is_some(),
            list,
            which,
            delete.is_some(),
            clear,
            compact,
            rename.is_some(),
            history,
            history_all,
            show_prompt,
            plugin.is_some(),
        ]
        .iter()
        .filter(|&&x| x)
        .count();

        // set_prompt can be combined with a prompt (set prompt, then send message)
        let combinable_commands = set_prompt.is_some();

        if exclusive_commands > 1 || (exclusive_commands > 0 && combinable_commands) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Only one command can be specified at a time",
            ));
        }

        if exclusive_commands > 0 && !prompt.is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Cannot specify both a command and a prompt",
            ));
        }

        Ok(Cli {
            switch,
            sub_context,
            list,
            which,
            delete,
            clear,
            compact,
            rename,
            history,
            history_all,
            num_messages,
            verbose,
            show_prompt,
            set_prompt,
            no_reflection,
            username,
            temp_username,
            plugin,
            prompt,
        })
    }

    pub fn print_help() {
        println!("chibi - A CLI tool for chatting with AI via OpenRouter");
        println!();
        println!("Usage:");
        println!("  chibi [OPTIONS] [PROMPT]");
        println!("  chibi [COMMAND]");
        println!();
        println!("Commands:");
        println!("  -s, --switch <NAME>       Switch to a different context (persistent)");
        println!("                            Use 'new' for auto-generated name (YYYYMMDD_HHMMSS)");
        println!(
            "                            Use 'new:prefix' for prefixed name (prefix_YYYYMMDD_HHMMSS)"
        );
        println!("  -S, --sub-context <NAME>  Run in a context without changing global state");
        println!("                            Useful for sub-agents. Same 'new' syntax supported.");
        println!("  -l, --list                List all contexts");
        println!("  -w, --which               Show current context name");
        println!("  -d, --delete <NAME>       Delete a context");
        println!("  -C, --clear               Clear current context");
        println!("  -c, --compact             Compact current context");
        println!("  -r, --rename <OLD> <NEW>  Rename a context");
        println!(
            "  -H, --history             Show recent messages from current context (default: 6)"
        );
        println!(
            "  -L, --history-all         Show messages from full transcript (current + archived)"
        );
        println!("  -n, --num-messages <N>    Number of messages to show (implies -H or -L)");
        println!("                            Positive N: last N messages");
        println!("                            Negative N: first N messages");
        println!("                            Zero: all messages");
        println!("  -p, --prompt              Show system prompt for current context");
        println!("  -e, --set-prompt <ARG>    Set system prompt (file path or literal text)");
        println!();
        println!("Options:");
        println!("  -v, --verbose             Show extra info (tools loaded, etc.)");
        println!("  -x, --no-reflection       Disable reflection prompt for this invocation");
        println!("  -u, --username <NAME>     Set username (persists to context's local.toml)");
        println!("  -U, --temp-username <NAME> Set username for this invocation only");
        println!("  -P, --plugin <NAME> [ARGS]  Run a plugin directly (bypasses LLM)");
        println!("  -h, --help                Show this help");
        println!("  -V, --version             Show version");
        println!();
        println!("Prompt input:");
        println!("  If arguments are provided after options, they are joined as the prompt.");
        println!(
            "  Use -- to force the rest to be a prompt (e.g., chibi -- -this starts with dash)"
        );
        println!("  If no arguments, read prompt from stdin (end with . on empty line)");
        println!("  Piped input: echo 'text' | chibi (can combine with arg prompt)");
        println!();
        println!("Examples:");
        println!("  chibi What is Rust?");
        println!("  chibi -s coding write a function");
        println!("  chibi -- -this prompt starts with dash");
        println!("  chibi -l");
        println!("  chibi -r old-name new-name");
        println!("  chibi -e prompt.md          Set prompt from file");
        println!("  chibi -e 'You are helpful'  Set prompt directly");
        println!("  chibi -P marketplace list   Run marketplace plugin directly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create args from a command string
    fn args(s: &str) -> Vec<String> {
        // Always include "chibi" as argv[0]
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
        assert!(!cli.list);
    }

    #[test]
    fn test_simple_prompt() {
        let cli = Cli::parse_from(&args("hello world")).unwrap();
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    #[test]
    fn test_switch_short() {
        let cli = Cli::parse_from(&args("-s coding")).unwrap();
        assert_eq!(cli.switch, Some("coding".to_string()));
    }

    #[test]
    fn test_switch_long() {
        let cli = Cli::parse_from(&args("--switch coding")).unwrap();
        assert_eq!(cli.switch, Some("coding".to_string()));
    }

    #[test]
    fn test_sub_context_short() {
        let cli = Cli::parse_from(&args("-S temp")).unwrap();
        assert_eq!(cli.sub_context, Some("temp".to_string()));
    }

    #[test]
    fn test_sub_context_long() {
        let cli = Cli::parse_from(&args("--sub-context temp")).unwrap();
        assert_eq!(cli.sub_context, Some("temp".to_string()));
    }

    #[test]
    fn test_list_short() {
        let cli = Cli::parse_from(&args("-l")).unwrap();
        assert!(cli.list);
    }

    #[test]
    fn test_list_long() {
        let cli = Cli::parse_from(&args("--list")).unwrap();
        assert!(cli.list);
    }

    #[test]
    fn test_which_short() {
        let cli = Cli::parse_from(&args("-w")).unwrap();
        assert!(cli.which);
    }

    #[test]
    fn test_delete_short() {
        let cli = Cli::parse_from(&args("-d old-context")).unwrap();
        assert_eq!(cli.delete, Some("old-context".to_string()));
    }

    #[test]
    fn test_clear_short() {
        let cli = Cli::parse_from(&args("-C")).unwrap();
        assert!(cli.clear);
    }

    #[test]
    fn test_compact_short() {
        let cli = Cli::parse_from(&args("-c")).unwrap();
        assert!(cli.compact);
    }

    #[test]
    fn test_rename_short() {
        let cli = Cli::parse_from(&args("-r old new")).unwrap();
        assert_eq!(cli.rename, Some(("old".to_string(), "new".to_string())));
    }

    #[test]
    fn test_rename_long() {
        let cli = Cli::parse_from(&args("--rename old new")).unwrap();
        assert_eq!(cli.rename, Some(("old".to_string(), "new".to_string())));
    }

    #[test]
    fn test_history_short() {
        let cli = Cli::parse_from(&args("-H")).unwrap();
        assert!(cli.history);
    }

    #[test]
    fn test_history_all_short() {
        let cli = Cli::parse_from(&args("-L")).unwrap();
        assert!(cli.history_all);
    }

    #[test]
    fn test_history_all_long() {
        let cli = Cli::parse_from(&args("--history-all")).unwrap();
        assert!(cli.history_all);
    }

    #[test]
    fn test_num_messages() {
        let cli = Cli::parse_from(&args("-n 10")).unwrap();
        assert_eq!(cli.num_messages, Some(10));
        assert!(cli.history); // -n implies -H
    }

    #[test]
    fn test_num_messages_negative() {
        let cli = Cli::parse_from(&args("-n -5")).unwrap();
        assert_eq!(cli.num_messages, Some(-5));
        assert!(cli.history); // -n implies -H
    }

    #[test]
    fn test_num_messages_with_history_all() {
        let cli = Cli::parse_from(&args("-L -n 10")).unwrap();
        assert_eq!(cli.num_messages, Some(10));
        assert!(cli.history_all);
        assert!(!cli.history); // -n with -L doesn't imply -H
    }

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
    fn test_show_prompt() {
        let cli = Cli::parse_from(&args("-p")).unwrap();
        assert!(cli.show_prompt);
    }

    #[test]
    fn test_set_prompt() {
        let cli = Cli::parse_from(&args("-e prompt.md")).unwrap();
        assert_eq!(cli.set_prompt, Some("prompt.md".to_string()));
    }

    #[test]
    fn test_no_reflection() {
        let cli = Cli::parse_from(&args("-x")).unwrap();
        assert!(cli.no_reflection);
    }

    #[test]
    fn test_username() {
        let cli = Cli::parse_from(&args("-u alice")).unwrap();
        assert_eq!(cli.username, Some("alice".to_string()));
    }

    #[test]
    fn test_temp_username() {
        let cli = Cli::parse_from(&args("-U bob")).unwrap();
        assert_eq!(cli.temp_username, Some("bob".to_string()));
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

    // === Combinable options ===

    #[test]
    fn test_set_prompt_with_prompt() {
        let cli = Cli::parse_from(&args("-e prompt.md hello")).unwrap();
        assert_eq!(cli.set_prompt, Some("prompt.md".to_string()));
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    #[test]
    fn test_verbose_with_prompt() {
        let cli = Cli::parse_from(&args("-v write code")).unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["write", "code"]);
    }

    #[test]
    fn test_sub_context_with_prompt() {
        let cli = Cli::parse_from(&args("-S temp hello")).unwrap();
        assert_eq!(cli.sub_context, Some("temp".to_string()));
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    // === Error cases ===

    #[test]
    fn test_switch_missing_arg() {
        let result = Cli::parse_from(&args("-s"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires an argument")
        );
    }

    #[test]
    fn test_delete_missing_arg() {
        let result = Cli::parse_from(&args("-d"));
        assert!(result.is_err());
    }

    #[test]
    fn test_rename_missing_args() {
        let result = Cli::parse_from(&args("-r old"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires two arguments")
        );
    }

    #[test]
    fn test_num_messages_invalid() {
        let result = Cli::parse_from(&args("-n abc"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid number"));
    }

    #[test]
    fn test_unknown_option() {
        let result = Cli::parse_from(&args("--unknown"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown option"));
    }

    // === Exclusive commands ===

    #[test]
    fn test_exclusive_commands_error() {
        let result = Cli::parse_from(&args("-l -w"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Only one command"));
    }

    #[test]
    fn test_command_with_prompt_error() {
        let result = Cli::parse_from(&args("-l hello"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot specify both")
        );
    }

    #[test]
    fn test_switch_is_exclusive() {
        let result = Cli::parse_from(&args("-s ctx hello"));
        assert!(result.is_err());
    }

    // === New context syntax ===

    #[test]
    fn test_switch_new() {
        let cli = Cli::parse_from(&args("-s new")).unwrap();
        assert_eq!(cli.switch, Some("new".to_string()));
    }

    #[test]
    fn test_switch_new_with_prefix() {
        let cli = Cli::parse_from(&args("-s new:myproject")).unwrap();
        assert_eq!(cli.switch, Some("new:myproject".to_string()));
    }

    // === Complex combinations ===

    #[test]
    fn test_multiple_non_exclusive_options() {
        let cli = Cli::parse_from(&args("-v -x -U test hello world")).unwrap();
        assert!(cli.verbose);
        assert!(cli.no_reflection);
        assert_eq!(cli.temp_username, Some("test".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    #[test]
    fn test_sub_context_with_verbose_and_prompt() {
        let cli = Cli::parse_from(&args("-S agent -v run task")).unwrap();
        assert_eq!(cli.sub_context, Some("agent".to_string()));
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["run", "task"]);
    }

    // =======================================================================
    // Prompt parsing behavior
    // Core rule: any argv string not starting with `-` begins the prompt
    // =======================================================================

    /// Core rule: a bare word immediately starts the prompt
    #[test]
    fn cli_bare_word_is_prompt() {
        let cli = Cli::parse_from(&args("hello")).unwrap();
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    /// Multiple bare words all become prompt parts
    #[test]
    fn cli_multiple_words_are_prompt() {
        let cli = Cli::parse_from(&args("explain this error")).unwrap();
        assert_eq!(cli.prompt, vec!["explain", "this", "error"]);
    }

    /// README pattern: chibi "add a users table to this schema"
    /// Words after a non-flag word continue the prompt
    #[test]
    fn cli_quoted_prompt_style() {
        // In shell, quotes group words. Here we simulate the parsed result.
        let a = vec![
            "chibi".to_string(),
            "add a users table to this schema".to_string(),
        ];
        let cli = Cli::parse_from(&a).unwrap();
        assert_eq!(cli.prompt, vec!["add a users table to this schema"]);
    }

    /// Options can precede the prompt: chibi -v "hello"
    #[test]
    fn cli_options_before_prompt() {
        let cli = Cli::parse_from(&args("-v hello")).unwrap();
        assert!(cli.verbose);
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    /// Multiple options before prompt
    #[test]
    fn cli_multiple_options_before_prompt() {
        let cli = Cli::parse_from(&args("-v -x hello world")).unwrap();
        assert!(cli.verbose);
        assert!(cli.no_reflection);
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    /// Sub-context is an option, not a command - can have prompt
    #[test]
    fn cli_sub_context_allows_prompt() {
        let cli = Cli::parse_from(&args("-S myctx hello world")).unwrap();
        assert_eq!(cli.sub_context, Some("myctx".to_string()));
        assert_eq!(cli.prompt, vec!["hello", "world"]);
    }

    /// Double-dash forces remaining args to be prompt (even if they look like flags)
    #[test]
    fn cli_double_dash_escapes_flags() {
        let cli = Cli::parse_from(&args("-- -v --help")).unwrap();
        assert_eq!(cli.prompt, vec!["-v", "--help"]);
        assert!(!cli.verbose); // -v was NOT parsed as flag
    }

    /// ANTI-PATTERN: "chibi command [options]" should NOT be allowed.
    /// If someone tries "chibi list" thinking it's a subcommand,
    /// it should be treated as the prompt "list" sent to the LLM.
    #[test]
    fn cli_no_subcommand_pattern() {
        // "chibi list" should parse "list" as a prompt, not a command
        let cli = Cli::parse_from(&args("list")).unwrap();
        assert_eq!(cli.prompt, vec!["list"]);
        assert!(!cli.list); // NOT the -l flag
    }

    /// Similarly, "chibi help" is a prompt, not --help
    #[test]
    fn cli_help_word_is_prompt() {
        let cli = Cli::parse_from(&args("help me understand rust")).unwrap();
        assert_eq!(cli.prompt, vec!["help", "me", "understand", "rust"]);
    }

    /// "chibi version" is a prompt asking about versions, not --version
    #[test]
    fn cli_version_word_is_prompt() {
        let cli = Cli::parse_from(&args("version")).unwrap();
        assert_eq!(cli.prompt, vec!["version"]);
    }

    /// Commands require the dash prefix: -l not "list"
    #[test]
    fn cli_commands_need_dash() {
        // This is the correct way to list contexts
        let cli = Cli::parse_from(&args("-l")).unwrap();
        assert!(cli.list);
        assert!(cli.prompt.is_empty());
    }

    /// Prompt can contain words that look like they could be commands
    #[test]
    fn cli_prompt_can_contain_command_words() {
        let cli = Cli::parse_from(&args("please delete the old files")).unwrap();
        assert_eq!(cli.prompt, vec!["please", "delete", "the", "old", "files"]);
        assert!(cli.delete.is_none()); // "delete" is part of prompt, not -d
    }

    /// Numbers in prompt work fine
    #[test]
    fn cli_numbers_in_prompt() {
        let cli = Cli::parse_from(&args("what is 2 + 2")).unwrap();
        assert_eq!(cli.prompt, vec!["what", "is", "2", "+", "2"]);
    }

    /// Special characters in prompt
    #[test]
    fn cli_special_chars_in_prompt() {
        let a = vec![
            "chibi".to_string(),
            "what's".to_string(),
            "this?".to_string(),
        ];
        let cli = Cli::parse_from(&a).unwrap();
        assert_eq!(cli.prompt, vec!["what's", "this?"]);
    }

    /// Once prompt starts, everything else is prompt (even things that look like flags)
    #[test]
    fn cli_flags_after_prompt_are_prompt() {
        let cli = Cli::parse_from(&args("hello -v world")).unwrap();
        // Once "hello" started the prompt, "-v" and "world" are all prompt
        assert_eq!(cli.prompt, vec!["hello", "-v", "world"]);
        assert!(!cli.verbose); // -v was NOT parsed as a flag
    }

    /// Options with arguments work before prompt
    #[test]
    fn cli_option_args_before_prompt() {
        let cli = Cli::parse_from(&args("-U alice hello")).unwrap();
        assert_eq!(cli.temp_username, Some("alice".to_string()));
        assert_eq!(cli.prompt, vec!["hello"]);
    }

    /// Empty prompt is valid (interactive mode)
    #[test]
    fn cli_empty_prompt_allowed() {
        let cli = Cli::parse_from(&args("")).unwrap();
        assert!(cli.prompt.is_empty());
    }

    /// Empty prompt with options is valid
    #[test]
    fn cli_options_only_no_prompt() {
        let cli = Cli::parse_from(&args("-v")).unwrap();
        assert!(cli.verbose);
        assert!(cli.prompt.is_empty());
    }

    // === Plugin invocation tests ===

    #[test]
    fn test_plugin_short() {
        let cli = Cli::parse_from(&args("-P myplugin")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert!(invocation.args.is_empty());
    }

    #[test]
    fn test_plugin_long() {
        let cli = Cli::parse_from(&args("--plugin myplugin")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert!(invocation.args.is_empty());
    }

    #[test]
    fn test_plugin_with_args() {
        let cli = Cli::parse_from(&args("-P myplugin list --all")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["list", "--all"]);
    }

    #[test]
    fn test_plugin_missing_name() {
        let result = Cli::parse_from(&args("-P"));
        assert!(result.is_err());
    }

    #[test]
    fn test_plugin_is_exclusive() {
        // -P should be exclusive with other commands
        // Note: flags AFTER -P become plugin args, so we test flag BEFORE -P
        let result = Cli::parse_from(&args("-l -P myplugin"));
        assert!(result.is_err());
    }

    #[test]
    fn test_plugin_captures_trailing_flags() {
        // Everything after plugin name becomes plugin args, even things that look like flags
        let cli = Cli::parse_from(&args("-P myplugin -l --verbose")).unwrap();
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["-l", "--verbose"]);
    }

    #[test]
    fn test_plugin_verbose_before() {
        // Verbose flag before -P should work
        let cli = Cli::parse_from(&args("-v -P myplugin arg1")).unwrap();
        assert!(cli.verbose);
        let invocation = cli.plugin.unwrap();
        assert_eq!(invocation.name, "myplugin");
        assert_eq!(invocation.args, vec!["arg1"]);
    }
}
