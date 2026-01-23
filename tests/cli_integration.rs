//! Integration tests for CLI behavior
//!
//! These tests verify the documented CLI patterns from README.md work correctly.
//! They test the actual binary behavior, not just argument parsing.
//!
//! Note: Tests that would send prompts to the LLM are avoided here since they
//! require API keys and network access. Those behaviors are tested via unit
//! tests in cli.rs which verify the argument parsing produces the correct
//! Cli struct.

use std::process::Command;

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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-l")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain "Context:" showing current context info
    assert!(stdout.contains("Context:"));
}

#[test]
fn integration_list_contexts() {
    // -L lists all contexts
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-L")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
}

#[test]
fn integration_unknown_flag_is_treated_as_prompt() {
    // With the clap-based parser, unknown flags are treated as positional args (prompt)
    // This is more permissive - users can ask "what does --unknown-flag mean?"
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--unknown-flag")
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-l", "-L"])
        .output()
        .expect("failed to run chibi");

    // Both should execute (no_chibi implied)
    assert!(output.status.success());
}

#[test]
fn integration_output_operations_dont_invoke_llm() {
    // -L with a prompt argument should still work
    // (the prompt is ignored when no_chibi is implied)
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-L", "hello"])
        .output()
        .expect("failed to run chibi");

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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-L")
        .output()
        .expect("failed to run chibi");

    // -L should succeed and return quickly (no API call)
    assert!(output.status.success());
}

/// Similarly "chibi help" should be a prompt, not trigger --help
#[test]
fn integration_help_word_is_prompt() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("help")
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("version")
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-V")
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-Dnonexistent_test_context_12345")
        .output()
        .expect("failed to run chibi");

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
    // -c new should switch to a new auto-generated context
    // Since -c alone doesn't produce output without prompt, use -l to see the context
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "new", "-l"])
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "new:myprefix", "-l"])
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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-C", "new", "-l"])
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

    let mut child = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--json-config")
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
