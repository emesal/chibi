// session.rs: CLI session state (current/previous context)
//
// This tracks which context the user is working with across CLI invocations.
// Separate from chibi-core's ContextState, which manages context metadata.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Session state persisted between CLI invocations.
///
/// Tracks the current and previous context names for context switching.
/// Stored in `~/.chibi/session.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub current_context: String,
    #[serde(default)]
    pub previous_context: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            current_context: "default".to_string(),
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

    /// Switch to a new context, updating previous_context.
    ///
    /// If switching to the same context, previous_context is unchanged.
    pub fn switch_context(&mut self, name: String) {
        if self.current_context != name {
            self.previous_context = Some(self.current_context.clone());
        }
        self.current_context = name;
    }

    /// Swap current and previous contexts.
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
        let current = std::mem::replace(&mut self.current_context, previous.clone());
        self.previous_context = Some(current);
        Ok(previous)
    }

    /// Check if a context name matches the current context.
    pub fn is_current(&self, name: &str) -> bool {
        self.current_context == name
    }

    /// Check if a context name matches the previous context.
    pub fn is_previous(&self, name: &str) -> bool {
        self.previous_context.as_deref() == Some(name)
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
        assert_eq!(session.current_context, "default");
        assert!(session.previous_context.is_none());
    }

    #[test]
    fn test_switch_context() {
        let mut session = Session::default();

        session.switch_context("test".to_string());
        assert_eq!(session.current_context, "test");
        assert_eq!(session.previous_context, Some("default".to_string()));

        session.switch_context("another".to_string());
        assert_eq!(session.current_context, "another");
        assert_eq!(session.previous_context, Some("test".to_string()));
    }

    #[test]
    fn test_switch_to_same_context() {
        let mut session = Session::default();
        session.switch_context("test".to_string());
        session.switch_context("test".to_string()); // same context

        assert_eq!(session.current_context, "test");
        // previous should still be "default", not "test"
        assert_eq!(session.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_swap_with_previous() {
        let mut session = Session::default();
        session.switch_context("test".to_string());

        let result = session.swap_with_previous().unwrap();
        assert_eq!(result, "default");
        assert_eq!(session.current_context, "default");
        assert_eq!(session.previous_context, Some("test".to_string()));

        // swap back
        let result = session.swap_with_previous().unwrap();
        assert_eq!(result, "test");
        assert_eq!(session.current_context, "test");
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
            current_context: "test".to_string(),
            previous_context: Some("".to_string()),
        };
        let result = session.swap_with_previous();
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let mut session = Session::default();
        session.switch_context("test".to_string());

        session.save(temp_dir.path()).unwrap();

        let loaded = Session::load(temp_dir.path()).unwrap();
        assert_eq!(loaded.current_context, "test");
        assert_eq!(loaded.previous_context, Some("default".to_string()));
    }

    #[test]
    fn test_load_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let session = Session::load(temp_dir.path()).unwrap();
        assert_eq!(session.current_context, "default");
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
    fn test_is_current_and_previous() {
        let mut session = Session::default();
        session.switch_context("test".to_string());

        assert!(session.is_current("test"));
        assert!(!session.is_current("default"));
        assert!(session.is_previous("default"));
        assert!(!session.is_previous("test"));
    }
}
