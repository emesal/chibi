//! VFS data types shared across the module.

use chrono::{DateTime, Utc};

/// Kind of entry in the VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsEntryKind {
    File,
    Directory,
}

/// Metadata for a VFS entry.
#[derive(Debug, Clone)]
pub struct VfsMetadata {
    pub size: u64,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub kind: VfsEntryKind,
}

/// A single entry returned by a directory listing.
///
/// Contains only name and kind — no metadata. Callers who need metadata
/// follow up with `VfsBackend::metadata()`. This keeps `list()` cheap for
/// backends where metadata is expensive.
#[derive(Debug, Clone)]
pub struct VfsEntry {
    pub name: String,
    pub kind: VfsEntryKind,
}
