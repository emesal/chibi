# extract shared execute_command() — implementation plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** extract duplicated command dispatch from chibi-cli and chibi-json into a shared `execute_command()` in chibi-core.

**Architecture:** new `execution.rs` module in chibi-core owns full lifecycle + command dispatch. binaries become thin pipelines: parse → resolve → build sinks → `execute_command()`. `OutputSink` gains `emit_markdown()` for presentation-aware markdown output.

**Tech Stack:** rust, async (tokio), trait objects (`dyn OutputSink`, `dyn ResponseSink`)

**Design doc:** `docs/plans/2026-02-14-execute-command-extraction-design.md`

---

### task 1: add `emit_markdown()` to `OutputSink`

**Files:**
- Modify: `crates/chibi-core/src/output.rs`
- Modify: `crates/chibi-cli/src/output.rs`
- Modify: `crates/chibi-json/src/output.rs`

**Step 1: add `emit_markdown` to the `OutputSink` trait**

In `crates/chibi-core/src/output.rs`, add to the trait:

```rust
/// Emit content that may contain markdown.
///
/// CLI renders this via streamdown; JSON emits raw text.
/// The default implementation falls back to `emit_result()`.
fn emit_markdown(&self, content: &str) -> io::Result<()> {
    self.emit_result(content);
    Ok(())
}
```

Use a default implementation so existing implementors don't break.

**Step 2: implement `emit_markdown` in `JsonOutputSink`**

In `crates/chibi-json/src/output.rs`, the default (falls back to `emit_result`) is correct. No change needed — but add an explicit implementation for clarity:

```rust
fn emit_markdown(&self, content: &str) -> io::Result<()> {
    self.emit_result(content);
    Ok(())
}
```

**Step 3: implement `emit_markdown` in CLI's `OutputHandler`**

In `crates/chibi-cli/src/output.rs`, `OutputHandler` doesn't have markdown config — it's a simple text emitter. For now, use the default (just prints). The CLI will pass a richer sink later (task 5) that has markdown config. For now:

```rust
fn emit_markdown(&self, content: &str) -> io::Result<()> {
    self.emit_result(content);
    Ok(())
}
```

**Step 4: build and verify**

Run: `cargo build --workspace`
Expected: compiles with no errors.

**Step 5: commit**

```
feat: add emit_markdown() to OutputSink trait (#143)
```

---

### task 2: add `CommandEffect` and create `execution.rs` skeleton

**Files:**
- Create: `crates/chibi-core/src/execution.rs`
- Modify: `crates/chibi-core/src/lib.rs`

**Step 1: create `execution.rs` with types and function signature**

Create `crates/chibi-core/src/execution.rs`:

```rust
//! Shared command execution for chibi binaries.
//!
//! Provides [`execute_command()`] which handles the full lifecycle:
//! init → pre-command housekeeping → command dispatch → post-command cleanup.
//! Both chibi-cli and chibi-json call this with their own `OutputSink` and
//! `ResponseSink` implementations.

use std::io;

use crate::api::sink::ResponseSink;
use crate::config::ResolvedConfig;
use crate::input::{Command, ExecutionFlags};
use crate::output::OutputSink;
use crate::Chibi;

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
pub async fn execute_command(
    chibi: &mut Chibi,
    context: &str,
    command: &Command,
    flags: &ExecutionFlags,
    config: &ResolvedConfig,
    username: Option<&str>,
    output: &dyn OutputSink,
    sink: &mut dyn ResponseSink,
) -> io::Result<CommandEffect> {
    todo!("task 3+4 fill this in")
}
```

**Step 2: register the module and re-export**

In `crates/chibi-core/src/lib.rs`, add:

```rust
pub mod execution;
```

And add to the re-exports:

```rust
pub use execution::{CommandEffect, execute_command};
```

**Step 3: build and verify**

Run: `cargo build --workspace`
Expected: compiles (the `todo!()` is fine at type level).

**Step 4: commit**

```
feat: add execution.rs skeleton with CommandEffect and execute_command signature (#143)
```

---

### task 3: implement lifecycle and non-send commands in `execute_command()`

**Files:**
- Modify: `crates/chibi-core/src/execution.rs`

This is the main extraction. Port each command handler from both binaries into
`execute_command()`, using the presentation-agnostic pattern. Reference both
`crates/chibi-cli/src/main.rs` (lines 574–927) and
`crates/chibi-json/src/main.rs` (lines 141–507) for the source logic.

**Step 1: implement the lifecycle wrapper**

Replace the `todo!()` body with:

```rust
pub async fn execute_command(
    chibi: &mut Chibi,
    context: &str,
    command: &Command,
    flags: &ExecutionFlags,
    config: &ResolvedConfig,
    username: Option<&str>,
    output: &dyn OutputSink,
    sink: &mut dyn ResponseSink,
) -> io::Result<CommandEffect> {
    let verbose = flags.verbose;

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
        chibi.app.state.contexts.push(
            crate::context::ContextEntry::with_created_at(
                context.to_string(),
                crate::context::now_timestamp(),
            ),
        );
    }

    // Touch context with debug destroy settings
    let debug_destroy_at = flags.debug.iter().find_map(|k| match k {
        crate::input::DebugKey::DestroyAt(ts) => Some(*ts),
        _ => None,
    });
    let debug_destroy_after = flags.debug.iter().find_map(|k| match k {
        crate::input::DebugKey::DestroyAfterSecondsInactive(secs) => Some(*secs),
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
    let effect = dispatch_command(chibi, context, command, flags, config, username, output, sink).await?;

    // --- post-command lifecycle ---

    // Shutdown (OnEnd hooks)
    let _ = chibi.shutdown();

    // Automatic cache cleanup
    let cleanup_config = chibi.resolve_config(context, None)?;
    if cleanup_config.auto_cleanup_cache {
        let removed = chibi
            .app
            .cleanup_all_tool_caches(cleanup_config.tool_cache_max_age_days)?;
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
```

**Step 2: implement `dispatch_command()` for non-send commands**

Add a private `dispatch_command()` function. Start with the non-send commands
(the ones that don't need `ResponseSink`). Port from both binaries, using
core's presentation-agnostic approach:

```rust
async fn dispatch_command(
    chibi: &mut Chibi,
    context: &str,
    command: &Command,
    flags: &ExecutionFlags,
    config: &ResolvedConfig,
    username: Option<&str>,
    output: &dyn OutputSink,
    sink: &mut dyn ResponseSink,
) -> io::Result<CommandEffect> {
    let verbose = flags.verbose;

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
                crate::api::compact_context_with_llm_manual(
                    &chibi.app, context, config, verbose,
                ).await?;
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
        Command::ShowLog { context: ctx, count } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            show_log(chibi, ctx_name, *count, verbose, output)?;
            Ok(CommandEffect::None)
        }
        Command::Inspect { context: ctx, thing } => {
            let ctx_name = ctx.as_deref().unwrap_or(context);
            inspect_context(chibi, ctx_name, thing, config, username, output)?;
            Ok(CommandEffect::None)
        }
        Command::SetSystemPrompt { context: ctx, prompt } => {
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
                io::Error::new(io::ErrorKind::NotFound, format!("Plugin '{}' not found", name))
            })?;
            let args_json = serde_json::json!({ "args": args });
            let result = crate::tools::execute_tool(tool, &args_json, verbose)?;
            output.emit_result(&result);
            Ok(CommandEffect::None)
        }
        Command::ClearCache { name } => {
            let ctx_name = name.as_deref().unwrap_or(context);
            chibi.app.clear_tool_cache(ctx_name)?;
            output.emit_result(&format!("Cleared tool cache for context '{}'", ctx_name));
            Ok(CommandEffect::None)
        }
        Command::CleanupCache => {
            let resolved = chibi.resolve_config(context, None)?;
            let removed = chibi
                .app
                .cleanup_all_tool_caches(resolved.tool_cache_max_age_days)?;
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
            output.emit_result(
                crate::model_info::format_model_toml(&metadata, *full).trim_end(),
            );
            Ok(CommandEffect::None)
        }
        Command::NoOp => Ok(CommandEffect::None),

        // Send-path commands — task 4
        Command::SendPrompt { .. }
        | Command::CallTool { .. }
        | Command::CheckInbox { .. }
        | Command::CheckAllInboxes => {
            todo!("send-path commands — implemented in task 4")
        }
    }
}
```

**Step 3: implement `show_log()` helper**

```rust
/// Show log entries for a context.
///
/// Renders message content via `emit_markdown()`. Tool calls and results
/// are emitted as plain text. In JSON mode, entries are emitted via `emit_entry()`.
fn show_log(
    chibi: &Chibi,
    context: &str,
    count: isize,
    verbose: bool,
    output: &dyn OutputSink,
) -> io::Result<()> {
    let entries = chibi.app.read_jsonl_transcript(context)?;

    let selected: Vec<_> = if count == 0 {
        entries.iter().collect()
    } else if count > 0 {
        let n = count as usize;
        entries.iter().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect()
    } else {
        let n = (-count) as usize;
        entries.iter().take(n).collect()
    };

    if output.is_json_mode() {
        for entry in selected {
            output.emit_entry(entry)?;
        }
        return Ok(());
    }

    for entry in selected {
        match entry.entry_type.as_str() {
            crate::context::ENTRY_TYPE_MESSAGE => {
                output.emit_result(&format!("[{}]", entry.from.to_uppercase()));
                output.emit_markdown(&entry.content)?;
                output.newline();
            }
            crate::context::ENTRY_TYPE_TOOL_CALL => {
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
            crate::context::ENTRY_TYPE_TOOL_RESULT => {
                if verbose {
                    output.emit_result(&format!("[TOOL RESULT: {}]\n{}\n", entry.from, entry.content));
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
```

**Step 4: implement `inspect_context()` helper**

```rust
/// Inspect a context property.
///
/// Renders markdown-bearing content (todos, goals) via `emit_markdown()`.
fn inspect_context(
    chibi: &Chibi,
    context: &str,
    thing: &crate::input::Inspectable,
    config: &ResolvedConfig,
    username: Option<&str>,
    output: &dyn OutputSink,
) -> io::Result<()> {
    use crate::input::Inspectable;

    match thing {
        Inspectable::List => {
            output.emit_result("Inspectable items:");
            for name in ["system_prompt", "reflection", "todos", "goals", "home"] {
                output.emit_result(&format!("  {}", name));
            }
            output.emit_result("  config.<field> (use 'config.list' to see fields)");
        }
        Inspectable::SystemPrompt => {
            let prompt = chibi.app.load_system_prompt_for(context)?;
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
            let todos = chibi.app.load_todos_for(context)?;
            if todos.is_empty() {
                output.emit_result("(no todos)");
            } else {
                output.emit_markdown(todos.trim_end())?;
            }
        }
        Inspectable::Goals => {
            let goals = chibi.app.load_goals_for(context)?;
            if goals.is_empty() {
                output.emit_result("(no goals)");
            } else {
                output.emit_markdown(goals.trim_end())?;
            }
        }
        Inspectable::Home => {
            output.emit_result(&chibi.home_dir().display().to_string());
        }
        Inspectable::ConfigField(field_path) => {
            let resolved = chibi.resolve_config(context, username)?;
            match resolved.get_field(field_path) {
                Some(value) => output.emit_result(&value),
                None => output.emit_result("(not set)"),
            }
        }
    }
    Ok(())
}
```

**Step 5: build and verify**

Run: `cargo build --workspace`
Expected: compiles. `todo!()` in send-path arms is fine — those are task 4.

**Step 6: commit**

```
feat: implement lifecycle and non-send commands in execute_command() (#143)
```

---

### task 4: implement send-path commands

**Files:**
- Modify: `crates/chibi-core/src/execution.rs`

**Step 1: add private `send_prompt()` helper**

```rust
/// Resolve config, acquire context lock, and send a prompt through the agentic loop.
///
/// Shared by SendPrompt, CallTool (with continuation), CheckInbox, CheckAllInboxes.
async fn send_prompt_inner(
    chibi: &Chibi,
    context: &str,
    prompt: &str,
    config: &ResolvedConfig,
    flags: &ExecutionFlags,
    fallback: Option<crate::tools::HandoffTarget>,
    sink: &mut dyn ResponseSink,
) -> io::Result<()> {
    let mut resolved = config.clone();
    if flags.no_tool_calls {
        resolved.no_tool_calls = true;
    }
    crate::gateway::ensure_context_window(&mut resolved);
    let use_reflection = resolved.reflection_enabled;

    let context_dir = chibi.app.context_dir(context);
    let _lock = crate::lock::ContextLock::acquire(
        &context_dir,
        chibi.app.config.lock_heartbeat_seconds,
    )?;

    let mut options = crate::api::PromptOptions::new(
        flags.verbose,
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
```

**Step 2: replace the `todo!()` arms in `dispatch_command()`**

Replace the send-path `todo!()` block with:

```rust
Command::SendPrompt { prompt } => {
    if !chibi.app.context_dir(context).exists() {
        let new_context = crate::context::Context::new(context.to_string());
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
        send_prompt_inner(chibi, context, &tool_context, config, flags, Some(fallback), sink).await?;
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
                messages.len(), ctx
            ),
            verbose,
        );
        if !chibi.app.context_dir(ctx).exists() {
            let new_context = crate::context::Context::new(ctx.clone());
            chibi.app.save_and_register_context(&new_context)?;
        }
        let inbox_config = chibi.resolve_config(ctx, None)?;
        send_prompt_inner(chibi, ctx, crate::INBOX_CHECK_PROMPT, &inbox_config, flags, None, sink).await?;
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
                messages.len(), ctx_name
            ),
            verbose,
        );
        let inbox_config = chibi.resolve_config(&ctx_name, None)?;
        send_prompt_inner(chibi, &ctx_name, crate::INBOX_CHECK_PROMPT, &inbox_config, flags, None, sink).await?;
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
```

**Step 3: build and verify**

Run: `cargo build --workspace`
Expected: compiles with no errors.

**Step 4: commit**

```
feat: implement send-path commands in execute_command() (#143)
```

---

### task 5: wire chibi-cli to use `execute_command()`

**Files:**
- Modify: `crates/chibi-cli/src/main.rs`
- Modify: `crates/chibi-cli/src/output.rs`

This task replaces the duplicated command dispatch in `execute_from_input()` with
a call to `chibi_core::execution::execute_command()`.

**Step 1: upgrade CLI's `OutputHandler` with markdown support**

The CLI needs `emit_markdown()` to actually render. Two options:

- (a) make `OutputHandler` carry a `MarkdownConfig`
- (b) use a wrapper struct that composes `OutputHandler` + markdown config

Go with (a): add an optional `MarkdownConfig` field to `OutputHandler`. When
present, `emit_markdown()` renders; when absent, falls back to plain text.

In `crates/chibi-cli/src/output.rs`:

```rust
use crate::markdown::{MarkdownConfig, MarkdownStream};

pub struct OutputHandler {
    markdown_config: Option<MarkdownConfig>,
}

impl OutputHandler {
    pub fn new() -> Self {
        Self { markdown_config: None }
    }

    pub fn with_markdown(config: MarkdownConfig) -> Self {
        Self { markdown_config: Some(config) }
    }
}
```

And implement `emit_markdown`:

```rust
fn emit_markdown(&self, content: &str) -> io::Result<()> {
    if let Some(ref config) = self.markdown_config {
        let mut md = MarkdownStream::new(config.clone());
        md.write_chunk(content)?;
        md.finish()?;
        if !content.ends_with('\n') {
            println!();
        }
    } else {
        self.emit_result(content);
    }
    Ok(())
}
```

**Step 2: simplify `execute_from_input()`**

Replace the `match &input.command` block (lines 575–927) with:

1. intercept `ShowHelp` and `ShowVersion` before calling core
2. call `execute_command()` for everything else
3. handle `CommandEffect` for session updates

```rust
async fn execute_from_input(
    input: ChibiInput,
    chibi: &mut Chibi,
    session: &mut Session,
    output: &dyn OutputSink,
    force_markdown: bool,
) -> io::Result<()> {
    let verbose = input.flags.verbose;

    // --- context resolution (CLI-specific) ---
    let working_context = match &input.context {
        ContextSelection::Current => session.implied_context.clone(),
        ContextSelection::Ephemeral { name } => {
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
            session.implied_context.clone()
        }
    };

    // --- username persistence (CLI-specific) ---
    let ephemeral_username: Option<&str> = match &input.username_override {
        Some(UsernameOverride::Persistent(username)) => {
            let mut local_config = chibi.app.load_local_config(&working_context)?;
            local_config.username = Some(username.clone());
            chibi.app.save_local_config(&working_context, &local_config)?;
            output.diagnostic(
                &format!("[Username '{}' saved to context '{}']", username, working_context),
                verbose,
            );
            None
        }
        Some(UsernameOverride::Ephemeral(username)) => Some(username.as_str()),
        None => None,
    };

    // --- intercept binary-specific commands ---
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

    // --- resolve core config ---
    let mut core_config = chibi.resolve_config(&working_context, ephemeral_username)?;
    if input.raw {
        // raw mode: handled by not giving OutputHandler a markdown config
    }
    if input.flags.no_tool_calls {
        core_config.no_tool_calls = true;
    }
    chibi_core::gateway::ensure_context_window(&mut core_config);

    // --- build response sink ---
    let show_tool_calls = !input.flags.hide_tool_calls || verbose;
    let show_thinking = input.flags.show_thinking || verbose;
    // Build markdown config for the CLI sink (if rendering is enabled)
    let cli_resolved = resolve_cli_config(chibi, &working_context, ephemeral_username)?;
    let md_config = if cli_resolved.render_markdown && !input.raw {
        Some(md_config_from_resolved(&cli_resolved, chibi.home_dir(), force_markdown))
    } else {
        None
    };
    let mut response_sink = CliResponseSink::new(
        output,
        md_config,
        verbose,
        show_tool_calls,
        show_thinking || cli_resolved.show_thinking,
    );

    // SYNC: chibi-json also calls execute_command — check crates/chibi-json/src/main.rs
    let effect = chibi_core::execution::execute_command(
        chibi,
        &working_context,
        &input.command,
        &input.flags,
        &core_config,
        ephemeral_username,
        output,
        &mut response_sink,
    ).await?;

    // --- handle effects (CLI-specific) ---
    match effect {
        chibi_core::CommandEffect::ContextDestroyed(ref ctx_name) => {
            if session
                .handle_context_destroyed(ctx_name, |name| chibi.app.context_dir(name).exists())
                .is_some()
            {
                session.save(chibi.home_dir())?;
            }
        }
        chibi_core::CommandEffect::ContextRenamed { ref old, ref new } => {
            if session.implied_context == *old {
                session.implied_context = new.clone();
                session.save(chibi.home_dir())?;
            }
            if session.previous_context.as_deref() == Some(old.as_str()) {
                session.previous_context = Some(new.clone());
                session.save(chibi.home_dir())?;
            }
        }
        chibi_core::CommandEffect::None => {}
    }

    // --- CLI-specific cleanup ---
    // Image cache cleanup
    if cli_resolved.image.cache_enabled {
        let image_cache_dir = chibi.home_dir().join("image_cache");
        match image_cache::cleanup_image_cache(
            &image_cache_dir,
            cli_resolved.image.cache_max_bytes,
            cli_resolved.image.cache_max_age_days,
        ) {
            Ok(removed) if removed > 0 => {
                output.diagnostic(
                    &format!(
                        "[Image cache cleanup: removed {} entries (max {} days, max {} MB)]",
                        removed,
                        cli_resolved.image.cache_max_age_days,
                        cli_resolved.image.cache_max_bytes / (1024 * 1024),
                    ),
                    verbose,
                );
            }
            _ => {}
        }
    }

    Ok(())
}
```

**Step 3: clean up removed functions**

Remove from `crates/chibi-cli/src/main.rs`:
- `show_log()` function (moved to core)
- `inspect_context()` function (moved to core)
- `set_prompt_for_context()` function (moved to core)
- `send_with_cli_sink()` function (replaced by core's send path)
- `CliSendOptions` struct (no longer needed)

Keep:
- `generate_new_context_name()`, `resolve_context_name()` (CLI-specific)
- `resolve_cli_config()`, `md_config_from_resolved()`, `md_config_defaults()` (CLI presentation)
- `render_markdown_output()` (still used for `--debug md=`)
- `build_interactive_permission_handler()`, etc. (CLI-specific)

**Step 4: update tests in `output.rs` if `OutputHandler::new()` signature changed**

If `OutputHandler` now has a field, update tests to use `OutputHandler::new()` (which initializes markdown_config to `None`).

**Step 5: build and run tests**

Run: `cargo build --workspace && cargo test --workspace`
Expected: compiles and all tests pass.

**Step 6: commit**

```
refactor: wire chibi-cli to use core execute_command() (#143)
```

---

### task 6: wire chibi-json to use `execute_command()`

**Files:**
- Modify: `crates/chibi-json/src/main.rs`

**Step 1: simplify `main()` and remove `execute_json_command()`**

Replace the lifecycle + dispatch in `main()` with:

```rust
#[tokio::main]
async fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--json-schema") {
        let schema = schemars::schema_for!(input::JsonInput);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
        return Ok(());
    }

    if args.iter().any(|a| a == "--version") {
        println!("chibi-json {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut json_str = String::new();
    io::stdin().read_to_string(&mut json_str)?;

    let mut json_input: input::JsonInput = serde_json::from_str(&json_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid JSON input: {}", e),
        )
    })?;

    let output = output::JsonOutputSink;

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose: json_input.flags.verbose,
        home: json_input.home.clone(),
        project_root: json_input.project_root.clone(),
    })?;

    chibi.set_permission_handler(Box::new(|_| Ok(true)));

    // Config flag overrides
    json_input.flags.verbose = json_input.flags.verbose || chibi.app.config.verbose;
    json_input.flags.hide_tool_calls =
        json_input.flags.hide_tool_calls || chibi.app.config.hide_tool_calls;
    json_input.flags.no_tool_calls =
        json_input.flags.no_tool_calls || chibi.app.config.no_tool_calls;

    let verbose = json_input.flags.verbose;
    let context = &json_input.context;

    output.diagnostic(&format!("[Loaded {} tool(s)]", chibi.tool_count()), verbose);

    // Resolve core config
    let mut config = chibi.resolve_config(context, json_input.username.as_deref())?;
    chibi_core::gateway::ensure_context_window(&mut config);

    let mut response_sink = sink::JsonResponseSink::new();

    // SYNC: chibi-cli also calls execute_command — check crates/chibi-cli/src/main.rs
    let _effect = chibi_core::execution::execute_command(
        &mut chibi,
        context,
        &json_input.command,
        &json_input.flags,
        &config,
        json_input.username.as_deref(),
        &output,
        &mut response_sink,
    ).await?;

    Ok(())
}
```

**Step 2: remove `execute_json_command()`**

Delete the entire `execute_json_command()` function (~380 lines).

**Step 3: clean up unused imports**

Remove imports that were only used by `execute_json_command()`:
- `Context`, `ContextEntry`, `now_timestamp` (lifecycle now in core)
- `DebugKey`, `Inspectable` (dispatch now in core)
- `PromptOptions`, `StatePaths`, `api`, `tools` (send path now in core)

**Step 4: build and run tests**

Run: `cargo build --workspace && cargo test --workspace`
Expected: compiles and all tests pass.

**Step 5: commit**

```
refactor: wire chibi-json to use core execute_command() (#143)
```

---

### task 7: verify and clean up

**Files:**
- All modified files from previous tasks

**Step 1: run full test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

**Step 2: run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings.

**Step 3: verify no dead code**

Check that removed functions from CLI are truly gone and no references remain:

- `show_log` should only exist in `execution.rs`
- `inspect_context` should only exist in `execution.rs`
- `set_prompt_for_context` should be gone (inlined in `execution.rs`)
- `send_with_cli_sink` should be gone
- `CliSendOptions` should be gone
- `execute_json_command` should be gone

**Step 4: update AGENTS.md architecture section**

Add `execution.rs` to the chibi-core file listing:

```
- `execution.rs` — Shared command dispatch (`execute_command`), lifecycle management
```

**Step 5: commit**

```
chore: cleanup dead code, update architecture docs (#143)
```

**Step 6: remind user**

Run `just pre-push` before pushing.
