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
fn integration_unknown_flag_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("--unknown-flag")
        .output()
        .expect("failed to run chibi");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown option"));
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

    // Should NOT show version (that would mean it parsed "version" as --version)
    assert!(
        !stdout.starts_with("chibi "),
        "Should not have treated 'version' as --version flag"
    );
}

/// Test -V is no longer a valid flag
#[test]
fn integration_short_v_version_is_invalid() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-V")
        .output()
        .expect("failed to run chibi");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown option"));
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
