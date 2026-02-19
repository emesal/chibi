//! Shared command execution for chibi binaries.
//!
//! Provides [`execute_command()`] which handles the full lifecycle:
//! init → pre-command housekeeping → command dispatch → post-command cleanup.
//! Both chibi-cli and chibi-json call this with their own `OutputSink` and
//! `ResponseSink` implementations.

use std::io;

use crate::Chibi;
use crate::api::PromptOptions;
use crate::api::sink::ResponseSink;
use crate::config::ResolvedConfig;
use crate::context;
use crate::input::{Command, DebugKey, ExecutionFlags, Inspectable};
use crate::output::OutputSink;
use crate::state::StatePaths;

/// Side effects of command execution that binaries may need to act on.
///
/// Core doesn't know about sessions or other binary-specific state.
/// It returns what happened so binaries can update accordingly.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandEffect {
    /// No side effects requiring binary attention.
    None,
    /// A context was destroyed. CLI should update session if needed.
    ContextDestroyed(String),
    /// A context was renamed. CLI should update session if needed.
    ContextRenamed { old: String, new: String },
    /// Config field inspection — binary resolves with its full config.
    InspectConfigField { context: String, field: String },
    /// Config field listing — binary emits its full field list.
    InspectConfigList { context: String },
}

/// Execute a command with full lifecycle management.
///
/// Handles init, housekeeping (auto-destroy, context touch), command dispatch,
/// shutdown, and cache cleanup. Binaries call this after resolving their own
/// input, config, and sinks.
///
/// # Arguments
/// - `context` — already resolved by binary (CLI via session, JSON from input)
/// - `config` — core config, resolved by binary before calling
/// - `username` — ephemeral username override for this invocation
#[allow(clippy::too_many_arguments)]
pub async fn execute_command<S: ResponseSink>(
    chibi: &mut Chibi,
    context: &str,
    command: &Command,
    flags: &ExecutionFlags,
    config: &ResolvedConfig,
    username: Option<&str>,
    output: &dyn OutputSink,
    sink: &mut S,
) -> io::Result<CommandEffect> {
    let verbose = config.verbose;

    // --- pre-command lifecycle ---

    // Initialize (OnStart hooks)
    let _ = chibi.init();

    // Auto-destroy expired contexts
    let destroyed = chibi.app.auto_destroy_expired_contexts(verbose)?;
    if !destroyed.is_empty() {
        chibi.save()?;
        output.diagnostic(
            &format!("[Auto-destroyed {} expired context(s)]", destroyed.len()),
            verbose,
        );
    }

    // Ensure context dir + ContextEntry exist
    chibi.app.ensure_context_dir(context)?;
    if !chibi.app.state.contexts.iter().any(|e| e.name == context) {
        chibi
            .app
            .state
            .contexts
            .push(context::ContextEntry::with_created_at(
                context.to_string(),
                context::now_timestamp(),
            ));
    }

    // Touch context with debug destroy settings
    let debug_destroy_at = flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAt(ts) => Some(*ts),
        _ => None,
    });
    let debug_destroy_after = flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
        _ => None,
    });
    if chibi.app.touch_context_with_destroy_settings(
        context,
        debug_destroy_at,
        debug_destroy_after,
    )? {
        chibi.save()?;
    }

    // --- command dispatch ---
    let effect = dispatch_command(
        chibi, context, command, flags, config, username, output, sink,
    )
    .await?;

    // --- post-command lifecycle ---

    // Shutdown (OnEnd hooks)
    let _ = chibi.shutdown();

    // Automatic cache cleanup
    let cleanup_config = chibi.resolve_config(context, None)?;
    if cleanup_config.auto_cleanup_cache {
        let removed = chibi
            .app
            .cleanup_all_tool_caches(cleanup_config.tool_cache_max_age_days)
            .await?;
        if removed > 0 {
            output.diagnostic(
                &format!(
                    "[Auto-cleanup: removed {} old cache entries (older than {} days)]",
                    removed,
                    cleanup_config.tool_cache_max_age_days + 1
                ),
                verbose,
            );
        }
    }

    Ok(effect)
}

/// Dispatch a command to the appropriate handler.
///
/// Send-path commands (SendPrompt, CallTool, CheckInbox, CheckAllInboxes)
/// use the provided `ResponseSink`. Non-send commands use `OutputSink` only.
#[allow(clippy::too_many_arguments)]
async fn dispatch_command<S: ResponseSink>(
    chibi: &mut Chibi,
    context: &str,
    command: &Command,
    flags: &ExecutionFlags,
    config: &ResolvedConfig,
    _username: Option<&str>,
    output: &dyn OutputSink,
    sink: &mut S,
) -> io::Result<CommandEffect> {
    let verbose = config.verbose;

    match command {
        Command::ShowHelp | Command::ShowVersion => {
            // Binary-specific — should be intercepted before reaching core.
            // If they arrive here, no-op gracefully.
            Ok(CommandEffect::None)
        }
        Command::ListContexts => {
            let contexts = chibi.list_contexts();
            for name in contexts {
                let context_dir = chibi.app.context_dir(&name);
                let status = crate::lock::ContextLock::get_status(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                );
                let marker = if name == context { "* " } else { "  " };
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                output.emit_result(&format!("{}{}{}", marker, name, status_str));
            }
            Ok(CommandEffect::None)
        }
        Command::ListCurrentContext => {
            let ctx = chibi.app.get_or_create_context(context)?;
            let context_dir = chibi.app.context_dir(context);
            let status = crate::lock::ContextLock::get_status(
                &context_dir,
                chibi.app.config.lock_heartbeat_seconds,
            );
            let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
            output.emit_result(&format!("Context: {}{}", context, status_str));
            output.emit_result(&format!("Messages: {}", ctx.messages.len()));
            if !ctx.summary.is_empty() {
                output.emit_result(&format!(
                    "Summary: {}",
                    ctx.summary.lines().next().unwrap_or("")
                ));
            }
            Ok(CommandEffect::None)
        }
        Command::DestroyContext { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            if !chibi.app.context_dir(ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else if !output.confirm(&format!("Destroy context '{}'?", ctx_name)) {
                output.emit_result("Aborted");
            } else {
                chibi.app.destroy_context(ctx_name)?;
                output.emit_result(&format!("Destroyed context: {}", ctx_name));
                return Ok(CommandEffect::ContextDestroyed(ctx_name.to_string()));
            }
            Ok(CommandEffect::None)
        }
        Command::ArchiveHistory { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            if name.is_none() {
                chibi.clear_context(ctx_name)?;
            } else {
                chibi.app.clear_context(ctx_name)?;
            }
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)",
                ctx_name
            ));
            Ok(CommandEffect::None)
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                crate::api::compact_context_by_name(&chibi.app, ctx_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                crate::api::compact_context_with_llm_manual(&chibi.app, context, config)
                    .await?;
            }
            Ok(CommandEffect::None)
        }
        Command::RenameContext { old, new } => {
            let old_name = old.as_deref().unwrap_or(context);
            chibi.app.rename_context(old_name, new)?;
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
            Ok(CommandEffect::ContextRenamed {
                old: old_name.to_string(),
                new: new.clone(),
            })
        }
        Command::ShowLog {
            context: ctx,
            count,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            show_log(chibi, ctx_name, *count, output)?;
            Ok(CommandEffect::None)
        }
        Command::Inspect {
            context: ctx,
            thing,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            match inspect_context(chibi, ctx_name, thing, output)? {
                Some(effect) => Ok(effect),
                None => Ok(CommandEffect::None),
            }
        }
        Command::SetSystemPrompt {
            context: ctx,
            prompt,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            let content = if std::path::Path::new(prompt).is_file() {
                std::fs::read_to_string(prompt)?
            } else {
                prompt.clone()
            };
            chibi.app.set_system_prompt_for(ctx_name, &content)?;
            output.diagnostic(
                &format!("[System prompt set for context '{}']", ctx_name),
                verbose,
            );
            Ok(CommandEffect::None)
        }
        Command::RunPlugin { name, args } => {
            let tool = crate::tools::find_tool(&chibi.tools, name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Plugin '{}' not found", name),
                )
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = crate::tools::execute_tool(tool, &args_json)?;
            output.emit_result(&result);
            Ok(CommandEffect::None)
        }
        Command::ClearCache { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.app.clear_tool_cache(ctx_name).await?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
            Ok(CommandEffect::None)
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(context, None)?;
            let removed = chibi
                .app
                .cleanup_all_tool_caches(resolved.tool_cache_max_age_days)
                .await?;
            output.emit_result(&format!(
                "Removed {} old cache entries (older than {} days)",
                removed,
                resolved.tool_cache_max_age_days + 1
            ));
            Ok(CommandEffect::None)
        }
        Command::ModelMetadata { model, full } => {
            let resolved = chibi.resolve_config(context, None)?;
            let gateway = crate::gateway::build_gateway(&resolved)?;
            let metadata = crate::model_info::fetch_metadata(&gateway, model).await?;
            output.emit_result(crate::model_info::format_model_toml(&metadata, *full).trim_end());
            Ok(CommandEffect::None)
        }
        Command::NoOp => Ok(CommandEffect::None),

        // --- send-path commands ---
        Command::SendPrompt { prompt } => {
            if !chibi.app.context_dir(context).exists() {
                let new_context = context::Context::new(context.to_string());
                chibi.app.save_and_register_context(&new_context)?;
            }
            send_prompt_inner(chibi, context, prompt, config, flags, None, sink).await?;
            Ok(CommandEffect::None)
        }
        Command::CallTool { name, args } => {
            let args_str = args.join(" ");
            let args_json: serde_json::Value = if args_str.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&args_str).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid JSON arguments: {}", e),
                    )
                })?
            };

            let result = chibi.execute_tool(context, name, args_json.clone()).await?;

            if flags.force_call_agent {
                let tool_context = format!(
                    "[User initiated tool call: {}]\n[Arguments: {}]\n[Result: {}]",
                    name, args_json, result
                );
                let fallback = crate::tools::HandoffTarget::Agent {
                    prompt: String::new(),
                };
                send_prompt_inner(
                    chibi,
                    context,
                    &tool_context,
                    config,
                    flags,
                    Some(fallback),
                    sink,
                )
                .await?;
            } else {
                output.emit_result(&result);
            }
            Ok(CommandEffect::None)
        }
        Command::CheckInbox { context: ctx } => {
            let messages = chibi.app.peek_inbox(ctx)?;
            if messages.is_empty() {
                output.diagnostic(&format!("[No messages in inbox for '{}']", ctx), verbose);
            } else {
                output.diagnostic(
                    &format!(
                        "[Processing {} message(s) from inbox for '{}']",
                        messages.len(),
                        ctx
                    ),
                    verbose,
                );
                if !chibi.app.context_dir(ctx).exists() {
                    let new_context = context::Context::new(ctx.clone());
                    chibi.app.save_and_register_context(&new_context)?;
                }
                let inbox_config = chibi.resolve_config(ctx, None)?;
                send_prompt_inner(
                    chibi,
                    ctx,
                    crate::INBOX_CHECK_PROMPT,
                    &inbox_config,
                    flags,
                    None,
                    sink,
                )
                .await?;
            }
            Ok(CommandEffect::None)
        }
        Command::CheckAllInboxes => {
            let contexts = chibi.app.list_contexts();
            let mut processed_count = 0;
            for ctx_name in contexts {
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
                let inbox_config = chibi.resolve_config(&ctx_name, None)?;
                send_prompt_inner(
                    chibi,
                    &ctx_name,
                    crate::INBOX_CHECK_PROMPT,
                    &inbox_config,
                    flags,
                    None,
                    sink,
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
            Ok(CommandEffect::None)
        }
    }
}

/// Resolve config, acquire context lock, and send a prompt through the agentic loop.
///
/// Shared by SendPrompt, CallTool (with continuation), CheckInbox, CheckAllInboxes.
async fn send_prompt_inner<S: ResponseSink>(
    chibi: &Chibi,
    context: &str,
    prompt: &str,
    config: &ResolvedConfig,
    flags: &ExecutionFlags,
    fallback: Option<crate::tools::HandoffTarget>,
    sink: &mut S,
) -> io::Result<()> {
    let mut resolved = config.clone();
    crate::gateway::ensure_context_window(&mut resolved);
    let use_reflection = resolved.reflection_enabled;

    let context_dir = chibi.app.context_dir(context);
    let _lock =
        crate::lock::ContextLock::acquire(&context_dir, chibi.app.config.lock_heartbeat_seconds)?;

    let mut options = PromptOptions::new(
        config.verbose,
        use_reflection,
        &flags.debug,
        false, // force_render is a CLI concern
    );
    if let Some(fb) = fallback {
        options = options.with_fallback(fb);
    }

    chibi
        .send_prompt_streaming(context, prompt, &resolved, &options, sink)
        .await
}

/// Show log entries for a context.
///
/// Selects entries by count and emits each via `emit_entry()`.
/// Formatting is the responsibility of the sink implementation.
fn show_log(chibi: &Chibi, context: &str, count: isize, output: &dyn OutputSink) -> io::Result<()> {
    let entries = chibi.app.read_jsonl_transcript(context)?;

    let selected: Vec<_> = if count == 0 {
        entries.iter().collect()
    } else if count > 0 {
        let n = count as usize;
        entries
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        let n = (-count) as usize;
        entries.iter().take(n).collect()
    };

    for entry in selected {
        output.emit_entry(entry)?;
    }
    Ok(())
}

/// Inspect a context property.
///
/// Renders markdown-bearing content (todos, goals) via `emit_markdown()`.
/// Returns `Some(CommandEffect)` for config-related inspections that require
/// binary-specific resolution (binaries may have extended config fields).
fn inspect_context(
    chibi: &Chibi,
    context: &str,
    thing: &Inspectable,
    output: &dyn OutputSink,
) -> io::Result<Option<CommandEffect>> {
    match thing {
        Inspectable::List => Ok(Some(CommandEffect::InspectConfigList {
            context: context.to_string(),
        })),
        Inspectable::SystemPrompt => {
            let prompt = chibi.app.load_system_prompt_for(context)?;
            if prompt.is_empty() {
                output.emit_result("(no system prompt set)");
            } else {
                output.emit_result(prompt.trim_end());
            }
            Ok(None)
        }
        Inspectable::Reflection => {
            let reflection = chibi.app.load_reflection()?;
            if reflection.is_empty() {
                output.emit_result("(no reflection set)");
            } else {
                output.emit_result(reflection.trim_end());
            }
            Ok(None)
        }
        Inspectable::Todos => {
            let todos = chibi.app.load_todos_for(context)?;
            if todos.is_empty() {
                output.emit_result("(no todos)");
            } else {
                output.emit_markdown(todos.trim_end())?;
            }
            Ok(None)
        }
        Inspectable::Goals => {
            let goals = chibi.app.load_goals_for(context)?;
            if goals.is_empty() {
                output.emit_result("(no goals)");
            } else {
                output.emit_markdown(goals.trim_end())?;
            }
            Ok(None)
        }
        Inspectable::Home => {
            output.emit_result(&chibi.home_dir().display().to_string());
            Ok(None)
        }
        Inspectable::ConfigField(field_path) => Ok(Some(CommandEffect::InspectConfigField {
            context: context.to_string(),
            field: field_path.clone(),
        })),
    }
}
