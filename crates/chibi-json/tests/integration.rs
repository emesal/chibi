use std::process::Command;

/// Helper: run chibi-json with JSON input on stdin
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
    let (_, _, success) = run_chibi_json("not json");
    assert!(!success);
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
