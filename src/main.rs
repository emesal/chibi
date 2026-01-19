mod api;
mod cli;
mod config;
mod context;
mod state;
mod tools;

use cli::Cli;
use context::{Context, now_timestamp};
use state::AppState;
use std::io::{self, BufRead, ErrorKind, IsTerminal};

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

/// Generate a unique context name for `-s new` or `-s new:prefix`
/// Format: [prefix_]YYYYMMDD_HHMMSS[_N]
fn generate_new_context_name(app: &AppState, prefix: Option<&str>) -> String {
    use chrono::Local;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let base_name = match prefix {
        Some(p) => format!("{}_{}", p, timestamp),
        None => timestamp,
    };

    // Check for collisions
    let existing = app.list_contexts();
    if !existing.contains(&base_name) {
        return base_name;
    }

    // Append _N until we find an unused name
    let mut n = 2;
    loop {
        let candidate = format!("{}_{}", base_name, n);
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse()?;
    let verbose = cli.verbose;
    // Reflection is enabled if: config allows it AND --no-reflection wasn't passed
    let mut app = AppState::load()?;
    let use_reflection = app.config.reflection_enabled && !cli.no_reflection;

    // Load tools at startup
    let tools = tools::load_tools(&app.tools_dir, verbose)?;
    if verbose && !tools.is_empty() {
        eprintln!("[Loaded {} tool(s): {}]",
            tools.len(),
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    // Handle sub-context: use a different context for this invocation without changing global state
    // This must be processed before other commands so the context is set up
    if let Some(name) = &cli.sub_context {
        let actual_name = if name == "new" {
            generate_new_context_name(&app, None)
        } else if let Some(prefix) = name.strip_prefix("new:") {
            if prefix.is_empty() {
                return Err(io::Error::new(ErrorKind::InvalidInput, "Prefix cannot be empty in '-S new:prefix'"));
            }
            generate_new_context_name(&app, Some(prefix))
        } else {
            name.clone()
        };

        // Switch context in memory only (don't save to state file)
        app.state.switch_context(actual_name)?;
        // Create the context directory if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: now_timestamp(),
                updated_at: 0,
                summary: String::new(),
            };
            app.save_current_context(&new_context)?;
        }
        if verbose {
            eprintln!("[Using sub-context: {}]", app.state.current_context);
        }
        // Note: we do NOT call app.save() here, so global state is unchanged
    }

    if let Some(name) = cli.switch {
        // Handle `-s new` and `-s new:prefix` for auto-generated names
        let actual_name = if name == "new" {
            generate_new_context_name(&app, None)
        } else if let Some(prefix) = name.strip_prefix("new:") {
            if prefix.is_empty() {
                return Err(io::Error::new(ErrorKind::InvalidInput, "Prefix cannot be empty in '-s new:prefix'"));
            }
            generate_new_context_name(&app, Some(prefix))
        } else {
            name
        };

        app.state.switch_context(actual_name)?;
        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: now_timestamp(),
                updated_at: 0,
                summary: String::new(),
            };
            app.save_current_context(&new_context)?;
        }
        app.save()?;
        // Print the new context name so user knows what was created
        if verbose {
            eprintln!("[Switched to context: {}]", app.state.current_context);
        }
    } else if cli.list {
        let contexts = app.list_contexts();
        let current = &app.state.current_context;
        for name in contexts {
            if &name == current {
                println!("* {}", name);
            } else {
                println!("  {}", name);
            }
        }
    } else if cli.which {
        println!("{}", app.state.current_context);
    } else if let Some(name) = cli.delete {
        match app.delete_context(&name) {
            Ok(true) => println!("Deleted context: {}", name),
            Ok(false) => println!("Context '{}' not found", name),
            Err(e) => return Err(e),
        }
    } else if cli.clear {
        app.clear_context()?;
        println!("Context cleared (history saved to transcript)");
    } else if cli.compact {
        api::compact_context_with_llm_manual(&app, verbose).await?;
    } else if let Some((old_name, new_name)) = cli.rename {
        app.rename_context(&old_name, &new_name)?;
        println!("Renamed context '{}' to '{}'", old_name, new_name);
    } else if cli.history {
        let context = app.get_current_context()?;
        let num = cli.num_messages.unwrap_or(6);
        let messages: Vec<_> = if num == 0 {
            context.messages.iter().collect()
        } else {
            context.messages.iter().rev().take(num).collect::<Vec<_>>().into_iter().rev().collect()
        };
        for msg in messages {
            if msg.role == "system" {
                continue;
            }
            println!("[{}]: {}\n", msg.role.to_uppercase(), msg.content);
        }
    } else if cli.show_prompt {
        let prompt = app.load_system_prompt()?;
        if prompt.is_empty() {
            println!("(no system prompt set)");
        } else {
            print!("{}", prompt);
            // Add newline if the prompt doesn't end with one
            if !prompt.ends_with('\n') {
                println!();
            }
        }
    } else {
        // Handle set_prompt if provided (can be combined with sending a prompt)
        let had_set_prompt = cli.set_prompt.is_some();
        if let Some(arg) = cli.set_prompt {
            // Check if arg is a file path
            let content = if std::path::Path::new(&arg).is_file() {
                std::fs::read_to_string(&arg)?
            } else {
                arg
            };
            app.set_system_prompt(&content)?;
            if verbose {
                eprintln!("[System prompt set for context '{}']", app.state.current_context);
            }
        }

        // Build prompt from args and/or stdin
        let stdin_is_pipe = !io::stdin().is_terminal();
        let arg_prompt = if cli.prompt.is_empty() {
            None
        } else {
            Some(cli.prompt.join(" "))
        };

        let prompt = match (stdin_is_pipe, arg_prompt) {
            // Piped input + arg prompt: concatenate (arg prompt first, then piped content)
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
            // Interactive: read from terminal (skip if -e was used without a prompt)
            (false, None) => {
                if had_set_prompt {
                    // -e was used alone, don't wait for interactive input
                    println!("System prompt set for context '{}'", app.state.current_context);
                    return Ok(());
                }
                read_prompt_interactive()?
            }
        };

        if prompt.trim().is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Prompt cannot be empty"));
        }

        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: now_timestamp(),
                updated_at: 0,
                summary: String::new(),
            };
            app.save_current_context(&new_context)?;
        }
        api::send_prompt(&app, prompt, &tools, verbose, use_reflection).await?;
    }

    Ok(())
}
