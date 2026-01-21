//! Unified input types for CLI and JSON input modes.
//!
//! This module provides the core types that represent what operation to perform
//! and how to perform it, regardless of whether the input came from CLI flags
//! or JSON input.

use crate::cli::Inspectable;
use serde::Deserialize;

/// What operation to perform (mutually exclusive commands)
#[derive(Debug, Clone)]
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
    ShowLog { context: Option<String>, count: isize },
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
#[derive(Debug, Clone, Default)]
pub struct Flags {
    /// Show verbose output (-v)
    pub verbose: bool,
    /// Output in JSON format (--json-output)
    pub json_output: bool,
    /// Don't invoke the LLM (-x)
    pub no_chibi: bool,
    /// Force LLM invocation (-X)
    pub force_chibi: bool,
}

/// Context selection mode
#[derive(Debug, Clone)]
pub enum ContextSelection {
    /// Use the current context (no switch)
    Current,
    /// Switch to a named context (-c)
    Switch {
        name: String,
        /// Whether to persist the switch to state.json
        persistent: bool,
    },
    /// Use a context transiently (-C)
    Transient { name: String },
}

impl Default for ContextSelection {
    fn default() -> Self {
        Self::Current
    }
}

/// Username override mode
#[derive(Debug, Clone)]
pub enum UsernameOverride {
    /// Persistent username (-u): saves to local.toml
    Persistent(String),
    /// Transient username (-U): this invocation only
    Transient(String),
}

/// Partial config for runtime overrides (all optional)
/// These can come from CLI flags or JSON input and override
/// values from config.toml and local.toml
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PartialRuntimeConfig {
    /// API key override
    pub api_key: Option<String>,
    /// Model override
    pub model: Option<String>,
    /// Base URL override
    pub base_url: Option<String>,
    /// Context window limit override
    pub context_window_limit: Option<usize>,
    /// Warning threshold percentage override
    pub warn_threshold_percent: Option<f32>,
    /// Auto-compact enabled override
    pub auto_compact: Option<bool>,
    /// Auto-compact threshold override
    pub auto_compact_threshold: Option<f32>,
    /// Max recursion depth override
    pub max_recursion_depth: Option<usize>,
    /// Reflection enabled override
    pub reflection_enabled: Option<bool>,
}

/// Unified input from CLI or JSON
/// This is the main type that represents a fully parsed user request
#[derive(Debug, Clone)]
pub struct ChibiInput {
    /// Runtime config overrides
    pub config: PartialRuntimeConfig,
    /// The command to execute
    pub command: Command,
    /// Behavioral flags
    pub flags: Flags,
    /// Context selection
    pub context: ContextSelection,
    /// Optional username override
    pub username_override: Option<UsernameOverride>,
}

impl Default for ChibiInput {
    fn default() -> Self {
        Self {
            config: PartialRuntimeConfig::default(),
            command: Command::NoOp,
            flags: Flags::default(),
            context: ContextSelection::Current,
            username_override: None,
        }
    }
}

impl ChibiInput {
    /// Check if this input should invoke the LLM
    pub fn should_invoke_llm(&self) -> bool {
        if self.flags.force_chibi {
            return true;
        }
        if self.flags.no_chibi {
            return false;
        }
        // SendPrompt is the only command that invokes the LLM
        matches!(self.command, Command::SendPrompt { .. })
    }

    /// Check if this is a command that produces output and implies no_chibi
    pub fn implies_no_chibi(&self) -> bool {
        matches!(
            self.command,
            Command::ListContexts
                | Command::ListCurrentContext
                | Command::DeleteContext { .. }
                | Command::RenameContext { old: Some(_), .. }
                | Command::ShowLog { .. }
                | Command::Inspect { .. }
                | Command::SetSystemPrompt {
                    context: Some(_),
                    ..
                }
                | Command::RunPlugin { .. }
                | Command::CallTool { .. }
                | Command::ShowHelp
                | Command::ShowVersion
        )
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
    fn test_should_invoke_llm_prompt() {
        let input = ChibiInput {
            command: Command::SendPrompt {
                prompt: "hello".to_string(),
            },
            ..Default::default()
        };
        assert!(input.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_noop() {
        let input = ChibiInput::default();
        assert!(!input.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_no_chibi_flag() {
        let mut input = ChibiInput {
            command: Command::SendPrompt {
                prompt: "hello".to_string(),
            },
            ..Default::default()
        };
        input.flags.no_chibi = true;
        assert!(!input.should_invoke_llm());
    }

    #[test]
    fn test_should_invoke_llm_force_chibi() {
        let mut input = ChibiInput {
            command: Command::ListContexts,
            ..Default::default()
        };
        input.flags.force_chibi = true;
        assert!(input.should_invoke_llm());
    }

    #[test]
    fn test_implies_no_chibi() {
        assert!(ChibiInput {
            command: Command::ListContexts,
            ..Default::default()
        }
        .implies_no_chibi());

        assert!(ChibiInput {
            command: Command::ShowLog {
                context: None,
                count: 10
            },
            ..Default::default()
        }
        .implies_no_chibi());

        assert!(!ChibiInput {
            command: Command::SendPrompt {
                prompt: "hello".to_string()
            },
            ..Default::default()
        }
        .implies_no_chibi());

        assert!(!ChibiInput {
            command: Command::ArchiveContext { name: None },
            ..Default::default()
        }
        .implies_no_chibi());
    }

    #[test]
    fn test_context_selection_default() {
        let ctx = ContextSelection::default();
        assert!(matches!(ctx, ContextSelection::Current));
    }

    #[test]
    fn test_partial_runtime_config_default() {
        let cfg = PartialRuntimeConfig::default();
        assert!(cfg.api_key.is_none());
        assert!(cfg.model.is_none());
        assert!(cfg.base_url.is_none());
    }
}
