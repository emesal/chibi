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
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-V")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("chibi"));
}

#[test]
fn integration_which_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-w")
        .output()
        .expect("failed to run chibi");

    assert!(output.status.success());
    // Should output a context name (non-empty)
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.trim().is_empty());
}

#[test]
fn integration_list_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-l")
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
fn integration_exclusive_commands_fail() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-l", "-w"])
        .output()
        .expect("failed to run chibi");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Only one command"));
}

#[test]
fn integration_command_with_prompt_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .args(["-l", "hello"])
        .output()
        .expect("failed to run chibi");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Cannot specify both"));
}

// =============================================================================
// Prompt behavior tests
// These verify that bare words are treated as prompts, not subcommands
// =============================================================================

/// Verify "-l" lists contexts (the correct way to invoke the list command)
#[test]
fn integration_dash_l_lists_contexts() {
    let output = Command::new(env!("CARGO_BIN_EXE_chibi"))
        .arg("-l")
        .output()
        .expect("failed to run chibi");

    // -l should succeed and return quickly (no API call)
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
