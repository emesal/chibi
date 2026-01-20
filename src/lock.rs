use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Manages a lockfile for a context directory.
/// The lock is acquired on creation and released on drop.
/// A heartbeat thread keeps the lock fresh by updating the timestamp.
pub struct ContextLock {
    lock_path: PathBuf,
    stop_heartbeat: Arc<AtomicBool>,
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

        // Start heartbeat thread
        let stop_heartbeat = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_heartbeat);
        let lock_path_clone = lock_path.clone();
        let heartbeat_interval = Duration::from_secs(heartbeat_secs);

        let heartbeat_handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                thread::sleep(heartbeat_interval);
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                // Update the lock file timestamp
                if let Err(_) = Self::touch(&lock_path_clone) {
                    // If we can't touch the lock file, stop the heartbeat
                    break;
                }
            }
        });

        Ok(ContextLock {
            lock_path,
            stop_heartbeat,
            heartbeat_handle: Some(heartbeat_handle),
        })
    }

    /// Write current Unix timestamp to the lock file
    fn touch(path: &Path) -> io::Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(path, timestamp.to_string())
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
            .unwrap()
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
        // Signal heartbeat thread to stop
        self.stop_heartbeat.store(true, Ordering::Relaxed);

        // Wait for heartbeat thread to finish
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }

        // Remove the lock file
        let _ = fs::remove_file(&self.lock_path);
    }
}
