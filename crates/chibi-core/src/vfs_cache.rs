//! VFS-backed tool output cache.
//!
//! Stores large tool outputs under `vfs:///sys/tool_cache/<context>/<id>`,
//! written as SYSTEM_CALLER and world-readable. Replaces the old `cache.rs`
//! flat-file system.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a unique cache ID.
/// Format: `{tool}_{timestamp_hex}_{args_hash}` — globally unique, no collision risk.
pub fn generate_cache_id(tool_name: &str, args: &serde_json::Value) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut hasher = DefaultHasher::new();
    args.to_string().hash(&mut hasher);
    let hash = hasher.finish();

    format!("{}_{:x}_{:08x}", tool_name, timestamp, hash as u32)
}

/// Return the VFS path (without URI scheme) for a cache entry.
pub fn vfs_path_for(context_name: &str, cache_id: &str) -> String {
    format!("/sys/tool_cache/{}/{}", context_name, cache_id)
}

/// Return the `vfs:///` URI for a cache entry.
///
/// `vfs_path_for` always returns a path beginning with `/`, so `"vfs://"` +
/// `"/sys/..."` concatenates to `"vfs:///sys/..."` — three slashes as required.
pub fn vfs_uri_for(context_name: &str, cache_id: &str) -> String {
    format!("vfs://{}", vfs_path_for(context_name, cache_id))
}

/// Check if content should be cached based on size threshold.
/// Does not cache empty or whitespace-only content.
pub fn should_cache(content: &str, threshold: usize) -> bool {
    if content.trim().is_empty() {
        return false;
    }
    content.len() > threshold
}

/// Generate the truncated stub message shown to the LLM instead of the full output.
pub fn truncated_message(
    vfs_uri: &str,
    tool_name: &str,
    content: &str,
    preview_chars: usize,
) -> String {
    let char_count = content.len();
    let token_estimate = char_count / 4;
    let line_count = content.lines().count();

    let preview: String = content.chars().take(preview_chars).collect();
    let preview = if let Some(pos) = preview.rfind('\n') {
        &preview[..pos]
    } else {
        &preview
    };

    format!(
        "[Output cached: {vfs_uri}]\n\
         Tool: {tool_name} | Size: {char_count} chars, ~{token_estimate} tokens | Lines: {line_count}\n\
         Preview:\n\
         ---\n\
         {preview}\n\
         ---\n\
         Use file_head, file_tail, file_lines, file_grep with path=\"{vfs_uri}\" to examine."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_generate_cache_id_format() {
        let id = generate_cache_id("web_fetch", &serde_json::json!({"url": "x"}));
        assert!(id.starts_with("web_fetch_"));
    }

    #[test]
    fn test_vfs_path_for() {
        let p = vfs_path_for("myctx", "tool_abc_123");
        assert_eq!(p, "/sys/tool_cache/myctx/tool_abc_123");
    }

    #[test]
    fn test_vfs_uri_for() {
        let uri = vfs_uri_for("myctx", "tool_abc_123");
        assert_eq!(uri, "vfs:///sys/tool_cache/myctx/tool_abc_123");
        assert!(crate::vfs::VfsPath::is_vfs_uri(&uri));
    }

    #[test]
    fn test_truncated_message_contains_uri() {
        let uri = "vfs:///sys/tool_cache/ctx/web_fetch_1_2";
        let msg = truncated_message(uri, "web_fetch", "line1\nline2\nline3", 200);
        assert!(msg.contains(uri));
        assert!(msg.contains("web_fetch"));
        assert!(msg.contains("file_head"));
    }

    #[test]
    fn test_truncated_message_preview_truncates_at_line() {
        let uri = "vfs:///sys/tool_cache/ctx/x";
        // Preview of 6 chars from "abc\ndef" should stop at newline → "abc"
        let msg = truncated_message(uri, "t", "abc\ndef\nghi", 6);
        assert!(msg.contains("abc"));
        assert!(!msg.contains("def"));
    }
}
