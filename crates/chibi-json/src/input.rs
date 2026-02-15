use std::path::PathBuf;

use chibi_core::input::{Command, ExecutionFlags};
use chibi_core::tools::UrlPolicy;
use schemars::JsonSchema;
use serde::Deserialize;

/// JSON-mode input -- read from stdin, stateless per invocation.
///
/// Unlike ChibiInput (CLI), context is always explicit (no "current" concept),
/// there's no session, and no context selection enum.
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
    /// URL security policy override (replaces config-level policy)
    #[serde(default)]
    pub url_policy: Option<UrlPolicy>,
}
