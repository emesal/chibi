mod api;
mod cli;
mod config;
mod context;
mod input;
mod json_input;
mod lock;
mod state;
mod tools;

use cli::Inspectable;
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
fn set_prompt_for_context(app: &AppState, context_name: &str, arg: &str, verbose: bool) -> io::Result<()> {
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

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = cli::parse()?;
    let verbose = cli.verbose;
    let mut app = AppState::load()?;

    // Load tools at startup
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

    // Execute on_start hook
    let hook_data = serde_json::json!({
        "current_context": app.state.current_context,
        "verbose": verbose,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::OnStart, &hook_data, verbose);

    // Track if we did an action (for determining if we should continue to prompt)
    let mut did_action = false;

    // Handle transient context (-C): use a different context for this invocation
    if let Some(ref name) = cli.transient_context {
        let actual_name = resolve_context_name(&app, name)?;
        let prev_context = app.state.current_context.clone();
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
            eprintln!("[Using transient context: {}]", app.state.current_context);
        }

        // Execute on_context_switch hook
        let hook_data = serde_json::json!({
            "from_context": prev_context,
            "to_context": app.state.current_context,
            "is_transient": true,
        });
        let _ = tools::execute_hook(
            &tools,
            tools::HookPoint::OnContextSwitch,
            &hook_data,
            verbose,
        );
    }

    // Handle persistent context switch (-c)
    if let Some(ref name) = cli.switch_context {
        let actual_name = resolve_context_name(&app, name)?;
        let prev_context = app.state.current_context.clone();
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
        if verbose {
            eprintln!("[Switched to context: {}]", app.state.current_context);
        }

        // Execute on_context_switch hook
        let hook_data = serde_json::json!({
            "from_context": prev_context,
            "to_context": app.state.current_context,
            "is_transient": false,
        });
        let _ = tools::execute_hook(
            &tools,
            tools::HookPoint::OnContextSwitch,
            &hook_data,
            verbose,
        );
        did_action = true;
    }

    // Handle archive current history (-a)
    if cli.archive_current_context {
        let context = app.get_current_context()?;
        let hook_data = serde_json::json!({
            "context_name": context.name,
            "message_count": context.messages.len(),
            "summary": context.summary,
        });
        let _ = tools::execute_hook(&tools, tools::HookPoint::PreClear, &hook_data, verbose);

        app.clear_context()?;
        if verbose {
            eprintln!("[Context archived (history saved to transcript)]");
        }

        let hook_data = serde_json::json!({
            "context_name": app.state.current_context,
        });
        let _ = tools::execute_hook(&tools, tools::HookPoint::PostClear, &hook_data, verbose);
        did_action = true;
    }

    // Handle archive other context's history (-A)
    if let Some(ref ctx_name) = cli.archive_context {
        app.clear_context_by_name(ctx_name)?;
        println!("Context '{}' archived (history saved to transcript)", ctx_name);
        did_action = true;
    }

    // Handle compact current context (-z)
    if cli.compact_current_context {
        api::compact_context_with_llm_manual(&app, verbose).await?;
        did_action = true;
    }

    // Handle compact other context (-Z)
    if let Some(ref ctx_name) = cli.compact_context {
        api::compact_context_by_name(&app, ctx_name, verbose).await?;
        println!("Context '{}' compacted", ctx_name);
        did_action = true;
    }

    // Handle rename current context (-r)
    if let Some(ref new_name) = cli.rename_current_context {
        let old_name = app.state.current_context.clone();
        app.rename_context(&old_name, new_name)?;
        if verbose {
            eprintln!("[Renamed context '{}' to '{}']", old_name, new_name);
        }
        did_action = true;
    }

    // Handle rename other context (-R)
    if let Some((ref old_name, ref new_name)) = cli.rename_context {
        app.rename_context(old_name, new_name)?;
        println!("Renamed context '{}' to '{}'", old_name, new_name);
        did_action = true;
    }

    // Handle set current system prompt (-y)
    if let Some(ref arg) = cli.set_current_system_prompt {
        set_prompt_for_context(&app, &app.state.current_context, arg, verbose)?;
        did_action = true;
    }

    // Handle set other context's system prompt (-Y)
    if let Some((ref ctx_name, ref arg)) = cli.set_system_prompt {
        set_prompt_for_context(&app, ctx_name, arg, verbose)?;
        println!("System prompt set for context '{}'", ctx_name);
        did_action = true;
    }

    // Handle set persistent username (-u)
    if let Some(ref username) = cli.set_username {
        let mut local_config = app.load_local_config(&app.state.current_context)?;
        local_config.username = Some(username.clone());
        app.save_local_config(&app.state.current_context, &local_config)?;
        if verbose {
            eprintln!(
                "[Username '{}' saved to context '{}']",
                username, app.state.current_context
            );
        }
        did_action = true;
    }

    // === Output-producing operations (these imply no_chibi) ===

    // Handle list current context info (-l)
    if cli.list_current_context {
        let context_name = &app.state.current_context;
        let context = app.get_current_context()?;
        let context_dir = app.context_dir(context_name);
        let status = lock::ContextLock::get_status(&context_dir, app.config.lock_heartbeat_seconds);
        let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();

        println!("Context: {}{}", context_name, status_str);
        println!("Messages: {}", context.messages.len());
        if !context.summary.is_empty() {
            println!("Summary: {}", context.summary.lines().next().unwrap_or(""));
        }

        // Show todos/goals if they exist
        let todos = app.load_todos_for(context_name)?;
        if !todos.is_empty() {
            let todo_lines: Vec<_> = todos.lines().take(3).collect();
            println!("Todos:");
            for line in todo_lines {
                println!("  {}", line);
            }
            if todos.lines().count() > 3 {
                println!("  ...");
            }
        }

        let goals = app.load_goals_for(context_name)?;
        if !goals.is_empty() {
            let goal_lines: Vec<_> = goals.lines().take(3).collect();
            println!("Goals:");
            for line in goal_lines {
                println!("  {}", line);
            }
            if goals.lines().count() > 3 {
                println!("  ...");
            }
        }
        did_action = true;
    }

    // Handle list all contexts (-L)
    if cli.list_contexts {
        let contexts = app.list_contexts();
        let current = &app.state.current_context;
        for name in contexts {
            let context_dir = app.context_dir(&name);
            let status =
                lock::ContextLock::get_status(&context_dir, app.config.lock_heartbeat_seconds);
            let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
            if &name == current {
                println!("* {}{}", name, status_str);
            } else {
                println!("  {}{}", name, status_str);
            }
        }
        did_action = true;
    }

    // Handle delete current context (-d)
    if cli.delete_current_context {
        let name = app.state.current_context.clone();
        match app.delete_context(&name) {
            Ok(true) => println!("Deleted context: {}", name),
            Ok(false) => println!("Context '{}' not found", name),
            Err(e) => return Err(e),
        }
        did_action = true;
    }

    // Handle delete other context (-D)
    if let Some(ref name) = cli.delete_context {
        match app.delete_context(name) {
            Ok(true) => println!("Deleted context: {}", name),
            Ok(false) => println!("Context '{}' not found", name),
            Err(e) => return Err(e),
        }
        did_action = true;
    }

    // Handle show current log (-g)
    if let Some(num) = cli.show_current_log {
        show_log(&app, &app.state.current_context, num, verbose)?;
        did_action = true;
    }

    // Handle show other context's log (-G)
    if let Some((ref ctx_name, num)) = cli.show_log {
        show_log(&app, ctx_name, num, verbose)?;
        did_action = true;
    }

    // Handle inspect current context (-n)
    if let Some(ref thing) = cli.inspect_current {
        inspect_context(&app, &app.state.current_context, thing)?;
        did_action = true;
    }

    // Handle inspect other context (-N)
    if let Some((ref ctx_name, ref thing)) = cli.inspect {
        inspect_context(&app, ctx_name, thing)?;
        did_action = true;
    }

    // Handle plugin invocation (-p)
    if let Some(ref invocation) = cli.plugin {
        let tool = tools::find_tool(&tools, &invocation.name).ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("Plugin '{}' not found", invocation.name),
            )
        })?;
        let args_json = serde_json::json!({ "args": invocation.args });
        let output = tools::execute_tool(tool, &args_json, verbose)?;
        print!("{}", output);
        did_action = true;
    }

    // Handle call-tool (-P)
    if let Some(ref invocation) = cli.call_tool {
        // First look for a plugin, then look for built-in tools
        let tool = tools::find_tool(&tools, &invocation.name);

        if let Some(tool) = tool {
            let args_json = serde_json::json!({ "args": invocation.args });
            let output = tools::execute_tool(tool, &args_json, verbose)?;
            print!("{}", output);
        } else {
            // Check for built-in tools
            match invocation.name.as_str() {
                "update_todos" | "update_goals" | "update_reflection" | "send_message" => {
                    // Built-in tools expect JSON args
                    let args_str = invocation.args.join(" ");
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

                    match invocation.name.as_str() {
                        "update_todos" => {
                            if let Some(content) = args_json.get("content").and_then(|v| v.as_str()) {
                                app.save_current_todos(content)?;
                                println!("Todos updated");
                            } else {
                                return Err(io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_todos requires {\"content\": \"...\"}",
                                ));
                            }
                        }
                        "update_goals" => {
                            if let Some(content) = args_json.get("content").and_then(|v| v.as_str()) {
                                app.save_current_goals(content)?;
                                println!("Goals updated");
                            } else {
                                return Err(io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_goals requires {\"content\": \"...\"}",
                                ));
                            }
                        }
                        "update_reflection" => {
                            if let Some(content) = args_json.get("content").and_then(|v| v.as_str()) {
                                app.save_reflection(content)?;
                                println!("Reflection updated");
                            } else {
                                return Err(io::Error::new(
                                    ErrorKind::InvalidInput,
                                    "update_reflection requires {\"content\": \"...\"}",
                                ));
                            }
                        }
                        "send_message" => {
                            let to = args_json.get("to").and_then(|v| v.as_str()).ok_or_else(|| {
                                io::Error::new(ErrorKind::InvalidInput, "send_message requires \"to\" field")
                            })?;
                            let message = args_json.get("message").and_then(|v| v.as_str()).ok_or_else(|| {
                                io::Error::new(ErrorKind::InvalidInput, "send_message requires \"message\" field")
                            })?;
                            app.send_inbox_message(to, message)?;
                            println!("Message sent to '{}'", to);
                        }
                        _ => unreachable!(),
                    }
                }
                _ => {
                    return Err(io::Error::new(
                        ErrorKind::NotFound,
                        format!("Tool '{}' not found", invocation.name),
                    ));
                }
            }
        }
        did_action = true;
    }

    // Now handle LLM invocation if should_invoke_llm() and we have a prompt
    if cli.should_invoke_llm() {
        // Resolve configuration
        let resolved = app.resolve_config(cli.set_username.as_deref(), cli.transient_username.as_deref())?;

        // Reflection is enabled by config (no longer a CLI flag)
        let use_reflection = app.config.reflection_enabled;

        // Build prompt from args and/or stdin
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
            // Interactive: read from terminal (skip if we already did an action)
            (false, None) => {
                if did_action {
                    // We already did something, don't wait for interactive input
                    String::new()
                } else {
                    read_prompt_interactive()?
                }
            }
        };

        if !prompt.trim().is_empty() {
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

            // Acquire context lock
            let context_dir = app.context_dir(&app.state.current_context);
            let _lock = lock::ContextLock::acquire(&context_dir, app.config.lock_heartbeat_seconds)?;

            api::send_prompt(&app, prompt, &tools, verbose, use_reflection, &resolved).await?;
        } else if !did_action {
            // No prompt and no action - this is an error
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Prompt cannot be empty",
            ));
        }
    }

    // Execute on_end hook
    let hook_data = serde_json::json!({
        "current_context": app.state.current_context,
    });
    let _ = tools::execute_hook(&tools, tools::HookPoint::OnEnd, &hook_data, verbose);

    Ok(())
}
