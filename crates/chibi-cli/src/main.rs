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

use chibi_core::input::Command;

use crate::cli::Cli;
use crate::config::{ImageConfig, ResolvedConfig, default_markdown_style, load_cli_config};
use crate::input::{ChibiInput, ContextSelection, UsernameOverride};
use crate::markdown::{MarkdownConfig, MarkdownStream};
use crate::output::OutputHandler;
use crate::session::Session;
use crate::sink::CliResponseSink;
use chibi_core::{Chibi, CommandEffect, CommandEvent, LoadOptions, OutputSink, PermissionHandler, StatePaths};
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;

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
    chibi_core::gateway::ensure_context_window(&mut core);

    Ok(ResolvedConfig {
        core,
        render_markdown: cli.render_markdown,
        verbose: cli.verbose,
        hide_tool_calls: cli.hide_tool_calls,
        show_thinking: cli.show_thinking,
        image: cli.image,
        markdown_style: cli.markdown_style,
    })
}

/// Resolve CLI-specific context names in Command variants.
///
/// Handles "-" (previous context), "new", and "new:prefix" in optional
/// context name fields. Core doesn't know about these — they're CLI
/// conveniences tied to session state.
fn resolve_command_names(
    command: &Command,
    chibi: &Chibi,
    session: &Session,
) -> io::Result<Command> {
    // Helper: resolve an optional name, defaulting to working_context
    let resolve_opt = |name: &Option<String>| -> io::Result<Option<String>> {
        match name {
            Some(n) => Ok(Some(resolve_context_name(chibi, session, n)?)),
            None => Ok(None),
        }
    };

    match command {
        Command::DestroyContext { name } => Ok(Command::DestroyContext {
            name: resolve_opt(name)?,
        }),
        Command::ArchiveHistory { name } => Ok(Command::ArchiveHistory {
            name: resolve_opt(name)?,
        }),
        Command::CompactContext { name } => Ok(Command::CompactContext {
            name: resolve_opt(name)?,
        }),
        Command::RenameContext { old, new } => Ok(Command::RenameContext {
            old: resolve_opt(old)?,
            new: new.clone(),
        }),
        Command::ShowLog { context, count } => Ok(Command::ShowLog {
            context: resolve_opt(context)?,
            count: *count,
        }),
        Command::Inspect { context, thing } => Ok(Command::Inspect {
            context: resolve_opt(context)?,
            thing: thing.clone(),
        }),
        Command::SetSystemPrompt { context, prompt } => Ok(Command::SetSystemPrompt {
            context: resolve_opt(context)?,
            prompt: prompt.clone(),
        }),
        Command::ClearCache { name } => Ok(Command::ClearCache {
            name: resolve_opt(name)?,
        }),
        Command::CheckInbox { context } => Ok(Command::CheckInbox {
            context: resolve_context_name(chibi, session, context)?,
        }),
        // All other commands pass through unchanged
        _ => Ok(command.clone()),
    }
}

/// Execute from ChibiInput.
///
/// Handles CLI-specific concerns (context selection, session, username overrides,
/// help/version, image cache) then delegates command dispatch to core's
/// `execute_command()`. Session updates are driven by `CommandEffect`.
async fn execute_from_input(
    input: ChibiInput,
    chibi: &mut Chibi,
    session: &mut Session,
    output: &dyn OutputSink,
    force_markdown: bool,
) -> io::Result<()> {
    let mut did_action = false;

    // Pre-resolution verbose: used for diagnostics before full config is resolved.
    let early_verbose = input.verbose_flag;

    // --- CLI-specific: context selection ---
    // working_context: the context we're actually operating on this invocation
    // implied_context: persisted in session.json, what you get when no context is specified
    let working_context = match &input.context {
        ContextSelection::Current => session.implied_context.clone(),
        ContextSelection::Ephemeral { name } => {
            let actual_name = resolve_context_name(chibi, session, name)?;
            chibi.app.ensure_context_dir(&actual_name)?;
            output.diagnostic(
                &format!("[Using ephemeral context: {}]", actual_name),
                early_verbose,
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
                early_verbose,
            );
            did_action = true;
            session.implied_context.clone()
        }
    };

    // --- CLI-specific: username override ---
    let ephemeral_username: Option<&str> = match &input.username_override {
        Some(UsernameOverride::Persistent(username)) => {
            let mut local_config = chibi.app.load_local_config(&working_context)?;
            local_config.username = Some(username.clone());
            chibi
                .app
                .save_local_config(&working_context, &local_config)?;
            output.emit_event(CommandEvent::UsernameSaved {
                username: username.clone(),
                context: working_context.clone(),
            });
            did_action = true;
            None // persistent was saved, no runtime override needed
        }
        Some(UsernameOverride::Ephemeral(username)) => Some(username.as_str()),
        None => None,
    };

    // --- CLI-specific: intercept ShowHelp / ShowVersion ---
    match &input.command {
        Command::ShowHelp => {
            Cli::print_help();
            return Ok(());
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi {}", env!("CARGO_PKG_VERSION")));
            return Ok(());
        }
        _ => {}
    }

    // --- resolve CLI-specific context names in command fields ---
    let command = resolve_command_names(&input.command, chibi, session)?;

    // --- resolve config and build CLI response sink ---
    let mut cli_config = resolve_cli_config(chibi, &working_context, ephemeral_username)?;

    // Apply per-invocation config overrides (-s/--set)
    if !input.config_overrides.is_empty() {
        cli_config
            .core
            .apply_overrides_from_pairs(&input.config_overrides)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
    }

    // CLI flags override cli.toml values
    if input.verbose_flag { cli_config.verbose = true; }
    if input.hide_tool_calls_flag { cli_config.hide_tool_calls = true; }
    if input.show_thinking_flag { cli_config.show_thinking = true; }

    // Derive presentation flags from CLI config
    let verbose = cli_config.verbose;
    let show_tool_calls = !cli_config.hide_tool_calls || verbose;
    let show_thinking = cli_config.show_thinking || verbose;

    let md_config = if cli_config.render_markdown && !input.raw {
        Some(md_config_from_resolved(
            &cli_config,
            chibi.home_dir(),
            force_markdown,
        ))
    } else {
        None
    };

    let mut sink = CliResponseSink::new(output, md_config, verbose, show_tool_calls, show_thinking);

    // --- delegate to core ---
    let effect = chibi_core::execute_command(
        chibi,
        &working_context,
        &command,
        &input.flags,
        &cli_config.core,
        ephemeral_username,
        output,
        &mut sink,
    )
    .await?;
    if !matches!(command, Command::NoOp) {
        did_action = true;
    }

    // --- CLI-specific: handle CommandEffect for session updates ---
    match &effect {
        CommandEffect::ContextDestroyed(name) => {
            if session
                .handle_context_destroyed(name, |n| chibi.app.context_dir(n).exists())
                .is_some()
            {
                session.save(chibi.home_dir())?;
            }
        }
        CommandEffect::ContextRenamed { old, new } => {
            if session.implied_context == *old {
                session.implied_context = new.clone();
                session.save(chibi.home_dir())?;
            }
            if session.previous_context.as_deref() == Some(old.as_str()) {
                session.previous_context = Some(new.clone());
                session.save(chibi.home_dir())?;
            }
        }
        CommandEffect::InspectConfigField {
            context: ctx,
            field,
        } => {
            let mut cfg = resolve_cli_config(chibi, ctx, ephemeral_username)?;
            // Re-apply per-invocation overrides so -s is visible via inspect
            if !input.config_overrides.is_empty() {
                cfg.core
                    .apply_overrides_from_pairs(&input.config_overrides)
                    .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
            }
            match cfg.get_field(field) {
                Some(value) => output.emit_result(&value),
                None => output.emit_result("(not set)"),
            }
        }
        CommandEffect::InspectConfigList { context: _ } => {
            output.emit_result("Inspectable items:");
            for name in ["system_prompt", "reflection", "todos", "goals", "home"] {
                output.emit_result(&format!("  {}", name));
            }
            for field in ResolvedConfig::list_fields() {
                output.emit_result(&format!("  {}", field));
            }
        }
        CommandEffect::None => {}
    }

    // --- CLI-specific: check if auto-destroy removed our session context ---
    if !chibi.app.context_dir(&session.implied_context).exists() {
        session.implied_context = "default".to_string();
        session.previous_context = None;
        session.save(chibi.home_dir())?;
    }

    // --- CLI-specific: image cache cleanup ---
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

    // --- no-action check ---
    if !did_action && matches!(command, Command::NoOp) {
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
    let input = cli::parse()?;

    // Handle --debug md=<FILENAME> early (renders markdown and quits, implies -x)
    if let Some(path) = &input.md_file {
        let content = std::fs::read_to_string(path).map_err(|e| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("Failed to read file '{}': {}", path, e),
            )
        })?;
        let mut md_cfg = md_config_defaults(true);
        md_cfg.force_render = input.force_markdown;
        render_markdown_output(&content, md_cfg)?;
        return Ok(());
    }

    // Handle --debug force-markdown
    let force_markdown = input.force_markdown;

    let load_verbose = input.verbose_flag;

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose: load_verbose,
        home: home_override,
        project_root: project_root_override,
    })?;
    chibi.set_permission_handler(select_permission_handler(trust_mode));
    let mut session = Session::load(chibi.home_dir())?;
    let output = OutputHandler::new(load_verbose);

    // Print tool lists if verbose
    if load_verbose {
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
