use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Manages a lockfile for a context directory.
/// The lock is acquired on creation and released on drop.
/// A heartbeat thread keeps the lock fresh by updating the timestamp.
pub struct ContextLock {
    lock_path: PathBuf,
    stop_signal: Arc<(Mutex<bool>, Condvar)>,
    heartbeat_handle: Option<thread::JoinHandle<()>>,
}

impl ContextLock {
    /// Acquire a lock for the given context directory.
    /// If a stale lock exists, it will be cleaned up.
    /// Spawns a background thread to keep the lock fresh.
    pub fn acquire(context_dir: &Path, heartbeat_secs: u64) -> io::Result<Self> {
        let lock_path = context_dir.join(".lock");

        // Check if lock exists and is stale
        if lock_path.exists() {
            if Self::is_stale(&lock_path, heartbeat_secs) {
                // Clean up stale lock
                fs::remove_file(&lock_path)?;
            } else {
                return Err(io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!(
                        "Context is locked by another process. Lock file: {}",
                        lock_path.display()
                    ),
                ));
            }
        }

        // Create the lock file with current timestamp
        Self::touch(&lock_path)?;

        // Start heartbeat thread with condvar for clean shutdown
        let stop_signal = Arc::new((Mutex::new(false), Condvar::new()));
        let stop_signal_clone = Arc::clone(&stop_signal);
        let lock_path_clone = lock_path.clone();
        let heartbeat_interval = Duration::from_secs(heartbeat_secs);

        let heartbeat_handle = thread::spawn(move || {
            let (lock, cvar) = &*stop_signal_clone;
            loop {
                let guard = lock.lock().unwrap();
                // Wait for heartbeat interval or until signaled to stop
                let result = cvar.wait_timeout(guard, heartbeat_interval).unwrap();
                if *result.0 {
                    // Signaled to stop
                    break;
                }
                // Timeout expired, update the lock file
                if Self::touch(&lock_path_clone).is_err() {
                    break;
                }
            }
        });

        Ok(ContextLock {
            lock_path,
            stop_signal,
            heartbeat_handle: Some(heartbeat_handle),
        })
    }

    /// Write current Unix timestamp to the lock file (atomic)
    fn touch(path: &Path) -> io::Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        crate::safe_io::atomic_write_text(path, &timestamp.to_string())
    }

    /// Check if a lock file is stale (older than 1.5x heartbeat interval)
    pub fn is_stale(lock_path: &Path, heartbeat_secs: u64) -> bool {
        if !lock_path.exists() {
            return true;
        }

        // Read the timestamp from the lock file
        let content = match fs::read_to_string(lock_path) {
            Ok(c) => c,
            Err(_) => return true, // Can't read = treat as stale
        };

        let lock_timestamp: u64 = match content.trim().parse() {
            Ok(t) => t,
            Err(_) => return true, // Invalid content = treat as stale
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        // Stale if older than 1.5x heartbeat interval
        let stale_threshold = (heartbeat_secs as f64 * 1.5) as u64;
        now.saturating_sub(lock_timestamp) > stale_threshold
    }

    /// Get display status for a context: Some("[active]"), Some("[stale]"), or None
    pub fn get_status(context_dir: &Path, heartbeat_secs: u64) -> Option<&'static str> {
        let lock_path = context_dir.join(".lock");
        if !lock_path.exists() {
            return None;
        }

        if Self::is_stale(&lock_path, heartbeat_secs) {
            Some("[stale]")
        } else {
            Some("[active]")
        }
    }
}

impl Drop for ContextLock {
    fn drop(&mut self) {
        // Signal heartbeat thread to stop and wake it immediately
        let (lock, cvar) = &*self.stop_signal;
        {
            let mut stop = lock.lock().unwrap();
            *stop = true;
        }
        cvar.notify_one();

        // Wait for heartbeat thread to finish
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }

        // Remove the lock file
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_acquire_lock_creates_file() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        assert!(!lock_path.exists());

        let lock = ContextLock::acquire(temp_dir.path(), 30).unwrap();

        assert!(lock_path.exists());

        // Lock file should contain a timestamp
        let content = fs::read_to_string(&lock_path).unwrap();
        let timestamp: u64 = content.trim().parse().unwrap();
        assert!(timestamp > 1704067200); // After Jan 1, 2024

        drop(lock);
    }

    #[test]
    fn test_lock_released_on_drop() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        {
            let _lock = ContextLock::acquire(temp_dir.path(), 30).unwrap();
            assert!(lock_path.exists());
        }

        // Lock file should be removed after drop
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_cannot_acquire_active_lock() {
        let temp_dir = create_test_dir();

        let _lock1 = ContextLock::acquire(temp_dir.path(), 30).unwrap();

        // Try to acquire another lock - should fail
        let result = ContextLock::acquire(temp_dir.path(), 30);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("locked by another process"));
    }

    #[test]
    fn test_is_stale_nonexistent_file() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Nonexistent lock should be considered stale
        assert!(ContextLock::is_stale(&lock_path, 30));
    }

    #[test]
    fn test_is_stale_fresh_lock() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a fresh lock file
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(&lock_path, now.to_string()).unwrap();

        // Should not be stale
        assert!(!ContextLock::is_stale(&lock_path, 30));
    }

    #[test]
    fn test_is_stale_old_lock() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a lock file with old timestamp (60 seconds ago)
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 60;
        fs::write(&lock_path, old.to_string()).unwrap();

        // Should be stale with 30-second heartbeat (1.5x = 45 seconds)
        assert!(ContextLock::is_stale(&lock_path, 30));
    }

    #[test]
    fn test_is_stale_invalid_content() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a lock file with invalid content
        fs::write(&lock_path, "not-a-number").unwrap();

        // Invalid content should be treated as stale
        assert!(ContextLock::is_stale(&lock_path, 30));
    }

    #[test]
    fn test_stale_lock_is_cleaned_up() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a stale lock file (60 seconds old)
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 60;
        fs::write(&lock_path, old.to_string()).unwrap();

        // Should be able to acquire lock (stale lock cleaned up)
        let lock = ContextLock::acquire(temp_dir.path(), 30).unwrap();

        // New timestamp should be fresh
        let content = fs::read_to_string(&lock_path).unwrap();
        let timestamp: u64 = content.trim().parse().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(now - timestamp < 5); // Within 5 seconds of now

        drop(lock);
    }

    #[test]
    fn test_get_status_no_lock() {
        let temp_dir = create_test_dir();

        let status = ContextLock::get_status(temp_dir.path(), 30);
        assert!(status.is_none());
    }

    #[test]
    fn test_get_status_active_lock() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a fresh lock
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(&lock_path, now.to_string()).unwrap();

        let status = ContextLock::get_status(temp_dir.path(), 30);
        assert_eq!(status, Some("[active]"));
    }

    #[test]
    fn test_get_status_stale_lock() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        // Create a stale lock (60 seconds old)
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 60;
        fs::write(&lock_path, old.to_string()).unwrap();

        let status = ContextLock::get_status(temp_dir.path(), 30);
        assert_eq!(status, Some("[stale]"));
    }

    #[test]
    fn test_stale_threshold_is_1_5x_heartbeat() {
        let temp_dir = create_test_dir();
        let lock_path = temp_dir.path().join(".lock");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // With 30-second heartbeat, stale threshold is 45 seconds

        // 40 seconds old should NOT be stale
        fs::write(&lock_path, (now - 40).to_string()).unwrap();
        assert!(!ContextLock::is_stale(&lock_path, 30));

        // 50 seconds old should be stale
        fs::write(&lock_path, (now - 50).to_string()).unwrap();
        assert!(ContextLock::is_stale(&lock_path, 30));
    }
}
