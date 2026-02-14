use std::io::{self, Read};

use chibi_core::context::{Context, ContextEntry, now_timestamp};
use chibi_core::input::{Command, DebugKey, Inspectable};
use chibi_core::{Chibi, LoadOptions, OutputSink, PromptOptions, StatePaths, api, tools};

mod input;
mod output;
mod sink;

#[tokio::main]
async fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // --json-schema: print input schema and exit
    if args.iter().any(|a| a == "--json-schema") {
        let schema = schemars::schema_for!(input::JsonInput);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
        return Ok(());
    }

    // --version
    if args.iter().any(|a| a == "--version") {
        println!("chibi-json {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Read JSON from stdin
    let mut json_str = String::new();
    io::stdin().read_to_string(&mut json_str)?;

    let mut json_input: input::JsonInput = serde_json::from_str(&json_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid JSON input: {}", e),
        )
    })?;

    let output = output::JsonOutputSink;
    let verbose = json_input.flags.verbose;

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose,
        home: json_input.home.clone(),
        project_root: json_input.project_root.clone(),
    })?;

    // Trust mode -- programmatic callers have already decided
    chibi.set_permission_handler(Box::new(|_| Ok(true)));

    // Config flag overrides
    json_input.flags.verbose = json_input.flags.verbose || chibi.app.config.verbose;
    json_input.flags.hide_tool_calls =
        json_input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
    json_input.flags.no_tool_calls =
        json_input.flags.no_tool_calls || chibi.app.config.no_tool_calls;

    let verbose = json_input.flags.verbose;
    let context = &json_input.context;

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

    // Ensure context dir exists
    chibi.app.ensure_context_dir(context)?;

    // Touch context with debug destroy settings
    let debug_destroy_at = json_input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAt(ts) => Some(*ts),
        _ => None,
    });
    let debug_destroy_after = json_input.flags.debug.iter().find_map(|k| match k {
        DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
        _ => None,
    });
    if !chibi.app.state.contexts.iter().any(|e| e.name == *context) {
        chibi.app.state.contexts.push(ContextEntry::with_created_at(
            context.clone(),
            now_timestamp(),
        ));
    }
    if chibi.app.touch_context_with_destroy_settings(
        context,
        debug_destroy_at,
        debug_destroy_after,
    )? {
        chibi.save()?;
    }

    output.diagnostic(&format!("[Loaded {} tool(s)]", chibi.tool_count()), verbose);

    // SYNC: chibi-cli also dispatches commands — check crates/chibi-cli/src/main.rs
    execute_json_command(&mut chibi, &json_input, &output).await?;

    // Shutdown (OnEnd hooks)
    let _ = chibi.shutdown();

    // Automatic cache cleanup
    let resolved = chibi.resolve_config(context, None)?;
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

    Ok(())
}

/// Execute a command from JSON input.
///
/// Mirrors chibi-cli's `execute_from_input` but without session, context
/// selection, or markdown rendering. Stateless per invocation, trust mode.
async fn execute_json_command(
    chibi: &mut Chibi,
    input: &input::JsonInput,
    output: &dyn OutputSink,
) -> io::Result<()> {
    let verbose = input.flags.verbose;
    let context = &input.context;

    match &input.command {
        Command::ShowHelp => {
            output.emit_result("Use --json-schema to see the input schema.");
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi-json {}", env!("CARGO_PKG_VERSION")));
        }
        Command::ListContexts => {
            let contexts = chibi.list_contexts();
            for name in contexts {
                let context_dir = chibi.app.context_dir(&name);
                let status = chibi_core::lock::ContextLock::get_status(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                );
                let marker = if &name == context { "* " } else { "  " };
                let status_str = status.map(|s| format!(" {}", s)).unwrap_or_default();
                output.emit_result(&format!("{}{}{}", marker, name, status_str));
            }
        }
        Command::ListCurrentContext => {
            let ctx = chibi.app.get_or_create_context(context)?;
            let context_dir = chibi.app.context_dir(context);
            let status = chibi_core::lock::ContextLock::get_status(
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
        }
        Command::DestroyContext { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            if !chibi.app.context_dir(ctx_name).exists() {
                output.emit_result(&format!("Context '{}' not found", ctx_name));
            } else {
                // Trust mode: auto-confirm
                chibi.app.destroy_context(ctx_name)?;
                output.emit_result(&format!("Destroyed context: {}", ctx_name));
            }
        }
        Command::ArchiveHistory { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.clear_context(ctx_name)?;
            output.emit_result(&format!(
                "Context '{}' archived (history saved to transcript)",
                ctx_name
            ));
        }
        Command::CompactContext { name } => {
            if let Some(ctx_name) = name {
                api::compact_context_by_name(&chibi.app, ctx_name, verbose).await?;
                output.emit_result(&format!("Context '{}' compacted", ctx_name));
            } else {
                let resolved = chibi.resolve_config(context, None)?;
                api::compact_context_with_llm_manual(&chibi.app, context, &resolved, verbose)
                    .await?;
                output.emit_result(&format!("Context '{}' compacted", context));
            }
        }
        Command::RenameContext { old, new } => {
            let old_name = old.as_deref().unwrap_or(context);
            chibi.app.rename_context(old_name, new)?;
            output.emit_result(&format!("Renamed context '{}' to '{}'", old_name, new));
        }
        Command::ShowLog {
            context: ctx,
            count,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            let entries = chibi.app.read_jsonl_transcript(ctx_name)?;
            let selected: Vec<_> = if *count == 0 {
                entries.iter().collect()
            } else if *count > 0 {
                let n = *count as usize;
                entries
                    .iter()
                    .rev()
                    .take(n)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            } else {
                let n = (-*count) as usize;
                entries.iter().take(n).collect()
            };
            for entry in selected {
                output.emit_entry(entry)?;
            }
        }
        Command::Inspect {
            context: ctx,
            thing,
        } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            match thing {
                Inspectable::List => {
                    let names = ["system_prompt", "reflection", "todos", "goals", "home"];
                    for name in &names {
                        output.emit_result(name);
                    }
                    output.emit_result("config.<field> (use 'config.list' to see fields)");
                }
                Inspectable::SystemPrompt => {
                    let prompt = chibi.app.load_system_prompt_for(ctx_name)?;
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
                    let todos = chibi.app.load_todos_for(ctx_name)?;
                    if todos.is_empty() {
                        output.emit_result("(no todos)");
                    } else {
                        output.emit_result(todos.trim_end());
                    }
                }
                Inspectable::Goals => {
                    let goals = chibi.app.load_goals_for(ctx_name)?;
                    if goals.is_empty() {
                        output.emit_result("(no goals)");
                    } else {
                        output.emit_result(goals.trim_end());
                    }
                }
                Inspectable::Home => {
                    output.emit_result(&chibi.home_dir().display().to_string());
                }
                Inspectable::ConfigField(field_path) => {
                    let resolved = chibi.resolve_config(ctx_name, input.username.as_deref())?;
                    match resolved.get_field(field_path) {
                        Some(value) => output.emit_result(&value),
                        None => output.emit_result("(not set)"),
                    }
                }
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
            output.emit_result(&format!("System prompt set for context '{}'", ctx_name));
        }
        Command::RunPlugin { name, args } => {
            let tool = tools::find_tool(&chibi.tools, name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Plugin '{}' not found", name),
                )
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = tools::execute_tool(tool, &args_json, verbose)?;
            output.emit_result(&result);
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

            if input.flags.force_call_agent {
                let tool_context = format!(
                    "[User initiated tool call: {}]\n[Arguments: {}]\n[Result: {}]",
                    name, args_json, result
                );
                let mut resolved = chibi.resolve_config(context, input.username.as_deref())?;
                chibi_core::gateway::ensure_context_window(&mut resolved);
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(context);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;
                let fallback = chibi_core::tools::HandoffTarget::Agent {
                    prompt: String::new(),
                };
                let options =
                    PromptOptions::new(verbose, use_reflection, &input.flags.debug, false)
                        .with_fallback(fallback);
                let mut response_sink = sink::JsonResponseSink::new();
                chibi
                    .send_prompt_streaming(
                        context,
                        &tool_context,
                        &resolved,
                        &options,
                        &mut response_sink,
                    )
                    .await?;
            } else {
                output.emit_result(&result);
            }
        }
        Command::ClearCache { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.app.clear_tool_cache(ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(context, None)?;
            let removed = chibi
                .app
                .cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
            output.emit_result(&format!(
                "Removed {} old cache entries (older than {} days)",
                removed, resolved.tool_cache_max_age_days
            ));
        }
        Command::SendPrompt { prompt } => {
            if !chibi.app.context_dir(context).exists() {
                let new_context = Context::new(context.clone());
                chibi.app.save_and_register_context(&new_context)?;
            }
            let mut resolved = chibi.resolve_config(context, input.username.as_deref())?;
            chibi_core::gateway::ensure_context_window(&mut resolved);
            if input.flags.no_tool_calls {
                resolved.no_tool_calls = true;
            }
            let use_reflection = resolved.reflection_enabled;
            let context_dir = chibi.app.context_dir(context);
            let _lock = chibi_core::lock::ContextLock::acquire(
                &context_dir,
                chibi.app.config.lock_heartbeat_seconds,
            )?;
            let options = PromptOptions::new(verbose, use_reflection, &input.flags.debug, false);
            let mut response_sink = sink::JsonResponseSink::new();
            chibi
                .send_prompt_streaming(context, prompt, &resolved, &options, &mut response_sink)
                .await?;
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
                    let new_context = Context::new(ctx.clone());
                    chibi.app.save_and_register_context(&new_context)?;
                }
                let mut resolved = chibi.resolve_config(ctx, None)?;
                chibi_core::gateway::ensure_context_window(&mut resolved);
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(ctx);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;
                let options =
                    PromptOptions::new(verbose, use_reflection, &input.flags.debug, false);
                let mut response_sink = sink::JsonResponseSink::new();
                chibi
                    .send_prompt_streaming(
                        ctx,
                        chibi_core::INBOX_CHECK_PROMPT,
                        &resolved,
                        &options,
                        &mut response_sink,
                    )
                    .await?;
            }
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
                let mut resolved = chibi.resolve_config(&ctx_name, None)?;
                chibi_core::gateway::ensure_context_window(&mut resolved);
                if input.flags.no_tool_calls {
                    resolved.no_tool_calls = true;
                }
                let use_reflection = resolved.reflection_enabled;
                let context_dir = chibi.app.context_dir(&ctx_name);
                let _lock = chibi_core::lock::ContextLock::acquire(
                    &context_dir,
                    chibi.app.config.lock_heartbeat_seconds,
                )?;
                let options =
                    PromptOptions::new(verbose, use_reflection, &input.flags.debug, false);
                let mut response_sink = sink::JsonResponseSink::new();
                chibi
                    .send_prompt_streaming(
                        &ctx_name,
                        chibi_core::INBOX_CHECK_PROMPT,
                        &resolved,
                        &options,
                        &mut response_sink,
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
        }
        Command::ModelMetadata { model, full } => {
            let resolved = chibi.resolve_config(context, None)?;
            let gateway = chibi_core::gateway::build_gateway(&resolved)?;
            let metadata = chibi_core::model_info::fetch_metadata(&gateway, model).await?;
            output.emit_result(
                chibi_core::model_info::format_model_toml(&metadata, *full).trim_end(),
            );
        }
        Command::NoOp => {}
    }

    Ok(())
}
