# Output Discipline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Route all chibi-cli output through `OutputHandler` for consistency and testability.

**Architecture:** No new abstractions. Pass `&OutputHandler` to functions that bypass it, convert raw `println!`/`eprintln!` to `output.emit_result()` / `output.diagnostic()`. Four small tasks, one commit.

**Tech Stack:** Rust, chibi-cli's existing `OutputHandler`.

---

## Progress

| Task | Status | Commit | Notes |
|------|--------|--------|-------|
| 1 | done | | |
| 2 | done | | |
| 3 | done | | |
| 4 | done | | |

---

### Task 1: Move `OutputHandler` construction earlier in `main()`

**Files:**
- Modify: `crates/chibi-cli/src/main.rs:1214-1241`

**Step 1: Move `OutputHandler::new()` up and convert verbose tool list**

In `main()`, the `OutputHandler` is constructed on line 1241 — after the verbose tool list (lines 1217–1239). Move it up to right after flag resolution, then convert the verbose `eprintln!` calls to `output.diagnostic()`.

Before (lines 1214–1241):
```rust
    let mut session = Session::load(chibi.home_dir())?;

    // Print tool lists if verbose
    if verbose {
        let builtin_names = chibi_core::tools::builtin_tool_names();
        eprintln!(
            "[Built-in ({}): {}]",
            builtin_names.len(),
            builtin_names.join(", ")
        );

        if chibi.tools.is_empty() {
            eprintln!("[No plugins loaded]");
        } else {
            eprintln!(
                "[Plugins ({}): {}]",
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
```

After:
```rust
    let mut session = Session::load(chibi.home_dir())?;
    let output = OutputHandler::new(input.flags.json_output);

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
```

**Step 2: Verify it compiles**

Run: `cargo build -p chibi-cli`
Expected: success

---

### Task 2: Route `inspect_context()` through `OutputHandler`

**Files:**
- Modify: `crates/chibi-cli/src/main.rs:216-294` (function signature + body)
- Modify: `crates/chibi-cli/src/main.rs:656` (call site)

**Step 1: Add `output: &OutputHandler` parameter and update call site**

Change function signature from:
```rust
fn inspect_context(
    chibi: &Chibi,
    context_name: &str,
    thing: &Inspectable,
    resolved_config: Option<&ResolvedConfig>,
    force_markdown: bool,
) -> io::Result<()> {
```

To:
```rust
fn inspect_context(
    chibi: &Chibi,
    context_name: &str,
    thing: &Inspectable,
    resolved_config: Option<&ResolvedConfig>,
    force_markdown: bool,
    output: &OutputHandler,
) -> io::Result<()> {
```

Update call site (line 656) to pass `output`:
```rust
inspect_context(chibi, &ctx_name, thing, Some(&config), force_markdown, output)?;
```

**Step 2: Convert `println!`/`print!` calls to `output.emit_result()`**

In the match body, convert each arm. The key conversions:

`Inspectable::List`:
```rust
Inspectable::List => {
    output.emit_result("Inspectable items:");
    for name in <Inspectable as InspectableExt>::all_names_cli() {
        output.emit_result(&format!("  {}", name));
    }
}
```

`Inspectable::SystemPrompt`:
```rust
Inspectable::SystemPrompt => {
    let prompt = chibi.app.load_system_prompt_for(context_name)?;
    if prompt.is_empty() {
        output.emit_result("(no system prompt set)");
    } else {
        output.emit_result(prompt.trim_end());
    }
}
```

`Inspectable::Reflection`:
```rust
Inspectable::Reflection => {
    let reflection = chibi.app.load_reflection()?;
    if reflection.is_empty() {
        output.emit_result("(no reflection set)");
    } else {
        output.emit_result(reflection.trim_end());
    }
}
```

`Inspectable::Todos` and `Inspectable::Goals` — these use `render_markdown_output()` which writes directly to stdout for TTY rendering. Leave the markdown path as-is (it's correct for CLI), but convert the empty-state messages:
```rust
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
```

(Same pattern for `Goals`.)

`Inspectable::Home`:
```rust
Inspectable::Home => {
    output.emit_result(&chibi.home_dir().display().to_string());
}
```

`Inspectable::ConfigField`:
```rust
Inspectable::ConfigField(field_path) => match config.get_field(field_path) {
    Some(value) => output.emit_result(&value.to_string()),
    None => output.emit_result("(not set)"),
},
```

**Step 3: Verify it compiles**

Run: `cargo build -p chibi-cli`
Expected: success

---

### Task 3: Route `show_log()` through `OutputHandler`

**Files:**
- Modify: `crates/chibi-cli/src/main.rs:297-372` (function signature + body)
- Modify: `crates/chibi-cli/src/main.rs:646` (call site)

**Step 1: Add `output: &OutputHandler` parameter and update call site**

Change signature from:
```rust
fn show_log(
    chibi: &Chibi,
    context_name: &str,
    num: isize,
    verbose: bool,
    resolved_config: &ResolvedConfig,
    force_markdown: bool,
) -> io::Result<()> {
```

To:
```rust
fn show_log(
    chibi: &Chibi,
    context_name: &str,
    num: isize,
    verbose: bool,
    resolved_config: &ResolvedConfig,
    force_markdown: bool,
    output: &OutputHandler,
) -> io::Result<()> {
```

Update call site (line 646):
```rust
show_log(chibi, &ctx_name, *count, verbose, &config, force_markdown, output)?;
```

**Step 2: Convert `println!` calls to `output.emit_result()`**

In the for loop over selected entries:

```rust
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
        output.emit_result(&format!("[{}]: {}\n", entry.entry_type.to_uppercase(), entry.content));
    }
}
```

**Step 3: Verify it compiles**

Run: `cargo build -p chibi-cli`
Expected: success

---

### Task 4: Route `ModelMetadata` through `OutputHandler`, build, and commit

**Files:**
- Modify: `crates/chibi-cli/src/main.rs:1006-1014` (ModelMetadata match arm)

**Step 1: Convert `print!` to `output.emit_result()`**

Change from:
```rust
Command::ModelMetadata { model, full } => {
    let resolved = chibi.resolve_config(&working_context, None)?;
    let gateway = chibi_core::gateway::build_gateway(&resolved)?;
    let metadata = chibi_core::model_info::fetch_metadata(&gateway, model).await?;
    print!(
        "{}",
        chibi_core::model_info::format_model_toml(&metadata, *full)
    );
    did_action = true;
}
```

To:
```rust
Command::ModelMetadata { model, full } => {
    let resolved = chibi.resolve_config(&working_context, None)?;
    let gateway = chibi_core::gateway::build_gateway(&resolved)?;
    let metadata = chibi_core::model_info::fetch_metadata(&gateway, model).await?;
    output.emit_result(
        chibi_core::model_info::format_model_toml(&metadata, *full).trim_end(),
    );
    did_action = true;
}
```

**Step 2: Run full build and tests**

Run: `cargo build -p chibi-cli && cargo test -p chibi-cli`
Expected: success

**Step 3: Commit**

```bash
git add crates/chibi-cli/src/main.rs
git commit -m "refactor: route all chibi-cli output through OutputHandler (#14)

inspect_context(), show_log(), ModelMetadata, and verbose tool list
now go through OutputHandler.emit_result() / .diagnostic() instead
of raw println!/eprintln!. Prepares for chibi-json extraction."
```

---

## Summary

| Task | What |
|------|------|
| 1 | Move OutputHandler construction up, convert verbose tool list |
| 2 | Route inspect_context() through OutputHandler |
| 3 | Route show_log() through OutputHandler |
| 4 | Route ModelMetadata, build, test, commit |

All four tasks modify only `crates/chibi-cli/src/main.rs`. Single commit at the end.
