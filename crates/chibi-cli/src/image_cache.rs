use chibi_core::safe_io;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageCacheMetadata {
    pub url: String,
    pub key: String,
    pub size_bytes: u64,
    pub created_at: u64,
    pub last_accessed_at: u64,
}

fn cache_key(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn img_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(format!("{}.img", key))
}

fn meta_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(format!("{}.meta.json", key))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Return cached image bytes on hit, updating `last_accessed_at` (best-effort).
/// Returns `None` on miss or any read error.
pub fn cache_get(cache_dir: &Path, url: &str) -> Option<Vec<u8>> {
    let key = cache_key(url);
    let img = img_path(cache_dir, &key);
    let meta = meta_path(cache_dir, &key);

    let bytes = fs::read(&img).ok()?;
    let meta_bytes = fs::read(&meta).ok()?;
    let mut metadata: ImageCacheMetadata = serde_json::from_slice(&meta_bytes).ok()?;

    // Best-effort update of last_accessed_at using atomic write pattern
    metadata.last_accessed_at = now_unix();
    let meta_tmp = cache_dir.join(format!("{}.meta.json.tmp", key));
    if let Ok(json_str) = serde_json::to_string(&metadata) {
        let _ = fs::write(&meta_tmp, json_str).and_then(|_| fs::rename(&meta_tmp, &meta));
    }

    Some(bytes)
}

/// Store image bytes in the cache. Atomic write via `.tmp` rename.
/// Triggers cleanup after writing.
pub fn cache_put(
    cache_dir: &Path,
    url: &str,
    bytes: &[u8],
    max_bytes: u64,
    max_age_days: u64,
) -> io::Result<()> {
    fs::create_dir_all(cache_dir)?;

    let key = cache_key(url);
    let img = img_path(cache_dir, &key);
    let meta = meta_path(cache_dir, &key);

    let now = now_unix();
    let metadata = ImageCacheMetadata {
        url: url.to_string(),
        key: key.clone(),
        size_bytes: bytes.len() as u64,
        created_at: now,
        last_accessed_at: now,
    };

    let meta_json =
        serde_json::to_string(&metadata).map_err(|e| io::Error::other(format!("{}", e)))?;

    // Atomic write with fsync (prevents corruption on crash)
    safe_io::atomic_write(&img, bytes)?;
    safe_io::atomic_write_text(&meta, &meta_json)?;

    // Trigger cleanup (best-effort)
    let _ = cleanup_image_cache(cache_dir, max_bytes, max_age_days);

    Ok(())
}

/// Two-phase eviction: age-based, then LRU by size.
/// Also removes orphan `.tmp` files and entries with corrupt sidecars.
/// Returns the number of entries removed.
pub fn cleanup_image_cache(
    cache_dir: &Path,
    max_bytes: u64,
    max_age_days: u64,
) -> io::Result<usize> {
    if !cache_dir.exists() {
        return Ok(0);
    }

    let now = now_unix();
    let max_age_secs = max_age_days * 86400;
    let mut removed = 0;

    // Remove orphan .tmp files
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }

    // Collect all cache entries by reading .meta.json files
    let mut entries: Vec<(ImageCacheMetadata, PathBuf, PathBuf)> = Vec::new();

    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.ends_with(".meta.json") {
            continue;
        }

        let meta_file = entry.path();
        let key = name_str.trim_end_matches(".meta.json").to_string();
        let img_file = img_path(cache_dir, &key);

        // Try to parse metadata
        let meta_bytes = match fs::read(&meta_file) {
            Ok(b) => b,
            Err(_) => {
                // Corrupt — remove both files
                let _ = fs::remove_file(&meta_file);
                let _ = fs::remove_file(&img_file);
                removed += 1;
                continue;
            }
        };

        let metadata: ImageCacheMetadata = match serde_json::from_slice(&meta_bytes) {
            Ok(m) => m,
            Err(_) => {
                // Corrupt sidecar — remove both files
                let _ = fs::remove_file(&meta_file);
                let _ = fs::remove_file(&img_file);
                removed += 1;
                continue;
            }
        };

        // Check if .img file exists; if not, remove orphan metadata
        if !img_file.exists() {
            let _ = fs::remove_file(&meta_file);
            removed += 1;
            continue;
        }

        entries.push((metadata, img_file, meta_file));
    }

    // Phase 1: age eviction
    let mut remaining: Vec<(ImageCacheMetadata, PathBuf, PathBuf)> = Vec::new();
    for (meta, img_file, meta_file) in entries {
        if now.saturating_sub(meta.created_at) > max_age_secs {
            let _ = fs::remove_file(&img_file);
            let _ = fs::remove_file(&meta_file);
            removed += 1;
        } else {
            remaining.push((meta, img_file, meta_file));
        }
    }

    // Phase 2: LRU eviction if over size limit
    let total_size: u64 = remaining.iter().map(|(m, _, _)| m.size_bytes).sum();
    if total_size > max_bytes {
        // Sort by last_accessed_at ascending (oldest accessed first)
        remaining.sort_by_key(|(m, _, _)| m.last_accessed_at);

        let mut current_size = total_size;
        for (meta, img_file, meta_file) in &remaining {
            if current_size <= max_bytes {
                break;
            }
            let _ = fs::remove_file(img_file);
            let _ = fs::remove_file(meta_file);
            current_size = current_size.saturating_sub(meta.size_bytes);
            removed += 1;
        }
    }

    Ok(removed)
}

/// Remove the entire cache directory.
#[allow(dead_code)]
pub fn clear_image_cache(cache_dir: &Path) -> io::Result<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_cache_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = cache_key("https://example.com/image.png");
        let k2 = cache_key("https://example.com/image.png");
        let k3 = cache_key("https://example.com/other.png");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_cache_put_and_get() {
        let dir = make_cache_dir();
        let url = "https://example.com/test.png";
        let data = b"fake image bytes";

        cache_put(dir.path(), url, data, 100_000_000, 30).unwrap();

        let got = cache_get(dir.path(), url);
        assert_eq!(got, Some(data.to_vec()));
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let dir = make_cache_dir();
        assert_eq!(cache_get(dir.path(), "https://nowhere.test/x.png"), None);
    }

    #[test]
    fn test_cache_get_updates_last_accessed() {
        let dir = make_cache_dir();
        let url = "https://example.com/access.png";
        let data = b"data";

        cache_put(dir.path(), url, data, 100_000_000, 30).unwrap();

        // Read metadata before access
        let key = cache_key(url);
        let meta_file = meta_path(dir.path(), &key);
        let before: ImageCacheMetadata =
            serde_json::from_slice(&fs::read(&meta_file).unwrap()).unwrap();

        // Small delay then access
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = cache_get(dir.path(), url);

        let after: ImageCacheMetadata =
            serde_json::from_slice(&fs::read(&meta_file).unwrap()).unwrap();
        assert!(after.last_accessed_at >= before.last_accessed_at);
    }

    #[test]
    fn test_cache_put_atomic_no_tmp_remains() {
        let dir = make_cache_dir();
        let url = "https://example.com/atomic.png";
        cache_put(dir.path(), url, b"img", 100_000_000, 30).unwrap();

        for entry in fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().ends_with(".tmp"),
                "found leftover .tmp file"
            );
        }
    }

    #[test]
    fn test_cleanup_age_eviction() {
        let dir = make_cache_dir();
        let url = "https://example.com/old.png";
        cache_put(dir.path(), url, b"old", 100_000_000, 30).unwrap();

        // Backdate the created_at
        let key = cache_key(url);
        let mpath = meta_path(dir.path(), &key);
        let mut meta: ImageCacheMetadata =
            serde_json::from_slice(&fs::read(&mpath).unwrap()).unwrap();
        meta.created_at = 0; // epoch — very old
        fs::write(&mpath, serde_json::to_string(&meta).unwrap()).unwrap();

        let removed = cleanup_image_cache(dir.path(), 100_000_000, 30).unwrap();
        assert_eq!(removed, 1);
        assert!(!img_path(dir.path(), &key).exists());
    }

    #[test]
    fn test_cleanup_lru_eviction() {
        let dir = make_cache_dir();

        // Put two entries, make the first one older-accessed
        let url1 = "https://example.com/a.png";
        let url2 = "https://example.com/b.png";
        let data = vec![0u8; 600];

        cache_put(dir.path(), url1, &data, 100_000_000, 30).unwrap();
        cache_put(dir.path(), url2, &data, 100_000_000, 30).unwrap();

        // Make url1 have an older last_accessed_at
        let key1 = cache_key(url1);
        let mpath1 = meta_path(dir.path(), &key1);
        let mut m1: ImageCacheMetadata =
            serde_json::from_slice(&fs::read(&mpath1).unwrap()).unwrap();
        m1.last_accessed_at = 1;
        fs::write(&mpath1, serde_json::to_string(&m1).unwrap()).unwrap();

        // Cleanup with max_bytes = 700 (only room for one ~600-byte entry)
        let removed = cleanup_image_cache(dir.path(), 700, 365_000).unwrap();
        assert_eq!(removed, 1);

        // url1 (older) should be gone, url2 should remain
        assert!(!img_path(dir.path(), &key1).exists());
        assert!(img_path(dir.path(), &cache_key(url2)).exists());
    }

    #[test]
    fn test_cleanup_orphan_tmp_removal() {
        let dir = make_cache_dir();
        fs::write(dir.path().join("abc123.img.tmp"), b"stale").unwrap();

        let removed = cleanup_image_cache(dir.path(), 100_000_000, 30).unwrap();
        assert_eq!(removed, 0);
        assert!(!dir.path().join("abc123.img.tmp").exists());
    }

    #[test]
    fn test_cleanup_corrupt_sidecar() {
        let dir = make_cache_dir();
        let key = "deadbeef";
        fs::write(img_path(dir.path(), key), b"img").unwrap();
        fs::write(meta_path(dir.path(), key), b"not json").unwrap();

        let removed = cleanup_image_cache(dir.path(), 100_000_000, 30).unwrap();
        assert_eq!(removed, 1);
        assert!(!img_path(dir.path(), key).exists());
    }

    #[test]
    fn test_clear_image_cache() {
        let dir = make_cache_dir();
        let sub = dir.path().join("image_cache");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("test.img"), b"data").unwrap();

        clear_image_cache(&sub).unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn test_cleanup_empty_dir() {
        let dir = make_cache_dir();
        let nonexistent = dir.path().join("does_not_exist");
        let removed = cleanup_image_cache(&nonexistent, 100_000_000, 30).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_cache_overwrite() {
        let dir = make_cache_dir();
        let url = "https://example.com/overwrite.png";

        cache_put(dir.path(), url, b"version1", 100_000_000, 30).unwrap();
        cache_put(dir.path(), url, b"version2", 100_000_000, 30).unwrap();

        let got = cache_get(dir.path(), url).unwrap();
        assert_eq!(got, b"version2");
    }
}
