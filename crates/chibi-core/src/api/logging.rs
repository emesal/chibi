//! Debug logging utilities for API requests and responses.
//!
//! These utilities log API interactions to JSONL files in the specified context's
//! directory when debug logging is enabled.

use crate::context::now_timestamp;
use crate::input::DebugKey;
use crate::state::AppState;
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;

/// Log data to a JSONL file in the specified context's directory if debug logging is enabled.
/// The `required_key` specifies which DebugKey enables this log. `DebugKey::All` always matches.
pub fn log_to_jsonl(
    app: &AppState,
    context_name: &str,
    debug: &[DebugKey],
    required_key: DebugKey,
    filename: &str,
    data_key: &str,
    data: &serde_json::Value,
) {
    let should_log = debug
        .iter()
        .any(|k| matches!(k, DebugKey::All) || *k == required_key);
    if !should_log {
        return;
    }

    let log_entry = json!({
        "timestamp": now_timestamp(),
        data_key: data,
    });

    let log_path = app.context_dir(context_name).join(filename);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path)
        && let Ok(json) = serde_json::to_string(&log_entry)
    {
        let _ = writeln!(file, "{}", json);
    }
}

/// Log an API request to requests.jsonl if debug logging is enabled
pub fn log_request_if_enabled(
    app: &AppState,
    context_name: &str,
    debug: &[DebugKey],
    request_body: &serde_json::Value,
) {
    log_to_jsonl(
        app,
        context_name,
        debug,
        DebugKey::RequestLog,
        "requests.jsonl",
        "request",
        request_body,
    );
}

/// Log response metadata to response_meta.jsonl if debug logging is enabled
pub fn log_response_meta_if_enabled(
    app: &AppState,
    context_name: &str,
    debug: &[DebugKey],
    response_meta: &serde_json::Value,
) {
    log_to_jsonl(
        app,
        context_name,
        debug,
        DebugKey::ResponseMeta,
        "response_meta.jsonl",
        "response",
        response_meta,
    );
}
