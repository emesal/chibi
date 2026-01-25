//! Safe file I/O utilities: atomic writes and file locking.
//!
//! This module provides building blocks for safe concurrent file access:
//! - [`atomic_write_json()`] - Write JSON atomically (temp file + rename)
//! - [`atomic_write()`] - Write bytes atomically
//! - [`FileLock`] - RAII file locking wrapper using fs2
//!
//! # Design Rationale
//!
//! These utilities address common file I/O hazards:
//!
//! **Atomic Writes**: Prevent data corruption from crashes during write operations.
//! By writing to a temporary file and renaming, the target file is either fully
//! updated or unchanged - never partially written.
//!
//! **File Locking**: Prevent race conditions when multiple processes access the
//! same files. Uses fs2's advisory locking which works across platforms.
//!
//! # Example
//!
//! ```ignore
//! use safe_io::{atomic_write_json, FileLock};
//!
//! // Atomic JSON write
//! let data = serde_json::json!({"key": "value"});
//! atomic_write_json(&path, &data)?;
//!
//! // File locking with RAII
//! {
//!     let _lock = FileLock::acquire(&lock_path)?;
//!     // ... operations protected by lock ...
//! } // lock released automatically
//! ```

use fs2::FileExt;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Atomically write JSON to a file.
///
/// Writes to a temporary file (`.tmp` suffix) with fsync, then renames to the
/// target path. This ensures the file is either fully written or unchanged -
/// never partially written due to crashes or power loss.
///
/// # Arguments
///
/// * `path` - The target file path
/// * `value` - The value to serialize as pretty-printed JSON
///
/// # Errors
///
/// Returns an error if:
/// - Serialization fails
/// - The temporary file cannot be created or written
/// - The rename operation fails (e.g., cross-device rename)
///
/// # Platform Notes
///
/// On Unix, rename is atomic within the same filesystem.
/// On Windows, rename may not be atomic but still provides crash safety.
pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write(path, &json)
}

/// Atomically write bytes to a file.
///
/// Writes to a temporary file (`.tmp` suffix) with fsync, then renames to the
/// target path. This ensures the file is either fully written or unchanged.
///
/// # Arguments
///
/// * `path` - The target file path
/// * `contents` - The bytes to write
///
/// # Errors
///
/// Returns an error if the temporary file cannot be created, written, synced,
/// or renamed.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write to temporary file with .tmp suffix
    let tmp_path = path.with_extension("tmp");

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)?;

    {
        let mut writer = BufWriter::new(&mut file);
        writer.write_all(contents)?;
        writer.flush()?;
    }

    // Sync to disk before rename
    file.sync_all()?;

    // Atomic rename (on same filesystem)
    fs::rename(&tmp_path, path)?;

    Ok(())
}

/// RAII file lock using fs2 exclusive locking.
///
/// The lock is acquired when created and automatically released when dropped.
/// Uses advisory locking - processes must cooperate by acquiring locks on the
/// same lock file path.
///
/// # Example
///
/// ```ignore
/// let _lock = FileLock::acquire(&Path::new("/tmp/myapp.lock"))?;
/// // ... protected operations ...
/// // lock automatically released when _lock goes out of scope
/// ```
///
/// # Blocking Behavior
///
/// `acquire()` blocks until the lock can be obtained. Use `try_acquire()` for
/// non-blocking lock attempts.
pub struct FileLock {
    file: File,
}

impl FileLock {
    /// Acquire an exclusive lock on the given path, blocking if necessary.
    ///
    /// Creates the lock file if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `lock_path` - Path to the lock file (usually a dedicated `.lock` file)
    ///
    /// # Errors
    ///
    /// Returns an error if the lock file cannot be created or the lock cannot
    /// be acquired.
    pub fn acquire(lock_path: &Path) -> io::Result<Self> {
        // Create parent directories if needed
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;

        // Block until we can acquire exclusive lock
        file.lock_exclusive()?;

        Ok(Self { file })
    }

    /// Try to acquire an exclusive lock without blocking.
    ///
    /// Returns `Ok(Some(FileLock))` if the lock was acquired, `Ok(None)` if
    /// the lock is held by another process, or `Err` on I/O error.
    #[allow(dead_code)] // Public API for future use
    pub fn try_acquire(lock_path: &Path) -> io::Result<Option<Self>> {
        // Create parent directories if needed
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            // fs2 returns a raw OS error that may not map to WouldBlock
            Err(e) if e.raw_os_error().is_some() => {
                // Check for common "resource temporarily unavailable" errors
                // EAGAIN (11 on Linux), EWOULDBLOCK (same as EAGAIN on Linux)
                // EACCES (33) on some systems for mandatory locks
                let os_err = e.raw_os_error().unwrap();
                if os_err == 11 || os_err == 35 {
                    // 35 is EAGAIN on macOS
                    Ok(None)
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Release the lock using fs2's FileExt trait; ignore errors during drop
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_basic() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        atomic_write(&path, b"hello world").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "hello world");
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nested").join("dir").join("test.txt");

        atomic_write(&path, b"nested content").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "nested content");
    }

    #[test]
    fn test_atomic_write_json_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("data.json");

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct TestData {
            name: String,
            value: i32,
        }

        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        atomic_write_json(&path, &data).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let parsed: TestData = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed, data);
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        atomic_write(&path, b"original").unwrap();
        atomic_write(&path, b"updated").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "updated");
    }

    #[test]
    fn test_atomic_write_no_tmp_file_left() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");
        let tmp_path = path.with_extension("tmp");

        atomic_write(&path, b"content").unwrap();

        assert!(path.exists());
        assert!(!tmp_path.exists(), "temp file should be cleaned up");
    }

    #[test]
    fn test_file_lock_acquire_release() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        {
            let _lock = FileLock::acquire(&lock_path).unwrap();
            assert!(lock_path.exists());
        }
        // Lock should be released after scope ends
    }

    #[test]
    fn test_file_lock_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("nested").join("dir").join("test.lock");

        let _lock = FileLock::acquire(&lock_path).unwrap();
        assert!(lock_path.exists());
    }

    #[test]
    fn test_file_lock_try_acquire_when_unlocked() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(result.is_some(), "should acquire lock when unlocked");
    }

    #[test]
    fn test_file_lock_try_acquire_when_locked() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        let _lock1 = FileLock::acquire(&lock_path).unwrap();
        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(
            result.is_none(),
            "should fail to acquire when already locked"
        );
    }

    #[test]
    fn test_file_lock_reentrant_different_handles() {
        // This tests that the same thread can't re-acquire the lock with a different handle
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        let _lock1 = FileLock::acquire(&lock_path).unwrap();
        // try_acquire should return None since lock is held
        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_file_lock_released_after_drop() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        {
            let _lock = FileLock::acquire(&lock_path).unwrap();
        }
        // Lock should be released, so we can acquire again
        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(
            result.is_some(),
            "should acquire after previous lock dropped"
        );
    }

    #[test]
    fn test_file_lock_blocks_across_threads() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");
        let lock_path_clone = lock_path.clone();

        let lock_held = Arc::new(AtomicBool::new(false));
        let lock_held_clone = Arc::clone(&lock_held);

        // Spawn thread that holds the lock
        let handle = thread::spawn(move || {
            let _lock = FileLock::acquire(&lock_path_clone).unwrap();
            lock_held_clone.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
            // Lock released when _lock goes out of scope
        });

        // Wait for thread to acquire lock
        while !lock_held.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(10));
        }

        // Try to acquire should fail while other thread holds lock
        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(
            result.is_none(),
            "should not acquire while other thread holds lock"
        );

        // Wait for thread to finish and release lock
        handle.join().unwrap();

        // Now we should be able to acquire
        let result = FileLock::try_acquire(&lock_path).unwrap();
        assert!(
            result.is_some(),
            "should acquire after other thread releases"
        );
    }
}
