mod api;
mod cli;
mod config;
mod context;
mod state;
mod tools;

use cli::Cli;
use context::{Context, now_timestamp};
use state::AppState;
use std::io::{self, BufRead, ErrorKind};

fn read_prompt_from_stdin() -> io::Result<String> {
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

    if let Some(name) = cli.switch {
        app.state.switch_context(name)?;
        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: now_timestamp(),
                updated_at: 0,
            };
            app.save_current_context(&new_context)?;
        }
        app.save()?;
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
    } else if let Some(arg) = cli.set_prompt {
        // Check if arg is a file path
        let content = if std::path::Path::new(&arg).is_file() {
            std::fs::read_to_string(&arg)?
        } else {
            arg
        };
        app.set_system_prompt(&content)?;
        println!("System prompt set for context '{}'", app.state.current_context);
    } else if !cli.prompt.is_empty() {
        let prompt = cli.prompt.join(" ");
        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: now_timestamp(),
                updated_at: 0,
            };
            app.save_current_context(&new_context)?;
        }
        api::send_prompt(&app, prompt, &tools, verbose, use_reflection).await?;
    } else {
        // No command and no prompt - read from stdin
        let prompt = read_prompt_from_stdin()?;
        if prompt.trim().is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Prompt cannot be empty"));
        }
        api::send_prompt(&app, prompt, &tools, verbose, use_reflection).await?;
    }

    Ok(())
}
