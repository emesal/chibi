use std::collections::BTreeMap;
use std::path::PathBuf;

use chibi_core::config::LocalConfig;
use chibi_core::input::{Command, ExecutionFlags};
use schemars::JsonSchema;
use serde::Deserialize;

/// JSON-mode input -- read from stdin, stateless per invocation.
///
/// Unlike ChibiInput (CLI), context is always explicit (no "current" concept),
/// there's no session, and no context selection enum.
///
/// Config override priority (highest → lowest):
/// `overrides` > `config` > `local.toml` > env > `config.toml` > defaults
#[derive(Debug, Deserialize, JsonSchema)]
pub struct JsonInput {
    /// The command to execute
    pub command: Command,
    /// Context name -- required, no "current" concept
    pub context: String,
    /// Execution flags
    #[serde(default)]
    pub flags: ExecutionFlags,
    /// Chibi home directory override
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Project root override
    #[serde(default)]
    pub project_root: Option<PathBuf>,
    /// Typed config overrides (same semantics as local.toml, schema-documented).
    /// Use `config.username` and `config.url_policy` for per-invocation overrides.
    #[serde(default)]
    pub config: Option<LocalConfig>,
    /// String-keyed overrides (highest priority, freeform escape hatch)
    #[serde(default)]
    pub overrides: Option<BTreeMap<String, String>>,
}
