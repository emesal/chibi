//! Unified input types for CLI and JSON input modes.
//!
//! This module provides the core types that represent what operation to perform
//! and how to perform it, regardless of whether the input came from CLI flags
//! or JSON input.

use crate::cli::Inspectable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What operation to perform (mutually exclusive commands)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Send a prompt to the LLM
    SendPrompt { prompt: String },
    /// List all contexts (-L)
    ListContexts,
    /// Show current context info (-l)
    ListCurrentContext,
    /// Destroy a context (-d/-D)
    DestroyContext { name: Option<String> },
    /// Archive a context's history (-a/-A)
    ArchiveHistory { name: Option<String> },
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
    /// Clear tool cache (--clear-cache/--clear-cache-for)
    ClearCache { name: Option<String> },
    /// Cleanup old cache entries (--cleanup-cache)
    CleanupCache,
    /// Show help
    ShowHelp,
    /// Show version
    ShowVersion,
    /// No operation - context switch only, no action
    NoOp,
}

/// Debug feature keys
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugKey {
    /// Log all API requests to requests.jsonl
    RequestLog,
    /// Log response metadata (usage stats, model info) to response_meta.jsonl
    ResponseMeta,
    /// Set destroy_at timestamp on the current context (e.g., "destroy_at=1234567890")
    DestroyAt(u64),
    /// Set destroy_after_seconds_inactive on the current context (e.g., "destroy_after_seconds_inactive=60")
    DestroyAfterSecondsInactive(u64),
    /// Enable all debug features (request_log, response_meta)
    All,
}

impl DebugKey {
    pub fn from_str(s: &str) -> Option<Self> {
        // Check for parameterized debug keys first
        if let Some(value) = s
            .strip_prefix("destroy_at=")
            .or_else(|| s.strip_prefix("destroy-at="))
        {
            return value.parse::<u64>().ok().map(DebugKey::DestroyAt);
        }
        if let Some(value) = s
            .strip_prefix("destroy_after_seconds_inactive=")
            .or_else(|| s.strip_prefix("destroy-after-seconds-inactive="))
        {
            return value
                .parse::<u64>()
                .ok()
                .map(DebugKey::DestroyAfterSecondsInactive);
        }

        match s {
            "request-log" | "request_log" => Some(DebugKey::RequestLog),
            "response-meta" | "response_meta" => Some(DebugKey::ResponseMeta),
            "all" => Some(DebugKey::All),
            _ => None,
        }
    }
}

/// Behavioral modifiers (flags that affect how commands run)
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    /// Debug feature to enable
    #[serde(default)]
    pub debug: Option<DebugKey>,
}

/// Context selection mode
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextSelection {
    /// Use the current context (no switch)
    #[default]
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

/// Username override mode
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsernameOverride {
    /// Persistent username (-u): saves to local.toml
    Persistent(String),
    /// Transient username (-U): this invocation only
    Transient(String),
}

/// Unified input from CLI or JSON
/// This is the main type that represents a fully parsed user request
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
        assert!(input.flags.debug.is_none());
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
    fn test_context_selection_transient_serialization() {
        let ctx = ContextSelection::Transient {
            name: "temp".to_string(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("transient"));
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

    // === Flags tests ===

    #[test]
    fn test_flags_default() {
        let flags = Flags::default();
        assert!(!flags.verbose);
        assert!(!flags.json_output);
        assert!(!flags.no_chibi);
        assert!(flags.debug.is_none());
    }

    #[test]
    fn test_flags_serialization() {
        let flags = Flags {
            verbose: true,
            json_output: true,
            no_chibi: false,
            debug: Some(DebugKey::RequestLog),
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("verbose"));
        assert!(json.contains("json_output"));
        assert!(json.contains("request_log"));
    }

    #[test]
    fn test_flags_deserialization() {
        let json = r#"{"verbose":true,"json_output":false,"no_chibi":true}"#;
        let flags: Flags = serde_json::from_str(json).unwrap();
        assert!(flags.verbose);
        assert!(!flags.json_output);
        assert!(flags.no_chibi);
    }

    // === DebugKey tests ===

    #[test]
    fn test_debug_key_from_str_request_log() {
        assert_eq!(
            DebugKey::from_str("request-log"),
            Some(DebugKey::RequestLog)
        );
        assert_eq!(
            DebugKey::from_str("request_log"),
            Some(DebugKey::RequestLog)
        );
    }

    #[test]
    fn test_debug_key_from_str_response_meta() {
        assert_eq!(
            DebugKey::from_str("response-meta"),
            Some(DebugKey::ResponseMeta)
        );
        assert_eq!(
            DebugKey::from_str("response_meta"),
            Some(DebugKey::ResponseMeta)
        );
    }

    #[test]
    fn test_debug_key_from_str_all() {
        assert_eq!(DebugKey::from_str("all"), Some(DebugKey::All));
    }

    #[test]
    fn test_debug_key_from_str_destroy_at() {
        assert_eq!(
            DebugKey::from_str("destroy_at=1234567890"),
            Some(DebugKey::DestroyAt(1234567890))
        );
        assert_eq!(
            DebugKey::from_str("destroy-at=1234567890"),
            Some(DebugKey::DestroyAt(1234567890))
        );
        // Invalid value
        assert_eq!(DebugKey::from_str("destroy_at=invalid"), None);
    }

    #[test]
    fn test_debug_key_from_str_destroy_after_seconds_inactive() {
        assert_eq!(
            DebugKey::from_str("destroy_after_seconds_inactive=60"),
            Some(DebugKey::DestroyAfterSecondsInactive(60))
        );
        assert_eq!(
            DebugKey::from_str("destroy-after-seconds-inactive=3600"),
            Some(DebugKey::DestroyAfterSecondsInactive(3600))
        );
        // Invalid value
        assert_eq!(
            DebugKey::from_str("destroy_after_seconds_inactive=invalid"),
            None
        );
    }

    #[test]
    fn test_debug_key_from_str_invalid() {
        assert_eq!(DebugKey::from_str("invalid"), None);
        assert_eq!(DebugKey::from_str(""), None);
        assert_eq!(DebugKey::from_str("REQUEST_LOG"), None); // case sensitive
    }

    #[test]
    fn test_debug_key_serialization() {
        let key = DebugKey::RequestLog;
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#""request_log""#);

        let key = DebugKey::ResponseMeta;
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#""response_meta""#);

        let key = DebugKey::All;
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#""all""#);

        // Parameterized variants serialize with their values
        let key = DebugKey::DestroyAt(1234567890);
        let json = serde_json::to_string(&key).unwrap();
        assert!(json.contains("destroy_at"));

        let key = DebugKey::DestroyAfterSecondsInactive(60);
        let json = serde_json::to_string(&key).unwrap();
        assert!(json.contains("destroy_after_seconds_inactive"));
    }

    #[test]
    fn test_debug_key_deserialization() {
        let key: DebugKey = serde_json::from_str(r#""request_log""#).unwrap();
        assert_eq!(key, DebugKey::RequestLog);

        let key: DebugKey = serde_json::from_str(r#""response_meta""#).unwrap();
        assert_eq!(key, DebugKey::ResponseMeta);

        let key: DebugKey = serde_json::from_str(r#""all""#).unwrap();
        assert_eq!(key, DebugKey::All);
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
    fn test_username_override_transient_serialization() {
        let override_ = UsernameOverride::Transient("bob".to_string());
        let json = serde_json::to_string(&override_).unwrap();
        assert!(json.contains("transient"));
        assert!(json.contains("bob"));
    }

    #[test]
    fn test_username_override_deserialization() {
        let json = r#"{"persistent":"alice"}"#;
        let override_: UsernameOverride = serde_json::from_str(json).unwrap();
        assert!(matches!(override_, UsernameOverride::Persistent(ref u) if u == "alice"));

        let json = r#"{"transient":"bob"}"#;
        let override_: UsernameOverride = serde_json::from_str(json).unwrap();
        assert!(matches!(override_, UsernameOverride::Transient(ref u) if u == "bob"));
    }

    // === Command tests ===

    #[test]
    fn test_command_send_prompt_serialization() {
        let cmd = Command::SendPrompt {
            prompt: "hello".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("send_prompt"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn test_command_list_contexts_serialization() {
        let cmd = Command::ListContexts;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#""list_contexts""#);
    }

    #[test]
    fn test_command_destroy_context_with_name() {
        let cmd = Command::DestroyContext {
            name: Some("test".to_string()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("destroy_context"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_command_destroy_context_current() {
        let cmd = Command::DestroyContext { name: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("destroy_context"));
        assert!(json.contains("null"));
    }

    #[test]
    fn test_command_rename_context() {
        let cmd = Command::RenameContext {
            old: Some("old".to_string()),
            new: "new".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("rename_context"));
        assert!(json.contains("old"));
        assert!(json.contains("new"));
    }

    #[test]
    fn test_command_show_log() {
        let cmd = Command::ShowLog {
            context: Some("test".to_string()),
            count: 10,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("show_log"));
        assert!(json.contains("test"));
        assert!(json.contains("10"));
    }

    #[test]
    fn test_command_inspect() {
        use crate::cli::Inspectable;
        let cmd = Command::Inspect {
            context: None,
            thing: Inspectable::Todos,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("inspect"));
        assert!(json.contains("todos"));
    }

    #[test]
    fn test_command_set_system_prompt() {
        let cmd = Command::SetSystemPrompt {
            context: Some("ctx".to_string()),
            prompt: "Be helpful".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("set_system_prompt"));
        assert!(json.contains("ctx"));
        assert!(json.contains("Be helpful"));
    }

    #[test]
    fn test_command_run_plugin() {
        let cmd = Command::RunPlugin {
            name: "myplugin".to_string(),
            args: vec!["--help".to_string()],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("run_plugin"));
        assert!(json.contains("myplugin"));
        assert!(json.contains("--help"));
    }

    #[test]
    fn test_command_call_tool() {
        let cmd = Command::CallTool {
            name: "update_todos".to_string(),
            args: vec![],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("call_tool"));
        assert!(json.contains("update_todos"));
    }

    #[test]
    fn test_command_show_help() {
        let cmd = Command::ShowHelp;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#""show_help""#);
    }

    #[test]
    fn test_command_show_version() {
        let cmd = Command::ShowVersion;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#""show_version""#);
    }

    #[test]
    fn test_command_no_op() {
        let cmd = Command::NoOp;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#""no_op""#);
    }

    // === Full ChibiInput round-trip tests ===

    #[test]
    fn test_chibi_input_full_round_trip() {
        use crate::cli::Inspectable;

        let input = ChibiInput {
            command: Command::Inspect {
                context: Some("test".to_string()),
                thing: Inspectable::SystemPrompt,
            },
            flags: Flags {
                verbose: true,
                json_output: true,
                no_chibi: true,
                debug: Some(DebugKey::All),
            },
            context: ContextSelection::Switch {
                name: "coding".to_string(),
                persistent: false,
            },
            username_override: Some(UsernameOverride::Transient("alice".to_string())),
        };

        let json = serde_json::to_string(&input).unwrap();
        let deserialized: ChibiInput = serde_json::from_str(&json).unwrap();

        assert!(
            matches!(deserialized.command, Command::Inspect { context: Some(ref c), thing: Inspectable::SystemPrompt } if c == "test")
        );
        assert!(deserialized.flags.verbose);
        assert!(deserialized.flags.json_output);
        assert!(deserialized.flags.no_chibi);
        assert_eq!(deserialized.flags.debug, Some(DebugKey::All));
        assert!(
            matches!(deserialized.context, ContextSelection::Switch { ref name, persistent: false } if name == "coding")
        );
        assert!(
            matches!(deserialized.username_override, Some(UsernameOverride::Transient(ref u)) if u == "alice")
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
