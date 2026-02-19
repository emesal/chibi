use std::path::Path;
use std::process::Command;

// === helpers ===

/// run chibi-json with JSON input on stdin
fn run_chibi_json(input: &str) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("Failed to run chibi-json");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// run chibi-json with a serde_json::Value, injecting home and project_root
fn run_chibi_json_with_home(
    mut input_json: serde_json::Value,
    home: &Path,
) -> (String, String, bool) {
    let obj = input_json
        .as_object_mut()
        .expect("input must be a JSON object");
    obj.insert("home".into(), serde_json::json!(home));
    obj.insert("project_root".into(), serde_json::json!(home));
    run_chibi_json(&input_json.to_string())
}

/// create a minimal context directory with transcript entries
fn setup_context(home: &Path, name: &str) {
    let ctx_dir = home.join("contexts").join(name);
    std::fs::create_dir_all(&ctx_dir).expect("failed to create context dir");
    let entries = concat!(
        r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}"#,
        "\n",
        r#"{"id":"2","timestamp":1234567891,"from":"ctx","to":"user","content":"hi there","entry_type":"message"}"#,
        "\n",
    );
    std::fs::write(ctx_dir.join("context.jsonl"), entries).expect("failed to write context.jsonl");
}

/// validate that every non-empty line of stdout parses as valid JSON with a "type"
/// or "entry_type" field (the latter for raw TranscriptEntry lines from show_log)
fn assert_valid_jsonl(stdout: &str) {
    for (i, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {} is not valid JSON: {}\nline: {}", i, e, line));
        assert!(
            parsed.get("type").is_some() || parsed.get("entry_type").is_some(),
            "line {} missing 'type' or 'entry_type' field: {}",
            i,
            line
        );
    }
}

// === existing flag tests ===

#[test]
fn test_json_schema_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--json-schema")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("JsonInput"));
}

#[test]
fn test_version_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--version")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi-json"));
}

#[test]
fn test_invalid_json_input() {
    let (_, stderr, success) = run_chibi_json("not json");
    assert!(!success);
    // done signal is the last line on stderr
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value =
        serde_json::from_str(done_line).expect("done signal should be valid JSON");
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["code"], "invalid_input");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .contains("Invalid JSON input")
    );
}

#[test]
fn test_show_version_command() {
    let input = serde_json::json!({
        "command": "show_version",
        "context": "default"
    });
    let (stdout, _, success) = run_chibi_json(&input.to_string());
    assert!(success, "chibi-json failed");
    assert!(stdout.contains("chibi-json"));
}

#[test]
fn test_show_help_command() {
    let input = serde_json::json!({
        "command": "show_help",
        "context": "default"
    });
    let (stdout, _, success) = run_chibi_json(&input.to_string());
    assert!(success, "chibi-json failed");
    assert!(stdout.contains("json-schema"));
}

// === command coverage tests ===

#[test]
fn test_list_contexts_empty() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "list_contexts", "context": "default"}),
        tmp.path(),
    );
    assert!(success, "list_contexts on empty home should succeed");
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_list_contexts_with_data() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "alpha");
    setup_context(tmp.path(), "beta");
    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "list_contexts", "context": "default"}),
        tmp.path(),
    );
    assert!(success, "list_contexts should succeed");
    assert!(stdout.contains("alpha"), "should list alpha context");
    assert!(stdout.contains("beta"), "should list beta context");
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_list_current_context() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "myctx");
    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "list_current_context", "context": "myctx"}),
        tmp.path(),
    );
    assert!(success, "list_current_context should succeed");
    assert!(stdout.contains("myctx"), "should mention the context name");
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_destroy_context() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "doomed");
    let ctx_dir = tmp.path().join("contexts").join("doomed");
    assert!(ctx_dir.exists(), "context dir should exist before destroy");

    let (_, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"destroy_context": {"name": "doomed"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "destroy_context should succeed");
    assert!(
        !ctx_dir.exists(),
        "context dir should be removed after destroy"
    );
}

#[test]
fn test_rename_context() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "oldname");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"rename_context": {"old": "oldname", "new": "newname"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "rename_context should succeed");
    assert!(
        tmp.path().join("contexts").join("newname").exists(),
        "new context dir should exist"
    );
    assert!(
        !tmp.path().join("contexts").join("oldname").exists(),
        "old context dir should be gone"
    );
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_show_log() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "logctx");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"show_log": {"context": "logctx", "count": 0}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "show_log should succeed");
    // show_log in json mode emits raw TranscriptEntry JSON — content appears as a field value
    assert!(
        stdout.contains("hello"),
        "should contain first entry content"
    );
    assert!(
        stdout.contains("hi there"),
        "should contain second entry content"
    );
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_show_log_empty() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    // create context dir with empty transcript
    let ctx_dir = tmp.path().join("contexts").join("empty");
    std::fs::create_dir_all(&ctx_dir).expect("failed to create context dir");
    std::fs::write(ctx_dir.join("context.jsonl"), "").expect("failed to write empty context");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"show_log": {"context": "empty", "count": 0}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "show_log on empty context should succeed");
    // no entries means no stdout content lines (or only empty)
    let content_lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        content_lines.is_empty(),
        "empty context should produce no output lines"
    );
}

#[test]
fn test_inspect_system_prompt() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "inspctx");
    // write a system prompt file
    let ctx_dir = tmp.path().join("contexts").join("inspctx");
    std::fs::write(
        ctx_dir.join("system_prompt.md"),
        "you are a helpful assistant",
    )
    .expect("failed to write system prompt");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"inspect": {"context": "inspctx", "thing": "system_prompt"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "inspect system_prompt should succeed");
    assert!(
        stdout.contains("you are a helpful assistant"),
        "should show the system prompt content"
    );
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_set_system_prompt() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "setctx");

    // set the prompt
    let (_, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"set_system_prompt": {"context": "setctx", "prompt": "Be concise"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "set_system_prompt should succeed");

    // verify via inspect
    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"inspect": {"context": "setctx", "thing": "system_prompt"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "inspect after set should succeed");
    assert!(
        stdout.contains("Be concise"),
        "inspect should show the prompt we set"
    );
}

#[test]
fn test_clear_cache() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "cachectx");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"clear_cache": {"name": "cachectx"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(
        success,
        "clear_cache should succeed on context without cache"
    );
    assert!(stdout.contains("Cleared"), "should confirm cache cleared");
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_cleanup_cache() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "default");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "cleanup_cache", "context": "default"}),
        tmp.path(),
    );
    assert!(success, "cleanup_cache should succeed");
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_noop() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "default");

    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "no_op", "context": "default"}),
        tmp.path(),
    );
    assert!(success, "no_op should succeed");
    // no_op produces no output
    let content_lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(content_lines.is_empty(), "no_op should produce no output");
}

#[test]
fn test_archive_history() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "archctx");
    let context_jsonl = tmp
        .path()
        .join("contexts")
        .join("archctx")
        .join("context.jsonl");
    assert!(
        !std::fs::read_to_string(&context_jsonl).unwrap().is_empty(),
        "context should have entries before archive"
    );

    let (_, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": {"archive_history": {"name": "archctx"}},
            "context": "default"
        }),
        tmp.path(),
    );
    assert!(success, "archive_history should succeed");
    // archive_history calls clear_context which empties context.jsonl (the LLM window)
    let content = std::fs::read_to_string(&context_jsonl).unwrap_or_default();
    assert!(
        content.trim().is_empty(),
        "context.jsonl should be empty after archive, got: {}",
        content
    );
}

// === error path tests ===

#[test]
fn test_missing_command_field() {
    let (stdout, stderr, success) = run_chibi_json(r#"{"context": "default"}"#);
    assert!(!success, "missing command should fail");
    assert!(stdout.is_empty(), "stdout should be empty on error");
    // done signal on stderr
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value = serde_json::from_str(done_line).expect("done should be JSON");
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["ok"], false);
}

#[test]
fn test_missing_context_field() {
    let (stdout, stderr, success) = run_chibi_json(r#"{"command": "no_op"}"#);
    assert!(!success, "missing context should fail");
    assert!(stdout.is_empty(), "stdout should be empty on error");
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value = serde_json::from_str(done_line).expect("done should be JSON");
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["ok"], false);
}

#[test]
fn test_unknown_command_variant() {
    let (stdout, stderr, success) =
        run_chibi_json(r#"{"command": "nonexistent_thing", "context": "default"}"#);
    assert!(!success, "unknown command should fail");
    assert!(stdout.is_empty(), "stdout should be empty on error");
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value = serde_json::from_str(done_line).expect("done should be JSON");
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["ok"], false);
}

// === JSONL format validation tests ===

#[test]
fn test_output_is_valid_jsonl() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let (stdout, _, success) = run_chibi_json_with_home(
        serde_json::json!({"command": "list_contexts", "context": "default"}),
        tmp.path(),
    );
    assert!(success);
    assert_valid_jsonl(&stdout);
}

#[test]
fn test_error_output_format() {
    let (stdout, stderr, success) = run_chibi_json(r#"{"command": "bogus", "context": "x"}"#);
    assert!(!success);
    assert!(stdout.is_empty(), "stdout should be empty on error");
    // done signal is the last line on stderr
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value =
        serde_json::from_str(done_line).expect("done signal should be valid JSON");
    assert_eq!(parsed["type"], "done", "done signal should have type=done");
    assert_eq!(parsed["ok"], false, "done signal should have ok=false");
    assert!(
        parsed.get("code").is_some(),
        "done signal should have a code field"
    );
    assert!(
        parsed.get("message").is_some(),
        "done signal should have a message field"
    );
}

#[test]
fn test_result_output_format() {
    let input = serde_json::json!({
        "command": "show_version",
        "context": "default"
    });
    let (stdout, _, success) = run_chibi_json(&input.to_string());
    assert!(success);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("result output should be valid JSON");
    assert_eq!(
        parsed["type"], "result",
        "result output should have type=result"
    );
    assert!(
        parsed.get("content").is_some(),
        "result output should have a content field"
    );
    assert!(
        parsed["content"].as_str().unwrap().contains("chibi-json"),
        "version result should contain chibi-json"
    );
}

// === per-invocation config override tests ===

#[test]
fn test_json_schema_includes_config_field() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--json-schema")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let schema: serde_json::Value =
        serde_json::from_str(&stdout).expect("schema should be valid JSON");

    // config and overrides fields should be present in the schema
    let properties = &schema["properties"];
    assert!(
        properties.get("config").is_some(),
        "schema should include 'config' field"
    );
    assert!(
        properties.get("overrides").is_some(),
        "schema should include 'overrides' field"
    );
}

#[test]
fn test_json_schema_config_contains_fuel() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi-json"))
        .arg("--json-schema")
        .output()
        .expect("Failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // fuel should appear somewhere in the schema (nested under config's LocalConfig)
    assert!(
        stdout.contains("fuel"),
        "schema should mention 'fuel' (the original bug)"
    );
}

#[test]
fn test_config_override_accepted() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "default");

    // config overrides should be accepted without error
    let (_, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": "no_op",
            "context": "default",
            "config": {"fuel": 5}
        }),
        tmp.path(),
    );
    assert!(success, "config override should be accepted");
}

#[test]
fn test_string_overrides_accepted() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "default");

    // string-keyed overrides should be accepted without error
    let (_, _, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": "no_op",
            "context": "default",
            "overrides": {"fuel": "5", "model": "test-model"}
        }),
        tmp.path(),
    );
    assert!(success, "string overrides should be accepted");
}

#[test]
fn test_invalid_string_override_errors() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    setup_context(tmp.path(), "default");

    let (stdout, stderr, success) = run_chibi_json_with_home(
        serde_json::json!({
            "command": "no_op",
            "context": "default",
            "overrides": {"fuel": "notanumber"}
        }),
        tmp.path(),
    );
    assert!(!success, "invalid override should fail");
    assert!(stdout.is_empty(), "stdout should be empty on error");
    let done_line = stderr.lines().last().expect("stderr should not be empty");
    let parsed: serde_json::Value = serde_json::from_str(done_line).expect("done should be JSON");
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["ok"], false);
}
