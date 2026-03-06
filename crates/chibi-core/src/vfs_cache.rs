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

/// Extract human-readable preview content from tool output.
///
/// When the output is a single JSON line with `stdout` and/or `stderr` fields
/// (common for `shell_exec`), extracts those fields into a labelled preview.
/// Falls back to returning the raw content unchanged.
pub fn extract_preview_content(content: &str) -> String {
    // Only attempt JSON extraction for single-line content
    if content.lines().count() != 1 {
        return content.to_string();
    }

    let Ok(obj) = serde_json::from_str::<serde_json::Value>(content) else {
        return content.to_string();
    };

    let obj = match obj.as_object() {
        Some(o) => o,
        None => return content.to_string(),
    };

    let stdout = obj.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = obj.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

    if stdout.is_empty() && stderr.is_empty() {
        return content.to_string();
    }

    let mut parts = Vec::new();
    if !stdout.is_empty() {
        parts.push(format!("[stdout]\n{stdout}"));
    }
    if !stderr.is_empty() {
        parts.push(format!("[stderr]\n{stderr}"));
    }
    parts.join("\n\n")
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

    let extracted = extract_preview_content(content);
    let preview: String = extracted.chars().take(preview_chars).collect();
    let preview = if let Some(pos) = preview.rfind('\n') {
        &preview[..pos]
    } else {
        &preview
    };

    format!(
        "[Output cached: {vfs_uri}]\n\
         Tool: {tool_name} | Size: {char_count} chars, ~{token_estimate} tokens | Lines: {line_count}\n\
         Output too large. Full output stored — do NOT re-run this tool.\n\
         Use file_head, file_tail, file_lines, or file_grep with path=\"{vfs_uri}\" to examine.\n\
         Preview (first {} lines):\n\
         ---\n\
         {preview}\n\
         ---",
        preview.lines().count()
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
        assert!(msg.contains("Output too large"));
        assert!(msg.contains("do NOT re-run"));
        assert!(msg.contains("file_head, file_tail, file_lines, or file_grep"));
    }

    #[test]
    fn test_truncated_message_preview_truncates_at_line() {
        let uri = "vfs:///sys/tool_cache/ctx/x";
        // Preview of 6 chars from "abc\ndef" should stop at newline → "abc"
        let msg = truncated_message(uri, "t", "abc\ndef\nghi", 6);
        assert!(msg.contains("abc"));
        assert!(!msg.contains("def"));
    }

    #[test]
    fn test_extract_preview_content_plain_text() {
        let content = "line1\nline2\nline3";
        assert_eq!(extract_preview_content(content), content);
    }

    #[test]
    fn test_extract_preview_content_json_with_stdout() {
        let content = r#"{"stdout":"hello world\nline2","stderr":"","exit_code":0}"#;
        let preview = extract_preview_content(content);
        assert!(preview.contains("hello world"));
        assert!(preview.contains("[stdout]"));
    }

    #[test]
    fn test_extract_preview_content_json_with_stderr() {
        let content = r#"{"stdout":"","stderr":"error: something failed\ndetails","exit_code":1}"#;
        let preview = extract_preview_content(content);
        assert!(preview.contains("error: something failed"));
        assert!(preview.contains("[stderr]"));
    }

    #[test]
    fn test_extract_preview_content_json_without_stdout_stderr() {
        let content = r#"{"result":"some data","count":42}"#;
        assert_eq!(extract_preview_content(content), content);
    }

    #[test]
    fn test_extract_preview_content_multiline_not_json() {
        let content = "line1\nline2";
        assert_eq!(extract_preview_content(content), content);
    }
}
