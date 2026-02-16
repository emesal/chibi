//! Summary cache with schema-hash invalidation.
//!
//! Caches LLM-generated tool summaries keyed by `server:tool:schema_hash`.
//! Persisted as JSONL in `CHIBI_HOME/mcp-bridge/cache.jsonl`.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// Persistent summary cache backed by a JSONL file.
pub struct SummaryCache {
    entries: HashMap<String, String>,
    path: PathBuf,
}

/// Single cache entry for JSONL serialisation.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    key: String,
    summary: String,
}

/// Build a cache key from server name, tool name, and schema hash.
fn cache_key(server: &str, tool: &str, schema: &serde_json::Value) -> String {
    let schema_str = serde_json::to_string(schema).unwrap_or_default();
    let hash = Sha256::digest(schema_str.as_bytes());
    // Use first 8 bytes (16 hex chars) for a compact but collision-resistant key
    let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{server}:{tool}:{hex}")
}

impl SummaryCache {
    /// Load cache from disk, or create an empty cache if the file doesn't exist.
    pub fn load(home: &Path) -> Self {
        let path = home.join("mcp-bridge").join("cache.jsonl");
        let entries = match std::fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .filter_map(|line| serde_json::from_str::<CacheEntry>(line).ok())
                .map(|e| (e.key, e.summary))
                .collect(),
            Err(_) => HashMap::new(),
        };
        Self { entries, path }
    }

    /// Save the cache to disk as JSONL.
    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content: String = self
            .entries
            .iter()
            .map(|(key, summary)| {
                serde_json::to_string(&CacheEntry {
                    key: key.clone(),
                    summary: summary.clone(),
                })
                .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&self.path, content)
    }

    /// Look up a cached summary for a tool.
    pub fn get(&self, server: &str, tool: &str, schema: &serde_json::Value) -> Option<&str> {
        let key = cache_key(server, tool, schema);
        self.entries.get(&key).map(|s| s.as_str())
    }

    /// Store a summary in the cache.
    pub fn set(&mut self, server: &str, tool: &str, schema: &serde_json::Value, summary: String) {
        let key = cache_key(server, tool, schema);
        self.entries.insert(key, summary);
    }

    /// Number of cached entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_cache_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SummaryCache::load(tmp.path());
        assert!(cache.get("srv", "tool", &json!({})).is_none());
    }

    #[test]
    fn set_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = SummaryCache::load(tmp.path());
        let schema = json!({"type": "object"});
        cache.set("srv", "greet", &schema, "says hello".into());
        assert_eq!(cache.get("srv", "greet", &schema), Some("says hello"));
    }

    #[test]
    fn different_schema_hash_is_cache_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = SummaryCache::load(tmp.path());
        let schema_v1 = json!({"type": "object", "properties": {"a": {}}});
        let schema_v2 = json!({"type": "object", "properties": {"a": {}, "b": {}}});
        cache.set("srv", "tool", &schema_v1, "v1 summary".into());
        assert_eq!(cache.get("srv", "tool", &schema_v1), Some("v1 summary"));
        assert!(cache.get("srv", "tool", &schema_v2).is_none());
    }

    #[test]
    fn save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let schema = json!({"type": "object"});

        {
            let mut cache = SummaryCache::load(tmp.path());
            cache.set("srv", "tool1", &schema, "summary one".into());
            cache.set("srv", "tool2", &schema, "summary two".into());
            cache.save().unwrap();
        }

        let cache = SummaryCache::load(tmp.path());
        assert_eq!(cache.get("srv", "tool1", &schema), Some("summary one"));
        assert_eq!(cache.get("srv", "tool2", &schema), Some("summary two"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn missing_file_returns_empty_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SummaryCache::load(tmp.path());
        assert_eq!(cache.len(), 0);
    }
}
