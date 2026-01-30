//! JSON input parsing for `--json-config` mode.
//!
//! This module handles parsing JSON input from stdin and converting it
//! to the unified `ChibiInput` format.
//!
//! ## JSON Format
//!
//! The JSON format mirrors the `ChibiInput` structure directly:
//!
//! ```json
//! {
//!   "command": "list_contexts"
//! }
//! ```
//!
//! ```json
//! {
//!   "command": { "send_prompt": { "prompt": "hello world" } },
//!   "context": { "switch": { "name": "coding" } },
//!   "flags": { "verbose": true }
//! }
//! ```
//!
//! ### Commands
//!
//! Simple commands (no arguments):
//! - `"list_contexts"`
//! - `"list_current_context"`
//! - `"show_help"`
//! - `"show_version"`
//! - `"no_op"`
//!
//! Commands with arguments:
//! - `{ "send_prompt": { "prompt": "..." } }`
//! - `{ "destroy_context": { "name": "..." } }` (name is optional, null = current)
//! - `{ "archive_history": { "name": "..." } }` (name is optional)
//! - `{ "compact_context": { "name": "..." } }` (name is optional)
//! - `{ "rename_context": { "old": "...", "new": "..." } }` (old is optional)
//! - `{ "show_log": { "context": "...", "count": 10 } }` (context is optional)
//! - `{ "inspect": { "context": "...", "thing": "todos" } }` (context is optional)
//! - `{ "set_system_prompt": { "context": "...", "prompt": "..." } }` (context is optional)
//! - `{ "run_plugin": { "name": "...", "args": [...] } }`
//! - `{ "call_tool": { "name": "...", "args": [...] } }`
//!
//! ### Context Selection
//!
//! - `"current"` (default)
//! - `{ "switch": { "name": "..." } }` (persistent is true by default)
//! - `{ "transient": { "name": "..." } }`
//!
//! ### Username Override
//!
//! - `{ "persistent": "username" }`
//! - `{ "transient": "username" }`

use chibi_core::input::ChibiInput;
use std::io::{self, ErrorKind};

/// Parse JSON input string to ChibiInput
pub fn from_str(s: &str) -> io::Result<ChibiInput> {
    serde_json::from_str(s)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Invalid JSON: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chibi_core::input::{Command, ContextSelection, DebugKey, Inspectable, UsernameOverride};

    #[test]
    fn test_parse_simple_prompt() {
        let json = r#"{"command": {"send_prompt": {"prompt": "Hello, world!"}}}"#;
        let input = from_str(json).unwrap();
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "Hello, world!")
        );
    }

    #[test]
    fn test_parse_with_verbose_flag() {
        let json = r#"{
            "command": {"send_prompt": {"prompt": "test"}},
            "flags": {"verbose": true}
        }"#;
        let input = from_str(json).unwrap();
        assert!(input.flags.verbose);
    }

    #[test]
    fn test_parse_list_contexts() {
        let json = r#"{"command": "list_contexts"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::ListContexts));
    }

    #[test]
    fn test_parse_list_current_context() {
        let json = r#"{"command": "list_current_context"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::ListCurrentContext));
    }

    #[test]
    fn test_parse_destroy_context_named() {
        let json = r#"{"command": {"destroy_context": {"name": "foo"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::DestroyContext { name: Some(ref n) } if n == "foo"
        ));
    }

    #[test]
    fn test_parse_destroy_context_current() {
        let json = r#"{"command": {"destroy_context": {"name": null}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::DestroyContext { name: None }
        ));
    }

    #[test]
    fn test_parse_rename_context() {
        let json = r#"{"command": {"rename_context": {"old": "foo", "new": "bar"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::RenameContext { old: Some(ref o), ref new } if o == "foo" && new == "bar"
        ));
    }

    #[test]
    fn test_parse_rename_current_context() {
        let json = r#"{"command": {"rename_context": {"old": null, "new": "bar"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::RenameContext { old: None, ref new } if new == "bar"
        ));
    }

    #[test]
    fn test_parse_show_log() {
        let json = r#"{"command": {"show_log": {"context": "test", "count": 10}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::ShowLog { context: Some(ref c), count: 10 } if c == "test"
        ));
    }

    #[test]
    fn test_parse_show_log_current() {
        let json = r#"{"command": {"show_log": {"count": 5}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::ShowLog {
                context: None,
                count: 5
            }
        ));
    }

    #[test]
    fn test_parse_inspect() {
        let json = r#"{"command": {"inspect": {"thing": "todos"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect {
                context: None,
                thing: Inspectable::Todos
            }
        ));
    }

    #[test]
    fn test_parse_inspect_with_context() {
        let json = r#"{"command": {"inspect": {"context": "other", "thing": "goals"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect { context: Some(ref c), thing: Inspectable::Goals } if c == "other"
        ));
    }

    #[test]
    fn test_parse_set_system_prompt() {
        let json = r#"{"command": {"set_system_prompt": {"prompt": "Be helpful"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::SetSystemPrompt { context: None, ref prompt } if prompt == "Be helpful"
        ));
    }

    #[test]
    fn test_parse_plugin() {
        let json = r#"{"command": {"run_plugin": {"name": "myplugin", "args": ["--help"]}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::RunPlugin { ref name, ref args } if name == "myplugin" && args == &["--help"]
        ));
    }

    #[test]
    fn test_parse_call_tool() {
        let json = r#"{"command": {"call_tool": {"name": "update_todos", "args": []}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::CallTool { ref name, .. } if name == "update_todos"
        ));
    }

    #[test]
    fn test_parse_context_switch() {
        let json = r#"{"command": "no_op", "context": {"switch": {"name": "coding"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.context,
            ContextSelection::Switch { ref name, persistent: true } if name == "coding"
        ));
    }

    #[test]
    fn test_parse_transient_context() {
        let json = r#"{"command": "no_op", "context": {"transient": {"name": "temp"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.context,
            ContextSelection::Transient { ref name } if name == "temp"
        ));
    }

    #[test]
    fn test_parse_username_persistent() {
        let json = r#"{"command": "no_op", "username_override": {"persistent": "alice"}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.username_override,
            Some(UsernameOverride::Persistent(ref u)) if u == "alice"
        ));
    }

    #[test]
    fn test_parse_username_transient() {
        let json = r#"{"command": "no_op", "username_override": {"transient": "bob"}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.username_override,
            Some(UsernameOverride::Transient(ref u)) if u == "bob"
        ));
    }

    #[test]
    fn test_parse_no_op() {
        let json = r#"{"command": "no_op"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::NoOp));
    }

    #[test]
    fn test_parse_help() {
        let json = r#"{"command": "show_help"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::ShowHelp));
    }

    #[test]
    fn test_parse_version() {
        let json = r#"{"command": "show_version"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::ShowVersion));
    }

    #[test]
    fn test_default_flags() {
        let json = r#"{"command": "list_contexts"}"#;
        let input = from_str(json).unwrap();
        assert!(!input.flags.verbose);
        assert!(!input.flags.json_output);
        assert!(!input.flags.no_chibi);
    }

    #[test]
    fn test_default_context() {
        let json = r#"{"command": "list_contexts"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.context, ContextSelection::Current));
    }

    #[test]
    fn test_archive_history() {
        let json = r#"{"command": {"archive_history": {"name": "old"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::ArchiveHistory { name: Some(ref n) } if n == "old"
        ));
    }

    #[test]
    fn test_compact_context() {
        let json = r#"{"command": {"compact_context": {}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::CompactContext { name: None }
        ));
    }

    #[test]
    fn test_all_flags() {
        let json = r#"{
            "command": "list_contexts",
            "flags": {"verbose": true, "json_output": true, "no_chibi": true}
        }"#;
        let input = from_str(json).unwrap();
        assert!(input.flags.verbose);
        assert!(input.flags.json_output);
        assert!(input.flags.no_chibi);
    }

    #[test]
    fn test_invalid_json() {
        let json = r#"{"command": invalid}"#;
        let result = from_str(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid JSON"));
    }

    #[test]
    fn test_complete_example() {
        let json = r#"{
            "command": {"send_prompt": {"prompt": "hello world"}},
            "flags": {"verbose": true},
            "context": {"switch": {"name": "coding"}},
            "username_override": {"transient": "alice"}
        }"#;
        let input = from_str(json).unwrap();
        assert!(
            matches!(input.command, Command::SendPrompt { ref prompt } if prompt == "hello world")
        );
        assert!(input.flags.verbose);
        assert!(
            matches!(input.context, ContextSelection::Switch { ref name, .. } if name == "coding")
        );
        assert!(
            matches!(input.username_override, Some(UsernameOverride::Transient(ref u)) if u == "alice")
        );
    }

    // === Missing command tests ===

    #[test]
    fn test_parse_clear_cache_current() {
        let json = r#"{"command": {"clear_cache": {}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::ClearCache { name: None }));
    }

    #[test]
    fn test_parse_clear_cache_named() {
        let json = r#"{"command": {"clear_cache": {"name": "myctx"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::ClearCache { name: Some(ref n) } if n == "myctx"
        ));
    }

    #[test]
    fn test_parse_cleanup_cache() {
        let json = r#"{"command": "cleanup_cache"}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.command, Command::CleanupCache));
    }

    // === Inspectable variant tests ===

    #[test]
    fn test_parse_inspect_system_prompt() {
        let json = r#"{"command": {"inspect": {"thing": "system_prompt"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect {
                thing: Inspectable::SystemPrompt,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_inspect_reflection() {
        let json = r#"{"command": {"inspect": {"thing": "reflection"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect {
                thing: Inspectable::Reflection,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_inspect_home() {
        let json = r#"{"command": {"inspect": {"thing": "home"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect {
                thing: Inspectable::Home,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_inspect_list() {
        let json = r#"{"command": {"inspect": {"thing": "list"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect {
                thing: Inspectable::List,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_inspect_config_field() {
        let json = r#"{"command": {"inspect": {"thing": "model"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect { thing: Inspectable::ConfigField(ref f), .. } if f == "model"
        ));
    }

    #[test]
    fn test_parse_inspect_config_field_nested() {
        let json = r#"{"command": {"inspect": {"thing": "api.temperature"}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.command,
            Command::Inspect { thing: Inspectable::ConfigField(ref f), .. } if f == "api.temperature"
        ));
    }

    // === DebugKey tests ===

    #[test]
    fn test_parse_debug_request_log() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": ["request_log"]}
        }"#;
        let input = from_str(json).unwrap();
        assert_eq!(input.flags.debug.len(), 1);
        assert!(matches!(input.flags.debug[0], DebugKey::RequestLog));
    }

    #[test]
    fn test_parse_debug_response_meta() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": ["response_meta"]}
        }"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.flags.debug[0], DebugKey::ResponseMeta));
    }

    #[test]
    fn test_parse_debug_all() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": ["all"]}
        }"#;
        let input = from_str(json).unwrap();
        assert!(matches!(input.flags.debug[0], DebugKey::All));
    }

    #[test]
    fn test_parse_debug_destroy_at() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": [{"destroy_at": 1234567890}]}
        }"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.flags.debug[0],
            DebugKey::DestroyAt(1234567890)
        ));
    }

    #[test]
    fn test_parse_debug_destroy_after_seconds_inactive() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": [{"destroy_after_seconds_inactive": 60}]}
        }"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.flags.debug[0],
            DebugKey::DestroyAfterSecondsInactive(60)
        ));
    }

    #[test]
    fn test_parse_debug_multiple() {
        let json = r#"{
            "command": "no_op",
            "flags": {"debug": ["request_log", "response_meta"]}
        }"#;
        let input = from_str(json).unwrap();
        assert_eq!(input.flags.debug.len(), 2);
    }

    // === Error handling tests ===

    #[test]
    fn test_invalid_command_name() {
        let json = r#"{"command": "not_a_real_command"}"#;
        let result = from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_prompt() {
        let json = r#"{"command": {"send_prompt": {}}}"#;
        let result = from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_new_in_rename() {
        let json = r#"{"command": {"rename_context": {"old": "foo"}}}"#;
        let result = from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_fields_are_ignored() {
        // serde default behavior with deny_unknown_fields not set
        let json = r#"{"command": "no_op", "extra_field": "ignored"}"#;
        let result = from_str(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_raw_flag() {
        let json = r#"{
            "command": "no_op",
            "flags": {"raw": true}
        }"#;
        let input = from_str(json).unwrap();
        assert!(input.flags.raw);
    }

    #[test]
    fn test_context_switch_non_persistent() {
        let json =
            r#"{"command": "no_op", "context": {"switch": {"name": "test", "persistent": false}}}"#;
        let input = from_str(json).unwrap();
        assert!(matches!(
            input.context,
            ContextSelection::Switch { ref name, persistent: false } if name == "test"
        ));
    }
}
