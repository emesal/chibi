// chibi-cli: CLI frontend for chibi
// Argument parsing, markdown rendering, TTY handling

mod cli;
mod config;
mod image_cache;
mod input;
mod markdown;
mod output;
mod session;
mod sink;

// Re-export key types for use by other modules
pub use cli::{Cli, InspectableExt, PluginInvocation, parse};
pub use config::{
    ConfigImageRenderMode, ImageAlignment, ImageConfig, ImageConfigOverride, MarkdownStyle,
    ResolvedConfig, default_markdown_style, load_cli_config,
};
pub use markdown::{MarkdownConfig, MarkdownStream};
pub use output::OutputHandler;
pub use session::Session;
pub use sink::CliResponseSink;

use chibi_core::context::{
    Context, ContextEntry, ENTRY_TYPE_MESSAGE, ENTRY_TYPE_TOOL_CALL, ENTRY_TYPE_TOOL_RESULT,
    now_timestamp,
};
use chibi_core::input::{Command, DebugKey};

use crate::input::{ChibiInput, ContextSelection, UsernameOverride};
use chibi_core::{
    Chibi, Inspectable, LoadOptions, OutputSink, PermissionHandler, PromptOptions, StatePaths, api,
    tools,
};
use std::io::{self, ErrorKind, IsTerminal, Write};
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

/// Build the interactive permission handler for gated operations.
///
/// Prompts the user via `/dev/tty` (not stdin, which may be piped) for Y/n
/// confirmation on file writes and shell execution. Default-allow on Enter
/// (empty input). Returns fail-safe deny if no TTY is available.
fn build_interactive_permission_handler() -> PermissionHandler {
    Box::new(|hook_data: &serde_json::Value| {
        use chibi_core::json_ext::JsonExt;

        let tool_name = hook_data.get_str_or("tool_name", "unknown");
        let display = hook_data
            .get_str("path")
            .or_else(|| hook_data.get_str("command"))
            .unwrap_or("(no details)");

        eprint!("[{}] {} [Y/n] ", tool_name, display);
        io::stderr().flush().ok();

        // Read from /dev/tty so piped stdin doesn't interfere
        let approved = match std::fs::File::open("/dev/tty") {
            Ok(tty) => {
                let mut reader = io::BufReader::new(tty);
                let mut response = String::new();
                if io::BufRead::read_line(&mut reader, &mut response).is_ok() {
                    // Default-allow: only deny on explicit "n" or "no"
                    !matches!(response.trim().to_lowercase().as_str(), "n" | "no")
                } else {
                    false
                }
            }
            Err(_) => false, // no TTY = fail-safe deny
        };

        Ok(approved)
    })
}

/// Build a trust-mode permission handler that auto-approves all operations.
///
/// Used with `-t`/`--trust` for headless/automation scenarios where all
/// permission-gated tools should execute without prompting.
fn build_trust_permission_handler() -> PermissionHandler {
    Box::new(|_hook_data: &serde_json::Value| Ok(true))
}

/// Select the appropriate permission handler based on trust mode.
fn select_permission_handler(trust: bool) -> PermissionHandler {
    if trust {
        build_trust_permission_handler()
    } else {
        build_interactive_permission_handler()
    }
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

/// Resolve "new" or "new:prefix" or "-" context names
fn resolve_context_name(chibi: &Chibi, session: &Session, name: &str) -> io::Result<String> {
    if name == "-" {
        session.get_previous()
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
///
/// When `context_window_limit` is unset (0), resolves it from ratatoskr's
/// model registry via a synchronous (no-network) lookup.
fn resolve_cli_config(
    chibi: &Chibi,
    context_name: &str,
    username_override: Option<&str>,
) -> io::Result<ResolvedConfig> {
    let mut core = chibi.resolve_config(context_name, username_override)?;
    let cli = load_cli_config(chibi.home_dir(), Some(context_name))?;

    // Resolve context_window_limit from ratatoskr registry if still unknown
    if core.context_window_limit == 0
        && let Ok(gateway) = chibi_core::gateway::build_gateway(&core)
    {
        chibi_core::gateway::resolve_context_window(&mut core, &gateway);
    }

    Ok(ResolvedConfig {
        core,
        render_markdown: cli.render_markdown,
        show_thinking: cli.show_thinking,
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
    output: &dyn OutputSink,
) -> io::Result<()> {
    // Resolve config if not provided
    let config_holder;
    let config = if let Some(cfg) = resolved_config {
        cfg
    } else {
        config_holder = resolve_cli_config(chibi, context_name, None)?;
        &config_holder
    };

    match thing {
        Inspectable::List => {
            output.emit_result("Inspectable items:");
            for name in <Inspectable as InspectableExt>::all_names_cli() {
                output.emit_result(&format!("  {}", name));
            }
        }
        Inspectable::SystemPrompt => {
            let prompt = chibi.app.load_system_prompt_for(context_name)?;
            if prompt.is_empty() {
                output.emit_result("(no system prompt set)");
            } else {
                output.emit_result(prompt.trim_end());
            }
        }
        Inspectable::Reflection => {
            let reflection = chibi.app.load_reflection()?;
            if reflection.is_empty() {
                output.emit_result("(no reflection set)");
            } else {
                output.emit_result(reflection.trim_end());
            }
        }
        Inspectable::Todos => {
            let todos = chibi.app.load_todos_for(context_name)?;
            if todos.is_empty() {
                output.emit_result("(no todos)");
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
                output.emit_result("(no goals)");
            } else {
                let md_cfg = md_config_from_resolved(config, chibi.home_dir(), force_markdown);
                render_markdown_output(&goals, md_cfg)?;
                if !goals.ends_with('\n') {
                    println!();
                }
            }
        }
        Inspectable::Home => {
            output.emit_result(&chibi.home_dir().display().to_string());
        }
        Inspectable::ConfigField(field_path) => match config.get_field(field_path) {
            Some(value) => output.emit_result(&value.to_string()),
            None => output.emit_result("(not set)"),
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
    output: &dyn OutputSink,
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
                output.emit_result(&format!("[{}]", entry.from.to_uppercase()));
                let md_cfg =
                    md_config_from_resolved(resolved_config, chibi.home_dir(), force_markdown);
                render_markdown_output(&entry.content, md_cfg)?;
                output.newline();
            }
            ENTRY_TYPE_TOOL_CALL => {
                if verbose {
                    output.emit_result(&format!("[TOOL CALL: {}]\n{}\n", entry.to, entry.content));
                } else {
                    let args_preview = if entry.content.len() > 60 {
                        format!("{}...", &entry.content[..60])
                    } else {
                        entry.content.clone()
                    };
                    output.emit_result(&format!("[TOOL: {}] {}", entry.to, args_preview));
                }
            }
            ENTRY_TYPE_TOOL_RESULT => {
                if verbose {
                    output.emit_result(&format!(
                        "[TOOL RESULT: {}]\n{}\n",
                        entry.from, entry.content
                    ));
                } else {
                    let size = entry.content.len();
                    let size_str = if size > 1024 {
                        format!("{:.1}kb", size as f64 / 1024.0)
                    } else {
                        format!("{}b", size)
                    };
                    output.emit_result(&format!("  -> {}", size_str));
                }
            }
            "compaction" => {
                if verbose {
                    output.emit_result(&format!("[COMPACTION]: {}\n", entry.content));
                }
            }
            _ => {
                if verbose {
                    output.emit_result(&format!(
                        "[{}]: {}\n",
                        entry.entry_type.to_uppercase(),
                        entry.content
                    ));
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
    session: &mut Session,
    output: &dyn OutputSink,
    force_markdown: bool,
) -> io::Result<()> {
    let verbose = input.flags.verbose;
    let show_tool_calls = !input.flags.hide_tool_calls || verbose;
    let show_thinking_flag = input.flags.show_thinking || verbose;
    // Initialize session (executes OnStart hooks)
    let _ = chibi.init();

    // Auto-destroy expired contexts
    let destroyed = chibi.app.auto_destroy_expired_contexts(verbose)?;
    if !destroyed.is_empty() {
        chibi.save()?;
        // If our session points to a destroyed context, reset to default
        if destroyed.contains(&session.implied_context) {
            session.implied_context = "default".to_string();
            session.previous_context = None;
            session.save(chibi.home_dir())?;
        }
        output.diagnostic(
            &format!("[Auto-destroyed {} expired context(s)]", destroyed.len()),
            verbose,
        );
    }

    let mut did_action = false;

    // Handle context selection and determine working_context
    // - working_context: the context we're actually operating on this invocation
    // - implied_context: persisted in session.json, what you get when no context is specified
    let working_context = match &input.context {
        ContextSelection::Current => session.implied_context.clone(),
        ContextSelection::Ephemeral { name } => {
            // Ephemeral: use context directly WITHOUT mutating session
            let actual_name = resolve_context_name(chibi, session, name)?;
            chibi.app.ensure_context_dir(&actual_name)?;
            output.diagnostic(
                &format!("[Using ephemeral context: {}]", actual_name),
                verbose,
            );
            actual_name
        }
        ContextSelection::Switch { name, persistent } => {
            if name == "-" {
                session.swap_with_previous()?;
            } else {
                let actual_name = resolve_context_name(chibi, session, name)?;
                session.switch_context(actual_name);
            }
            chibi.app.ensure_context_dir(&session.implied_context)?;

            if *persistent {
                session.save(chibi.home_dir())?;
                chibi.save()?;
            }
            output.diagnostic(
                &format!("[Switched to context: {}]", &session.implied_context),
                verbose,
            );
            did_action = true;
            session.implied_context.clone()
        }
    };

    // Touch the working context
    let current_ctx = working_context.clone();
    let debug_destroy_at = input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAt(ts) => Some(*ts),
        _ => None,
    });
    let debug_destroy_after = input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
        _ => None,
    });
    // Ensure ContextEntry exists before applying destroy settings (fix for #47 regression)
    if !chibi
        .app
        .state
        .contexts
        .iter()
        .any(|e| e.name == current_ctx)
    {
        chibi.app.state.contexts.push(ContextEntry::with_created_at(
            current_ctx.clone(),
            now_timestamp(),
        ));
    }
    if chibi.app.touch_context_with_destroy_settings(
        &current_ctx,
        debug_destroy_at,
        debug_destroy_after,
    )? {
        chibi.save()?;
    }

    // Handle username override
    // Persistent (-u) is saved to local.toml; ephemeral (-U) is used for this invocation only
    let ephemeral_username: Option<&str> = match &input.username_override {
        Some(UsernameOverride::Persistent(username)) => {
            let mut local_config = chibi.app.load_local_config(&current_ctx)?;
            local_config.username = Some(username.clone());
            chibi.app.save_local_config(&current_ctx, &local_config)?;
            output.diagnostic(
                &format!(
                    "[Username '{}' saved to context '{}']",
                    username, current_ctx
                ),
                verbose,
            );
            did_action = true;
            None // persistent was saved, no runtime override needed
        }
        Some(UsernameOverride::Ephemeral(username)) => Some(username.as_str()),
        None => None,
    };

    // SYNC: chibi-json also dispatches commands — check crates/chibi-json/src/main.rs
    match &input.command {
        Command::ShowHelp => {
            Cli::print_help();
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi {}", env!("CARGO_PKG_VERSION")));
        }
        Command::ListContexts => {
            let contexts = chibi.list_contexts();
            let implied = &session.implied_context;
            for name in contexts {
                let context_dir = chibi.app.context_dir(&name);
                let status = chibi_core::lock::ContextLock::get_status(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                );
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                if &name == implied {
                    output.emit_result(&format!("* {}{}", name, status_str));
                } else {
                    output.emit_result(&format!("  {}{}", name, status_str));
                }
            }
            did_action = true;
        }
        Command::ListCurrentContext => {
            let context_name = &working_context;
            let context = chibi.app.get_or_create_context(context_name)?;
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
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };

            if !chibi.app.context_dir(&ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else if !output.confirm(&format!("Destroy context '{}'?", ctx_name)) {
                output.emit_result("Aborted");
            } else {
                // Handle session fallback if destroying current context
                if session
                    .handle_context_destroyed(&ctx_name, |name| {
                        chibi.app.context_dir(name).exists()
                    })
                    .is_some()
                {
                    session.save(chibi.home_dir())?;
                }
                chibi.app.destroy_context(&ctx_name)?;
                output.emit_result(&format!("Destroyed context: {}", ctx_name));
            }
            did_action = true;
        }
        Command::ArchiveHistory { name } => {
            let ctx_name = match name {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };
            if name.is_none() {
                // Use wrapper with hooks for current context
                chibi.clear_context(&ctx_name)?;
            } else {
                // For named contexts, just clear without hooks (hooks are for interactive use)
                chibi.app.clear_context(&ctx_name)?;
            }
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)",
                ctx_name
            ));
            did_action = true;
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                let resolved_name = resolve_context_name(chibi, session, ctx_name)?;
                api::compact_context_by_name(&chibi.app, &resolved_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                let resolved = chibi.resolve_config(&working_context, None)?;
                api::compact_context_with_llm_manual(
                    &chibi.app,
                    &working_context,
                    &resolved,
                    verbose,
                )
                .await?;
            }
            did_action = true;
        }
        Command::RenameContext { old, new } => {
            let old_name = match old {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };
            chibi.app.rename_context(&old_name, new)?;
            // Update session if we renamed the implied context
            if session.implied_context == old_name {
                session.implied_context = new.clone();
                session.save(chibi.home_dir())?;
            }
            // Also update previous_context if needed
            if session.previous_context.as_deref() == Some(&old_name) {
                session.previous_context = Some(new.clone());
                session.save(chibi.home_dir())?;
            }
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
            did_action = true;
        }
        Command::ShowLog { context, count } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };
            let config = resolve_cli_config(chibi, &ctx_name, None)?;
            show_log(
                chibi,
                &ctx_name,
                *count,
                verbose,
                &config,
                force_markdown,
                output,
            )?;
            did_action = true;
        }
        Command::Inspect { context, thing } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };
            // Pass ephemeral username so -U works with -n
            let config = resolve_cli_config(chibi, &ctx_name, ephemeral_username)?;
            inspect_context(
                chibi,
                &ctx_name,
                thing,
                Some(&config),
                force_markdown,
                output,
            )?;
            did_action = true;
        }
        Command::SetSystemPrompt { context, prompt } => {
            let ctx_name = match context {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
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

            let result = chibi.execute_tool(&working_context, name, args_json.clone())?;

            if input.flags.force_call_agent {
                // Tool-first with continuation to LLM
                let tool_context = format!(
                    "[User initiated tool call: {}]\n[Arguments: {}]\n[Result: {}]",
                    name, args_json, result
                );

                let mut resolved = resolve_cli_config(chibi, &working_context, ephemeral_username)?;
                if input.raw {
                    resolved.render_markdown = false;
                }
                if input.flags.no_tool_calls {
                    resolved.core.no_tool_calls = true;
                }
                let use_reflection = resolved.core.reflection_enabled;

                let context_dir = chibi.app.context_dir(&working_context);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;

                let fallback = chibi_core::tools::HandoffTarget::Agent {
                    prompt: String::new(),
                };
                let options = PromptOptions::new(
                    verbose,
                    use_reflection,
                    &input.flags.debug,
                    force_markdown,
                )
                .with_fallback(fallback);

                let md_config = if resolved.render_markdown && !input.raw {
                    Some(md_config_from_resolved(
                        &resolved,
                        chibi.home_dir(),
                        force_markdown,
                    ))
                } else {
                    None
                };

                let mut sink = CliResponseSink::new(
                    output,
                    md_config,
                    verbose,
                    show_tool_calls,
                    show_thinking_flag || resolved.show_thinking,
                );
                chibi
                    .send_prompt_streaming(
                        &working_context,
                        &tool_context,
                        &resolved.core,
                        &options,
                        &mut sink,
                    )
                    .await?;
            } else {
                // Tool-first with immediate return (default for -P)
                output.emit_result(&result);
            }
            did_action = true;
        }
        Command::ClearCache { name } => {
            let ctx_name = match name {
                Some(n) => resolve_context_name(chibi, session, n)?,
                None => working_context.clone(),
            };
            chibi.app.clear_tool_cache(&ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
            did_action = true;
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(&working_context, None)?;
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
            let ctx_name = working_context.clone();
            if !chibi.app.context_dir(&ctx_name).exists() {
                let new_context = Context::new(ctx_name.clone());
                chibi.app.save_and_register_context(&new_context)?;
            }

            // Resolve config with runtime override (ephemeral username extracted earlier)
            let mut resolved = resolve_cli_config(chibi, &ctx_name, ephemeral_username)?;
            if input.raw {
                resolved.render_markdown = false;
            }
            if input.flags.no_tool_calls {
                resolved.core.no_tool_calls = true;
            }
            let use_reflection = resolved.core.reflection_enabled;

            // Acquire context lock
            let context_dir = chibi.app.context_dir(&ctx_name);
            let _lock = chibi_core::lock::ContextLock::acquire(
                &context_dir,
                chibi.app.config.lock_heartbeat_seconds,
            )?;

            let options = PromptOptions::new(
                verbose,
                use_reflection,
                &input.flags.debug,
                force_markdown,
            );

            // Create markdown config if enabled
            let md_config = if resolved.render_markdown && !input.raw {
                Some(md_config_from_resolved(
                    &resolved,
                    chibi.home_dir(),
                    force_markdown,
                ))
            } else {
                None
            };

            let mut sink = CliResponseSink::new(
                output,
                md_config,
                verbose,
                show_tool_calls,
                show_thinking_flag || resolved.show_thinking,
            );
            chibi
                .send_prompt_streaming(
                    &working_context,
                    prompt,
                    &resolved.core,
                    &options,
                    &mut sink,
                )
                .await?;
            did_action = true;
        }
        Command::CheckInbox { context } => {
            let ctx_name = resolve_context_name(chibi, session, context)?;

            // Peek to see if there are messages
            let messages = chibi.app.peek_inbox(&ctx_name)?;
            if messages.is_empty() {
                output.diagnostic(
                    &format!("[No messages in inbox for '{}'.]", ctx_name),
                    verbose,
                );
            } else {
                output.diagnostic(
                    &format!(
                        "[Processing {} message(s) from inbox for '{}']",
                        messages.len(),
                        ctx_name
                    ),
                    verbose,
                );

                // Ensure context exists
                if !chibi.app.context_dir(&ctx_name).exists() {
                    let new_context = Context::new(ctx_name.clone());
                    chibi.app.save_and_register_context(&new_context)?;
                }

                // Resolve config for this context
                let mut resolved = resolve_cli_config(chibi, &ctx_name, None)?;
                if input.raw {
                    resolved.render_markdown = false;
                }
                if input.flags.no_tool_calls {
                    resolved.core.no_tool_calls = true;
                }
                let use_reflection = resolved.core.reflection_enabled;

                // Acquire context lock
                let context_dir = chibi.app.context_dir(&ctx_name);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;

                let options = PromptOptions::new(
                    verbose,
                    use_reflection,
                    &input.flags.debug,
                    force_markdown,
                );

                // Create markdown stream if enabled
                let md_config = if resolved.render_markdown && !input.raw {
                    Some(md_config_from_resolved(
                        &resolved,
                        chibi.home_dir(),
                        force_markdown,
                    ))
                } else {
                    None
                };

                let mut sink = CliResponseSink::new(
                    output,
                    md_config,
                    verbose,
                    show_tool_calls,
                    show_thinking_flag || resolved.show_thinking,
                );
                chibi
                    .send_prompt_streaming(
                        &ctx_name,
                        chibi_core::INBOX_CHECK_PROMPT,
                        &resolved.core,
                        &options,
                        &mut sink,
                    )
                    .await?;
            }
            did_action = true;
        }
        Command::CheckAllInboxes => {
            let contexts = chibi.app.list_contexts();
            let mut processed_count = 0;

            for ctx_name in contexts {
                // Peek to see if there are messages
                let messages = chibi.app.peek_inbox(&ctx_name)?;
                if messages.is_empty() {
                    continue;
                }

                output.diagnostic(
                    &format!(
                        "[Processing {} message(s) from inbox for '{}']",
                        messages.len(),
                        ctx_name
                    ),
                    verbose,
                );

                // Resolve config for this context
                let mut resolved = resolve_cli_config(chibi, &ctx_name, None)?;
                if input.raw {
                    resolved.render_markdown = false;
                }
                if input.flags.no_tool_calls {
                    resolved.core.no_tool_calls = true;
                }
                let use_reflection = resolved.core.reflection_enabled;

                // Acquire context lock
                let context_dir = chibi.app.context_dir(&ctx_name);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;

                let options = PromptOptions::new(
                    verbose,
                    use_reflection,
                    &input.flags.debug,
                    force_markdown,
                );

                // Create markdown stream if enabled
                let md_config = if resolved.render_markdown && !input.raw {
                    Some(md_config_from_resolved(
                        &resolved,
                        chibi.home_dir(),
                        force_markdown,
                    ))
                } else {
                    None
                };

                let mut sink = CliResponseSink::new(
                    output,
                    md_config,
                    verbose,
                    show_tool_calls,
                    show_thinking_flag || resolved.show_thinking,
                );
                chibi
                    .send_prompt_streaming(
                        &ctx_name,
                        chibi_core::INBOX_CHECK_PROMPT,
                        &resolved.core,
                        &options,
                        &mut sink,
                    )
                    .await?;

                processed_count += 1;
            }

            if processed_count == 0 {
                output.diagnostic("[No messages in any inbox.]", verbose);
            } else {
                output.diagnostic(
                    &format!("[Processed inboxes for {} context(s).]", processed_count),
                    verbose,
                );
            }
            did_action = true;
        }
        Command::ModelMetadata { model, full } => {
            let resolved = chibi.resolve_config(&working_context, None)?;
            let gateway = chibi_core::gateway::build_gateway(&resolved)?;
            let metadata = chibi_core::model_info::fetch_metadata(&gateway, model).await?;
            output.emit_result(
                chibi_core::model_info::format_model_toml(&metadata, *full).trim_end(),
            );
            did_action = true;
        }
        Command::NoOp => {
            // No operation - just context switch, already handled above
        }
    }

    // Shutdown session (executes OnEnd hooks)
    let _ = chibi.shutdown();

    // Automatic cache cleanup
    let resolved = chibi.resolve_config(&working_context, None)?;
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
    let cli_config = resolve_cli_config(chibi, &working_context, None)?;
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

/// Extract --project-root flag value from args (before full CLI parsing)
fn extract_project_root_override(args: &[String]) -> Option<PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--project-root" {
            return iter.next().map(PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--project-root=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Check for early flags (before full CLI parsing)
    let args: Vec<String> = std::env::args().collect();

    let trust_mode = args.iter().any(|a| a == "--trust" || a == "-t");
    let home_override = extract_home_override(&args);
    let project_root_override = extract_project_root_override(&args);

    // Parse CLI arguments to ChibiInput
    let mut input = cli::parse()?;

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

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose: input.flags.verbose,
        home: home_override,
        project_root: project_root_override,
    })?;
    chibi.set_permission_handler(select_permission_handler(trust_mode));
    // CLI flags override config settings
    let verbose = input.flags.verbose || chibi.app.config.verbose;
    input.flags.verbose = verbose;
    input.flags.hide_tool_calls = input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
    input.flags.no_tool_calls = input.flags.no_tool_calls || chibi.app.config.no_tool_calls;
    let mut session = Session::load(chibi.home_dir())?;
    let output = OutputHandler::new();

    // Print tool lists if verbose
    if verbose {
        let builtin_names = chibi_core::tools::builtin_tool_names();
        output.diagnostic(
            &format!(
                "[Built-in ({}): {}]",
                builtin_names.len(),
                builtin_names.join(", ")
            ),
            true,
        );

        if chibi.tools.is_empty() {
            output.diagnostic("[No plugins loaded]", true);
        } else {
            output.diagnostic(
                &format!(
                    "[Plugins ({}): {}]",
                    chibi.tool_count(),
                    chibi
                        .tools
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                true,
            );
        }
    }

    execute_from_input(input, &mut chibi, &mut session, &output, force_markdown).await
}
