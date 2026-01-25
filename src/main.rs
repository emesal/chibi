mod api;
mod cache;
mod cli;
mod config;
mod context;
mod inbox;
mod input;
mod json_input;
mod llm;
mod lock;
mod output;
mod partition;
mod state;
mod tools;

use cli::Inspectable;
use context::{Context, ENTRY_TYPE_MESSAGE, ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT};
use input::{ChibiInput, Command, ContextSelection, UsernameOverride};
use output::OutputHandler;
use state::AppState;
use std::io::{self, ErrorKind, IsTerminal, Read, Write};

/// Prompt user for confirmation (y/N). Returns true if user confirms.
/// If stdin is not a terminal (piped input), returns false.
fn confirm_action(prompt: &str) -> bool {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return false;
    }

    eprint!("{} [y/N] ", prompt);
    io::stderr().flush().ok();

    let mut input = String::new();
    if stdin.read_line(&mut input).is_err() {
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

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
fn inspect_context(
    app: &AppState,
    context_name: &str,
    thing: &Inspectable,
    resolved_config: Option<&config::ResolvedConfig>,
) -> io::Result<()> {
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
        Inspectable::Home => {
            println!("{}", app.chibi_dir.display());
        }
        Inspectable::ConfigField(field_path) => {
            if let Some(config) = resolved_config {
                match config.get_field(field_path) {
                    Some(value) => println!("{}", value),
                    None => println!("(not set)"),
                }
            } else {
                // Need to resolve config to inspect config fields
                let config = app.resolve_config(None, None)?;
                match config.get_field(field_path) {
                    Some(value) => println!("{}", value),
                    None => println!("(not set)"),
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
            ENTRY_TYPE_MESSAGE => {
                println!("[{}]: {}\n", entry.from.to_uppercase(), entry.content);
            }
            ENTRY_TYPE_TOOL_CALL => {
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
            ENTRY_TYPE_TOOL_RESULT => {
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

    // Auto-destroy expired contexts (always enabled)
    let destroyed = app.auto_destroy_expired_contexts(verbose)?;
    if !destroyed.is_empty() {
        app.save()?;
        output.diagnostic(
            &format!("[Auto-destroyed {} expired context(s)]", destroyed.len()),
            verbose,
        );
    }

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

    // Touch the current context to update last_activity_at and apply debug destroy settings
    let current_ctx = app.state.current_context.clone();
    let debug_destroy_at = match &input.flags.debug {
        Some(input::DebugKey::DestroyAt(ts)) => Some(*ts),
        _ => None,
    };
    let debug_destroy_after = match &input.flags.debug {
        Some(input::DebugKey::DestroyAfterSecondsInactive(secs)) => Some(*secs),
        _ => None,
    };
    if app.touch_context_with_destroy_settings(
        &current_ctx,
        debug_destroy_at,
        debug_destroy_after,
    )? {
        app.save()?;
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
        Command::DestroyContext { name } => {
            let ctx_name = name
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());

            // Check if context exists before prompting
            if !app.context_dir(&ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else if !confirm_action(&format!("Destroy context '{}'?", ctx_name)) {
                // Prompt for confirmation
                output.emit_result("Aborted");
            } else {
                match app.destroy_context(&ctx_name) {
                    Ok(Some(switched_to)) => {
                        output.emit_result(&format!(
                            "Destroyed context '{}', switched to '{}'",
                            ctx_name, switched_to
                        ));
                    }
                    Ok(None) => {
                        output.emit_result(&format!("Destroyed context: {}", ctx_name));
                    }
                    Err(e) => return Err(e),
                }
            }
            did_action = true;
        }
        Command::ArchiveHistory { name } => {
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
                // Compute resolved config for the current context
                let resolved = app.resolve_config(None, None)?;
                api::compact_context_with_llm_manual(app, &resolved, verbose).await?;
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
            // Pass None for resolved_config - it will be resolved on demand if needed
            inspect_context(app, &ctx_name, thing, None)?;
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
            // Parse args - either JSON string or empty
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

            // Try built-in tools first, then plugins
            let result =
                if let Some(builtin_result) = tools::execute_builtin_tool(app, name, &args_json) {
                    builtin_result?
                } else if let Some(tool) = tools::find_tool(tools, name) {
                    tools::execute_tool(tool, &args_json, verbose)?
                } else {
                    return Err(io::Error::new(
                        ErrorKind::NotFound,
                        format!("Tool '{}' not found", name),
                    ));
                };

            output.emit_result(&result);
            did_action = true;
        }
        Command::ClearCache { name } => {
            let ctx_name = name
                .clone()
                .unwrap_or_else(|| app.state.current_context.clone());
            app.clear_tool_cache(&ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
            did_action = true;
        }
        Command::CleanupCache => {
            let resolved = app.resolve_config(None, None)?;
            let removed = app.cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
            output.emit_result(&format!(
                "Removed {} old cache entries (older than {} days)",
                removed, resolved.tool_cache_max_age_days
            ));
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

            let options = api::PromptOptions::new(
                verbose,
                use_reflection,
                json_output,
                input.flags.debug.as_ref(),
            );
            api::send_prompt(app, prompt.clone(), tools, &resolved, &options).await?;
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

    // Automatic cache cleanup (if enabled)
    let resolved = app.resolve_config(None, None)?;
    if resolved.auto_cleanup_cache {
        let removed = app.cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
        if removed > 0 {
            output.diagnostic(
                &format!(
                    "[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                    removed,
                    resolved.tool_cache_max_age_days + 1
                ),
                verbose,
            );
        }
    }

    // Check for no action and no prompt
    if !did_action && matches!(input.command, Command::NoOp) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "No operation specified",
        ));
    }

    Ok(())
}

/// Extract --home flag value from args (before full CLI parsing)
fn extract_home_override(args: &[String]) -> Option<std::path::PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--home" {
            return iter.next().map(std::path::PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--home=") {
            return Some(std::path::PathBuf::from(path));
        }
    }
    None
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Check for early flags (before full CLI parsing)
    let args: Vec<String> = std::env::args().collect();
    let is_json_config = args.iter().any(|a| a == "--json-config");
    let cli_json_output = args.iter().any(|a| a == "--json-output");
    let home_override = extract_home_override(&args);

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
        let mut app = AppState::load(home_override)?;

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
    let mut app = AppState::load(home_override)?;

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
