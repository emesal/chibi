use std::io::{self, ErrorKind};

#[derive(Debug)]
pub struct Cli {
    pub switch: Option<String>,
    pub list: bool,
    pub which: bool,
    pub delete: Option<String>,
    pub clear: bool,
    pub compact: bool,
    pub rename: Option<(String, String)>,
    pub history: bool,
    pub num_messages: Option<usize>,
    pub prompt: Vec<String>,
}

impl Cli {
    pub fn parse() -> io::Result<Self> {
        let args: Vec<String> = std::env::args().collect();
        
        let mut switch = None;
        let mut list = false;
        let mut which = false;
        let mut delete = None;
        let mut clear = false;
        let mut compact = false;
        let mut rename = None;
        let mut history = false;
        let mut num_messages: Option<usize> = None;
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
            
            if arg == "-h" || arg == "--help" {
                Self::print_help();
                std::process::exit(0);
            }
            
            if arg == "-v" || arg == "--version" {
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
        let commands = [switch.is_some(), list, which, delete.is_some(), clear, compact, rename.is_some(), history]
            .iter()
            .filter(|&&x| x)
            .count();
        
        if commands > 1 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Only one command can be specified at a time"));
        }
        
        if commands > 0 && !prompt.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Cannot specify both a command and a prompt"));
        }
        
        Ok(Cli {
            switch,
            list,
            which,
            delete,
            clear,
            compact,
            rename,
            history,
            num_messages,
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
        println!("  -s, --switch <NAME>     Switch to a different context");
        println!("  -l, --list              List all contexts");
        println!("  -w, --which             Show current context name");
        println!("  -d, --delete <NAME>     Delete a context");
        println!("  -C, --clear             Clear current context");
        println!("  -c, --compact           Compact current context");
        println!("  -r, --rename <OLD> <NEW>  Rename a context");
        println!("  -H, --history           Show recent messages (default: 6)");
        println!("  -n, --num-messages <N>  Number of messages to show (0 = all, implies -H)");
        println!();
        println!("Prompt input:");
        println!("  If arguments are provided after options, they are joined as the prompt.");
        println!("  Use -- to force the rest to be a prompt (e.g., chibi -- -this starts with dash)");
        println!("  If no arguments, read prompt from stdin (end with . on empty line)");
        println!();
        println!("Examples:");
        println!("  chibi What is Rust?");
        println!("  chibi -s coding write a function");
        println!("  chibi -- -this prompt starts with dash");
        println!("  chibi -l");
        println!("  chibi -r old-name new-name");
    }
}
