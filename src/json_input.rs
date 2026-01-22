//! JSON input parsing for `--json-config` mode.
//!
//! This module handles parsing JSON input from stdin and converting it
//! to the unified `ChibiInput` format.

use crate::cli::Inspectable;
use crate::input::{ChibiInput, Command, ContextSelection, Flags, UsernameOverride};
use serde::Deserialize;
use std::io::{self, ErrorKind};

/// JSON input structure for `--json-config` mode.
///
/// This struct is deserialized from JSON provided on stdin.
/// All fields are optional except when needed for specific operations.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct JsonInput {
    // === Prompt ===
    /// The prompt to send to the LLM (required when sending to LLM)
    pub prompt: Option<String>,

    // === Context operations ===
    /// Switch to a named context (persistent)
    pub switch_context: Option<String>,
    /// Use a context transiently for this invocation
    pub transient_context: Option<String>,
    /// List all contexts
    pub list_contexts: Option<bool>,
    /// List current context info
    pub list_current_context: Option<bool>,
    /// Delete context (None = current, Some(name) = named)
    pub delete_context: Option<String>,
    /// Delete current context
    pub delete_current_context: Option<bool>,
    /// Archive context (None = current, Some(name) = named)
    pub archive_context: Option<String>,
    /// Archive current context
    pub archive_current_context: Option<bool>,
    /// Compact context (None = current, Some(name) = named)
    pub compact_context: Option<String>,
    /// Compact current context
    pub compact_current_context: Option<bool>,
    /// Rename context (requires old and new names)
    pub rename_context: Option<RenameArgs>,
    /// Rename current context to a new name
    pub rename_current_context: Option<String>,
    /// Show log entries
    pub show_log: Option<ShowLogArgs>,
    /// Show log entries for current context
    pub show_current_log: Option<isize>,
    /// Inspect something
    pub inspect: Option<InspectArgs>,
    /// Inspect current context
    pub inspect_current: Option<String>,
    /// Set system prompt
    pub set_system_prompt: Option<SetSystemPromptArgs>,
    /// Set system prompt for current context
    pub set_current_system_prompt: Option<String>,

    // === Username ===
    /// Set username (persists to local.toml)
    pub set_username: Option<String>,
    /// Set username for this invocation only
    pub transient_username: Option<String>,

    // === Plugin/tool operations ===
    /// Run a plugin directly
    pub plugin: Option<PluginArgs>,
    /// Call a tool directly
    pub call_tool: Option<ToolArgs>,

    // === Control flags ===
    /// Show verbose output
    pub verbose: Option<bool>,
    /// Output in JSON format (can also be set via CLI `--json-output`)
    pub json_output: Option<bool>,
    /// Don't invoke the LLM
    pub no_chibi: Option<bool>,
    /// Force LLM invocation
    pub force_chibi: Option<bool>,

    // === Special ===
    /// Show help (returns help text as output)
    pub help: Option<bool>,
    /// Show version (returns version as output)
    pub version: Option<bool>,
}

/// Arguments for rename_context operation
#[derive(Debug, Clone, Deserialize)]
pub struct RenameArgs {
    pub old: String,
    pub new: String,
}

/// Arguments for show_log operation
#[derive(Debug, Clone, Deserialize)]
pub struct ShowLogArgs {
    /// Context name (optional, defaults to current)
    pub context: Option<String>,
    /// Number of entries to show (negative for first N)
    pub count: isize,
}

/// Arguments for inspect operation
#[derive(Debug, Clone, Deserialize)]
pub struct InspectArgs {
    /// Context name (optional, defaults to current)
    pub context: Option<String>,
    /// Thing to inspect (system_prompt, reflection, todos, goals, list)
    pub thing: String,
}

/// Arguments for set_system_prompt operation
#[derive(Debug, Clone, Deserialize)]
pub struct SetSystemPromptArgs {
    /// Context name (required for this variant)
    pub context: String,
    /// The system prompt content or file path
    pub prompt: String,
}

/// Arguments for plugin invocation
#[derive(Debug, Clone, Deserialize)]
pub struct PluginArgs {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Arguments for tool invocation
#[derive(Debug, Clone, Deserialize)]
pub struct ToolArgs {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl JsonInput {
    /// Parse JSON from a string
    pub fn from_str(s: &str) -> io::Result<Self> {
        serde_json::from_str(s)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Invalid JSON: {}", e)))
    }

    /// Check if this JSON requests help
    pub fn is_help(&self) -> bool {
        self.help.unwrap_or(false)
    }

    /// Check if this JSON requests version
    pub fn is_version(&self) -> bool {
        self.version.unwrap_or(false)
    }

    /// Convert to ChibiInput
    pub fn to_input(&self) -> io::Result<ChibiInput> {
        // Determine context selection
        let context = if let Some(ref name) = self.transient_context {
            ContextSelection::Transient { name: name.clone() }
        } else if let Some(ref name) = self.switch_context {
            ContextSelection::Switch {
                name: name.clone(),
                persistent: true,
            }
        } else {
            ContextSelection::Current
        };

        // Determine username override
        let username_override = if let Some(ref name) = self.transient_username {
            Some(UsernameOverride::Transient(name.clone()))
        } else if let Some(ref name) = self.set_username {
            Some(UsernameOverride::Persistent(name.clone()))
        } else {
            None
        };

        // Determine command - check operations in priority order
        let command = if self.is_help() {
            Command::ShowHelp
        } else if self.is_version() {
            Command::ShowVersion
        } else if self.list_contexts.unwrap_or(false) {
            Command::ListContexts
        } else if self.list_current_context.unwrap_or(false) {
            Command::ListCurrentContext
        } else if self.delete_current_context.unwrap_or(false) {
            Command::DeleteContext { name: None }
        } else if let Some(ref name) = self.delete_context {
            Command::DeleteContext {
                name: Some(name.clone()),
            }
        } else if self.archive_current_context.unwrap_or(false) {
            Command::ArchiveContext { name: None }
        } else if let Some(ref name) = self.archive_context {
            Command::ArchiveContext {
                name: Some(name.clone()),
            }
        } else if self.compact_current_context.unwrap_or(false) {
            Command::CompactContext { name: None }
        } else if let Some(ref name) = self.compact_context {
            Command::CompactContext {
                name: Some(name.clone()),
            }
        } else if let Some(ref new_name) = self.rename_current_context {
            Command::RenameContext {
                old: None,
                new: new_name.clone(),
            }
        } else if let Some(ref args) = self.rename_context {
            Command::RenameContext {
                old: Some(args.old.clone()),
                new: args.new.clone(),
            }
        } else if let Some(count) = self.show_current_log {
            Command::ShowLog {
                context: None,
                count,
            }
        } else if let Some(ref args) = self.show_log {
            Command::ShowLog {
                context: args.context.clone(),
                count: args.count,
            }
        } else if let Some(ref thing_str) = self.inspect_current {
            let thing = Inspectable::from_str(thing_str).ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "Unknown inspectable: {}. Valid options: {:?}",
                        thing_str,
                        Inspectable::all_names()
                    ),
                )
            })?;
            Command::Inspect {
                context: None,
                thing,
            }
        } else if let Some(ref args) = self.inspect {
            let thing = Inspectable::from_str(&args.thing).ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "Unknown inspectable: {}. Valid options: {:?}",
                        args.thing,
                        Inspectable::all_names()
                    ),
                )
            })?;
            Command::Inspect {
                context: args.context.clone(),
                thing,
            }
        } else if let Some(ref prompt) = self.set_current_system_prompt {
            Command::SetSystemPrompt {
                context: None,
                prompt: prompt.clone(),
            }
        } else if let Some(ref args) = self.set_system_prompt {
            Command::SetSystemPrompt {
                context: Some(args.context.clone()),
                prompt: args.prompt.clone(),
            }
        } else if let Some(ref args) = self.plugin {
            Command::RunPlugin {
                name: args.name.clone(),
                args: args.args.clone(),
            }
        } else if let Some(ref args) = self.call_tool {
            Command::CallTool {
                name: args.name.clone(),
                args: args.args.clone(),
            }
        } else if let Some(ref prompt) = self.prompt {
            Command::SendPrompt {
                prompt: prompt.clone(),
            }
        } else {
            Command::NoOp
        };

        // Determine flags
        let mut no_chibi = self.no_chibi.unwrap_or(false);
        let force_chibi = self.force_chibi.unwrap_or(false);

        // Apply implied no_chibi for output operations
        let implies_no_chibi = matches!(
            command,
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
        );

        if implies_no_chibi && !force_chibi {
            no_chibi = true;
        }

        // force_chibi wins over no_chibi
        let no_chibi = if force_chibi { false } else { no_chibi };

        let flags = Flags {
            verbose: self.verbose.unwrap_or(false),
            json_output: self.json_output.unwrap_or(false),
            no_chibi,
        };

        Ok(ChibiInput {
            command,
            flags,
            context,
            username_override,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_prompt() {
        let json = r#"{"prompt": "Hello, world!"}"#;
        let input = JsonInput::from_str(json).unwrap();
        assert_eq!(input.prompt, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_parse_with_verbose_flag() {
        let json = r#"{
            "prompt": "test",
            "verbose": true
        }"#;
        let input = JsonInput::from_str(json).unwrap();
        assert_eq!(input.verbose, Some(true));
    }

    #[test]
    fn test_parse_list_contexts() {
        let json = r#"{"list_contexts": true}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(chibi_input.command, Command::ListContexts));
        assert!(chibi_input.flags.no_chibi); // implied
    }

    #[test]
    fn test_parse_rename_context() {
        let json = r#"{"rename_context": {"old": "foo", "new": "bar"}}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.command,
            Command::RenameContext {
                old: Some(_),
                new: _
            }
        ));
    }

    #[test]
    fn test_parse_show_log() {
        let json = r#"{"show_log": {"context": "test", "count": 10}}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.command,
            Command::ShowLog { context: Some(_), count: 10 }
        ));
    }

    #[test]
    fn test_parse_inspect() {
        let json = r#"{"inspect": {"thing": "todos"}}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.command,
            Command::Inspect { context: None, thing: Inspectable::Todos }
        ));
    }

    #[test]
    fn test_parse_invalid_inspectable() {
        let json = r#"{"inspect_current": "invalid"}"#;
        let input = JsonInput::from_str(json).unwrap();
        let result = input.to_input();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown inspectable"));
    }

    #[test]
    fn test_parse_plugin() {
        let json = r#"{"plugin": {"name": "myplugin", "args": ["--help"]}}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(chibi_input.command, Command::RunPlugin { .. }));
    }

    #[test]
    fn test_parse_context_selection() {
        let json = r#"{"prompt": "test", "switch_context": "coding"}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.context,
            ContextSelection::Switch { name: _, persistent: true }
        ));
    }

    #[test]
    fn test_parse_transient_context() {
        let json = r#"{"prompt": "test", "transient_context": "temp"}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.context,
            ContextSelection::Transient { name: _ }
        ));
    }

    #[test]
    fn test_parse_username_override() {
        let json = r#"{"prompt": "test", "transient_username": "alice"}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(
            chibi_input.username_override,
            Some(UsernameOverride::Transient(_))
        ));
    }

    #[test]
    fn test_parse_force_chibi_overrides_implied() {
        let json = r#"{"list_contexts": true, "force_chibi": true}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        // force_chibi overrides the implied no_chibi from list_contexts
        assert!(!chibi_input.flags.no_chibi);
    }

    #[test]
    fn test_parse_help() {
        let json = r#"{"help": true}"#;
        let input = JsonInput::from_str(json).unwrap();
        assert!(input.is_help());
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(chibi_input.command, Command::ShowHelp));
    }

    #[test]
    fn test_parse_version() {
        let json = r#"{"version": true}"#;
        let input = JsonInput::from_str(json).unwrap();
        assert!(input.is_version());
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(chibi_input.command, Command::ShowVersion));
    }

    #[test]
    fn test_reject_unknown_fields() {
        let json = r#"{"unknown_field": "value"}"#;
        let result = JsonInput::from_str(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown field"));
    }

    #[test]
    fn test_prompt_command_parsing() {
        let json = r#"{"prompt": "test"}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();

        // The command should be parsed correctly
        assert!(matches!(chibi_input.command, Command::SendPrompt { ref prompt } if prompt == "test"));
    }

    #[test]
    fn test_empty_json() {
        let json = r#"{}"#;
        let input = JsonInput::from_str(json).unwrap();
        let chibi_input = input.to_input().unwrap();
        assert!(matches!(chibi_input.command, Command::NoOp));
    }
}
