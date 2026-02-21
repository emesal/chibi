//! Unified input types for CLI and JSON input modes.
//!
//! This module provides the core types that represent what operation to perform
//! and how to perform it, regardless of whether the input came from CLI flags
//! or JSON input.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Inspectable things via -n/-N
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Inspectable {
    // File-based items (context-specific)
    SystemPrompt,
    Reflection,
    Todos,
    Goals,
    // Global items
    Home,
    // Lists all inspectable items
    List,
    // Config field (dynamic path like "model", "api.temperature", etc.)
    // Note: untagged must be last for serde to work correctly
    #[serde(untagged)]
    ConfigField(String),
}

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
    /// Set model for a context (-m/--set-model, -M/--set-model-for-context)
    SetModel {
        context: Option<String>,
        model: String,
    },
    /// Run a plugin directly (-p)
    RunPlugin { name: String, args: Vec<String> },
    /// Call a tool directly (-P)
    CallTool { name: String, args: Vec<String> },
    /// Clear tool cache (--clear-cache/--clear-cache-for)
    ClearCache { name: Option<String> },
    /// Cleanup old cache entries (--cleanup-cache)
    CleanupCache,
    /// Check inbox for a specific context and process any messages (-B)
    CheckInbox { context: String },
    /// Check all context inboxes and process any messages (-b)
    CheckAllInboxes,
    /// Show model metadata from registry (-m/-M)
    ModelMetadata { model: String, full: bool },
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
    pub fn parse(s: &str) -> Option<Self> {
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

    /// Parse a comma-separated list of debug keys (e.g. "request-log,force-markdown,md=README.md").
    /// Invalid segments are silently ignored.
    pub fn parse_list(s: &str) -> Vec<Self> {
        s.split(',')
            .filter_map(|segment| Self::parse(segment.trim()))
            .collect()
    }
}

/// Execution flags — ephemeral command modifiers.
///
/// These are per-invocation imperative commands, not behavioural config.
/// Behavioural settings (verbose, hide_tool_calls, no_tool_calls, show_thinking)
/// live in `ResolvedConfig` and are set via `set_field` / `-s` / config files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionFlags {
    /// Force handoff to agent
    #[serde(default)]
    pub force_call_agent: bool,
    /// Force handoff to user immediately
    #[serde(default)]
    pub force_call_user: bool,
    /// Debug features to enable
    #[serde(default)]
    pub debug: Vec<DebugKey>,
}

// CLI-specific types (ContextSelection, UsernameOverride, ChibiInput) have been
// moved to chibi-cli/src/input.rs. chibi-core's API takes context names as
// parameters — it doesn't care *how* the context was selected.

#[cfg(test)]
mod tests {
    use super::*;

    // === ExecutionFlags tests ===

    #[test]
    fn test_flags_default() {
        let flags = ExecutionFlags::default();
        assert!(!flags.force_call_agent);
        assert!(!flags.force_call_user);
        assert!(flags.debug.is_empty());
    }

    #[test]
    fn test_flags_serialization() {
        let flags = ExecutionFlags {
            force_call_user: false,
            force_call_agent: true,
            debug: vec![DebugKey::RequestLog],
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("force_call_agent"));
        assert!(json.contains("request_log"));
    }

    #[test]
    fn test_flags_deserialization() {
        let json = r#"{"force_call_agent":true,"force_call_user":true}"#;
        let flags: ExecutionFlags = serde_json::from_str(json).unwrap();
        assert!(flags.force_call_agent);
        assert!(flags.force_call_user);
    }

    // === DebugKey tests ===

    #[test]
    fn test_debug_key_from_str_request_log() {
        assert_eq!(DebugKey::parse("request-log"), Some(DebugKey::RequestLog));
        assert_eq!(DebugKey::parse("request_log"), Some(DebugKey::RequestLog));
    }

    #[test]
    fn test_debug_key_from_str_response_meta() {
        assert_eq!(
            DebugKey::parse("response-meta"),
            Some(DebugKey::ResponseMeta)
        );
        assert_eq!(
            DebugKey::parse("response_meta"),
            Some(DebugKey::ResponseMeta)
        );
    }

    #[test]
    fn test_debug_key_from_str_all() {
        assert_eq!(DebugKey::parse("all"), Some(DebugKey::All));
    }

    #[test]
    fn test_debug_key_from_str_destroy_at() {
        assert_eq!(
            DebugKey::parse("destroy_at=1234567890"),
            Some(DebugKey::DestroyAt(1234567890))
        );
        assert_eq!(
            DebugKey::parse("destroy-at=1234567890"),
            Some(DebugKey::DestroyAt(1234567890))
        );
        // Invalid value
        assert_eq!(DebugKey::parse("destroy_at=invalid"), None);
    }

    #[test]
    fn test_debug_key_from_str_destroy_after_seconds_inactive() {
        assert_eq!(
            DebugKey::parse("destroy_after_seconds_inactive=60"),
            Some(DebugKey::DestroyAfterSecondsInactive(60))
        );
        assert_eq!(
            DebugKey::parse("destroy-after-seconds-inactive=3600"),
            Some(DebugKey::DestroyAfterSecondsInactive(3600))
        );
        // Invalid value
        assert_eq!(
            DebugKey::parse("destroy_after_seconds_inactive=invalid"),
            None
        );
    }

    #[test]
    fn test_debug_key_cli_only_keys_not_parsed_by_core() {
        // md= and force-markdown are CLI-only keys, not recognized by core
        assert_eq!(DebugKey::parse("md=README.md"), None);
        assert_eq!(DebugKey::parse("force-markdown"), None);
        assert_eq!(DebugKey::parse("force_markdown"), None);
    }

    #[test]
    fn test_debug_key_from_str_invalid() {
        assert_eq!(DebugKey::parse("invalid"), None);
        assert_eq!(DebugKey::parse(""), None);
        assert_eq!(DebugKey::parse("REQUEST_LOG"), None); // case sensitive
    }

    #[test]
    fn test_debug_key_parse_list_single() {
        assert_eq!(
            DebugKey::parse_list("request-log"),
            vec![DebugKey::RequestLog]
        );
    }

    #[test]
    fn test_debug_key_parse_list_multiple() {
        assert_eq!(
            DebugKey::parse_list("request-log,response-meta"),
            vec![DebugKey::RequestLog, DebugKey::ResponseMeta]
        );
    }

    #[test]
    fn test_debug_key_parse_list_with_parameterized() {
        assert_eq!(
            DebugKey::parse_list("request-log,destroy_at=1234567890"),
            vec![DebugKey::RequestLog, DebugKey::DestroyAt(1234567890)]
        );
    }

    #[test]
    fn test_debug_key_parse_list_ignores_invalid() {
        assert_eq!(
            DebugKey::parse_list("request-log,invalid,response-meta"),
            vec![DebugKey::RequestLog, DebugKey::ResponseMeta]
        );
    }

    #[test]
    fn test_debug_key_parse_list_empty() {
        assert_eq!(DebugKey::parse_list(""), Vec::<DebugKey>::new());
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

    // === ExecutionFlags tests ===

    #[test]
    fn test_execution_flags_default() {
        let flags = ExecutionFlags::default();
        assert!(!flags.force_call_agent);
        assert!(!flags.force_call_user);
        assert!(flags.debug.is_empty());
    }

    #[test]
    fn test_execution_flags_serialization() {
        let flags = ExecutionFlags {
            force_call_agent: true,
            force_call_user: false,
            debug: vec![DebugKey::RequestLog],
        };
        let json = serde_json::to_string(&flags).unwrap();
        let deser: ExecutionFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.force_call_agent, flags.force_call_agent);
        assert_eq!(deser.force_call_user, flags.force_call_user);
        assert_eq!(deser.debug.len(), 1);
    }
}
