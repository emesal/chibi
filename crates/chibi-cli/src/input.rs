//! CLI-specific input types for context and username selection.
//!
//! These types handle how the CLI interprets context and username arguments.
//! chibi-core's API takes context names as parameters — it doesn't care *how*
//! the context was selected. These are CLI concerns.
//!
//! ## Context Selection
//!
//! - `Current` — use the implied context from session.json
//! - `Switch` — switch to a named context (persistent or non-persistent)
//! - `Ephemeral` — use a context for this invocation only, without updating session
//!
//! ## Username Override
//!
//! - `Persistent` — save username to local.toml
//! - `Ephemeral` — use username for this invocation only

use chibi_core::input::{Command, Flags};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Context selection mode.
///
/// Determines how the CLI selects which context to operate on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextSelection {
    /// Use the implied context from session.json (no switch)
    #[default]
    Current,
    /// Switch to a named context (-c)
    Switch {
        name: String,
        /// Whether to persist the switch to session.json
        #[serde(default = "default_true")]
        persistent: bool,
    },
    /// Use a context ephemerally (-C) — does NOT update session.json
    Ephemeral { name: String },
}

fn default_true() -> bool {
    true
}

/// Username override mode.
///
/// Determines how the CLI handles username overrides.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsernameOverride {
    /// Persistent username (-u): saves to local.toml
    Persistent(String),
    /// Ephemeral username (-U): this invocation only
    Ephemeral(String),
}

/// Unified input from CLI or JSON.
///
/// This is the main type that represents a fully parsed user request.
/// It combines a command from chibi-core with CLI-specific selection modes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    use chibi_core::input::DebugKey;

    // === ChibiInput tests ===

    #[test]
    fn test_default_input() {
        let input = ChibiInput::default();
        assert!(matches!(input.command, Command::NoOp));
        assert!(!input.flags.verbose);
        assert!(!input.flags.json_output);
        assert!(matches!(input.context, ContextSelection::Current));
    }

    #[test]
    fn test_default_input_no_username_override() {
        let input = ChibiInput::default();
        assert!(input.username_override.is_none());
    }

    #[test]
    fn test_default_input_no_debug() {
        let input = ChibiInput::default();
        assert!(input.flags.debug.is_empty());
    }

    // === ContextSelection tests ===

    #[test]
    fn test_context_selection_default() {
        let ctx = ContextSelection::default();
        assert!(matches!(ctx, ContextSelection::Current));
    }

    #[test]
    fn test_context_selection_switch_serialization() {
        let ctx = ContextSelection::Switch {
            name: "test".to_string(),
            persistent: true,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("switch"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_context_selection_ephemeral_serialization() {
        let ctx = ContextSelection::Ephemeral {
            name: "temp".to_string(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("ephemeral"));
        assert!(json.contains("temp"));
    }

    #[test]
    fn test_context_selection_current_serialization() {
        let ctx = ContextSelection::Current;
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("current"));
    }

    #[test]
    fn test_context_selection_deserialization() {
        let json = r#"{"switch":{"name":"coding","persistent":true}}"#;
        let ctx: ContextSelection = serde_json::from_str(json).unwrap();
        assert!(
            matches!(ctx, ContextSelection::Switch { ref name, persistent: true } if name == "coding")
        );
    }

    // === UsernameOverride tests ===

    #[test]
    fn test_username_override_persistent_serialization() {
        let override_ = UsernameOverride::Persistent("alice".to_string());
        let json = serde_json::to_string(&override_).unwrap();
        assert!(json.contains("persistent"));
        assert!(json.contains("alice"));
    }

    #[test]
    fn test_username_override_ephemeral_serialization() {
        let override_ = UsernameOverride::Ephemeral("bob".to_string());
        let json = serde_json::to_string(&override_).unwrap();
        assert!(json.contains("ephemeral"));
        assert!(json.contains("bob"));
    }

    #[test]
    fn test_username_override_deserialization() {
        let json = r#"{"persistent":"alice"}"#;
        let override_: UsernameOverride = serde_json::from_str(json).unwrap();
        assert!(matches!(override_, UsernameOverride::Persistent(ref u) if u == "alice"));

        let json = r#"{"ephemeral":"bob"}"#;
        let override_: UsernameOverride = serde_json::from_str(json).unwrap();
        assert!(matches!(override_, UsernameOverride::Ephemeral(ref u) if u == "bob"));
    }

    // === Full ChibiInput round-trip tests ===

    #[test]
    fn test_chibi_input_full_round_trip() {
        use chibi_core::Inspectable;

        let input = ChibiInput {
            command: Command::Inspect {
                context: Some("test".to_string()),
                thing: Inspectable::SystemPrompt,
            },
            flags: Flags {
                verbose: true,
                json_output: true,
                force_return: true,
                force_recurse: false,
                raw: false,
                debug: vec![DebugKey::All],
            },
            context: ContextSelection::Switch {
                name: "coding".to_string(),
                persistent: false,
            },
            username_override: Some(UsernameOverride::Ephemeral("alice".to_string())),
        };

        let json = serde_json::to_string(&input).unwrap();
        let deserialized: ChibiInput = serde_json::from_str(&json).unwrap();

        assert!(
            matches!(deserialized.command, Command::Inspect { context: Some(ref c), thing: Inspectable::SystemPrompt } if c == "test")
        );
        assert!(deserialized.flags.verbose);
        assert!(deserialized.flags.json_output);
        assert!(deserialized.flags.force_return);
        assert_eq!(deserialized.flags.debug, vec![DebugKey::All]);
        assert!(
            matches!(deserialized.context, ContextSelection::Switch { ref name, persistent: false } if name == "coding")
        );
        assert!(
            matches!(deserialized.username_override, Some(UsernameOverride::Ephemeral(ref u)) if u == "alice")
        );
    }

    #[test]
    fn test_chibi_input_minimal_round_trip() {
        let input = ChibiInput {
            command: Command::ListContexts,
            flags: Flags::default(),
            context: ContextSelection::Current,
            username_override: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        let deserialized: ChibiInput = serde_json::from_str(&json).unwrap();

        assert!(matches!(deserialized.command, Command::ListContexts));
        assert!(matches!(deserialized.context, ContextSelection::Current));
        assert!(deserialized.username_override.is_none());
    }
}
