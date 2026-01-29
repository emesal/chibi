// chibi-cli: CLI frontend for chibi
// Argument parsing, markdown rendering, TTY handling

mod cli;
mod config;
mod image_cache;
mod json_input;
mod markdown;
mod output;
mod sink;

// Re-export key types for use by other modules
pub use cli::{parse, Cli, InspectableExt, PluginInvocation};
pub use config::{
    default_markdown_style, load_cli_config, ConfigImageRenderMode, ImageAlignment, ImageConfig,
    ImageConfigOverride, MarkdownStyle, ResolvedConfig,
};
pub use json_input::from_str as parse_json_input;
pub use markdown::{MarkdownConfig, MarkdownStream};
pub use output::OutputHandler;
pub use sink::CliResponseSink;

use chibi_core::context::{
    Context, ENTRY_TYPE_MESSAGE, ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT,
};
use chibi_core::input::{ChibiInput, Command, ContextSelection, DebugKey, UsernameOverride};
use chibi_core::{api, tools, Chibi, Inspectable, PromptOptions};
use std::io::{self, ErrorKind, IsTerminal, Read, Write};
use std::path::PathBuf;

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

/// Render markdown content to stdout if appropriate.
fn render_markdown_output(content: &str, config: MarkdownConfig) -> io::Result<()> {
    let mut md = MarkdownStream::new(config);
    md.write_chunk(content)?;
    md.finish()?;
    Ok(())
}

/// Build a MarkdownConfig from a ResolvedConfig.
fn md_config_from_resolved(
    config: &ResolvedConfig,
    chibi_dir: &std::path::Path,
    force_render: bool,
) -> MarkdownConfig {
    MarkdownConfig::from_resolved(config, chibi_dir, force_render)
}

/// Build a MarkdownConfig with safe defaults (used when no config is loaded).
fn md_config_defaults(render: bool) -> MarkdownConfig {
    MarkdownConfig {
        render_markdown: render,
        force_render: false,
        image: ImageConfig::default(),
        image_cache_dir: None,
        markdown_style: default_markdown_style(),
    }
}

/// Generate a unique context name for `-c new` or `-c new:prefix`
/// Format: [prefix_]YYYYMMDD_HHMMSS[_N]
fn generate_new_context_name(chibi: &Chibi, prefix: Option<&str>) -> String {
    use chrono::Local;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let base_name = match prefix {
        Some(p) => format!("{}_{}", p, timestamp),
        None => timestamp,
    };

    // Check for collisions
    let existing = chibi.list_contexts();
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

/// Resolve previous context reference
fn resolve_previous_context(chibi: &Chibi) -> io::Result<String> {
    chibi
        .app
        .state
        .previous_context
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                "No previous context available (use -c to switch contexts first)",
            )
        })
}

/// Resolve "new" or "new:prefix" or "-" context names
fn resolve_context_name(chibi: &Chibi, name: &str) -> io::Result<String> {
    if name == "-" {
        resolve_previous_context(chibi)
    } else if name == "new" {
        Ok(generate_new_context_name(chibi, None))
    } else if let Some(prefix) = name.strip_prefix("new:") {
        if prefix.is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Prefix cannot be empty in 'new:prefix'",
            ));
        }
        Ok(generate_new_context_name(chibi, Some(prefix)))
    } else {
        Ok(name.to_string())
    }
}

/// Build CLI ResolvedConfig from Chibi facade.
/// Combines core config with presentation settings from cli.toml.
fn resolve_cli_config(
    chibi: &Chibi,
    persistent_username: Option<&str>,
    transient_username: Option<&str>,
) -> io::Result<ResolvedConfig> {
    let core = chibi.resolve_config(persistent_username, transient_username)?;
    let cli = load_cli_config(chibi.home_dir(), Some(chibi.current_context_name()))?;

    Ok(ResolvedConfig {
        core,
        render_markdown: cli.render_markdown,
        image: cli.image,
        markdown_style: cli.markdown_style,
    })
}

fn inspect_context(
    chibi: &Chibi,
    context_name: &str,
    thing: &Inspectable,
    resolved_config: Option<&ResolvedConfig>,
    force_markdown: bool,
) -> io::Result<()> {
    // Resolve config if not provided
    let config_holder;
    let config = if let Some(cfg) = resolved_config {
        cfg
    } else {
        config_holder = resolve_cli_config(chibi, None, None)?;
        &config_holder
    };

    match thing {
        Inspectable::List => {
            println!("Inspectable items:");
            for name in <Inspectable as InspectableExt>::all_names_cli() {
                println!("  {}", name);
            }
        }
        Inspectable::SystemPrompt => {
            let prompt = chibi.app.load_system_prompt_for(context_name)?;
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
            let reflection = chibi.app.load_reflection()?;
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
            let todos = chibi.app.load_todos_for(context_name)?;
            if todos.is_empty() {
                println!("(no todos)");
            } else {
                let md_cfg = md_config_from_resolved(config, chibi.home_dir(), force_markdown);
                render_markdown_output(&todos, md_cfg)?;
                if !todos.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Goals => {
            let goals = chibi.app.load_goals_for(context_name)?;
            if goals.is_empty() {
                println!("(no goals)");
            } else {
                let md_cfg = md_config_from_resolved(config, chibi.home_dir(), force_markdown);
                render_markdown_output(&goals, md_cfg)?;
                if !goals.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Home => {
            println!("{}", chibi.home_dir().display());
        }
        Inspectable::ConfigField(field_path) => match config.get_field(field_path) {
            Some(value) => println!("{}", value),
            None => println!("(not set)"),
        },
    }
    Ok(())
}

/// Show log entries for a context
fn show_log(
    chibi: &Chibi,
    context_name: &str,
    num: isize,
    verbose: bool,
    resolved_config: &ResolvedConfig,
    force_markdown: bool,
) -> io::Result<()> {
    let entries = chibi.app.read_jsonl_transcript(context_name)?;

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
                println!("[{}]", entry.from.to_uppercase());
                let md_cfg =
                    md_config_from_resolved(resolved_config, chibi.home_dir(), force_markdown);
                render_markdown_output(&entry.content, md_cfg)?;
                println!();
            }
            ENTRY_TYPE_TOOL_CALL => {
                if verbose {
                    println!("[TOOL CALL: {}]\n{}\n", entry.to, entry.content);
                } else {
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
                    println!("[TOOL RESULT: {}]\n{}\n", entry.from, entry.content);
                } else {
                    let size = entry.content.len();
                    let size_str = if size > 1024 {
                        format!("{:.1}kb", size as f64 / 1024.0)
                    } else {
                        format!("{}b", size)
                    };
                    println!("  -> {}", size_str);
                }
            }
            "compaction" => {
                if verbose {
                    println!("[COMPACTION]: {}\n", entry.content);
                }
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
    chibi: &Chibi,
    context_name: &str,
    arg: &str,
    verbose: bool,
) -> io::Result<()> {
    let content = if std::path::Path::new(arg).is_file() {
        std::fs::read_to_string(arg)?
    } else {
        arg.to_string()
    };
    chibi.app.set_system_prompt_for(context_name, &content)?;
    if verbose {
        eprintln!("[System prompt set for context '{}']", context_name);
    }
    Ok(())
}

/// Execute from ChibiInput
async fn execute_from_input(
    input: ChibiInput,
    chibi: &mut Chibi,
    output: &OutputHandler,
    force_markdown: bool,
) -> io::Result<()> {
    let verbose = input.flags.verbose;
    let json_output = input.flags.json_output;

    // Execute on_start hook
    let hook_data = serde_json::json!({
        "current_context": chibi.app.state.current_context,
        "verbose": verbose,
    });
    let _ = tools::execute_hook(&chibi.tools, tools::HookPoint::OnStart, &hook_data, verbose);

    // Auto-destroy expired contexts
    let destroyed = chibi.app.auto_destroy_expired_contexts(verbose)?;
    if !destroyed.is_empty() {
        chibi.save()?;
        output.diagnostic(
            &format!("[Auto-destroyed {} expired context(s)]", destroyed.len()),
            verbose,
        );
    }

    let mut did_action = false;

    // Handle context selection
    match &input.context {
        ContextSelection::Current => {}
        ContextSelection::Transient { name } => {
            let actual_name = resolve_context_name(chibi, name)?;
            let prev_context = chibi.app.state.current_context.clone();
            chibi.switch_context(&actual_name)?;
            output.diagnostic(
                &format!("[Using transient context: {}]", actual_name),
                verbose,
            );
            let hook_data = serde_json::json!({
                "from_context": prev_context,
                "to_context": actual_name,
                "is_transient": true,
            });
            let _ = tools::execute_hook(
                &chibi.tools,
                tools::HookPoint::OnContextSwitch,
                &hook_data,
                verbose,
            );
        }
        ContextSelection::Switch { name, persistent } => {
            let prev_context = chibi.app.state.current_context.clone();
            if name == "-" {
                let previous = resolve_previous_context(chibi)?;
                chibi.app.state.current_context = previous;
                chibi.app.state.previous_context = Some(prev_context.clone());
            } else {
                let actual_name = resolve_context_name(chibi, name)?;
                chibi.switch_context(&actual_name)?;
            }

            if *persistent {
                chibi.save()?;
            }
            output.diagnostic(
                &format!("[Switched to context: {}]", chibi.app.state.current_context),
                verbose,
            );
            let hook_data = serde_json::json!({
                "from_context": prev_context,
                "to_context": chibi.app.state.current_context,
                "is_transient": !persistent,
            });
            let _ = tools::execute_hook(
                &chibi.tools,
                tools::HookPoint::OnContextSwitch,
                &hook_data,
                verbose,
            );
            did_action = true;
        }
    }

    // Touch the current context
    let current_ctx = chibi.app.state.current_context.clone();
    let debug_destroy_at = input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAt(ts) => Some(*ts),
        _ => None,
    });
    let debug_destroy_after = input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
        _ => None,
    });
    if chibi.app.touch_context_with_destroy_settings(
        &current_ctx,
        debug_destroy_at,
        debug_destroy_after,
    )? {
        chibi.save()?;
    }

    // Handle username override
    if let Some(ref override_) = input.username_override {
        match override_ {
            UsernameOverride::Persistent(username) => {
                let mut local_config = chibi.app.load_local_config(&current_ctx)?;
                local_config.username = Some(username.clone());
                chibi.app.save_local_config(&current_ctx, &local_config)?;
                output.diagnostic(
                    &format!("[Username '{}' saved to context '{}']", username, current_ctx),
                    verbose,
                );
                did_action = true;
            }
            UsernameOverride::Transient(_) => {
                // Applied via resolve_config later
            }
        }
    }

    // Execute command
    match &input.command {
        Command::ShowHelp => {
            Cli::print_help();
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi {}", env!("CARGO_PKG_VERSION")));
        }
        Command::ListContexts => {
            let contexts = chibi.list_contexts();
            let current = chibi.current_context_name();
            for name in contexts {
                let context_dir = chibi.app.context_dir(&name);
                let status = chibi_core::lock::ContextLock::get_status(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                );
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                if name == current {
                    output.emit_result(&format!("* {}{}", name, status_str));
                } else {
                    output.emit_result(&format!("  {}{}", name, status_str));
                }
            }
            did_action = true;
        }
        Command::ListCurrentContext => {
            let context_name = chibi.current_context_name();
            let context = chibi.current_context()?;
            let context_dir = chibi.app.context_dir(context_name);
            let status = chibi_core::lock::ContextLock::get_status(
                &context_dir,
                chibi.app.config.lock_heartbeat_seconds,
            );
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
            let ctx_name = match name {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };

            if !chibi.app.context_dir(&ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else if !confirm_action(&format!("Destroy context '{}'?", ctx_name)) {
                output.emit_result("Aborted");
            } else {
                match chibi.app.destroy_context(&ctx_name) {
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
            let ctx_name = match name {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            if name.is_none() {
                let context = chibi.current_context()?;
                let hook_data = serde_json::json!({
                    "context_name": context.name,
                    "message_count": context.messages.len(),
                    "summary": context.summary,
                });
                let _ = tools::execute_hook(
                    &chibi.tools,
                    tools::HookPoint::PreClear,
                    &hook_data,
                    verbose,
                );
                chibi.app.clear_context()?;
                let hook_data = serde_json::json!({
                    "context_name": chibi.current_context_name(),
                });
                let _ = tools::execute_hook(
                    &chibi.tools,
                    tools::HookPoint::PostClear,
                    &hook_data,
                    verbose,
                );
            } else {
                chibi.app.clear_context_by_name(&ctx_name)?;
            }
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)",
                ctx_name
            ));
            did_action = true;
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                let resolved_name = resolve_context_name(chibi, ctx_name)?;
                api::compact_context_by_name(&chibi.app, &resolved_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                let resolved = chibi.resolve_config(None, None)?;
                api::compact_context_with_llm_manual(&chibi.app, &resolved, verbose).await?;
            }
            did_action = true;
        }
        Command::RenameContext { old, new } => {
            let old_name = match old {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            chibi.app.rename_context(&old_name, new)?;
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
            did_action = true;
        }
        Command::ShowLog { context, count } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            let config = resolve_cli_config(chibi, None, None)?;
            show_log(chibi, &ctx_name, *count, verbose, &config, force_markdown)?;
            did_action = true;
        }
        Command::Inspect { context, thing } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            inspect_context(chibi, &ctx_name, thing, None, force_markdown)?;
            did_action = true;
        }
        Command::SetSystemPrompt { context, prompt } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            set_prompt_for_context(chibi, &ctx_name, prompt, verbose)?;
            did_action = true;
        }
        Command::RunPlugin { name, args } => {
            let tool = tools::find_tool(&chibi.tools, name).ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, format!("Plugin '{}' not found", name))
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = tools::execute_tool(tool, &args_json, verbose)?;
            output.emit_result(&result);
            did_action = true;
        }
        Command::CallTool { name, args } => {
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

            let result = chibi.execute_tool(name, args_json)?;
            output.emit_result(&result);
            did_action = true;
        }
        Command::ClearCache { name } => {
            let ctx_name = match name {
                Some(n) => resolve_context_name(chibi, n)?,
                None => chibi.current_context_name().to_string(),
            };
            chibi.app.clear_tool_cache(&ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
            did_action = true;
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(None, None)?;
            let removed = chibi
                .app
                .cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
            output.emit_result(&format!(
                "Removed {} old cache entries (older than {} days)",
                removed, resolved.tool_cache_max_age_days
            ));
            did_action = true;
        }
        Command::SendPrompt { prompt } => {
            // Ensure context exists
            let ctx_name = chibi.current_context_name().to_string();
            if !chibi.app.context_dir(&ctx_name).exists() {
                let new_context = Context::new(ctx_name.clone());
                chibi.app.save_current_context(&new_context)?;
            }

            // Resolve config with runtime overrides
            let (persistent_username, transient_username) = match &input.username_override {
                Some(UsernameOverride::Persistent(u)) => (Some(u.as_str()), None),
                Some(UsernameOverride::Transient(u)) => (None, Some(u.as_str())),
                None => (None, None),
            };
            let mut resolved = resolve_cli_config(chibi, persistent_username, transient_username)?;
            if input.flags.raw {
                resolved.render_markdown = false;
            }
            let use_reflection = chibi.app.config.reflection_enabled;

            // Acquire context lock
            let context_dir = chibi.app.context_dir(&ctx_name);
            let _lock = chibi_core::lock::ContextLock::acquire(
                &context_dir,
                chibi.app.config.lock_heartbeat_seconds,
            )?;

            let options = PromptOptions::new(
                verbose,
                use_reflection,
                json_output,
                &input.flags.debug,
                force_markdown,
            );

            // Create markdown stream if enabled
            let markdown = if resolved.render_markdown && !input.flags.raw {
                let md_cfg = md_config_from_resolved(&resolved, chibi.home_dir(), force_markdown);
                Some(MarkdownStream::new(md_cfg))
            } else {
                None
            };

            let mut sink = CliResponseSink::new(output, markdown, verbose);
            chibi
                .send_prompt_streaming(prompt, &resolved.core, &options, &mut sink)
                .await?;
            did_action = true;
        }
        Command::NoOp => {
            // No operation - just context switch, already handled above
        }
    }

    // Execute on_end hook
    let hook_data = serde_json::json!({
        "current_context": chibi.current_context_name(),
    });
    let _ = tools::execute_hook(&chibi.tools, tools::HookPoint::OnEnd, &hook_data, verbose);

    // Automatic cache cleanup
    let resolved = chibi.resolve_config(None, None)?;
    if resolved.auto_cleanup_cache {
        let removed = chibi
            .app
            .cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
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

    // Image cache cleanup
    let cli_config = resolve_cli_config(chibi, None, None)?;
    if cli_config.image.cache_enabled {
        let image_cache_dir = chibi.home_dir().join("image_cache");
        match image_cache::cleanup_image_cache(
            &image_cache_dir,
            cli_config.image.cache_max_bytes,
            cli_config.image.cache_max_age_days,
        ) {
            Ok(removed) if removed > 0 => {
                output.diagnostic(
                    &format!(
                        "[Image cache cleanup: removed {} entries (max {} days, max {} MB)]",
                        removed,
                        cli_config.image.cache_max_age_days,
                        cli_config.image.cache_max_bytes / (1024 * 1024),
                    ),
                    verbose,
                );
            }
            _ => {}
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
fn extract_home_override(args: &[String]) -> Option<PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--home" {
            return iter.next().map(PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--home=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Check for early flags (before full CLI parsing)
    let args: Vec<String> = std::env::args().collect();

    // --json-schema: print the JSON schema for --json-config input and exit immediately
    if args.iter().any(|a| a == "--json-schema") {
        let schema = schemars::schema_for!(ChibiInput);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
        return Ok(());
    }

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
            Cli::print_help();
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
        let mut chibi = match home_override {
            Some(path) => Chibi::from_home(&path)?,
            None => Chibi::load()?,
        };

        output.diagnostic(
            &format!("[Loaded {} tool(s)]", chibi.tool_count()),
            input.flags.verbose,
        );

        return execute_from_input(input, &mut chibi, &output, false).await;
    }

    // CLI mode: parse to ChibiInput and use unified execution
    let input = cli::parse()?;

    // Handle --debug md=<FILENAME> early (renders markdown and quits, implies -x)
    if let Some(path) = input.flags.debug.iter().find_map(|k| match k {
        DebugKey::Md(p) => Some(p),
        _ => None,
    }) {
        let content = std::fs::read_to_string(path).map_err(|e| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("Failed to read file '{}': {}", path, e),
            )
        })?;
        let force_render = input
            .flags
            .debug
            .iter()
            .any(|k| matches!(k, DebugKey::ForceMarkdown));
        let mut md_cfg = md_config_defaults(true);
        md_cfg.force_render = force_render;
        render_markdown_output(&content, md_cfg)?;
        return Ok(());
    }

    // Handle --debug force-markdown
    let force_markdown = input
        .flags
        .debug
        .iter()
        .any(|k| matches!(k, DebugKey::ForceMarkdown));

    let verbose = input.flags.verbose;
    let mut chibi = match home_override {
        Some(path) => Chibi::from_home(&path)?,
        None => Chibi::load()?,
    };

    // Reload tools if verbose (to get the list for display)
    if verbose {
        chibi.reload_tools_verbose()?;
        if !chibi.tools.is_empty() {
            eprintln!(
                "[Loaded {} tool(s): {}]",
                chibi.tool_count(),
                chibi
                    .tools
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    let output = OutputHandler::new(input.flags.json_output);

    execute_from_input(input, &mut chibi, &output, force_markdown).await
}
