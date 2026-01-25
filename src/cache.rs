//! Tool output caching for large results.
//!
//! When tool outputs exceed the configured threshold, they are cached to disk
//! and a truncated summary is sent to the LLM instead.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Metadata about a cached tool output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// Unique cache ID (format: {tool}_{timestamp}_{hash})
    pub id: String,
    /// Name of the tool that produced this output
    pub tool_name: String,
    /// Unix timestamp when cached
    pub timestamp: u64,
    /// Hash of the tool arguments (for deduplication)
    pub args_hash: String,
    /// Size in characters
    pub char_count: usize,
    /// Approximate token count (chars / 4)
    pub token_estimate: usize,
    /// Number of lines
    pub line_count: usize,
}

/// A cache entry representing a cached tool output
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub metadata: CacheMetadata,
    /// Path to the cache file
    pub cache_path: std::path::PathBuf,
    /// Path to the metadata file (kept for potential future use)
    #[allow(dead_code)]
    pub meta_path: std::path::PathBuf,
}

/// Check if content should be cached based on size threshold
pub fn should_cache(content: &str, threshold: usize) -> bool {
    // Don't cache empty or whitespace-only content
    if content.trim().is_empty() {
        return false;
    }
    content.len() > threshold
}

/// Generate a unique cache ID
fn generate_cache_id(tool_name: &str, args: &serde_json::Value) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut hasher = DefaultHasher::new();
    args.to_string().hash(&mut hasher);
    let hash = hasher.finish();

    format!("{}_{:x}_{:08x}", tool_name, timestamp, hash as u32)
}

/// Cache tool output to disk
///
/// Returns the cache entry on success
pub fn cache_output(
    cache_dir: &Path,
    tool_name: &str,
    content: &str,
    args: &serde_json::Value,
) -> io::Result<CacheEntry> {
    // Ensure cache directory exists
    fs::create_dir_all(cache_dir)?;

    let cache_id = generate_cache_id(tool_name, args);

    let cache_path = cache_dir.join(format!("{}.cache", cache_id));
    let meta_path = cache_dir.join(format!("{}.meta.json", cache_id));

    // Compute statistics
    let char_count = content.len();
    let line_count = content.lines().count();
    let token_estimate = char_count / 4; // rough approximation

    let mut hasher = DefaultHasher::new();
    args.to_string().hash(&mut hasher);
    let args_hash = format!("{:016x}", hasher.finish());

    let metadata = CacheMetadata {
        id: cache_id,
        tool_name: tool_name.to_string(),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        args_hash,
        char_count,
        token_estimate,
        line_count,
    };

    // Write content to cache file atomically (write to temp, then rename)
    let temp_path = cache_dir.join(format!("{}.tmp", metadata.id));
    fs::write(&temp_path, content)?;
    fs::rename(&temp_path, &cache_path)?;

    // Write metadata
    let meta_json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| io::Error::other(format!("Failed to serialize cache metadata: {}", e)))?;
    fs::write(&meta_path, meta_json)?;

    Ok(CacheEntry {
        metadata,
        cache_path,
        meta_path,
    })
}

/// Generate a truncated message to send to the LLM instead of the full output
pub fn generate_truncated_message(entry: &CacheEntry, preview_chars: usize) -> io::Result<String> {
    let content = fs::read_to_string(&entry.cache_path)?;
    let preview: String = content.chars().take(preview_chars).collect();

    // Truncate at last complete line if possible
    let preview = if let Some(pos) = preview.rfind('\n') {
        &preview[..pos]
    } else {
        &preview
    };

    Ok(format!(
        "[Output cached: {}]\n\
         Tool: {} | Size: {} chars, ~{} tokens | Lines: {}\n\
         Preview:\n\
         ---\n\
         {}\n\
         ---\n\
         Use file_head, file_tail, file_lines, file_grep with cache_id=\"{}\" to examine.",
        entry.metadata.id,
        entry.metadata.tool_name,
        entry.metadata.char_count,
        entry.metadata.token_estimate,
        entry.metadata.line_count,
        preview,
        entry.metadata.id
    ))
}

/// Read the first N lines from a cached file
pub fn read_cache_head(cache_path: &Path, n: usize) -> io::Result<String> {
    let file = File::open(cache_path)?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().take(n).collect::<Result<_, _>>()?;
    Ok(lines.join("\n"))
}

/// Read the last N lines from a cached file
pub fn read_cache_tail(cache_path: &Path, n: usize) -> io::Result<String> {
    let content = fs::read_to_string(cache_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}

/// Read lines from start to end (1-indexed, inclusive)
pub fn read_cache_lines(cache_path: &Path, start: usize, end: usize) -> io::Result<String> {
    let file = File::open(cache_path)?;
    let reader = BufReader::new(file);

    // Convert 1-indexed to 0-indexed: line N is at index N-1
    // For inclusive end, we want i < end (0-indexed end is end itself)
    let lines: Vec<String> = reader
        .lines()
        .enumerate()
        .filter(|(i, _)| *i >= start.saturating_sub(1) && *i < end)
        .map(|(_, line)| line)
        .collect::<Result<_, _>>()?;

    Ok(lines.join("\n"))
}

/// Search for a pattern in a cached file, returning matching lines with context
pub fn read_cache_grep(
    cache_path: &Path,
    pattern: &str,
    context_before: usize,
    context_after: usize,
) -> io::Result<String> {
    let content = fs::read_to_string(cache_path)?;
    let lines: Vec<&str> = content.lines().collect();

    let regex = regex::Regex::new(pattern).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid regex: {}", e))
    })?;

    let mut result = Vec::new();
    let mut last_end = 0;

    for (i, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            let start = i.saturating_sub(context_before);
            let end = (i + context_after + 1).min(lines.len());

            // Add separator if there's a gap
            if start > last_end && !result.is_empty() {
                result.push("--".to_string());
            }

            // Add lines we haven't added yet
            let range_start = start.max(last_end);
            for (j, line) in lines.iter().enumerate().take(end).skip(range_start) {
                let prefix = if j == i { ">" } else { " " };
                result.push(format!("{}{}:{}", prefix, j + 1, line));
            }

            last_end = end;
        }
    }

    Ok(result.join("\n"))
}

/// List all cache entries for a context
pub fn list_cache_entries(cache_dir: &Path) -> io::Result<Vec<CacheMetadata>> {
    if !cache_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Check for .meta.json files
        let is_meta_file = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".meta.json"));

        if !is_meta_file {
            continue;
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<CacheMetadata>(&content) else {
            continue;
        };
        entries.push(metadata);
    }

    // Sort by timestamp (newest first)
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(entries)
}

/// Clean up cache entries older than max_age_days
/// Note: max_age_days is offset by 1, so:
/// - 0 = delete after 24 hours (1 day)
/// - 1 = delete after 48 hours (2 days)
/// - 7 = delete after 8 days (default)
pub fn cleanup_old_cache(cache_dir: &Path, max_age_days: u64) -> io::Result<usize> {
    if !cache_dir.exists() {
        return Ok(0);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Add 1 to max_age_days so 0 means 1 day, 1 means 2 days, etc.
    let max_age_secs = (max_age_days + 1) * 24 * 60 * 60;
    let cutoff = now.saturating_sub(max_age_secs);

    let mut removed = 0;

    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Check for .meta.json files
        let is_meta_file = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".meta.json"));

        if !is_meta_file {
            continue;
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<CacheMetadata>(&content) else {
            continue;
        };

        if metadata.timestamp < cutoff {
            // Remove both cache and meta files
            let cache_path = cache_dir.join(format!("{}.cache", metadata.id));
            let _ = fs::remove_file(&cache_path);
            let _ = fs::remove_file(&path);
            removed += 1;
        }
    }

    Ok(removed)
}

/// Remove all cache files for a context
pub fn clear_cache(cache_dir: &Path) -> io::Result<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

/// Resolve a cache_id to its cache file path
pub fn resolve_cache_path(cache_dir: &Path, cache_id: &str) -> io::Result<std::path::PathBuf> {
    let cache_path = cache_dir.join(format!("{}.cache", cache_id));
    if cache_path.exists() {
        Ok(cache_path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Cache entry not found: {}", cache_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_should_cache_threshold() {
        assert!(!should_cache("short", 100));
        assert!(should_cache(&"x".repeat(101), 100));
    }

    #[test]
    fn test_should_cache_empty() {
        assert!(!should_cache("", 10));
        assert!(!should_cache("   ", 10));
        assert!(!should_cache("\n\n", 10));
    }

    #[test]
    fn test_cache_output() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5";
        let args = serde_json::json!({"url": "https://example.com"});

        let entry = cache_output(temp_dir.path(), "web_fetch", content, &args).unwrap();

        assert!(entry.cache_path.exists());
        assert!(entry.meta_path.exists());
        assert_eq!(entry.metadata.tool_name, "web_fetch");
        assert_eq!(entry.metadata.line_count, 5);
        assert_eq!(entry.metadata.char_count, content.len());
    }

    #[test]
    fn test_generate_truncated_message() {
        let temp_dir = TempDir::new().unwrap();
        let content = "This is a test\nWith multiple lines\nAnd some content";
        let args = serde_json::json!({});

        let entry = cache_output(temp_dir.path(), "test_tool", content, &args).unwrap();
        let msg = generate_truncated_message(&entry, 100).unwrap();

        assert!(msg.contains("[Output cached:"));
        assert!(msg.contains("test_tool"));
        assert!(msg.contains("file_head"));
    }

    #[test]
    fn test_read_cache_head() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5";
        let args = serde_json::json!({});

        let entry = cache_output(temp_dir.path(), "test", content, &args).unwrap();
        let head = read_cache_head(&entry.cache_path, 2).unwrap();

        assert_eq!(head, "line1\nline2");
    }

    #[test]
    fn test_read_cache_tail() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5";
        let args = serde_json::json!({});

        let entry = cache_output(temp_dir.path(), "test", content, &args).unwrap();
        let tail = read_cache_tail(&entry.cache_path, 2).unwrap();

        assert_eq!(tail, "line4\nline5");
    }

    #[test]
    fn test_read_cache_lines() {
        let temp_dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\nline4\nline5";
        let args = serde_json::json!({});

        let entry = cache_output(temp_dir.path(), "test", content, &args).unwrap();
        let lines = read_cache_lines(&entry.cache_path, 2, 4).unwrap();

        assert_eq!(lines, "line2\nline3\nline4");
    }

    #[test]
    fn test_read_cache_grep() {
        let temp_dir = TempDir::new().unwrap();
        let content = "apple\nbanana\ncherry\napricot\nblueberry";
        let args = serde_json::json!({});

        let entry = cache_output(temp_dir.path(), "test", content, &args).unwrap();
        let result = read_cache_grep(&entry.cache_path, "^a", 0, 0).unwrap();

        assert!(result.contains("apple"));
        assert!(result.contains("apricot"));
        assert!(!result.contains("banana"));
    }

    #[test]
    fn test_list_cache_entries() {
        let temp_dir = TempDir::new().unwrap();

        // Create a few cache entries
        cache_output(temp_dir.path(), "tool1", "content1", &serde_json::json!({})).unwrap();
        cache_output(temp_dir.path(), "tool2", "content2", &serde_json::json!({})).unwrap();

        let entries = list_cache_entries(temp_dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_clear_cache() {
        let temp_dir = TempDir::new().unwrap();

        cache_output(temp_dir.path(), "test", "content", &serde_json::json!({})).unwrap();
        assert!(temp_dir.path().exists());

        clear_cache(temp_dir.path()).unwrap();
        assert!(!temp_dir.path().exists());
    }
}
