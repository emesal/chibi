use crate::input::{Command, ExecutionFlags};

/// What core needs to execute any command.
///
/// Each binary (chibi-cli, chibi-json) maps its own input type to this.
/// Core never sees CLI concepts (sessions, persistent switches) or JSON
/// concepts (always-on JSON mode).
pub struct ExecutionRequest {
    /// The command to execute
    pub command: Command,
    /// Context name — always explicit, already resolved by the caller
    pub context: String,
    /// Execution-only flags
    pub flags: ExecutionFlags,
    /// Runtime username override (already resolved by caller)
    pub username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_request_default_fields() {
        let req = ExecutionRequest {
            command: Command::NoOp,
            context: "test".to_string(),
            flags: ExecutionFlags::default(),
            username: None,
        };
        assert_eq!(req.context, "test");
        assert!(req.username.is_none());
    }
}
