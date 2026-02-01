//! JSON value extraction helpers.
//!
//! Provides concise accessors for common JSON value extraction patterns,
//! eliminating repetitive `.get().and_then().unwrap_or()` chains.

use serde_json::Value;

/// Extension trait for JSON value extraction
pub trait JsonExt {
    /// Get a string value, returning None if key missing or not a string
    fn get_str(&self, key: &str) -> Option<&str>;

    /// Get a string value with a default
    fn get_str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str;

    /// Get a bool value, returning None if key missing or not a bool
    fn get_bool(&self, key: &str) -> Option<bool>;

    /// Get a bool value with a default (commonly false)
    fn get_bool_or(&self, key: &str, default: bool) -> bool;

    /// Get a u64 value, returning None if key missing or not a number
    fn get_u64(&self, key: &str) -> Option<u64>;

    /// Get a u64 value with a default
    fn get_u64_or(&self, key: &str, default: u64) -> u64;

    /// Get an array value, returning None if key missing or not an array
    fn get_array(&self, key: &str) -> Option<&Vec<Value>>;
}

impl JsonExt for Value {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }

    fn get_str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get_str(key).unwrap_or(default)
    }

    fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| v.as_bool())
    }

    fn get_bool_or(&self, key: &str, default: bool) -> bool {
        self.get_bool(key).unwrap_or(default)
    }

    fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|v| v.as_u64())
    }

    fn get_u64_or(&self, key: &str, default: u64) -> u64 {
        self.get_u64(key).unwrap_or(default)
    }

    fn get_array(&self, key: &str) -> Option<&Vec<Value>> {
        self.get(key).and_then(|v| v.as_array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_str() {
        let v = json!({"name": "test", "count": 42});
        assert_eq!(v.get_str("name"), Some("test"));
        assert_eq!(v.get_str("missing"), None);
        assert_eq!(v.get_str("count"), None); // not a string
    }

    #[test]
    fn test_get_str_or() {
        let v = json!({"name": "test"});
        assert_eq!(v.get_str_or("name", "default"), "test");
        assert_eq!(v.get_str_or("missing", "default"), "default");
    }

    #[test]
    fn test_get_bool() {
        let v = json!({"flag": true, "name": "test"});
        assert_eq!(v.get_bool("flag"), Some(true));
        assert_eq!(v.get_bool("missing"), None);
        assert_eq!(v.get_bool("name"), None); // not a bool
    }

    #[test]
    fn test_get_bool_or() {
        let v = json!({"flag": true});
        assert!(v.get_bool_or("flag", false));
        assert!(!v.get_bool_or("missing", false));
    }

    #[test]
    fn test_get_u64() {
        let v = json!({"count": 42, "name": "test"});
        assert_eq!(v.get_u64("count"), Some(42));
        assert_eq!(v.get_u64("missing"), None);
        assert_eq!(v.get_u64("name"), None); // not a number
    }

    #[test]
    fn test_get_u64_or() {
        let v = json!({"count": 42});
        assert_eq!(v.get_u64_or("count", 0), 42);
        assert_eq!(v.get_u64_or("missing", 100), 100);
    }

    #[test]
    fn test_get_array() {
        let v = json!({"items": [1, 2, 3], "name": "test"});
        assert!(v.get_array("items").is_some());
        assert_eq!(v.get_array("items").unwrap().len(), 3);
        assert!(v.get_array("missing").is_none());
        assert!(v.get_array("name").is_none()); // not an array
    }
}
