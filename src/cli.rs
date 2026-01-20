use std::io::{self, ErrorKind};

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
    pub num_messages: Option<usize>,
    pub verbose: bool,
    pub show_prompt: bool,
    pub set_prompt: Option<String>,
    pub no_reflection: bool,
    pub username: Option<String>,
    pub temp_username: Option<String>,
    pub prompt: Vec<String>,
}

impl Cli {
    pub fn parse() -> io::Result<Self> {
        let args: Vec<String> = std::env::args().collect();

        let mut switch = None;
        let mut sub_context = None;
        let mut list = false;
        let mut which = false;
        let mut delete = None;
        let mut clear = false;
        let mut compact = false;
        let mut rename = None;
        let mut history = false;
        let mut num_messages: Option<usize> = None;
        let mut verbose = false;
        let mut show_prompt = false;
        let mut set_prompt = None;
        let mut no_reflection = false;
        let mut username = None;
        let mut temp_username = None;
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
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                switch = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-S" || arg == "--sub-context" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
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
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
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
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires two arguments", arg)));
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

            if arg == "-n" || arg == "--num-messages" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                num_messages = Some(args[i + 1].parse().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", args[i + 1]))
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
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
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
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                username = Some(args[i + 1].clone());
                i += 2;
                continue;
            }

            if arg == "-U" || arg == "--temp-username" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                temp_username = Some(args[i + 1].clone());
                i += 2;
                continue;
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
                return Err(io::Error::new(ErrorKind::InvalidInput, format!("Unknown option: {}", arg)));
            }

            // This is the start of the prompt
            is_prompt = true;
            prompt.push(arg.clone());
            i += 1;
        }

        // -n implies -H
        if num_messages.is_some() {
            history = true;
        }

        // Validate argument combinations
        // These are "exclusive" commands that can't be combined with prompts
        let exclusive_commands = [switch.is_some(), list, which, delete.is_some(), clear, compact, rename.is_some(), history, show_prompt]
            .iter()
            .filter(|&&x| x)
            .count();

        // set_prompt can be combined with a prompt (set prompt, then send message)
        let combinable_commands = set_prompt.is_some();

        if exclusive_commands > 1 || (exclusive_commands > 0 && combinable_commands) {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Only one command can be specified at a time"));
        }

        if exclusive_commands > 0 && !prompt.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Cannot specify both a command and a prompt"));
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
            num_messages,
            verbose,
            show_prompt,
            set_prompt,
            no_reflection,
            username,
            temp_username,
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
        println!("                            Use 'new:prefix' for prefixed name (prefix_YYYYMMDD_HHMMSS)");
        println!("  -S, --sub-context <NAME>  Run in a context without changing global state");
        println!("                            Useful for sub-agents. Same 'new' syntax supported.");
        println!("  -l, --list                List all contexts");
        println!("  -w, --which               Show current context name");
        println!("  -d, --delete <NAME>       Delete a context");
        println!("  -C, --clear               Clear current context");
        println!("  -c, --compact             Compact current context");
        println!("  -r, --rename <OLD> <NEW>  Rename a context");
        println!("  -H, --history             Show recent messages (default: 6)");
        println!("  -n, --num-messages <N>    Number of messages to show (0 = all, implies -H)");
        println!("  -p, --prompt              Show system prompt for current context");
        println!("  -e, --set-prompt <ARG>    Set system prompt (file path or literal text)");
        println!();
        println!("Options:");
        println!("  -v, --verbose             Show extra info (tools loaded, etc.)");
        println!("  -x, --no-reflection       Disable reflection prompt for this invocation");
        println!("  -u, --username <NAME>     Set username (persists to context's local.toml)");
        println!("  -U, --temp-username <NAME> Set username for this invocation only");
        println!("  -h, --help                Show this help");
        println!("  -V, --version             Show version");
        println!();
        println!("Prompt input:");
        println!("  If arguments are provided after options, they are joined as the prompt.");
        println!("  Use -- to force the rest to be a prompt (e.g., chibi -- -this starts with dash)");
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
    }
}
