//! Unified input types for CLI and JSON input modes.
//!
//! This module provides the core types that represent what operation to perform
//! and how to perform it, regardless of whether the input came from CLI flags
//! or JSON input.

use crate::cli::Inspectable;
use serde::{Deserialize, Serialize};

/// What operation to perform (mutually exclusive commands)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Send a prompt to the LLM
    SendPrompt { prompt: String },
    /// List all contexts (-L)
    ListContexts,
    /// Show current context info (-l)
    ListCurrentContext,
    /// Delete a context (-d/-D)
    DeleteContext { name: Option<String> },
    /// Archive a context (-a/-A)
    ArchiveContext { name: Option<String> },
    /// Compact a context (-z/-Z)
    CompactContext { name: Option<String> },
    /// Rename a context (-r/-R)
    RenameContext { old: Option<String>, new: String },
    /// Show log entries (-g/-G)
    ShowLog {
        context: Option<String>,
        count: isize,
    },
    /// Inspect something (-n/-N)
    Inspect {
        context: Option<String>,
        thing: Inspectable,
    },
    /// Set system prompt (-y/-Y)
    SetSystemPrompt {
        context: Option<String>,
        prompt: String,
    },
    /// Run a plugin directly (-p)
    RunPlugin { name: String, args: Vec<String> },
    /// Call a tool directly (-P)
    CallTool { name: String, args: Vec<String> },
    /// Show help
    ShowHelp,
    /// Show version
    ShowVersion,
    /// No operation - context switch only, no action
    NoOp,
}

/// Behavioral modifiers (flags that affect how commands run)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Flags {
    /// Show verbose output (-v)
    #[serde(default)]
    pub verbose: bool,
    /// Output in JSON format (--json-output)
    #[serde(default)]
    pub json_output: bool,
    /// Don't invoke the LLM (-x)
    #[serde(default)]
    pub no_chibi: bool,
}

/// Context selection mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSelection {
    /// Use the current context (no switch)
    Current,
    /// Switch to a named context (-c)
    Switch {
        name: String,
        /// Whether to persist the switch to state.json
        #[serde(default = "default_true")]
        persistent: bool,
    },
    /// Use a context transiently (-C)
    Transient { name: String },
}

fn default_true() -> bool {
    true
}

impl Default for ContextSelection {
    fn default() -> Self {
        Self::Current
    }
}

/// Username override mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameOverride {
    /// Persistent username (-u): saves to local.toml
    Persistent(String),
    /// Transient username (-U): this invocation only
    Transient(String),
}

/// Unified input from CLI or JSON
/// This is the main type that represents a fully parsed user request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChibiInput {
    /// The command to execute
    pub command: Command,
    /// Behavioral flags
    #[serde(default)]
    pub flags: Flags,
    /// Context selection
    #[serde(default)]
    pub context: ContextSelection,
    /// Optional username override
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_override: Option<UsernameOverride>,
}

impl Default for ChibiInput {
    fn default() -> Self {
        Self {
            command: Command::NoOp,
            flags: Flags::default(),
            context: ContextSelection::Current,
            username_override: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_input() {
        let input = ChibiInput::default();
        assert!(matches!(input.command, Command::NoOp));
        assert!(!input.flags.verbose);
        assert!(!input.flags.json_output);
        assert!(matches!(input.context, ContextSelection::Current));
    }

    #[test]
    fn test_context_selection_default() {
        let ctx = ContextSelection::default();
        assert!(matches!(ctx, ContextSelection::Current));
    }
}
