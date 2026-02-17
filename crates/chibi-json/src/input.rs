use std::collections::BTreeMap;
use std::path::PathBuf;

use chibi_core::config::LocalConfig;
use chibi_core::input::{Command, ExecutionFlags};
use chibi_core::tools::UrlPolicy;
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
    /// Runtime username override
    #[serde(default)]
    pub username: Option<String>,
    /// Chibi home directory override
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Project root override
    #[serde(default)]
    pub project_root: Option<PathBuf>,
    /// URL security policy override (replaces config-level policy).
    /// Prefer using `config.url_policy` instead — this field is kept for
    /// backwards compatibility.
    #[serde(default)]
    pub url_policy: Option<UrlPolicy>,
    /// Typed config overrides (same semantics as local.toml, schema-documented)
    #[serde(default)]
    pub config: Option<LocalConfig>,
    /// String-keyed overrides (highest priority, freeform escape hatch)
    #[serde(default)]
    pub overrides: Option<BTreeMap<String, String>>,
}
