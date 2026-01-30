//! # Partitioned Context Storage
//!
//! This module implements time-partitioned JSONL storage for context entries,
//! replacing the single-file `context.jsonl` approach with a more scalable design.
//!
//! ## Design Rationale
//!
//! Large conversation contexts can accumulate thousands of entries over time.
//! A single JSONL file becomes problematic:
//! - Reading the entire file on every operation is slow
//! - Appending is fast, but time-range queries require full scans
//! - No way to efficiently check if an entry exists without reading everything
//!
//! ## Solution: Time-Partitioned Storage
//!
//! Entries are split across multiple partition files:
//!
//! ```text
//! contexts/<name>/
//! ├── manifest.json          # Tracks all partitions and their metadata
//! ├── active.jsonl           # Current write partition (append-only)
//! └── partitions/
//!     ├── <start>-<end>.jsonl  # Archived read-only partitions
//!     └── <start>-<end>.bloom  # Bloom filter for fast ID lookups
//! ```
//!
//! ## Rotation Policy
//!
//! The active partition rotates when any threshold is reached:
//! - **Entry count**: Default 1000 entries per partition
//! - **Token count**: Default 100,000 estimated LLM tokens (configurable bytes/token, default 3)
//! - **Age**: Default 30 days since first entry
//!
//! On rotation, the active partition is moved to `partitions/` and a bloom
//! filter is built for term-based search lookups.
//!
//! ## Legacy Migration
//!
//! Existing `context.jsonl` files are transparently adopted as the active
//! partition without data movement. Migration only occurs on first rotation.
//!
//! ## Bloom Filters
//!
//! Each archived partition has an optional bloom filter (`.bloom` file)
//! containing tokenized content terms for search optimization. When searching,
//! partitions whose bloom filter indicates no matching terms are skipped.
//!
//! Filter parameters are automatically optimized by fastbloom for the
//! expected vocabulary size with <1% false positive rate.
//!
//! See: <https://github.com/tomtomwombat/fastbloom>

use crate::context::TranscriptEntry;
use crate::jsonl::read_jsonl_file;
use crate::safe_io::{FileLock, atomic_write_json};
use fastbloom::BloomFilter;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

// ============================================================================
// Constants
// ============================================================================

/// Default maximum entries per partition before rotation.
pub const DEFAULT_PARTITION_MAX_ENTRIES: usize = 1000;

/// Default maximum age of a partition in seconds (30 days).
pub const DEFAULT_PARTITION_MAX_AGE_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Default maximum tokens per partition before rotation (100k tokens).
pub const DEFAULT_PARTITION_MAX_TOKENS: usize = 100_000;

/// Default bytes per token for estimation (conservative for mixed content).
pub const DEFAULT_BYTES_PER_TOKEN: usize = 3;

/// Target false positive rate for bloom filters.
const BLOOM_FALSE_POSITIVE_RATE: f64 = 0.01;

/// Estimates token count from text using configurable bytes-per-token heuristic.
///
/// Uses byte length (not character count) for O(1) performance.
/// A lower bytes_per_token value produces higher token estimates (more conservative).
/// Default of 3 bytes/token handles mixed English/CJK content safely.
#[inline]
fn estimate_tokens(text: &str, bytes_per_token: usize) -> usize {
    let divisor = bytes_per_token.max(1); // Prevent division by zero
    text.len().div_ceil(divisor)
}

// ============================================================================
// Configuration Types
// ============================================================================

/// Storage configuration for partitioned context storage.
///
/// All fields are optional; defaults are applied when loading a `PartitionManager`.
/// This allows partial overrides in both global `config.toml` and per-context `local.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct StorageConfig {
    /// Maximum entries per partition before rotation.
    /// When the active partition reaches this count, it rotates to an archived partition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_max_entries: Option<usize>,

    /// Maximum partition age in seconds before rotation.
    /// Rotation triggers if the first entry in active partition is older than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_max_age_seconds: Option<u64>,

    /// Maximum estimated tokens per partition before rotation.
    /// Default 100k tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_max_tokens: Option<usize>,

    /// Bytes per token for estimation heuristic.
    /// Lower values = more conservative (higher token estimates).
    /// Default is 3 (handles mixed English/CJK content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_per_token: Option<usize>,

    /// Whether to build bloom filter indexes for archived partitions.
    /// Bloom filters enable fast term-based search across partitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_bloom_filters: Option<bool>,
}

/// Minimum partition age in seconds (1 minute).
/// Prevents excessively frequent rotation from misconfiguration.
const MIN_PARTITION_AGE_SECONDS: u64 = 60;

impl StorageConfig {
    /// Returns the effective max entries threshold.
    /// Enforces minimum of 1 to prevent infinite loops from zero values.
    #[inline]
    pub fn max_entries(&self) -> usize {
        self.partition_max_entries
            .unwrap_or(DEFAULT_PARTITION_MAX_ENTRIES)
            .max(1)
    }

    /// Returns the effective max age threshold in seconds.
    /// Enforces minimum of 60 seconds to prevent excessively frequent rotation.
    #[inline]
    pub fn max_age_seconds(&self) -> u64 {
        self.partition_max_age_seconds
            .unwrap_or(DEFAULT_PARTITION_MAX_AGE_SECONDS)
            .max(MIN_PARTITION_AGE_SECONDS)
    }

    /// Returns the effective max tokens threshold.
    /// Enforces minimum of 1 to prevent infinite loops from zero values.
    #[inline]
    pub fn max_tokens(&self) -> usize {
        self.partition_max_tokens
            .unwrap_or(DEFAULT_PARTITION_MAX_TOKENS)
            .max(1)
    }

    /// Returns the bytes-per-token value for estimation.
    /// Enforces minimum of 1 to prevent divide-by-zero errors.
    #[inline]
    pub fn bytes_per_token(&self) -> usize {
        self.bytes_per_token
            .unwrap_or(DEFAULT_BYTES_PER_TOKEN)
            .max(1)
    }

    /// Returns whether bloom filters should be built.
    #[inline]
    pub fn bloom_filters_enabled(&self) -> bool {
        self.enable_bloom_filters.unwrap_or(true)
    }
}

// ============================================================================
// Manifest Types
// ============================================================================

/// Manifest tracking all partitions for a context.
///
/// Stored as `manifest.json` in the context directory. Contains metadata
/// about archived partitions and the current rotation policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version for forward compatibility.
    pub version: u32,

    /// Filename of the current active (write) partition, relative to context dir.
    pub active_partition: String,

    /// Archived read-only partitions, ordered oldest to newest.
    pub partitions: Vec<PartitionMeta>,

    /// Current rotation policy settings.
    pub rotation_policy: RotationPolicy,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            active_partition: "active.jsonl".to_string(),
            partitions: Vec::new(),
            rotation_policy: RotationPolicy::default(),
        }
    }
}

/// Metadata for a single archived partition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionMeta {
    /// Path to the partition file, relative to context directory.
    pub file: String,

    /// Unix timestamp of the first entry (inclusive).
    pub start_ts: u64,

    /// Unix timestamp of the last entry (inclusive).
    pub end_ts: u64,

    /// Number of entries in this partition.
    pub entry_count: usize,

    /// Estimated token count in this partition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count: Option<usize>,

    /// Optional bloom filter file path, relative to context directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bloom_file: Option<String>,
}

/// Policy controlling when the active partition rotates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    /// Rotate when entry count reaches this threshold.
    pub max_entries: usize,

    /// Rotate when the partition age exceeds this (seconds since first entry).
    pub max_age_seconds: u64,

    /// Rotate when estimated token count reaches this threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_PARTITION_MAX_ENTRIES,
            max_age_seconds: DEFAULT_PARTITION_MAX_AGE_SECONDS,
            max_tokens: Some(DEFAULT_PARTITION_MAX_TOKENS),
        }
    }
}

// ============================================================================
// Active Partition State
// ============================================================================

/// Cached state about the active partition, avoiding repeated file scans.
///
/// This struct tracks entry counts, token estimates, and timestamps for the
/// active partition without requiring a full file scan on every operation.
///
/// # Caching Strategy
///
/// The state is updated incrementally:
/// - [`record_append()`] updates after writing an entry
/// - [`reset()`] clears after rotation
///
/// For cross-session caching (e.g., in `AppState`), the state can be cloned
/// and restored via [`PartitionManager::load_with_cached_state()`].
#[derive(Debug, Default, Clone)]
pub struct ActiveState {
    /// Number of entries in active partition.
    entry_count: usize,

    /// Estimated token count in active partition.
    token_count: usize,

    /// Timestamp of first entry, if any.
    first_entry_ts: Option<u64>,
}

impl ActiveState {
    /// Scans a JSONL file to extract count, tokens, and first timestamp.
    fn from_file(path: &Path, bytes_per_token: usize) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut count = 0;
        let mut tokens = 0;
        let mut first_ts = None;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<TranscriptEntry>(&line) {
                if first_ts.is_none() {
                    first_ts = Some(entry.timestamp);
                }
                count += 1;
                tokens += estimate_tokens(&entry.content, bytes_per_token);
            }
        }

        Ok(Self {
            entry_count: count,
            token_count: tokens,
            first_entry_ts: first_ts,
        })
    }

    /// Updates state after appending an entry.
    fn record_append(&mut self, entry: &TranscriptEntry, bytes_per_token: usize) {
        self.entry_count += 1;
        self.token_count = self
            .token_count
            .saturating_add(estimate_tokens(&entry.content, bytes_per_token));
        if self.first_entry_ts.is_none() {
            self.first_entry_ts = Some(entry.timestamp);
        }
    }

    /// Resets state after rotation.
    fn reset(&mut self) {
        self.entry_count = 0;
        self.token_count = 0;
        self.first_entry_ts = None;
    }

    /// Returns the number of entries tracked in this state.
    #[cfg(test)]
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }
}

// ============================================================================
// Partition Manager
// ============================================================================

/// Manages partitioned storage for a single context.
///
/// The `PartitionManager` handles:
/// - Reading entries from archived and active partitions
/// - Appending entries to the active partition
/// - Rotating partitions when thresholds are exceeded
/// - Building bloom filters for archived partitions
///
/// # Example
///
/// ```no_run
/// // Requires a context directory with transcript/ subdirectory.
/// use chibi_core::partition::{PartitionManager, StorageConfig};
/// use std::path::Path;
///
/// # fn example() -> std::io::Result<()> {
/// let config = StorageConfig::default();
/// let mut pm = PartitionManager::load_with_config(Path::new("/path/to/context"), config)?;
/// // pm.append_entry(&entry)?;
/// // pm.rotate_if_needed()?;
/// let entries = pm.read_all_entries()?;
/// # Ok(())
/// # }
/// ```
pub struct PartitionManager {
    /// Path to the context directory.
    context_dir: PathBuf,

    /// Current manifest state.
    manifest: Manifest,

    /// Storage configuration.
    config: StorageConfig,

    /// Cached state about the active partition.
    active: ActiveState,
}

impl PartitionManager {
    /// Returns the path to the lock file for this partition manager.
    ///
    /// All write operations (append, rotate) acquire this lock to prevent
    /// concurrent modifications from corrupting the partition state.
    fn lock_path(&self) -> PathBuf {
        self.context_dir.join(".transcript.lock")
    }

    /// Loads with custom storage configuration.
    ///
    /// # Migration Behavior
    ///
    /// - If `manifest.json` exists, loads it and applies config overrides
    /// - If only `context.jsonl` exists (legacy), creates manifest using it as active partition
    /// - Otherwise, creates fresh manifest with config-specified thresholds
    pub fn load_with_config(context_dir: &Path, config: StorageConfig) -> io::Result<Self> {
        let manifest_path = context_dir.join("manifest.json");
        let legacy_path = context_dir.join("context.jsonl");

        let manifest = if manifest_path.exists() {
            Self::load_manifest(&manifest_path)?
        } else if legacy_path.exists() {
            // Legacy migration: use context.jsonl as active partition
            Manifest {
                version: 1,
                active_partition: "context.jsonl".to_string(),
                partitions: Vec::new(),
                rotation_policy: RotationPolicy {
                    max_entries: config.max_entries(),
                    max_age_seconds: config.max_age_seconds(),
                    max_tokens: Some(config.max_tokens()),
                },
            }
        } else {
            // Fresh context
            Manifest {
                rotation_policy: RotationPolicy {
                    max_entries: config.max_entries(),
                    max_age_seconds: config.max_age_seconds(),
                    max_tokens: Some(config.max_tokens()),
                },
                ..Manifest::default()
            }
        };

        // Scan active partition for count and first timestamp
        let active_path = context_dir.join(&manifest.active_partition);
        let active = ActiveState::from_file(&active_path, config.bytes_per_token())?;

        Ok(Self {
            context_dir: context_dir.to_path_buf(),
            manifest,
            config,
            active,
        })
    }

    /// Loads with custom storage configuration and optional cached active state.
    ///
    /// If `cached_state` is provided and valid (matches current active partition),
    /// it is used instead of scanning the file. This avoids repeated file scans
    /// when making multiple operations on the same context within a session.
    ///
    /// # Validation
    ///
    /// The cached state is validated by checking that the active partition file
    /// exists. If the file doesn't exist but cached state claims entries exist,
    /// the cache is invalidated and the file is re-scanned.
    pub fn load_with_cached_state(
        context_dir: &Path,
        config: StorageConfig,
        cached_state: Option<ActiveState>,
    ) -> io::Result<Self> {
        let manifest_path = context_dir.join("manifest.json");
        let legacy_path = context_dir.join("context.jsonl");

        let manifest = if manifest_path.exists() {
            Self::load_manifest(&manifest_path)?
        } else if legacy_path.exists() {
            Manifest {
                version: 1,
                active_partition: "context.jsonl".to_string(),
                partitions: Vec::new(),
                rotation_policy: RotationPolicy {
                    max_entries: config.max_entries(),
                    max_age_seconds: config.max_age_seconds(),
                    max_tokens: Some(config.max_tokens()),
                },
            }
        } else {
            Manifest {
                rotation_policy: RotationPolicy {
                    max_entries: config.max_entries(),
                    max_age_seconds: config.max_age_seconds(),
                    max_tokens: Some(config.max_tokens()),
                },
                ..Manifest::default()
            }
        };

        let active_path = context_dir.join(&manifest.active_partition);

        // Validate and use cached state if provided
        let active = if let Some(cached) = cached_state {
            // Validate: if file doesn't exist but cache has entries, re-scan
            if !active_path.exists() && cached.entry_count > 0 {
                ActiveState::default()
            } else {
                cached
            }
        } else {
            ActiveState::from_file(&active_path, config.bytes_per_token())?
        };

        Ok(Self {
            context_dir: context_dir.to_path_buf(),
            manifest,
            config,
            active,
        })
    }

    /// Returns a clone of the current active partition state for caching.
    ///
    /// Use this to capture state after operations and restore it later via
    /// [`load_with_cached_state()`].
    pub fn active_state(&self) -> ActiveState {
        self.active.clone()
    }

    /// Loads manifest from disk with error context.
    fn load_manifest(path: &Path) -> io::Result<Manifest> {
        let file = File::open(path)?;
        serde_json::from_reader(BufReader::new(file)).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {}", e))
        })
    }

    /// Persists the manifest to disk atomically.
    ///
    /// Uses atomic write (temp file + rename) to prevent corruption from
    /// crashes during the write operation.
    fn save_manifest(&self) -> io::Result<()> {
        let path = self.context_dir.join("manifest.json");
        atomic_write_json(&path, &self.manifest)
    }

    // ========================================================================
    // Read Operations
    // ========================================================================

    /// Reads all entries from all partitions (archived + active).
    ///
    /// Entries are returned in chronological order: archived partitions first
    /// (oldest to newest), then the active partition.
    pub fn read_all_entries(&self) -> io::Result<Vec<TranscriptEntry>> {
        let mut entries = Vec::with_capacity(self.total_entry_count());

        // Archived partitions (already ordered oldest to newest)
        for partition in &self.manifest.partitions {
            let path = self.context_dir.join(&partition.file);
            if path.exists() {
                entries.extend(read_jsonl_file::<TranscriptEntry>(&path)?);
            }
        }

        // Active partition
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            entries.extend(read_jsonl_file::<TranscriptEntry>(&active_path)?);
        }

        Ok(entries)
    }

    // ========================================================================
    // Write Operations
    // ========================================================================

    /// Appends a single entry to the active partition.
    ///
    /// This is an append-only operation; the partition file is opened in
    /// append mode to minimize I/O and avoid rewriting existing content.
    ///
    /// # Atomicity
    ///
    /// Each entry is written as a complete JSON line. Rotation thresholds are
    /// checked *after* appending, ensuring entries are never split across
    /// partitions. Call `rotate_if_needed()` after appending to trigger
    /// rotation if thresholds are exceeded.
    ///
    /// # Concurrency
    ///
    /// Acquires an exclusive file lock to prevent race conditions with
    /// concurrent writers. The lock is released when the method returns.
    ///
    /// # Durability
    ///
    /// Writes are flushed to disk via fsync to ensure durability. This prevents
    /// data loss on crash but may impact write performance on high-throughput workloads.
    pub fn append_entry(&mut self, entry: &TranscriptEntry) -> io::Result<()> {
        // Acquire lock for write operation
        let _lock = FileLock::acquire(&self.lock_path())?;

        fs::create_dir_all(&self.context_dir)?;

        let active_path = self.context_dir.join(&self.manifest.active_partition);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
        writeln!(file, "{}", json)?;

        // Ensure durability: flush OS buffer cache to disk
        file.sync_all()?;

        self.active
            .record_append(entry, self.config.bytes_per_token());
        Ok(())
    }

    // ========================================================================
    // Rotation
    // ========================================================================

    /// Returns true if the active partition should be rotated.
    ///
    /// Rotation is triggered when any threshold is reached:
    /// - Entry count >= configured max_entries
    /// - Time since first entry >= configured max_age_seconds
    /// - Estimated token count >= configured max_tokens
    #[must_use]
    pub fn needs_rotation(&self) -> bool {
        let policy = &self.manifest.rotation_policy;

        // Entry count threshold
        if self.active.entry_count >= policy.max_entries {
            return true;
        }

        // Token count threshold
        if let Some(max_tokens) = policy.max_tokens
            && self.active.token_count >= max_tokens
        {
            return true;
        }

        // Age threshold
        if let Some(first_ts) = self.active.first_entry_ts {
            let age = crate::context::now_timestamp().saturating_sub(first_ts);
            if age >= policy.max_age_seconds {
                return true;
            }
        }

        false
    }

    /// Rotates the active partition if thresholds are exceeded.
    ///
    /// Returns `true` if rotation occurred, `false` otherwise.
    #[must_use = "check if rotation occurred to handle partition state changes"]
    pub fn rotate_if_needed(&mut self) -> io::Result<bool> {
        if !self.needs_rotation() {
            return Ok(false);
        }
        self.rotate()?;
        Ok(true)
    }

    /// Forces rotation of the active partition.
    ///
    /// # Process
    ///
    /// 1. Read entries to determine timestamp range
    /// 2. Move active partition to `partitions/<start>-<end>.jsonl`
    /// 3. Build bloom filter if enabled
    /// 4. Add partition metadata to manifest
    /// 5. Reset active partition to `active.jsonl`
    ///
    /// # Concurrency
    ///
    /// Acquires an exclusive file lock to prevent race conditions with
    /// concurrent writers. The lock is released when the method returns.
    pub fn rotate(&mut self) -> io::Result<()> {
        // Acquire lock for rotation operation
        let _lock = FileLock::acquire(&self.lock_path())?;

        let active_path = self.context_dir.join(&self.manifest.active_partition);

        // Nothing to rotate if empty
        if !active_path.exists() || self.active.entry_count == 0 {
            return Ok(());
        }

        // Read entries to get timestamp range
        let entries = read_jsonl_file::<TranscriptEntry>(&active_path)?;
        if entries.is_empty() {
            return Ok(());
        }

        let start_ts = entries.first().map(|e| e.timestamp).unwrap_or(0);
        let end_ts = entries.last().map(|e| e.timestamp).unwrap_or(0);

        // Create partitions directory
        let partitions_dir = self.context_dir.join("partitions");
        fs::create_dir_all(&partitions_dir)?;

        // Generate partition filename from timestamp range
        let partition_name = format!("{}-{}.jsonl", start_ts, end_ts);
        let partition_rel = format!("partitions/{}", partition_name);
        let partition_path = self.context_dir.join(&partition_rel);

        // Move active to partitions directory
        fs::rename(&active_path, &partition_path)?;

        // Build bloom filter if enabled
        let bloom_file = if self.config.bloom_filters_enabled() {
            match build_bloom_filter(&entries, &partition_path) {
                Ok(bloom_path) => Some(
                    bloom_path
                        .strip_prefix(&self.context_dir)
                        .unwrap_or(&bloom_path)
                        .to_string_lossy()
                        .into_owned(),
                ),
                Err(e) => {
                    eprintln!("[WARN] Bloom filter build failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Calculate token count for the partition
        let bytes_per_token = self.config.bytes_per_token();
        let token_count: usize = entries
            .iter()
            .map(|e| estimate_tokens(&e.content, bytes_per_token))
            .fold(0usize, |acc, n| acc.saturating_add(n));

        // Record partition metadata
        self.manifest.partitions.push(PartitionMeta {
            file: partition_rel,
            start_ts,
            end_ts,
            entry_count: entries.len(),
            token_count: Some(token_count),
            bloom_file,
        });

        // Reset to default active partition
        self.manifest.active_partition = "active.jsonl".to_string();
        self.active.reset();

        self.save_manifest()?;
        Ok(())
    }

    // ========================================================================
    // Search Operations
    // ========================================================================

    /// Searches for entries containing a term.
    ///
    /// Uses bloom filters to skip partitions that definitely don't contain
    /// the term. Returns matching entries and search statistics.
    ///
    /// The search is case-insensitive and matches substring in content.
    ///
    /// # Bloom Filter Behavior
    ///
    /// The bloom filter uses `any` semantics for multi-word queries: a partition
    /// is scanned if ANY query token might be present. This is correct for
    /// substring search because matching "foo bar" requires matching "foo bar"
    /// as a substring, not matching "foo" AND "bar" separately. The bloom filter
    /// acts as a quick pre-filter to skip partitions that definitely don't
    /// contain any of the query words.
    #[allow(dead_code)] // Public API, will be used when search CLI is added
    pub fn search(&self, query: &str) -> io::Result<SearchResult> {
        let query_lower = query.to_lowercase();
        let mut entries = Vec::new();
        let mut partitions_scanned = 0;
        let mut partitions_skipped = 0;

        // Search archived partitions
        for partition in &self.manifest.partitions {
            // Check bloom filter if available
            if let Some(ref bloom_file) = partition.bloom_file {
                let bloom_path = self.context_dir.join(bloom_file);
                if bloom_path.exists()
                    && let Ok(bloom) = load_bloom_filter(&bloom_path)
                {
                    // Tokenize query and check if ANY token is present
                    // The bloom filter contains individual words, not multi-word phrases
                    let query_tokens: Vec<String> = tokenize(&query_lower).collect();
                    let has_match = query_tokens.is_empty()
                        || query_tokens.iter().any(|token| bloom.contains(token));

                    if !has_match {
                        partitions_skipped += 1;
                        continue;
                    }
                }
            }

            // Scan the partition
            partitions_scanned += 1;
            let path = self.context_dir.join(&partition.file);
            if path.exists() {
                for entry in read_jsonl_file::<TranscriptEntry>(&path)? {
                    if entry.content.to_lowercase().contains(&query_lower) {
                        entries.push(entry);
                    }
                }
            }
        }

        // Always search active partition (no bloom filter yet)
        partitions_scanned += 1;
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            for entry in read_jsonl_file::<TranscriptEntry>(&active_path)? {
                if entry.content.to_lowercase().contains(&query_lower) {
                    entries.push(entry);
                }
            }
        }

        Ok(SearchResult {
            entries,
            partitions_scanned,
            partitions_skipped,
        })
    }

    /// Checks if an entry with the given ID exists in any partition.
    ///
    /// Scans all partitions to find the entry. Returns `true` if found.
    #[allow(dead_code)] // Public API, will be used when needed
    pub fn entry_might_exist(&self, id: &str) -> io::Result<bool> {
        // Check active partition first (linear scan, but typically small)
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            for entry in read_jsonl_file::<TranscriptEntry>(&active_path)? {
                if entry.id == id {
                    return Ok(true);
                }
            }
        }

        // Check archived partitions
        for partition in &self.manifest.partitions {
            let path = self.context_dir.join(&partition.file);
            if path.exists() {
                for entry in read_jsonl_file::<TranscriptEntry>(&path)? {
                    if entry.id == id {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    // ========================================================================
    // Accessors
    // ========================================================================

    /// Returns the total entry count across all partitions.
    fn total_entry_count(&self) -> usize {
        let archived: usize = self.manifest.partitions.iter().map(|p| p.entry_count).sum();
        archived + self.active.entry_count
    }
}

// Test-only implementations
#[cfg(test)]
impl PartitionManager {
    /// Loads with default configuration (test convenience).
    fn load(context_dir: &Path) -> io::Result<Self> {
        Self::load_with_config(context_dir, StorageConfig::default())
    }

    /// Returns the number of entries in the active partition.
    fn active_entry_count(&self) -> usize {
        self.active.entry_count
    }

    /// Reads entries within a timestamp range (inclusive).
    fn read_entries_in_range(&self, from_ts: u64, to_ts: u64) -> io::Result<Vec<TranscriptEntry>> {
        let mut entries = Vec::new();

        // Check archived partitions that overlap the range
        for partition in &self.manifest.partitions {
            if !partition.overlaps(from_ts, to_ts) {
                continue;
            }

            let path = self.context_dir.join(&partition.file);
            if path.exists() {
                for entry in read_jsonl_file::<TranscriptEntry>(&path)? {
                    if entry.timestamp >= from_ts && entry.timestamp <= to_ts {
                        entries.push(entry);
                    }
                }
            }
        }

        // Check active partition
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            for entry in read_jsonl_file::<TranscriptEntry>(&active_path)? {
                if entry.timestamp >= from_ts && entry.timestamp <= to_ts {
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
impl PartitionMeta {
    /// Returns true if this partition's time range overlaps with the given range.
    fn overlaps(&self, from_ts: u64, to_ts: u64) -> bool {
        self.start_ts <= to_ts && self.end_ts >= from_ts
    }
}

#[cfg(test)]
impl StorageConfig {
    /// Merges another config, preferring `other`'s values when present.
    fn merge(&self, other: &StorageConfig) -> StorageConfig {
        StorageConfig {
            partition_max_entries: other.partition_max_entries.or(self.partition_max_entries),
            partition_max_age_seconds: other
                .partition_max_age_seconds
                .or(self.partition_max_age_seconds),
            partition_max_tokens: other.partition_max_tokens.or(self.partition_max_tokens),
            bytes_per_token: other.bytes_per_token.or(self.bytes_per_token),
            enable_bloom_filters: other.enable_bloom_filters.or(self.enable_bloom_filters),
        }
    }
}

// ============================================================================
// Bloom Filter Implementation (using fastbloom by tomtomwombat)
// ============================================================================

/// Builds a bloom filter for search terms found in entries.
///
/// Extracts words from entry content and stores them in a bloom filter
/// for fast "does this partition contain term X" lookups.
fn build_bloom_filter(entries: &[TranscriptEntry], partition_path: &Path) -> io::Result<PathBuf> {
    // Collect terms into a Vec; fastbloom requires ExactSizeIterator
    let terms: Vec<String> = entries.iter().flat_map(|e| tokenize(&e.content)).collect();

    let bloom = BloomFilter::with_false_pos(BLOOM_FALSE_POSITIVE_RATE).items(terms.iter());

    let bloom_path = partition_path.with_extension("bloom");
    let serialized =
        serde_json::to_vec(&bloom).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::safe_io::atomic_write(&bloom_path, &serialized)?;
    Ok(bloom_path)
}

/// Loads a bloom filter from disk.
fn load_bloom_filter(path: &Path) -> io::Result<BloomFilter> {
    let data = fs::read(path)?;
    serde_json::from_slice(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Tokenizes text into lowercase words for search indexing.
fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
}

/// Result from a search query.
#[derive(Debug)]
#[allow(dead_code)] // Public API, will be used when search CLI is added
pub struct SearchResult {
    /// Matching entries.
    pub entries: Vec<TranscriptEntry>,
    /// Partitions that were actually scanned.
    pub partitions_scanned: usize,
    /// Partitions skipped due to bloom filter.
    pub partitions_skipped: usize,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::now_timestamp;
    use tempfile::TempDir;
    use uuid::Uuid;

    // Test helpers

    fn make_entry(content: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: "user".to_string(),
            to: "context".to_string(),
            content: content.to_string(),
            entry_type: "message".to_string(),
            metadata: None,
        }
    }

    fn make_entry_with_ts(content: &str, timestamp: u64) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp,
            from: "user".to_string(),
            to: "context".to_string(),
            content: content.to_string(),
            entry_type: "message".to_string(),
            metadata: None,
        }
    }

    // StorageConfig tests

    #[test]
    fn test_storage_config_defaults() {
        let config = StorageConfig::default();
        assert_eq!(config.max_entries(), DEFAULT_PARTITION_MAX_ENTRIES);
        assert_eq!(config.max_age_seconds(), DEFAULT_PARTITION_MAX_AGE_SECONDS);
        assert!(config.bloom_filters_enabled());
    }

    #[test]
    fn test_storage_config_minimum_guards() {
        // Test that zero values are clamped to minimums
        let config = StorageConfig {
            partition_max_entries: Some(0),
            partition_max_age_seconds: Some(0),
            partition_max_tokens: Some(0),
            bytes_per_token: Some(0),
            enable_bloom_filters: None,
        };

        // All should be clamped to at least 1 (or 60 for age)
        assert_eq!(config.max_entries(), 1);
        assert_eq!(config.max_age_seconds(), MIN_PARTITION_AGE_SECONDS);
        assert_eq!(config.max_tokens(), 1);
        assert_eq!(config.bytes_per_token(), 1);
    }

    #[test]
    fn test_storage_config_merge() {
        let base = StorageConfig {
            partition_max_entries: Some(500),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: Some(50_000),
            bytes_per_token: Some(4),
            enable_bloom_filters: Some(true),
        };
        let override_ = StorageConfig {
            partition_max_entries: Some(1000),
            partition_max_age_seconds: None,
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };

        let merged = base.merge(&override_);
        assert_eq!(merged.partition_max_entries, Some(1000)); // overridden
        assert_eq!(merged.partition_max_age_seconds, Some(86400)); // from base
        assert_eq!(merged.partition_max_tokens, Some(50_000)); // from base
        assert_eq!(merged.enable_bloom_filters, Some(false)); // overridden
    }

    // Manifest tests

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let manifest = Manifest {
            version: 1,
            active_partition: "active.jsonl".to_string(),
            partitions: vec![PartitionMeta {
                file: "partitions/1000-2000.jsonl".to_string(),
                start_ts: 1000,
                end_ts: 2000,
                entry_count: 100,
                token_count: Some(5000),
                bloom_file: Some("partitions/1000-2000.bloom".to_string()),
            }],
            rotation_policy: RotationPolicy {
                max_entries: 500,
                max_age_seconds: 86400,
                max_tokens: Some(50_000),
            },
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.active_partition, "active.jsonl");
        assert_eq!(parsed.partitions.len(), 1);
        assert_eq!(parsed.partitions[0].entry_count, 100);
        assert_eq!(parsed.rotation_policy.max_entries, 500);
    }

    #[test]
    fn test_partition_meta_overlaps() {
        let meta = PartitionMeta {
            file: "test.jsonl".to_string(),
            start_ts: 1000,
            end_ts: 2000,
            entry_count: 10,
            token_count: None,
            bloom_file: None,
        };

        // Fully inside
        assert!(meta.overlaps(1200, 1800));
        // Overlaps start
        assert!(meta.overlaps(800, 1200));
        // Overlaps end
        assert!(meta.overlaps(1800, 2200));
        // Contains partition
        assert!(meta.overlaps(500, 2500));
        // Before
        assert!(!meta.overlaps(100, 500));
        // After
        assert!(!meta.overlaps(2500, 3000));
    }

    // PartitionManager tests

    #[test]
    fn test_partition_manager_fresh_context() {
        let temp_dir = TempDir::new().unwrap();
        let pm = PartitionManager::load(temp_dir.path()).unwrap();

        assert_eq!(pm.active_entry_count(), 0);
        assert_eq!(pm.manifest.active_partition, "active.jsonl");
        assert!(pm.manifest.partitions.is_empty());
    }

    #[test]
    fn test_append_and_read_entries() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        let entry1 = make_entry("Hello");
        let entry2 = make_entry("World");

        pm.append_entry(&entry1).unwrap();
        pm.append_entry(&entry2).unwrap();

        assert_eq!(pm.active_entry_count(), 2);

        let entries = pm.read_all_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "Hello");
        assert_eq!(entries[1].content, "World");
    }

    #[test]
    fn test_read_entries_in_range() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        pm.append_entry(&make_entry_with_ts("Old", 1000)).unwrap();
        pm.append_entry(&make_entry_with_ts("Middle", 2000))
            .unwrap();
        pm.append_entry(&make_entry_with_ts("New", 3000)).unwrap();

        let entries = pm.read_entries_in_range(1500, 2500).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Middle");

        let all = pm.read_entries_in_range(0, 5000).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_rotation_at_threshold() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(3),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(true),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        for i in 0..3 {
            pm.append_entry(&make_entry(&format!("Entry {}", i)))
                .unwrap();
        }

        assert!(pm.needs_rotation());
        assert!(pm.rotate_if_needed().unwrap());

        assert_eq!(pm.active_entry_count(), 0);
        assert_eq!(pm.manifest.partitions.len(), 1);
        assert_eq!(pm.manifest.partitions[0].entry_count, 3);

        // Verify files exist
        let manifest_path = temp_dir.path().join("manifest.json");
        assert!(manifest_path.exists());

        let partition_path = temp_dir.path().join(&pm.manifest.partitions[0].file);
        assert!(partition_path.exists());

        if let Some(ref bloom) = pm.manifest.partitions[0].bloom_file {
            assert!(temp_dir.path().join(bloom).exists());
        }
    }

    #[test]
    fn test_read_after_rotation() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        pm.append_entry(&make_entry("Entry 1")).unwrap();
        pm.append_entry(&make_entry("Entry 2")).unwrap();
        pm.rotate_if_needed().unwrap();
        pm.append_entry(&make_entry("Entry 3")).unwrap();

        let entries = pm.read_all_entries().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].content, "Entry 1");
        assert_eq!(entries[1].content, "Entry 2");
        assert_eq!(entries[2].content, "Entry 3");
    }

    #[test]
    fn test_legacy_migration() {
        let temp_dir = TempDir::new().unwrap();

        // Create legacy context.jsonl
        let legacy_path = temp_dir.path().join("context.jsonl");
        let entry = make_entry("Legacy content");
        let json = serde_json::to_string(&entry).unwrap();
        fs::write(&legacy_path, format!("{}\n", json)).unwrap();

        let pm = PartitionManager::load(temp_dir.path()).unwrap();

        assert_eq!(pm.manifest.active_partition, "context.jsonl");
        assert_eq!(pm.active_entry_count(), 1);

        let entries = pm.read_all_entries().unwrap();
        assert_eq!(entries[0].content, "Legacy content");
    }

    #[test]
    fn test_reload_after_save() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };

        {
            let mut pm =
                PartitionManager::load_with_config(temp_dir.path(), config.clone()).unwrap();
            pm.append_entry(&make_entry("Entry 1")).unwrap();
            pm.append_entry(&make_entry("Entry 2")).unwrap();
            pm.rotate_if_needed().unwrap();
            pm.append_entry(&make_entry("Entry 3")).unwrap();
        }

        {
            let pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();
            assert_eq!(pm.manifest.partitions.len(), 1);
            assert_eq!(pm.active_entry_count(), 1);
            assert_eq!(pm.read_all_entries().unwrap().len(), 3);
        }
    }

    #[test]
    fn test_bloom_filter_operations() {
        let id1 = "test-id-1";
        let id2 = "test-id-2";
        let unknown = "unknown-id";

        // Test using fastbloom directly
        let bloom = BloomFilter::with_false_pos(BLOOM_FALSE_POSITIVE_RATE).items([id1, id2].iter());

        assert!(bloom.contains(&id1));
        assert!(bloom.contains(&id2));
        // Unknown should not be in the filter (false positives possible but unlikely)
        assert!(!bloom.contains(&unknown));
    }

    #[test]
    fn test_total_entry_count() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        pm.append_entry(&make_entry("1")).unwrap();
        pm.append_entry(&make_entry("2")).unwrap();
        pm.rotate_if_needed().unwrap();
        pm.append_entry(&make_entry("3")).unwrap();
        pm.append_entry(&make_entry("4")).unwrap();
        pm.rotate_if_needed().unwrap();
        pm.append_entry(&make_entry("5")).unwrap();

        assert_eq!(pm.total_entry_count(), 5);
        assert_eq!(pm.manifest.partitions.len(), 2);
        assert_eq!(pm.active_entry_count(), 1);
    }

    #[test]
    fn test_search_basic() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        pm.append_entry(&make_entry("Hello world")).unwrap();
        pm.append_entry(&make_entry("Goodbye world")).unwrap();
        pm.append_entry(&make_entry("Hello again")).unwrap();

        let result = pm.search("hello").unwrap();
        assert_eq!(result.entries.len(), 2);
        assert!(result.entries.iter().any(|e| e.content == "Hello world"));
        assert!(result.entries.iter().any(|e| e.content == "Hello again"));
    }

    #[test]
    fn test_search_uses_bloom_filter() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(true),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        // Create entries with specific words
        pm.append_entry(&make_entry("apple banana")).unwrap();
        pm.append_entry(&make_entry("cherry date")).unwrap();
        pm.rotate_if_needed().unwrap(); // Rotates, creates bloom filter

        pm.append_entry(&make_entry("elderberry fig")).unwrap();

        // Search for word in archived partition
        let result = pm.search("apple").unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].content, "apple banana");

        // Search for word not in any partition
        let result = pm.search("zebra").unwrap();
        assert_eq!(result.entries.len(), 0);
        assert!(result.partitions_skipped > 0); // Bloom filter skipped partitions
    }

    #[test]
    fn test_entry_might_exist() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        let entry = make_entry("Test content");
        let entry_id = entry.id.clone();
        pm.append_entry(&entry).unwrap();

        assert!(pm.entry_might_exist(&entry_id).unwrap());
        assert!(!pm.entry_might_exist("nonexistent-id").unwrap());
    }

    #[test]
    fn test_tokenize() {
        let tokens: Vec<String> = tokenize("Hello, World! Test123").collect();
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test123".to_string()));
    }

    // === Caching tests (Issue #1) ===

    #[test]
    fn test_active_state_getter() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        // Initially empty
        let state = pm.active_state();
        assert_eq!(state.entry_count, 0);

        // After append
        pm.append_entry(&make_entry("Hello")).unwrap();
        let state = pm.active_state();
        assert_eq!(state.entry_count, 1);
        assert!(state.first_entry_ts.is_some());
    }

    #[test]
    fn test_load_with_cached_state_uses_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig::default();

        // Create entries and get the state
        let cached_state = {
            let mut pm =
                PartitionManager::load_with_config(temp_dir.path(), config.clone()).unwrap();
            pm.append_entry(&make_entry("Entry 1")).unwrap();
            pm.append_entry(&make_entry("Entry 2")).unwrap();
            pm.active_state()
        };

        // Load with cached state - should skip file scan
        let pm = PartitionManager::load_with_cached_state(
            temp_dir.path(),
            config,
            Some(cached_state.clone()),
        )
        .unwrap();

        // State should match what we cached
        assert_eq!(pm.active_state().entry_count, cached_state.entry_count);
        assert_eq!(
            pm.active_state().first_entry_ts,
            cached_state.first_entry_ts
        );
    }

    #[test]
    fn test_load_with_cached_state_none_scans_file() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig::default();

        // Create some entries
        {
            let mut pm =
                PartitionManager::load_with_config(temp_dir.path(), config.clone()).unwrap();
            pm.append_entry(&make_entry("Entry 1")).unwrap();
            pm.append_entry(&make_entry("Entry 2")).unwrap();
        }

        // Load without cached state - should scan file
        let pm = PartitionManager::load_with_cached_state(temp_dir.path(), config, None).unwrap();

        assert_eq!(pm.active_state().entry_count, 2);
    }

    #[test]
    fn test_load_with_cached_state_invalidates_stale_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig::default();

        // Create a "stale" cache that claims entries exist
        let stale_cache = ActiveState {
            entry_count: 5,
            token_count: 100,
            first_entry_ts: Some(12345),
        };

        // But the file doesn't exist - cache should be invalidated
        let pm =
            PartitionManager::load_with_cached_state(temp_dir.path(), config, Some(stale_cache))
                .unwrap();

        // Should have detected stale cache and returned default state
        assert_eq!(pm.active_state().entry_count, 0);
    }

    #[test]
    fn test_active_state_reset_after_rotation() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        // Add entries up to threshold
        pm.append_entry(&make_entry("Entry 1")).unwrap();
        pm.append_entry(&make_entry("Entry 2")).unwrap();

        // State should show 2 entries before rotation
        assert_eq!(pm.active_state().entry_count, 2);

        // Trigger rotation
        pm.rotate_if_needed().unwrap();

        // State should be reset after rotation
        assert_eq!(pm.active_state().entry_count, 0);
        assert!(pm.active_state().first_entry_ts.is_none());
    }

    // === Atomic manifest write tests (Issue #3) ===

    #[test]
    fn test_manifest_atomic_write_no_tmp_left() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        // Trigger rotation which writes manifest
        pm.append_entry(&make_entry("Entry 1")).unwrap();
        pm.append_entry(&make_entry("Entry 2")).unwrap();
        pm.rotate_if_needed().unwrap();

        // Check that no .tmp file remains
        let manifest_path = temp_dir.path().join("manifest.json");
        let tmp_path = temp_dir.path().join("manifest.tmp");

        assert!(manifest_path.exists(), "manifest.json should exist");
        assert!(
            !tmp_path.exists(),
            "manifest.tmp should not exist after atomic write"
        );
    }

    // === File locking tests (Issue #2) ===

    #[test]
    fn test_lock_file_created_on_append() {
        let temp_dir = TempDir::new().unwrap();
        let mut pm = PartitionManager::load(temp_dir.path()).unwrap();

        pm.append_entry(&make_entry("Hello")).unwrap();

        // Lock file should have been created
        let lock_path = temp_dir.path().join(".transcript.lock");
        assert!(lock_path.exists(), "lock file should be created on append");
    }

    #[test]
    fn test_lock_file_created_on_rotate() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            partition_max_entries: Some(2),
            partition_max_age_seconds: Some(86400),
            partition_max_tokens: None,
            bytes_per_token: None,
            enable_bloom_filters: Some(false),
        };
        let mut pm = PartitionManager::load_with_config(temp_dir.path(), config).unwrap();

        pm.append_entry(&make_entry("Entry 1")).unwrap();
        pm.append_entry(&make_entry("Entry 2")).unwrap();
        pm.rotate_if_needed().unwrap();

        // Lock file should exist
        let lock_path = temp_dir.path().join(".transcript.lock");
        assert!(lock_path.exists(), "lock file should be created on rotate");
    }
}
