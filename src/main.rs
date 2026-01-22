mod api;
mod cli;
mod config;
mod context;
mod input;
mod json_input;
mod llm;
mod lock;
mod output;
mod state;
mod tools;

use cli::Inspectable;
use context::Context;
use input::{ChibiInput, Command, ContextSelection, UsernameOverride};
use output::OutputHandler;
use state::AppState;
use std::io::{self, ErrorKind, Read};

/// Generate a unique context name for `-c new` or `-c new:prefix`
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

/// Resolve "new" or "new:prefix" context names
fn resolve_context_name(app: &AppState, name: &str) -> io::Result<String> {
    if name == "new" {
        Ok(generate_new_context_name(app, None))
    } else if let Some(prefix) = name.strip_prefix("new:") {
        if prefix.is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Prefix cannot be empty in 'new:prefix'",
            ));
        }
        Ok(generate_new_context_name(app, Some(prefix)))
    } else {
        Ok(name.to_string())
    }
}

/// Display inspectable content for a context
fn inspect_context(app: &AppState, context_name: &str, thing: &Inspectable) -> io::Result<()> {
    match thing {
        Inspectable::List => {
            println!("Inspectable items:");
            for name in Inspectable::all_names() {
                println!("  {}", name);
            }
        }
        Inspectable::SystemPrompt => {
            let prompt = app.load_system_prompt_for(context_name)?;
            if prompt.is_empty() {
                println!("(no system prompt set)");
            } else {
                print!("{}", prompt);
                if !prompt.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Reflection => {
            let reflection = app.load_reflection()?;
            if reflection.is_empty() {
                println!("(no reflection set)");
            } else {
                print!("{}", reflection);
                if !reflection.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Todos => {
            let todos = app.load_todos_for(context_name)?;
            if todos.is_empty() {
                println!("(no todos)");
            } else {
                print!("{}", todos);
                if !todos.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Goals => {
            let goals = app.load_goals_for(context_name)?;
            if goals.is_empty() {
                println!("(no goals)");
            } else {
                print!("{}", goals);
                if !goals.ends_with('\n') {
                    println!();
                }
            }
        }
    }
    Ok(())
}

/// Show log entries for a context
/// - Without verbose: shows messages, with condensed tool call summaries
/// - With verbose: shows all entries with full content
fn show_log(app: &AppState, context_name: &str, num: isize, verbose: bool) -> io::Result<()> {
    let entries = app.read_jsonl_transcript(context_name)?;

    // Select entries based on num parameter
    let selected: Vec<_> = if num == 0 {
        entries.iter().collect()
    } else if num > 0 {
        let n = num as usize;
        entries
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        let n = (-num) as usize;
        entries.iter().take(n).collect()
    };

    for entry in selected {
        match entry.entry_type.as_str() {
            "message" => {
                println!("[{}]: {}\n", entry.from.to_uppercase(), entry.content);
            }
            "tool_call" => {
                if verbose {
                    // Full tool call display
                    println!("[TOOL CALL: {}]\n{}\n", entry.to, entry.content);
                } else {
                    // Condensed: show tool name and truncated args
                    let args_preview = if entry.content.len() > 60 {
                        format!("{}...", &entry.content[..60])
                    } else {
                        entry.content.clone()
                    };
                    println!("[TOOL: {}] {}", entry.to, args_preview);
                }
            }
            "tool_result" => {
                if verbose {
                    // Full tool result display
                    println!("[TOOL RESULT: {}]\n{}\n", entry.from, entry.content);
                } else {
                    // Condensed: show size
                    let size = entry.content.len();
                    let size_str = if size > 1024 {
                        format!("{:.1}kb", size as f64 / 1024.0)
                    } else {
                        format!("{}b", size)
                    };
                    println!("  → {}", size_str);
                }
            }
            "compaction" => {
                if verbose {
                    println!("[COMPACTION]: {}\n", entry.content);
                }
                // Non-verbose: skip compaction markers
            }
            _ => {
                if verbose {
                    println!("[{}]: {}\n", entry.entry_type.to_uppercase(), entry.content);
                }
            }
        }
    }
    Ok(())
}

/// Set system prompt for a context
fn set_prompt_for_context(
    app: &AppState,
    context_name: &str,
    arg: &str,
    verbose: bool,
) -> io::Result<()> {
    let content = if std::path::Path::new(arg).is_file() {
        std::fs::read_to_string(arg)?
    } else {
        arg.to_string()
    };
    app.set_system_prompt_for(context_name, &content)?;
    if verbose {
        eprintln!("[System prompt set for context '{}']", context_name);
    }
    Ok(())
}

/// Execute from ChibiInput (used for JSON mode)
async fn execute_from_input(
    input: ChibiInput,
    app: &mut AppState,
    tools: &[tools::Tool],
    output: &OutputHandler,
) -> io::Result<()> {
    let verbose = input.flags.verbose;
    let json_output = input.flags.json_output;

    // Execute on_start hook
    let hook_data = serde_json::json!({
        "current_context": app.state.current_context,
        "verbose": verbose,
    });
    let _ = tools::execute_hook(tools, tools::HookPoint::OnStart, &hook_data, verbose);

    // Track if we did an action
    let mut did_action = false;

    // Handle context selection
    match &input.context {
        ContextSelection::Current => {}
        ContextSelection::Transient { name } => {
            let actual_name = resolve_context_name(app, name)?;
            let prev_context = app.state.current_context.clone();
            app.state.switch_context(actual_name)?;
            if !app.context_dir(&app.state.current_context).exists() {
                let new_context = Context::new(app.state.current_context.clone());
                app.save_current_context(&new_context)?;
            }
            output.diagnostic(
                &format!("[Using transient context: {}]", app.state.current_context),
                verbose,
            );
            let hook_data = serde_json::json!({
                "from_context": prev_context,
                "to_context": app.state.current_context,
                "is_transient": true,
            });
            let _ = tools::execute_hook(
                tools,
                tools::HookPoint::OnContextSwitch,
                &hook_data,
                verbose,
            );
        }
        ContextSelection::Switch { name, persistent } => {
            let actual_name = resolve_context_name(app, name)?;
            let prev_context = app.state.current_context.clone();
            app.state.switch_context(actual_name)?;
            if !app.context_dir(&app.state.current_context).exists() {
                let new_context = Context::new(app.state.current_context.clone());
                app.save_current_context(&new_context)?;
            }
            if *persistent {
                app.save()?;
            }
            output.diagnostic(
                &format!("[Switched to context: {}]", app.state.current_context),
                verbose,
            );
            let hook_data = serde_json::json!({
                "from_context": prev_context,
                "to_context": app.state.current_context,
                "is_transient": !persistent,
            });
            let _ = tools::execute_hook(
                tools,
                tools::HookPoint::OnContextSwitch,
                &hook_data,
                verbose,
            );
            did_action = true;
        }
    }

    // Handle username override
    if let Some(ref override_) = input.username_override {
        match override_ {
            UsernameOverride::Persistent(username) => {
                let mut local_config = app.load_local_config(&app.state.current_context)?;
                local_config.username = Some(username.clone());
                app.save_local_config(&app.state.current_context, &local_config)?;
                output.diagnostic(
                    &format!(
                        "[Username '{}' saved to context '{}']",
                        username, app.state.current_context
                    ),
                    verbose,
                );
                did_action = true;
            }
            UsernameOverride::Transient(_) => {
                // Transient username is applied via resolve_config later
            }
        }
    }

    // Execute the command
    match &input.command {
        Command::ShowHelp => {
            cli::Cli::print_help();
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi {}", env!("CARGO_PKG_VERSION")));
        }
        Command::ListContexts => {
            let contexts = app.list_contexts();
            let current = &app.state.current_context;
            for name in contexts {
                let context_dir = app.context_dir(&name);
                let status =
                    lock::ContextLock::get_status(&context_dir, app.config.lock_heartbeat_seconds);
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                if &name == current {
                    output.emit_result(&format!("* {}{}", name, status_str));
                } else {
                    output.emit_result(&format!("  {}{}", name, status_str));
                }
            }
            did_action = true;
        }
        Command::ListCurrentContext => {
            let context_name = &app.state.current_context;
            let context = app.get_current_context()?;
            let context_dir = app.context_dir(context_name);
            let status =
                lock::ContextLock::get_status(&context_dir, app.config.lock_heartbeat_seconds);
            let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
            output.emit_result(&format!("Context: {}{}", context_name, status_str));
            output.emit_result(&format!("Messages: {}", context.messages.len()));
            if !context.summary.is_empty() {
                output.emit_result(&format!(
                    "Summary: {}",
                    context.summary.lines().next().unwrap_or("")
                ));
            }
            did_action = true;
        }
        Command::DeleteContext { name } => {
            let ctx_name = name
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            match app.delete_context(&ctx_name) {
                Ok(true) => output.emit_result(&format!("Deleted context: {}", ctx_name)),
                Ok(false) => output.emit_result(&format!("Context '{}' not found", ctx_name)),
                Err(e) => return Err(e),
            }
            did_action = true;
        }
        Command::ArchiveContext { name } => {
            let ctx_name = name
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            if name.is_none() {
                let context = app.get_current_context()?;
                let hook_data = serde_json::json!({
                    "context_name": context.name,
                    "message_count": context.messages.len(),
                    "summary": context.summary,
                });
                let _ = tools::execute_hook(tools, tools::HookPoint::PreClear, &hook_data, verbose);
                app.clear_context()?;
                let hook_data = serde_json::json!({
                    "context_name": app.state.current_context,
                });
                let _ =
                    tools::execute_hook(tools, tools::HookPoint::PostClear, &hook_data, verbose);
            } else {
                app.clear_context_by_name(&ctx_name)?;
            }
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)",
                ctx_name
            ));
            did_action = true;
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                api::compact_context_by_name(app, ctx_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                api::compact_context_with_llm_manual(app, verbose).await?;
            }
            did_action = true;
        }
        Command::RenameContext { old, new } => {
            let old_name = old
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            app.rename_context(&old_name, new)?;
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
            did_action = true;
        }
        Command::ShowLog { context, count } => {
            let ctx_name = context
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            show_log(app, &ctx_name, *count, verbose)?;
            did_action = true;
        }
        Command::Inspect { context, thing } => {
            let ctx_name = context
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            inspect_context(app, &ctx_name, thing)?;
            did_action = true;
        }
        Command::SetSystemPrompt { context, prompt } => {
            let ctx_name = context
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            set_prompt_for_context(app, &ctx_name, prompt, verbose)?;
            did_action = true;
        }
        Command::RunPlugin { name, args } => {
            let tool = tools::find_tool(tools, name).ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, format!("Plugin '{}' not found", name))
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = tools::execute_tool(tool, &args_json, verbose)?;
            output.emit_result(&result);
            did_action = true;
        }
        Command::CallTool { name, args } => {
            if let Some(tool) = tools::find_tool(tools, name) {
                let args_json = serde_json::json!({ "args": args });
                let result = tools::execute_tool(tool, &args_json, verbose)?;
                output.emit_result(&result);
            } else {
                // Check for built-in tools
                let args_str = args.join(" ");
                let args_json: serde_json::Value = if args_str.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&args_str).map_err(|e| {
                        io::Error::new(
                            ErrorKind::InvalidInput,
                            format!("Invalid JSON arguments: {}", e),
                        )
                    })?
                };
                match name.as_str() {
                    "update_todos" => {
                        let content = args_json
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_todos requires {\"content\": \"...\"}",
                                )
                            })?;
                        app.save_current_todos(content)?;
                        output.emit_result("Todos updated");
                    }
                    "update_goals" => {
                        let content = args_json
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_goals requires {\"content\": \"...\"}",
                                )
                            })?;
                        app.save_current_goals(content)?;
                        output.emit_result("Goals updated");
                    }
                    "update_reflection" => {
                        let content = args_json
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_reflection requires {\"content\": \"...\"}",
                                )
                            })?;
                        app.save_reflection(content)?;
                        output.emit_result("Reflection updated");
                    }
                    "send_message" => {
                        let to = args_json
                            .get("to")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "send_message requires \"to\" field",
                                )
                            })?;
                        let message = args_json
                            .get("message")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "send_message requires \"message\" field",
                                )
                            })?;
                        app.send_inbox_message(to, message)?;
                        output.emit_result(&format!("Message sent to '{}'", to));
                    }
                    _ => {
                        return Err(io::Error::new(
                            ErrorKind::NotFound,
                            format!("Tool '{}' not found", name),
                        ));
                    }
                }
            }
            did_action = true;
        }
        Command::SendPrompt { prompt } => {
            // Ensure context exists
            if !app.context_dir(&app.state.current_context).exists() {
                let new_context = Context::new(app.state.current_context.clone());
                app.save_current_context(&new_context)?;
            }

            // Resolve config with runtime overrides
            let (persistent_username, transient_username) = match &input.username_override {
                Some(UsernameOverride::Persistent(u)) => (Some(u.as_str()), None),
                Some(UsernameOverride::Transient(u)) => (None, Some(u.as_str())),
                None => (None, None),
            };
            let resolved = app.resolve_config(persistent_username, transient_username)?;
            let use_reflection = app.config.reflection_enabled;

            // Acquire context lock
            let context_dir = app.context_dir(&app.state.current_context);
            let _lock =
                lock::ContextLock::acquire(&context_dir, app.config.lock_heartbeat_seconds)?;

            api::send_prompt(
                app,
                prompt.clone(),
                tools,
                verbose,
                use_reflection,
                &resolved,
                json_output,
            )
            .await?;
            did_action = true;
        }
        Command::NoOp => {
            // No operation - just context switch, already handled above
        }
    }

    // Execute on_end hook
    let hook_data = serde_json::json!({
        "current_context": app.state.current_context,
    });
    let _ = tools::execute_hook(tools, tools::HookPoint::OnEnd, &hook_data, verbose);

    // Check for no action and no prompt
    if !did_action && matches!(input.command, Command::NoOp) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "No operation specified",
        ));
    }

    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Check for --json-config early (before full CLI parsing)
    let args: Vec<String> = std::env::args().collect();
    let is_json_config = args.iter().any(|a| a == "--json-config");
    let cli_json_output = args.iter().any(|a| a == "--json-output");

    if is_json_config {
        // JSON mode: read from stdin and parse directly to ChibiInput
        let mut json_str = String::new();
        io::stdin().read_to_string(&mut json_str)?;

        let mut input = json_input::from_str(&json_str)?;

        // Handle help and version early
        if matches!(input.command, Command::ShowHelp) {
            cli::Cli::print_help();
            return Ok(());
        }
        if matches!(input.command, Command::ShowVersion) {
            println!("chibi {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        // CLI --json-output flag also sets json_output mode
        if cli_json_output {
            input.flags.json_output = true;
        }

        let output = OutputHandler::new(input.flags.json_output);
        let mut app = AppState::load()?;

        let tools = tools::load_tools(&app.plugins_dir, input.flags.verbose)?;
        output.diagnostic(
            &format!("[Loaded {} tool(s)]", tools.len()),
            input.flags.verbose,
        );

        return execute_from_input(input, &mut app, &tools, &output).await;
    }

    // CLI mode: parse to ChibiInput and use unified execution
    let input = cli::parse()?;
    let verbose = input.flags.verbose;
    let mut app = AppState::load()?;

    // Load tools
    let tools = tools::load_tools(&app.plugins_dir, verbose)?;
    if verbose && !tools.is_empty() {
        eprintln!(
            "[Loaded {} tool(s): {}]",
            tools.len(),
            tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Use OutputHandler for CLI mode (non-JSON output)
    let output = OutputHandler::new(input.flags.json_output);

    execute_from_input(input, &mut app, &tools, &output).await
}
