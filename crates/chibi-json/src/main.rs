use std::io::{self, Read};

use chibi_core::input::Command;
use chibi_core::{Chibi, LoadOptions, OutputSink};

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

    let mut chibi = Chibi::load_with_options(LoadOptions {
        verbose: json_input.flags.verbose,
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

    output.diagnostic(&format!("[Loaded {} tool(s)]", chibi.tool_count()), verbose);

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
    // per-invocation URL policy override (highest priority, whole-object)
    if json_input.url_policy.is_some() {
        resolved.url_policy = json_input.url_policy.clone();
    }
    let mut response_sink = sink::JsonResponseSink::new();

    // Delegate to core — handles init, auto-destroy, touch, dispatch, shutdown, cache cleanup
    chibi_core::execute_command(
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

    Ok(())
}
