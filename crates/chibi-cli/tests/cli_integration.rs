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
//! Tests that create contexts use `--destroy-after-inactive 1`
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

    // Both should execute (force_call_user implied)
    assert!(output.status.success());
}

#[test]
fn integration_output_operations_dont_invoke_llm() {
    // -L with a prompt argument should still work
    // (the prompt is ignored when force_call_user is implied)
    let output = run_chibi_with_test_home(&["-L", "hello"]);

    // Should succeed - -L implies force_call_user, so prompt is not sent
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

/// Test ephemeral context with -C new
#[test]
fn integration_ephemeral_new_context() {
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

// =============================================================================
// Previous context reference (-) tests
// =============================================================================

#[test]
fn integration_switch_to_previous_context() {
    let temp_home = setup_test_home();

    // Switch to context "prev_test_1"
    let output1 = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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
fn integration_ephemeral_switch_to_previous() {
    let temp_home = setup_test_home();

    // Create and switch to contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
            "-c",
            "eph_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
            "-c",
            "eph_2",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Use "-C -" to ephemerally use previous
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-C", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("eph_1"), "Should ephemerally use eph_1");
}

#[test]
fn integration_delete_previous_context() {
    let temp_home = setup_test_home();

    // Create contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
            "-c",
            "del_test_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
            "-c",
            "arch_test_1",
            "-l",
        ])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args([
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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
            "--destroy-after-inactive",
            "1",
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

// =============================================================================
// Session persistence integration tests (issue #81)
// =============================================================================

/// Helper to read and parse session.json from a temp home
fn read_session(temp_home: &TempDir) -> serde_json::Value {
    let session_path = temp_home.path().join("session.json");
    if session_path.exists() {
        let content = fs::read_to_string(&session_path).expect("failed to read session.json");
        serde_json::from_str(&content).expect("failed to parse session.json")
    } else {
        serde_json::json!({})
    }
}

/// Test that `-c name` creates context and updates session.json
#[test]
fn integration_session_switch_persists() {
    let temp_home = setup_test_home();

    // Switch to a new context
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "session_test_1", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output.status.success());

    // Verify session.json was updated
    let session = read_session(&temp_home);
    assert_eq!(
        session["implied_context"], "session_test_1",
        "session.json should have updated implied_context"
    );
}

/// Test that `-C name` (ephemeral) does NOT persist to session.json
#[test]
fn integration_session_ephemeral_does_not_persist() {
    let temp_home = setup_test_home();

    // First, switch to a context persistently to establish session
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "persistent_ctx", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Now use ephemeral switch
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-C", "ephemeral_ctx", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ephemeral_ctx"),
        "Should be using ephemeral_ctx"
    );

    // Verify session.json still has persistent_ctx as implied
    let session = read_session(&temp_home);
    assert_eq!(
        session["implied_context"], "persistent_ctx",
        "session.json should NOT be updated by ephemeral switch"
    );
}

// Note: Destroy fallback tests are in session.rs unit tests.
// Integration testing of destroy requires TTY confirmation which can't be
// simulated in subprocess tests (confirm_action returns false for non-TTY stdin).
// See Session::handle_context_destroyed for the fallback logic.

/// Test that renaming current context updates session.json
#[test]
fn integration_session_rename_updates_current() {
    let temp_home = setup_test_home();

    // Create a context
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "rename_me", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "rename_me");

    // Rename current context
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-r", "new_name"]) // rename current to new_name
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(
        output.status.success(),
        "rename should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify session.json was updated
    let session = read_session(&temp_home);
    assert_eq!(
        session["implied_context"], "new_name",
        "session should reflect renamed context"
    );
}

/// Test that renaming previous context updates session.previous_context
#[test]
fn integration_session_rename_updates_previous() {
    let temp_home = setup_test_home();

    // Create two contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "old_prev", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "current", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "current");
    assert_eq!(session["previous_context"], "old_prev");

    // Rename the previous context
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-R", "old_prev", "new_prev"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(
        output.status.success(),
        "rename should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify session.json was updated
    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "current");
    assert_eq!(
        session["previous_context"], "new_prev",
        "previous_context should be updated after rename"
    );
}

/// Test that session.json records correct previous_context after multiple switches
#[test]
fn integration_session_tracks_previous_correctly() {
    let temp_home = setup_test_home();

    // Switch through multiple contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_1", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Previous should be "default" (the initial context)
    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "ctx_1");
    assert_eq!(session["previous_context"], "default");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_2", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "ctx_2");
    assert_eq!(
        session["previous_context"], "ctx_1",
        "previous should be ctx_1, not default"
    );

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_3", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "ctx_3");
    assert_eq!(
        session["previous_context"], "ctx_2",
        "previous should be ctx_2"
    );
}

/// Test that switching to current context doesn't change previous
#[test]
fn integration_session_switch_to_same_preserves_previous() {
    let temp_home = setup_test_home();

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_a", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_b", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["previous_context"], "ctx_a");

    // Switch to same context (ctx_b)
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "ctx_b", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "ctx_b");
    assert_eq!(
        session["previous_context"], "ctx_a",
        "previous should still be ctx_a, not ctx_b"
    );
}

/// Test that swap (`-c -`) updates session.json correctly
#[test]
fn integration_session_swap_persists() {
    let temp_home = setup_test_home();

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "swap_a", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "swap_b", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Now swap
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert!(output.status.success());

    let session = read_session(&temp_home);
    assert_eq!(
        session["implied_context"], "swap_a",
        "after swap, current should be swap_a"
    );
    assert_eq!(
        session["previous_context"], "swap_b",
        "after swap, previous should be swap_b"
    );

    // Swap again
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "-", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    let session = read_session(&temp_home);
    assert_eq!(session["implied_context"], "swap_b");
    assert_eq!(session["previous_context"], "swap_a");
}

/// Test session.json is not created until first persistent switch
#[test]
fn integration_session_created_on_first_switch() {
    let temp_home = setup_test_home();
    let session_path = temp_home.path().join("session.json");

    // Initially no session.json
    assert!(
        !session_path.exists(),
        "session.json should not exist initially"
    );

    // Run a command that doesn't switch contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-L"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Still no session.json (no switch happened)
    assert!(
        !session_path.exists(),
        "session.json should not exist after -L"
    );

    // Now switch contexts
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-c", "first_ctx", "-l"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Now session.json should exist
    assert!(
        session_path.exists(),
        "session.json should exist after first switch"
    );
}

// =============================================================================
// Username override tests (persistent vs ephemeral)
// =============================================================================

/// Helper to create a test home with a specific username in config
fn setup_test_home_with_username(username: &str) -> TempDir {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_content = format!(
        r#"
api_key = "test-key-not-real"
model = "test-model"
context_window_limit = 8000
warn_threshold_percent = 75.0
username = "{}"
"#,
        username
    );
    fs::write(temp_dir.path().join("config.toml"), config_content)
        .expect("failed to write config.toml");
    temp_dir
}

#[test]
fn integration_username_from_global_config() {
    // Username should come from config.toml when no override is specified
    let temp_home = setup_test_home_with_username("globaluser");

    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "globaluser");
}

#[test]
fn integration_persistent_username_saves_to_local_config() {
    // -u should save the username to local.toml
    let temp_home = setup_test_home_with_username("globaluser");

    // Set persistent username
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-u", "persistentuser"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());

    // Verify the username was saved to local.toml
    let local_config_path = temp_home.path().join("contexts/default/local.toml");
    assert!(
        local_config_path.exists(),
        "local.toml should exist after -u"
    );

    let local_content = fs::read_to_string(&local_config_path).expect("failed to read local.toml");
    assert!(
        local_content.contains("persistentuser"),
        "local.toml should contain the persistent username"
    );

    // Now verify -n username shows the persistent username (not the global one)
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "persistentuser",
        "username should be persistentuser after -u"
    );
}

#[test]
fn integration_ephemeral_username_overrides_but_does_not_persist() {
    // -U should override the username for this invocation only, not save to local.toml
    let temp_home = setup_test_home_with_username("globaluser");

    // First, set a persistent username so we have local.toml
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-u", "persistentuser"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Verify it's set
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "persistentuser"
    );

    // Now use ephemeral username and check it via -n in same invocation
    // Note: -U and -n together should show the ephemeral username
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-U", "ephemeraluser", "-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "ephemeraluser",
        "username should be ephemeraluser when using -U"
    );

    // Verify local.toml still has persistentuser (ephemeral didn't persist)
    let local_config_path = temp_home.path().join("contexts/default/local.toml");
    let local_content = fs::read_to_string(&local_config_path).expect("failed to read local.toml");
    assert!(
        local_content.contains("persistentuser"),
        "local.toml should still contain persistentuser, not ephemeraluser"
    );
    assert!(
        !local_content.contains("ephemeraluser"),
        "local.toml should NOT contain ephemeraluser"
    );

    // And verify that without -U, we get the persistent username again
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "persistentuser",
        "username should be back to persistentuser without -U"
    );
}

#[test]
fn integration_ephemeral_username_overrides_persistent() {
    // -U should take priority over what's in local.toml
    let temp_home = setup_test_home_with_username("globaluser");

    // Set persistent username first
    let _ = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-u", "persistentuser"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    // Now verify -U overrides the persistent username
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-U", "ephemeraluser", "-n", "username"])
        .env("CHIBI_HOME", temp_home.path())
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "ephemeraluser",
        "ephemeral username (-U) should override persistent username from local.toml"
    );
}
