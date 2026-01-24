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
//! The active partition rotates when either threshold is reached:
//! - **Entry count**: Default 1000 entries per partition
//! - **Age**: Default 30 days since first entry
//!
//! On rotation, the active partition is moved to `partitions/` and a bloom
//! filter is built for efficient entry ID lookups.
//!
//! ## Legacy Migration
//!
//! Existing `context.jsonl` files are transparently adopted as the active
//! partition without data movement. Migration only occurs on first rotation.
//!
//! ## Bloom Filters
//!
//! Each archived partition can have an optional bloom filter (`.bloom` file)
//! for probabilistic entry ID lookups. This enables fast "definitely not here"
//! checks without reading partition files.
//!
//! Filter parameters are automatically optimized by fastbloom for the
//! expected number of entries with <1% false positive rate.
//!
//! See: <https://github.com/tomtomwombat/fastbloom>

use crate::context::TranscriptEntry;
use fastbloom::BloomFilter;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

// ============================================================================
// Constants
// ============================================================================

/// Default maximum entries per partition before rotation.
pub const DEFAULT_PARTITION_MAX_ENTRIES: usize = 1000;

/// Default maximum age of a partition in seconds (30 days).
pub const DEFAULT_PARTITION_MAX_AGE_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Target false positive rate for bloom filters.
const BLOOM_FALSE_POSITIVE_RATE: f64 = 0.01;

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

    /// Whether to build bloom filter indexes for archived partitions.
    /// Bloom filters enable fast entry ID lookups without reading partition files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_bloom_filters: Option<bool>,
}

impl StorageConfig {
    /// Returns the effective max entries threshold.
    #[inline]
    pub fn max_entries(&self) -> usize {
        self.partition_max_entries
            .unwrap_or(DEFAULT_PARTITION_MAX_ENTRIES)
    }

    /// Returns the effective max age threshold in seconds.
    #[inline]
    pub fn max_age_seconds(&self) -> u64 {
        self.partition_max_age_seconds
            .unwrap_or(DEFAULT_PARTITION_MAX_AGE_SECONDS)
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
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_PARTITION_MAX_ENTRIES,
            max_age_seconds: DEFAULT_PARTITION_MAX_AGE_SECONDS,
        }
    }
}

// ============================================================================
// Active Partition State
// ============================================================================

/// Cached state about the active partition, avoiding repeated file scans.
#[derive(Debug, Default)]
struct ActiveState {
    /// Number of entries in active partition.
    entry_count: usize,

    /// Timestamp of first entry, if any.
    first_entry_ts: Option<u64>,
}

impl ActiveState {
    /// Scans a JSONL file to extract count and first timestamp.
    fn from_file(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut count = 0;
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
            }
        }

        Ok(Self {
            entry_count: count,
            first_entry_ts: first_ts,
        })
    }

    /// Updates state after appending an entry.
    fn record_append(&mut self, entry: &TranscriptEntry) {
        self.entry_count += 1;
        if self.first_entry_ts.is_none() {
            self.first_entry_ts = Some(entry.timestamp);
        }
    }

    /// Resets state after rotation.
    fn reset(&mut self) {
        self.entry_count = 0;
        self.first_entry_ts = None;
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
/// ```ignore
/// let mut pm = PartitionManager::load(&context_dir)?;
/// pm.append_entry(&entry)?;
/// pm.rotate_if_needed()?;
/// let entries = pm.read_all_entries()?;
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
                },
            }
        } else {
            // Fresh context
            Manifest {
                rotation_policy: RotationPolicy {
                    max_entries: config.max_entries(),
                    max_age_seconds: config.max_age_seconds(),
                },
                ..Manifest::default()
            }
        };

        // Scan active partition for count and first timestamp
        let active_path = context_dir.join(&manifest.active_partition);
        let active = ActiveState::from_file(&active_path)?;

        Ok(Self {
            context_dir: context_dir.to_path_buf(),
            manifest,
            config,
            active,
        })
    }

    /// Loads manifest from disk with error context.
    fn load_manifest(path: &Path) -> io::Result<Manifest> {
        let file = File::open(path)?;
        serde_json::from_reader(BufReader::new(file)).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {}", e))
        })
    }

    /// Persists the manifest to disk.
    fn save_manifest(&self) -> io::Result<()> {
        let path = self.context_dir.join("manifest.json");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        serde_json::to_writer_pretty(BufWriter::new(file), &self.manifest)
            .map_err(|e| io::Error::other(format!("Failed to write manifest: {}", e)))
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
                entries.extend(read_jsonl_file(&path)?);
            }
        }

        // Active partition
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            entries.extend(read_jsonl_file(&active_path)?);
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
    pub fn append_entry(&mut self, entry: &TranscriptEntry) -> io::Result<()> {
        fs::create_dir_all(&self.context_dir)?;

        let active_path = self.context_dir.join(&self.manifest.active_partition);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
        writeln!(file, "{}", json)?;

        self.active.record_append(entry);
        Ok(())
    }

    /// Writes entries to the active partition, replacing existing content.
    ///
    /// Used during migration and compaction. For normal operation, prefer `append_entry`.
    pub fn write_entries(&mut self, entries: &[TranscriptEntry]) -> io::Result<()> {
        fs::create_dir_all(&self.context_dir)?;

        let active_path = self.context_dir.join(&self.manifest.active_partition);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&active_path)?;

        let mut writer = BufWriter::new(file);
        for entry in entries {
            let json = serde_json::to_string(entry)
                .map_err(|e| io::Error::other(format!("JSON serialize: {}", e)))?;
            writeln!(writer, "{}", json)?;
        }

        // Update cached state
        self.active.entry_count = entries.len();
        self.active.first_entry_ts = entries.first().map(|e| e.timestamp);

        self.save_manifest()?;
        Ok(())
    }

    // ========================================================================
    // Rotation
    // ========================================================================

    /// Returns true if the active partition should be rotated.
    ///
    /// Rotation is triggered when either:
    /// - Entry count >= configured max_entries
    /// - Time since first entry >= configured max_age_seconds
    pub fn needs_rotation(&self) -> bool {
        let policy = &self.manifest.rotation_policy;

        // Entry count threshold
        if self.active.entry_count >= policy.max_entries {
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
    pub fn rotate(&mut self) -> io::Result<()> {
        let active_path = self.context_dir.join(&self.manifest.active_partition);

        // Nothing to rotate if empty
        if !active_path.exists() || self.active.entry_count == 0 {
            return Ok(());
        }

        // Read entries to get timestamp range
        let entries = read_jsonl_file(&active_path)?;
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

        // Record partition metadata
        self.manifest.partitions.push(PartitionMeta {
            file: partition_rel,
            start_ts,
            end_ts,
            entry_count: entries.len(),
            bloom_file,
        });

        // Reset to default active partition
        self.manifest.active_partition = "active.jsonl".to_string();
        self.active.reset();

        self.save_manifest()?;
        Ok(())
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
                for entry in read_jsonl_file(&path)? {
                    if entry.timestamp >= from_ts && entry.timestamp <= to_ts {
                        entries.push(entry);
                    }
                }
            }
        }

        // Check active partition
        let active_path = self.context_dir.join(&self.manifest.active_partition);
        if active_path.exists() {
            for entry in read_jsonl_file(&active_path)? {
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
            enable_bloom_filters: other.enable_bloom_filters.or(self.enable_bloom_filters),
        }
    }
}

// ============================================================================
// File I/O Helpers
// ============================================================================

/// Reads all entries from a JSONL file.
///
/// Malformed lines are skipped with a warning to stderr.
fn read_jsonl_file(path: &Path) -> io::Result<Vec<TranscriptEntry>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!(
                    "[WARN] {}:{}: skipping malformed entry: {}",
                    path.display(),
                    line_num + 1,
                    e
                );
            }
        }
    }

    Ok(entries)
}

// ============================================================================
// Bloom Filter Implementation (using fastbloom by tomtomwombat)
// ============================================================================

/// Builds a bloom filter for entry IDs and writes it to disk.
///
/// Uses fastbloom for optimized bloom filter operations (2-20x faster than alternatives).
/// The bloom filter file is named by replacing `.jsonl` with `.bloom`.
fn build_bloom_filter(entries: &[TranscriptEntry], partition_path: &Path) -> io::Result<PathBuf> {
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    let bloom = BloomFilter::with_false_pos(BLOOM_FALSE_POSITIVE_RATE).items(ids.iter());

    let bloom_path = partition_path.with_extension("bloom");
    let serialized =
        serde_json::to_vec(&bloom).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&bloom_path, serialized)?;
    Ok(bloom_path)
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
    fn test_storage_config_merge() {
        let base = StorageConfig {
            partition_max_entries: Some(500),
            partition_max_age_seconds: Some(86400),
            enable_bloom_filters: Some(true),
        };
        let override_ = StorageConfig {
            partition_max_entries: Some(1000),
            partition_max_age_seconds: None,
            enable_bloom_filters: Some(false),
        };

        let merged = base.merge(&override_);
        assert_eq!(merged.partition_max_entries, Some(1000)); // overridden
        assert_eq!(merged.partition_max_age_seconds, Some(86400)); // from base
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
                bloom_file: Some("partitions/1000-2000.bloom".to_string()),
            }],
            rotation_policy: RotationPolicy {
                max_entries: 500,
                max_age_seconds: 86400,
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
}
