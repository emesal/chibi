use std::io::{self, Read};

use chibi_core::input::Command;
use chibi_core::{Chibi, LoadOptions, OutputSink};

mod input;
mod output;
mod sink;

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    match rt.block_on(run()) {
        Ok(()) => {}
        Err(e) => {
            let json = serde_json::json!({
                "type": "error",
                "message": e.to_string(),
            });
            println!("{}", json);
            std::process::exit(1);
        }
    }
}

async fn run() -> io::Result<()> {
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

    let json_input: input::JsonInput = serde_json::from_str(&json_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid JSON input: {}", e),
        )
    })?;

    let output = output::JsonOutputSink;

    // Pre-resolution verbose: check typed config and string-keyed overrides
    let load_verbose = json_input
        .overrides
        .as_ref()
        .and_then(|o| o.get("verbose"))
        .map(|v| v == "true")
        .or_else(|| json_input.config.as_ref().and_then(|c| c.verbose))
        .unwrap_or(false);

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose: load_verbose,
        home: json_input.home.clone(),
        project_root: json_input.project_root.clone(),
    })?;

    // Trust mode -- programmatic callers have already decided
    chibi.set_permission_handler(Box::new(|_| Ok(true)));

    let context = &json_input.context;

    output.diagnostic(&format!("[Loaded {} tool(s)]", chibi.tool_count()), load_verbose);

    // Intercept binary-specific commands before delegating to core
    match &json_input.command {
        Command::ShowHelp => {
            output.emit_result("Use --json-schema to see the input schema.");
            return Ok(());
        }
        Command::ShowVersion => {
            output.emit_result(&format!("chibi-json {}", env!("CARGO_PKG_VERSION")));
            return Ok(());
        }
        _ => {}
    }

    // Resolve core config and build response sink
    let mut resolved = chibi.resolve_config(context, json_input.username.as_deref())?;
    // Legacy per-invocation URL policy override (prefer config.url_policy instead)
    if json_input.url_policy.is_some() {
        resolved.url_policy = json_input.url_policy.clone();
    }
    // Typed config overrides (same semantics as local.toml but per-invocation)
    if let Some(ref config_override) = json_input.config {
        config_override.apply_overrides(&mut resolved);
    }
    // String-keyed overrides (highest priority, freeform escape hatch)
    if let Some(ref overrides) = json_input.overrides {
        let pairs: Vec<_> = overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        resolved
            .apply_overrides_from_pairs(&pairs)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    }
    let mut response_sink = sink::JsonResponseSink::new();

    // Delegate to core — handles init, auto-destroy, touch, dispatch, shutdown, cache cleanup
    let effect = chibi_core::execute_command(
        &mut chibi,
        context,
        &json_input.command,
        &json_input.flags,
        &resolved,
        json_input.username.as_deref(),
        &output,
        &mut response_sink,
    )
    .await?;

    // Handle config inspection effects — JSON mode only has core fields
    match &effect {
        chibi_core::CommandEffect::InspectConfigField { field, .. } => {
            match resolved.get_field(field) {
                Some(value) => output.emit_result(&value),
                None => output.emit_result("(not set)"),
            }
        }
        chibi_core::CommandEffect::InspectConfigList { .. } => {
            output.emit_result("Inspectable items:");
            for name in ["system_prompt", "reflection", "todos", "goals", "home"] {
                output.emit_result(&format!("  {}", name));
            }
            for field in chibi_core::config::ResolvedConfig::list_fields() {
                output.emit_result(&format!("  {}", field));
            }
        }
        _ => {}
    }

    Ok(())
}
