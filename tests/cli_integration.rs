//! Integration tests for CLI behavior
//!
//! These tests verify the documented CLI patterns from README.md work correctly.
//! They test the actual binary behavior, not just argument parsing.
//!
//! Note: Tests that would send prompts to the LLM are avoided here since they
//! require API keys and network access. Those behaviors are tested via unit
//! tests in cli.rs which verify the argument parsing produces the correct
//! Cli struct.
//!
//! ## Context Cleanup
//!
//! Tests that create contexts use `--debug destroy_after_seconds_inactive=1`
//! to mark them for automatic cleanup. This ensures test contexts don't
//! accumulate in the user's ~/.chibi directory. The auto-destroy feature
//! runs at every chibi invocation, so subsequent normal usage cleans up
//! these test contexts automatically.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Create a temporary CHIBI_HOME with minimal config for testing.
/// Returns the TempDir (must be kept alive for the duration of the test).
fn setup_test_home() -> TempDir {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_content = r#"
api_key = "test-key-not-real"
model = "test-model"
context_window_limit = 8000
warn_threshold_percent = 75.0
"#;
    fs::write(temp_dir.path().join("config.toml"), config_content)
        .expect("failed to write config.toml");
    temp_dir
}

/// Run chibi with CHIBI_HOME set to a temp directory
fn run_chibi_with_test_home(args: &[&str]) -> std::process::Output {
    let temp_home = setup_test_home();
    Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(args)
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi")
}

// =============================================================================
// CLI parsing tests (no API calls needed)
// =============================================================================

#[test]
fn integration_help_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-h")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi"));
    assert!(stdout.contains("Usage"));
}

#[test]
fn integration_version_flag() {
    // --version (no -V short form anymore)
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--version")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi"));
}

#[test]
fn integration_list_current_context() {
    // -l now shows current context info
    let output = run_chibi_with_test_home(&["-l"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain "Context:" showing current context info
    assert!(stdout.contains("Context:"));
}

#[test]
fn integration_list_contexts() {
    // -L lists all contexts
    let output = run_chibi_with_test_home(&["-L"]);

    assert!(output.status.success());
}

#[test]
fn integration_unknown_flag_is_treated_as_prompt() {
    // With the clap-based parser, unknown flags are treated as positional args (prompt)
    // This is more permissive - users can ask "what does --unknown-flag mean?"
    let temp_home = setup_test_home();
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--unknown-flag")
        .env("CHIBI_HOME", temp_home.path())
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("failed to run chibi");

    // May fail due to missing API key (treating --unknown-flag as prompt), but not
    // due to unknown flag parsing. The key thing is it doesn't error with "Unknown option"
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Unknown option"));
}

#[test]
fn integration_multiple_operations_can_combine() {
    // In the new CLI design, multiple operations can combine
    // -l and -L together should work (both execute, no error)
    let output = run_chibi_with_test_home(&["-l", "-L"]);

    // Both should execute (no_chibi implied)
    assert!(output.status.success());
}

#[test]
fn integration_output_operations_dont_invoke_llm() {
    // -L with a prompt argument should still work
    // (the prompt is ignored when no_chibi is implied)
    let output = run_chibi_with_test_home(&["-L", "hello"]);

    // Should succeed - -L implies no_chibi, so prompt is not sent
    assert!(output.status.success());
}

// =============================================================================
// Prompt behavior tests
// These verify that bare words are treated as prompts, not subcommands
// =============================================================================

/// Verify "-L" lists all contexts
#[test]
fn integration_dash_upper_l_lists_contexts() {
    let output = run_chibi_with_test_home(&["-L"]);

    // -L should succeed and return quickly (no API call)
    assert!(output.status.success());
}

/// Similarly "chibi help" should be a prompt, not trigger --help
#[test]
fn integration_help_word_is_prompt() {
    let temp_home = setup_test_home();
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("help")
        .env("CHIBI_HOME", temp_home.path())
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("failed to run chibi");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should NOT show help text (that would mean it parsed "help" as --help)
    assert!(
        !stdout.contains("Usage:"),
        "Should not have treated 'help' as --help flag"
    );
}

/// "chibi version" should be a prompt, not trigger --version
#[test]
fn integration_version_word_is_prompt() {
    let temp_home = setup_test_home();
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("version")
        .env("CHIBI_HOME", temp_home.path())
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("failed to run chibi");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should NOT be exactly "chibi X.Y.Z" (the version format)
    // It might fail due to missing API key, or return something else
    // But definitely should not match the version pattern "chibi \d+\.\d+\.\d+"
    let version_pattern = regex::Regex::new(r"^chibi \d+\.\d+\.\d+$").unwrap();
    let first_line = stdout.lines().next().unwrap_or("");
    assert!(
        !version_pattern.is_match(first_line.trim()),
        "Should not have treated 'version' as --version flag, got: {}",
        stdout
    );
}

/// Test -V is treated as a prompt (not a flag, not an error)
#[test]
fn integration_short_v_is_treated_as_prompt() {
    // With clap-based parser, -V is treated as positional arg (prompt)
    // This is more permissive - users can ask "what does -V mean?"
    let temp_home = setup_test_home();
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-V")
        .env("CHIBI_HOME", temp_home.path())
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("failed to run chibi");

    // Should NOT error with "Unknown option" - it's treated as prompt
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Unknown option"));
}

/// Test attached argument form (-Dname)
#[test]
fn integration_attached_arg_form() {
    // -Dnonexistent should work as a valid form (even if context doesn't exist)
    let output = run_chibi_with_test_home(&["-Dnonexistent_test_context_12345"]);

    // Should succeed (context not found is not an error, just prints message)
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("not found"));
}

// =============================================================================
// "new" and "new:prefix" context name tests
// =============================================================================

/// Test that -c new creates a timestamped context name
#[test]
fn integration_switch_to_new_context() {
    let temp_home = setup_test_home();

    // -c new should switch to a new auto-generated context
    // Since -c alone doesn't produce output without prompt, use -l to see the context
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "new", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show a context name that looks like a timestamp: YYYYMMDD_HHMMSS
    // Pattern: 8 digits, underscore, 6 digits
    assert!(stdout.contains("Context:"), "Should show context info");

    // The context name should be in the format YYYYMMDD_HHMMSS
    // Check that stdout contains something matching this pattern
    let has_timestamp_format = stdout.lines().any(|line| {
        if line.starts_with("Context:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[1];
                // Check if name looks like YYYYMMDD_HHMMSS
                let chars: Vec<char> = name.chars().collect();
                chars.len() >= 15
                    && chars[0..8].iter().all(|c| c.is_ascii_digit())
                    && chars[8] == '_'
                    && chars[9..15].iter().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        } else {
            false
        }
    });

    assert!(
        has_timestamp_format,
        "Context name should be in YYYYMMDD_HHMMSS format, got: {}",
        stdout
    );
}

/// Test that -c new:prefix creates a prefixed timestamped context name
#[test]
fn integration_switch_to_new_context_with_prefix() {
    let temp_home = setup_test_home();

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "new:myprefix", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show a context name starting with "myprefix_"
    assert!(
        stdout.contains("myprefix_"),
        "Context name should start with prefix, got: {}",
        stdout
    );
}

/// Test that -c new: (empty prefix) is an error
#[test]
fn integration_new_context_empty_prefix_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "new:", "-l"])
        .output()
        .expect("failed to run chibi");

    // Should fail - empty prefix is not allowed
    assert!(
        !output.status.success(),
        "Empty prefix should cause an error"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty") || stderr.contains("Prefix"),
        "Error should mention empty prefix, got: {}",
        stderr
    );
}

/// Test transient context with -C new
#[test]
fn integration_transient_new_context() {
    let temp_home = setup_test_home();

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-C", "new", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show context info with a timestamped name
    assert!(stdout.contains("Context:"));
}

// =============================================================================
// JSON config mode tests
// =============================================================================

/// Test JSON config mode with --json-config
#[test]
fn integration_json_config_mode_help() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    // Send help command via JSON
    let json_input = r#"{"command": "show_help"}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage") || stdout.contains("chibi"));
}

/// Test JSON config mode with version command
#[test]
fn integration_json_config_mode_version() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    let json_input = r#"{"command": "show_version"}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi"));
}

/// Test JSON config mode with list_contexts command
#[test]
fn integration_json_config_mode_list_contexts() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    let json_input = r#"{"command": "list_contexts"}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
}

/// Test JSON config mode with context switch
#[test]
fn integration_json_config_mode_context_switch() {
    use std::io::Write;
    use std::process::Stdio;

    let temp_home = setup_test_home();

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-config")
        .env("CHIBI_HOME", temp_home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    // Switch to a context transiently and list current context info
    let json_input = r#"{
        "command": "list_current_context",
        "context": {"transient": {"name": "json_test_context"}}
    }"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("json_test_context"));
}

// =============================================================================
// --json-schema tests
// =============================================================================

/// Test --json-schema outputs valid JSON schema and exits
#[test]
fn integration_json_schema_outputs_valid_schema() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-schema")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should be valid JSON
    let schema: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json-schema output should be valid JSON");

    // Should be a JSON Schema document
    assert_eq!(
        schema["$schema"], "http://json-schema.org/draft-07/schema#",
        "Should be a JSON Schema draft-07 document"
    );
    assert_eq!(schema["title"], "ChibiInput");
    assert_eq!(schema["type"], "object");

    // Should define the Command type
    assert!(
        schema["definitions"]["Command"].is_object(),
        "Should define Command type"
    );
}

/// Test --json-schema ignores all other flags
#[test]
fn integration_json_schema_ignores_other_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["--json-schema", "-v", "-L", "--json-output"])
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should still be valid JSON schema, not list-contexts output
    let schema: serde_json::Value =
        serde_json::from_str(&stdout).expect("should output schema even with other flags");
    assert_eq!(schema["title"], "ChibiInput");
}

/// Test --json-schema schema contains expected command variants
#[test]
fn integration_json_schema_contains_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-schema")
        .output()
        .expect("failed to run chibi");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // The Command definition should reference key command names
    let schema_str = serde_json::to_string(&schema["definitions"]["Command"]).unwrap();
    assert!(
        schema_str.contains("send_prompt"),
        "should document send_prompt"
    );
    assert!(
        schema_str.contains("list_contexts"),
        "should document list_contexts"
    );
    assert!(
        schema_str.contains("destroy_context"),
        "should document destroy_context"
    );
}

/// Test --json-schema does not require a chibi home directory
#[test]
fn integration_json_schema_no_home_needed() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-schema")
        .env("CHIBI_HOME", "/nonexistent/path/that/does/not/exist")
        .output()
        .expect("failed to run chibi");

    // Should succeed even with a bogus home dir (exits before loading state)
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let schema: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(schema["title"], "ChibiInput");
}

// =============================================================================
// Inspect config fields tests (issue #18)
// =============================================================================

#[test]
fn integration_inspect_list_includes_config_fields() {
    // -n list should show both file-based items and config fields
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-n")
        .arg("list")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // File-based items
    assert!(stdout.contains("system_prompt"));
    assert!(stdout.contains("todos"));
    assert!(stdout.contains("goals"));
    assert!(stdout.contains("reflection"));

    // Config fields (issue #18)
    assert!(stdout.contains("model"));
    assert!(stdout.contains("username"));
    assert!(stdout.contains("api.temperature"));
    assert!(stdout.contains("api.reasoning.effort"));
}

#[test]
fn integration_inspect_config_field_model() {
    // -n model should output the configured model
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-n")
        .arg("model")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should output something (the model name from config)
    // We can't test the exact value since it depends on user's config
    assert!(!stdout.is_empty());
}

#[test]
fn integration_inspect_config_field_username() {
    // -n username should output the configured username
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-n")
        .arg("username")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should output something (the username from config)
    assert!(!stdout.is_empty());
}

#[test]
fn integration_inspect_invalid_config_field() {
    // -n with an invalid field should fail
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-n")
        .arg("invalid_nonexistent_field")
        .output()
        .expect("failed to run chibi");

    // Should fail since the field doesn't exist
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid") || stderr.contains("Unknown"));
}

// =============================================================================
// --home flag tests
// =============================================================================

/// Test --home flag overrides default directory
#[test]
fn integration_home_flag() {
    let temp_home = setup_test_home();
    let temp_path = temp_home.path().to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["--home", &temp_path, "-n", "home"])
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim() == temp_path,
        "Expected home to be '{}', got '{}'",
        temp_path,
        stdout.trim()
    );
}

/// Test --home=path form (attached argument)
#[test]
fn integration_home_flag_attached() {
    let temp_home = setup_test_home();
    let temp_path = temp_home.path().to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg(format!("--home={}", temp_path))
        .args(["-n", "home"])
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim() == temp_path,
        "Expected home to be '{}', got '{}'",
        temp_path,
        stdout.trim()
    );
}

/// Test --home takes precedence over CHIBI_HOME env var
#[test]
fn integration_home_flag_overrides_env() {
    let temp_home = setup_test_home();
    let temp_path = temp_home.path().to_string_lossy().to_string();

    // Create another temp dir for the env var (should be ignored)
    let env_home = setup_test_home();

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["--home", &temp_path, "-n", "home"])
        .env("CHIBI_HOME", env_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim() == temp_path,
        "--home should override CHIBI_HOME env var"
    );
}

/// Test --home works with --json-config
#[test]
fn integration_home_flag_with_json_config() {
    use std::io::Write;
    use std::process::Stdio;

    let temp_home = setup_test_home();
    let temp_path = temp_home.path().to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["--home", &temp_path, "--json-config"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    let json_input = r#"{"command": {"inspect": {"thing": "home"}}}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim() == temp_path,
        "--home should work with --json-config, expected '{}', got '{}'",
        temp_path,
        stdout.trim()
    );
}

/// Test --home=path form works with --json-config
#[test]
fn integration_home_flag_attached_with_json_config() {
    use std::io::Write;
    use std::process::Stdio;

    let temp_home = setup_test_home();
    let temp_path = temp_home.path().to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg(format!("--home={}", temp_path))
        .arg("--json-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn chibi");

    let json_input = r#"{"command": {"inspect": {"thing": "home"}}}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(json_input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to read output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim() == temp_path,
        "--home=path should work with --json-config"
    );
}

// =============================================================================
// Previous context reference (-) tests
// =============================================================================

#[test]
fn integration_switch_to_previous_context() {
    let temp_home = setup_test_home();

    // Switch to context "prev_test_1"
    let output1 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "prev_test_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output1.status.success());

    // Switch to context "prev_test_2"
    let output2 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "prev_test_2",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output2.status.success());

    // Switch back using "-"
    let output3 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output3.status.success());
    let stdout = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout.contains("prev_test_1"),
        "Should switch back to prev_test_1"
    );
}

#[test]
fn integration_previous_context_error_when_none() {
    let temp_home = setup_test_home();

    // Try "-c -" without any prior switches
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Should fail
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No previous context") || stderr.contains("previous context"));
}

#[test]
fn integration_context_name_dash_is_invalid() {
    let temp_home = setup_test_home();

    // Create a temporary context first
    let output1 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "temp_ctx",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output1.status.success());

    // Try to rename it to "-"
    let output2 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-R", "temp_ctx", "-"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Should fail
    assert!(!output2.status.success());
    let stderr = String::from_utf8_lossy(&output2.stderr);
    assert!(stderr.contains("reserved") || stderr.contains("Invalid"));
}

#[test]
fn integration_transient_switch_to_previous() {
    let temp_home = setup_test_home();

    // Create and switch to contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "trans_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "trans_2",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Use "-C -" to transiently use previous
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-C", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("trans_1"), "Should transiently use trans_1");
}

#[test]
fn integration_delete_previous_context() {
    let temp_home = setup_test_home();

    // Create contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "del_test_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "del_test_2",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Delete previous context (del_test_1) using "-D -"
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-D", "-"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
}

#[test]
fn integration_archive_previous_context() {
    let temp_home = setup_test_home();

    // Create contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "arch_test_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "arch_test_2",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Archive previous context (arch_test_1) using "-A -"
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-A", "-"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    if !output.status.success() {
        eprintln!(
            "Archive failed with stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(output.status.success());
}

#[test]
fn integration_previous_context_with_new() {
    let temp_home = setup_test_home();

    // Switch to a context
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "first_ctx",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Create a new context
    let output1 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "new",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output1.status.success());

    // Switch back using "-"
    let output2 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output2.status.success());
    let stdout = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout.contains("first_ctx"),
        "Should switch back to first_ctx"
    );
}

#[test]
fn integration_previous_context_swaps_like_cd() {
    let temp_home = setup_test_home();

    // Switch to context "swap_a"
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "swap_a",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Switch to context "swap_b"
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--debug",
            "destroy_after_seconds_inactive=1",
            "-c",
            "swap_b",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Now we're in swap_b, previous is swap_a
    // Use "-c -" to go back to swap_a
    let output1 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(
        stdout1.contains("swap_a"),
        "Should be in swap_a after first -c -"
    );

    // Use "-c -" again - should go back to swap_b (swap behavior)
    let output2 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("swap_b"),
        "Should be in swap_b after second -c - (swap behavior)"
    );

    // Use "-c -" once more - should be back in swap_a
    let output3 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output3.status.success());
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout3.contains("swap_a"),
        "Should be in swap_a after third -c - (swap behavior)"
    );
}
