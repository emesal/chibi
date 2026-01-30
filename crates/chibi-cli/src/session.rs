// session.rs: CLI session state (implied/previous context)
//
// This tracks which context the user is working with across CLI invocations.
// Separate from chibi-core's ContextState, which manages context metadata.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Session state persisted between CLI invocations.
///
/// Tracks the implied and previous context names for context switching.
/// - `implied_context`: The context used when no `-c`/`-C` is specified
/// - `previous_context`: The last context switched away from (for `-c -`)
///
/// Stored in `~/.chibi/session.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub implied_context: String,
    #[serde(default)]
    pub previous_context: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            implied_context: "default".to_string(),
            previous_context: None,
        }
    }
}

impl Session {
    /// Load session from `session.json` in the given chibi directory.
    ///
    /// Returns default session if file doesn't exist.
    pub fn load(chibi_dir: &Path) -> io::Result<Self> {
        let path = chibi_dir.join("session.json");
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        } else {
            Ok(Self::default())
        }
    }

    /// Save session to `session.json` in the given chibi directory.
    pub fn save(&self, chibi_dir: &Path) -> io::Result<()> {
        let path = chibi_dir.join("session.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)
    }

    /// Switch to a new context, updating previous_context and implied_context.
    ///
    /// If switching to the same context, previous_context is unchanged.
    pub fn switch_context(&mut self, name: String) {
        if self.implied_context != name {
            self.previous_context = Some(self.implied_context.clone());
        }
        self.implied_context = name;
    }

    /// Swap implied and previous contexts.
    ///
    /// Returns the name of the context switched to (the previous context).
    /// Returns an error if there is no previous context.
    pub fn swap_with_previous(&mut self) -> io::Result<String> {
        let previous = self
            .previous_context
            .take()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "No previous context available (use -c to switch contexts first)",
                )
            })?;
        let current = std::mem::replace(&mut self.implied_context, previous.clone());
        self.previous_context = Some(current);
        Ok(previous)
    }

    /// Check if a context name matches the implied context.
    pub fn is_implied(&self, name: &str) -> bool {
        self.implied_context == name
    }

    /// Get the previous context name, or error if none available
    pub fn get_previous(&self) -> io::Result<String> {
        self.previous_context
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "No previous context available (use -c to switch contexts first)",
                )
            })
    }

    /// Check if a context name matches the previous context.
    pub fn is_previous(&self, name: &str) -> bool {
        self.previous_context.as_deref() == Some(name)
    }

    /// Handle session state after a context is destroyed.
    ///
    /// If the destroyed context was the implied context, switches to a fallback:
    /// - Previous context if it exists and is valid
    /// - Otherwise "default"
    ///
    /// The `context_exists` closure is used to check if a context directory exists.
    /// Returns the new implied context name if a switch occurred, None otherwise.
    pub fn handle_context_destroyed<F>(
        &mut self,
        destroyed: &str,
        context_exists: F,
    ) -> Option<String>
    where
        F: Fn(&str) -> bool,
    {
        if self.implied_context != destroyed {
            // Not destroying implied context, no session changes needed
            // (but clear previous if it was the destroyed one)
            if self.previous_context.as_deref() == Some(destroyed) {
                self.previous_context = None;
            }
            return None;
        }

        // Destroying implied context - find a fallback
        let fallback = self
            .previous_context
            .as_ref()
            .filter(|p| *p != destroyed && context_exists(p))
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        self.implied_context = fallback.clone();
        self.previous_context = None;
        Some(fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_session() {
        let session = Session::default();
        assert_eq!(session.implied_context, "default");
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_switch_context() {
        let mut session = Session::default();

        session.switch_context("test".to_string());
        assert_eq!(session.implied_context, "test");
        assert_eq!(session.previous_context, Some("default".to_string()));

        session.switch_context("another".to_string());
        assert_eq!(session.implied_context, "another");
        assert_eq!(session.previous_context, Some("test".to_string()));
    }

    #[test]
    fn test_switch_to_same_context() {
        let mut session = Session::default();
        session.switch_context("test".to_string());
        session.switch_context("test".to_string()); // same context

        assert_eq!(session.implied_context, "test");
        // previous should still be "default", not "test"
        assert_eq!(session.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_swap_with_previous() {
        let mut session = Session::default();
        session.switch_context("test".to_string());

        let result = session.swap_with_previous().unwrap();
        assert_eq!(result, "default");
        assert_eq!(session.implied_context, "default");
        assert_eq!(session.previous_context, Some("test".to_string()));

        // swap back
        let result = session.swap_with_previous().unwrap();
        assert_eq!(result, "test");
        assert_eq!(session.implied_context, "test");
        assert_eq!(session.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_swap_no_previous() {
        let mut session = Session::default();
        let result = session.swap_with_previous();
        assert!(result.is_err());
    }

    #[test]
    fn test_swap_empty_previous() {
        let mut session = Session {
            implied_context: "test".to_string(),
            previous_context: Some("".to_string()),
        };
        let result = session.swap_with_previous();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_previous() {
        let mut session = Session::default();
        session.switch_context("test".to_string());

        let result = session.get_previous().unwrap();
        assert_eq!(result, "default");
    }

    #[test]
    fn test_get_previous_none() {
        let session = Session::default();
        let result = session.get_previous();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_previous_empty() {
        let session = Session {
            implied_context: "test".to_string(),
            previous_context: Some("".to_string()),
        };
        let result = session.get_previous();
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let mut session = Session::default();
        session.switch_context("test".to_string());

        session.save(temp_dir.path()).unwrap();

        let loaded = Session::load(temp_dir.path()).unwrap();
        assert_eq!(loaded.implied_context, "test");
        assert_eq!(loaded.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_load_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let session = Session::load(temp_dir.path()).unwrap();
        assert_eq!(session.implied_context, "default");
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_load_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("session.json");
        fs::write(&path, "not valid json").unwrap();

        let result = Session::load(temp_dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_is_implied_and_previous() {
        let mut session = Session::default();
        session.switch_context("test".to_string());

        assert!(session.is_implied("test"));
        assert!(!session.is_implied("default"));
        assert!(session.is_previous("default"));
        assert!(!session.is_previous("test"));
    }

    #[test]
    fn test_handle_context_destroyed_current_falls_back_to_previous() {
        let mut session = Session::default();
        session.switch_context("ctx_a".to_string());
        session.switch_context("ctx_b".to_string());

        // ctx_b is current, ctx_a is previous
        assert_eq!(session.implied_context, "ctx_b");
        assert_eq!(session.previous_context, Some("ctx_a".to_string()));

        // Destroy current (ctx_b), previous (ctx_a) exists
        let result = session.handle_context_destroyed("ctx_b", |name| name == "ctx_a");

        assert_eq!(result, Some("ctx_a".to_string()));
        assert_eq!(session.implied_context, "ctx_a");
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_handle_context_destroyed_current_falls_back_to_default() {
        let mut session = Session::default();
        session.switch_context("lone_ctx".to_string());

        // lone_ctx is current, previous is "default" but doesn't exist
        assert_eq!(session.implied_context, "lone_ctx");
        assert_eq!(session.previous_context, Some("default".to_string()));

        // Destroy current, previous doesn't exist as a directory
        let result = session.handle_context_destroyed("lone_ctx", |_| false);

        assert_eq!(result, Some("default".to_string()));
        assert_eq!(session.implied_context, "default");
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_handle_context_destroyed_not_current() {
        let mut session = Session::default();
        session.switch_context("ctx_a".to_string());
        session.switch_context("ctx_b".to_string());

        // Destroy ctx_a (not current), should return None and not change current
        let result = session.handle_context_destroyed("ctx_a", |_| true);

        assert!(result.is_none());
        assert_eq!(session.implied_context, "ctx_b");
        // previous_context should be cleared since it was destroyed
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_handle_context_destroyed_previous_same_as_destroyed() {
        let mut session = Session {
            implied_context: "current".to_string(),
            previous_context: Some("current".to_string()), // edge case: same as current
        };

        // Destroy current, but previous points to same context
        let result = session.handle_context_destroyed("current", |_| true);

        // Should fall back to default since previous == destroyed
        assert_eq!(result, Some("default".to_string()));
        assert_eq!(session.implied_context, "default");
    }

    #[test]
    fn test_handle_context_destroyed_no_previous() {
        let mut session = Session {
            implied_context: "only_ctx".to_string(),
            previous_context: None,
        };

        let result = session.handle_context_destroyed("only_ctx", |_| true);

        assert_eq!(result, Some("default".to_string()));
        assert_eq!(session.implied_context, "default");
        assert!(session.previous_context.is_none());
    }
}
