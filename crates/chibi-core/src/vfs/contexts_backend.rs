//! Virtual VFS backend for `/sys/contexts/`.
//!
//! Exposes read-only context metadata as virtual files. Each context
//! appears as a directory containing `state.json` (generated on read)
//! and `transcript/` (read-through to on-disk partition files).
//!
//! This backend is mounted at `/sys/contexts/` by `Chibi::load_with_options()`.
//! VFS routing strips the mount prefix, so this backend receives paths
//! relative to `/sys/contexts/` (e.g. `/alice/state.json`, not
//! `/sys/contexts/alice/state.json`).

use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use super::backend::{BoxFuture, ReadOnlyVfsBackend};
use super::flock::{FlockRegistry, resolve_flock_vfs_root, site_flock_name};
use super::path::VfsPath;
use super::types::{VfsEntry, VfsEntryKind, VfsMetadata};
use crate::context::{ContextEntry, ContextState};
use crate::partition::{Manifest, PartitionManager, StorageConfig};

/// Read-only VFS backend synthesising context metadata.
///
/// Receives stripped paths (the `/sys/contexts` prefix is already removed
/// by `Vfs::resolve_backend`). The path structure is:
///
/// - `/` — lists all contexts as directories
/// - `/<name>/` — lists `state.json` and `transcript/`
/// - `/<name>/state.json` — generated JSON with context metadata
/// - `/<name>/transcript/manifest.json` — read-through from disk
/// - `/<name>/transcript/partitions/<file>` — read-through from disk
/// - `/<name>/transcript/active.jsonl` — read-through from disk
pub struct ContextsBackend {
    /// Shared context state (source of truth for names + metadata).
    /// The same `Arc` that `AppState.state` holds after the Task 5 refactor.
    state: Arc<RwLock<ContextState>>,
    /// Root chibi data directory (e.g. `~/.chibi`). Used to locate
    /// transcript files at `<data_dir>/contexts/<name>/transcript/`.
    data_dir: PathBuf,
    /// Site identifier for flock membership lookups.
    site_id: String,
}

/// JSON structure for `/sys/contexts/<name>/state.json`.
#[derive(Serialize)]
struct ContextStateJson {
    created_at: u64,
    last_activity_at: u64,
    prompt_count: usize,
    auto_destroy_at: Option<u64>,
    auto_destroy_after_inactive_secs: Option<u64>,
    flocks: Vec<String>,
    paths: ContextPaths,
}

/// VFS path references in `state.json`.
#[derive(Serialize)]
struct ContextPaths {
    todos: String,
    goals: Vec<String>,
}

impl ContextsBackend {
    pub fn new(state: Arc<RwLock<ContextState>>, data_dir: PathBuf, site_id: String) -> Self {
        Self {
            state,
            data_dir,
            site_id,
        }
    }

    /// Look up a context entry by name. Returns `NotFound` if missing.
    fn find_context(&self, name: &str) -> io::Result<ContextEntry> {
        let state = self
            .state
            .read()
            .map_err(|_| io::Error::other("ContextsBackend: state lock poisoned"))?;
        state
            .contexts
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("no context: {name}")))
    }

    /// Context directory on disk (e.g. `~/.chibi/contexts/<name>`).
    fn context_dir(&self, name: &str) -> PathBuf {
        self.data_dir.join("contexts").join(name)
    }

    /// Transcript directory on disk. Handles legacy layout where transcript
    /// files live directly in the context dir rather than a `transcript/` subdir.
    fn transcript_dir(&self, name: &str) -> PathBuf {
        let ctx_dir = self.context_dir(name);
        let transcript_subdir = ctx_dir.join("transcript");
        if transcript_subdir.is_dir() {
            transcript_subdir
        } else {
            ctx_dir
        }
    }

    /// Compute the prompt count for a context using `PartitionManager`.
    ///
    /// This uses the same path that `AppState::prompt_count` uses, ensuring
    /// consistent counts. Archived partitions use cached `prompt_count` from
    /// the manifest; the active partition uses the in-memory `ActiveState`
    /// built during `PartitionManager::load`. No per-line scanning needed.
    fn prompt_count_for(&self, name: &str) -> io::Result<usize> {
        let transcript_dir = self.transcript_dir(name);
        // NOTE: PartitionManager::load is #[cfg(test)] only — use load_with_config instead
        let pm = PartitionManager::load_with_config(&transcript_dir, StorageConfig::default())?;
        Ok(pm.total_prompt_count())
    }

    /// Build `state.json` content for a context.
    fn build_state_json(&self, name: &str) -> io::Result<Vec<u8>> {
        let entry = self.find_context(name)?;

        // Load flock registry from disk.
        // Note: we read this directly from disk rather than via VFS to avoid
        // a circular dependency (ContextsBackend is itself a VFS backend).
        let vfs_root = self.data_dir.join("vfs");
        let registry_path = vfs_root.join("flocks").join("registry.json");
        let registry: FlockRegistry = if registry_path.exists() {
            let data = std::fs::read_to_string(&registry_path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            FlockRegistry::default()
        };

        // Flocks: site flock + explicit memberships.
        let site_flock = site_flock_name(&self.site_id);
        let explicit = registry.flocks_for(name, &site_flock);
        let mut flocks = vec![site_flock.clone()];
        flocks.extend(explicit.clone());

        // Goal paths: one per flock (site + explicit).
        let mut goal_paths = Vec::new();
        if let Ok(root) = resolve_flock_vfs_root(&site_flock, &self.site_id) {
            goal_paths.push(format!("{}/goals.md", root.as_str()));
        }
        for flock_name in &explicit {
            if let Ok(root) = resolve_flock_vfs_root(flock_name, &self.site_id) {
                goal_paths.push(format!("{}/goals.md", root.as_str()));
            }
        }

        // Prompt count via PartitionManager (no line scanning).
        let prompt_count = self.prompt_count_for(name).unwrap_or(0);

        let state = ContextStateJson {
            created_at: entry.created_at,
            last_activity_at: entry.last_activity_at,
            prompt_count,
            auto_destroy_at: if entry.destroy_at == 0 {
                None
            } else {
                Some(entry.destroy_at)
            },
            auto_destroy_after_inactive_secs: if entry.destroy_after_seconds_inactive == 0 {
                None
            } else {
                Some(entry.destroy_after_seconds_inactive)
            },
            flocks,
            paths: ContextPaths {
                todos: format!("/home/{}/todos.md", name),
                goals: goal_paths,
            },
        };

        serde_json::to_vec_pretty(&state).map_err(|e| io::Error::other(e.to_string()))
    }

    /// Load the partition manifest for a context.
    fn load_manifest(&self, name: &str) -> io::Result<Manifest> {
        let dir = self.transcript_dir(name);
        let manifest_path = dir.join("manifest.json");
        if manifest_path.exists() {
            let data = std::fs::read_to_string(&manifest_path)?;
            serde_json::from_str(&data)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("bad manifest: {e}")))
        } else {
            Ok(Manifest::default())
        }
    }

    /// Parse a stripped path into `(context_name, remainder)`.
    /// E.g. `/foo/state.json` → `("foo", "state.json")`.
    /// Returns `None` for root (`/` or empty).
    fn parse_path(path: &VfsPath) -> Option<(&str, &str)> {
        let p = path.as_str().trim_start_matches('/');
        if p.is_empty() {
            return None;
        }
        let (name, rest) = match p.find('/') {
            Some(i) => (&p[..i], &p[i + 1..]),
            None => (p, ""),
        };
        Some((name, rest))
    }
}

impl ReadOnlyVfsBackend for ContextsBackend {
    fn backend_name(&self) -> &str {
        "virtual context metadata"
    }

    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            let (name, rest) = Self::parse_path(path).ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "cannot read directory; use list()")
            })?;

            match rest {
                "" => Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "cannot read directory; use list()",
                )),
                "state.json" => self.build_state_json(name),
                "transcript/manifest.json" => {
                    let path = self.transcript_dir(name).join("manifest.json");
                    std::fs::read(&path).map_err(|e| {
                        if e.kind() == ErrorKind::NotFound {
                            io::Error::new(
                                ErrorKind::NotFound,
                                format!("no transcript manifest for context '{name}'"),
                            )
                        } else {
                            e
                        }
                    })
                }
                rest if rest.starts_with("transcript/partitions/") => {
                    let file = rest.strip_prefix("transcript/partitions/").unwrap();
                    let _ = self.find_context(name)?; // verify context exists
                    let path = self.transcript_dir(name).join("partitions").join(file);
                    std::fs::read(&path)
                }
                "transcript/active.jsonl" => {
                    let _ = self.find_context(name)?;
                    let manifest = self.load_manifest(name)?;
                    let path = self.transcript_dir(name).join(&manifest.active_partition);
                    std::fs::read(&path)
                }
                _ => Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("no virtual file: {}", path),
                )),
            }
        })
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/').trim_end_matches('/');

            if p.is_empty() {
                // Root: list all contexts as directories.
                let state = self
                    .state
                    .read()
                    .map_err(|_| io::Error::other("ContextsBackend: state lock poisoned"))?;
                return Ok(state
                    .contexts
                    .iter()
                    .map(|c| VfsEntry {
                        name: c.name.clone(),
                        kind: VfsEntryKind::Directory,
                    })
                    .collect());
            }

            let (name, rest) = match p.find('/') {
                Some(i) => (&p[..i], &p[i + 1..]),
                None => (p, ""),
            };

            let _ = self.find_context(name)?; // verify context exists

            match rest {
                "" => Ok(vec![
                    VfsEntry {
                        name: "state.json".into(),
                        kind: VfsEntryKind::File,
                    },
                    VfsEntry {
                        name: "transcript".into(),
                        kind: VfsEntryKind::Directory,
                    },
                ]),
                "transcript" => Ok(vec![
                    VfsEntry {
                        name: "manifest.json".into(),
                        kind: VfsEntryKind::File,
                    },
                    VfsEntry {
                        name: "active.jsonl".into(),
                        kind: VfsEntryKind::File,
                    },
                    VfsEntry {
                        name: "partitions".into(),
                        kind: VfsEntryKind::Directory,
                    },
                ]),
                "transcript/partitions" => {
                    let manifest = self.load_manifest(name)?;
                    Ok(manifest
                        .partitions
                        .iter()
                        .filter_map(|p| {
                            let file_name = p.file.rsplit('/').next()?;
                            Some(VfsEntry {
                                name: file_name.to_string(),
                                kind: VfsEntryKind::File,
                            })
                        })
                        .collect())
                }
                _ => Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("no virtual directory: {}", path),
                )),
            }
        })
    }

    fn exists<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/');
            if p.is_empty() {
                return Ok(true); // root always exists
            }

            let Some((name, rest)) = Self::parse_path(path) else {
                return Ok(true);
            };

            if self.find_context(name).is_err() {
                return Ok(false);
            }

            match rest {
                "" | "state.json" | "transcript" | "transcript/partitions" => Ok(true),
                "transcript/manifest.json" => {
                    Ok(self.transcript_dir(name).join("manifest.json").exists())
                }
                "transcript/active.jsonl" => {
                    let manifest = self.load_manifest(name).unwrap_or_default();
                    Ok(self
                        .transcript_dir(name)
                        .join(&manifest.active_partition)
                        .exists())
                }
                rest if rest.starts_with("transcript/partitions/") => {
                    let file = rest.strip_prefix("transcript/partitions/").unwrap();
                    Ok(self
                        .transcript_dir(name)
                        .join("partitions")
                        .join(file)
                        .exists())
                }
                _ => Ok(false),
            }
        })
    }

    fn metadata<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<VfsMetadata>> {
        Box::pin(async move {
            let p = path.as_str().trim_start_matches('/');
            let is_dir = p.is_empty()
                || Self::parse_path(path)
                    .map(|(_, rest)| matches!(rest, "" | "transcript" | "transcript/partitions"))
                    .unwrap_or(true);

            Ok(VfsMetadata {
                size: 0,
                created: None,
                modified: None,
                kind: if is_dir {
                    VfsEntryKind::Directory
                } else {
                    VfsEntryKind::File
                },
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::backend::VfsBackend;
    use tempfile::TempDir;

    fn mock_state(names: &[&str]) -> Arc<RwLock<ContextState>> {
        let contexts = names
            .iter()
            .map(|n| ContextEntry {
                name: n.to_string(),
                created_at: 1000,
                last_activity_at: 2000,
                destroy_after_seconds_inactive: 0,
                destroy_at: 0,
                cwd: None,
            })
            .collect();
        Arc::new(RwLock::new(ContextState { contexts }))
    }

    fn make_backend(
        state: Arc<RwLock<ContextState>>,
        data_dir: &std::path::Path,
    ) -> ContextsBackend {
        std::fs::create_dir_all(data_dir.join("contexts")).unwrap();
        std::fs::create_dir_all(data_dir.join("vfs").join("flocks")).unwrap();
        ContextsBackend::new(state, data_dir.to_path_buf(), "test-site".into())
    }

    #[tokio::test]
    async fn test_list_root_returns_contexts() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice", "bob"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let entries = backend.list(&VfsPath::new("/").unwrap()).await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"bob"));
        assert!(entries.iter().all(|e| e.kind == VfsEntryKind::Directory));
    }

    #[tokio::test]
    async fn test_list_context_dir() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        let entries = backend
            .list(&VfsPath::new("/alice").unwrap())
            .await
            .unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"state.json"));
        assert!(names.contains(&"transcript"));
    }

    #[tokio::test]
    async fn test_read_state_json() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        let data = backend
            .read(&VfsPath::new("/alice/state.json").unwrap())
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(json["created_at"], 1000);
        assert_eq!(json["last_activity_at"], 2000);
        assert_eq!(json["prompt_count"], 0);
        assert!(json["auto_destroy_at"].is_null());
        assert!(json["flocks"].is_array());
        assert_eq!(json["paths"]["todos"], "/home/alice/todos.md");
    }

    #[tokio::test]
    async fn test_read_nonexistent_context() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&[]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let err = backend
            .read(&VfsPath::new("/ghost/state.json").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_exists() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());
        std::fs::create_dir_all(tmp.path().join("contexts/alice")).unwrap();

        assert!(backend.exists(&VfsPath::new("/").unwrap()).await.unwrap());
        assert!(
            backend
                .exists(&VfsPath::new("/alice").unwrap())
                .await
                .unwrap()
        );
        assert!(
            backend
                .exists(&VfsPath::new("/alice/state.json").unwrap())
                .await
                .unwrap()
        );
        assert!(
            !backend
                .exists(&VfsPath::new("/ghost").unwrap())
                .await
                .unwrap()
        );
        assert!(
            !backend
                .exists(&VfsPath::new("/alice/nope.txt").unwrap())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_write_rejected() {
        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let path = VfsPath::new("/alice/state.json").unwrap();
        let err = backend.write(&path, b"nope").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn test_transcript_manifest_read_through() {
        use serde_json::json;

        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let transcript_dir = tmp.path().join("contexts/alice/transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let manifest = json!({
            "version": 1,
            "active_partition": "active.jsonl",
            "partitions": [{
                "file": "partitions/1000-2000.jsonl",
                "start_ts": 1000,
                "end_ts": 2000,
                "entry_count": 10,
                "prompt_count": 3
            }],
            "rotation_policy": {
                "max_entries": 1000,
                "max_age_seconds": 2592000
            }
        });
        std::fs::write(
            transcript_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let data = backend
            .read(&VfsPath::new("/alice/transcript/manifest.json").unwrap())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(parsed["version"], 1);
    }

    #[tokio::test]
    async fn test_list_transcript_partitions() {
        use serde_json::json;

        let tmp = TempDir::new().unwrap();
        let state = mock_state(&["alice"]);
        let backend: &dyn VfsBackend = &make_backend(state, tmp.path());

        let transcript_dir = tmp.path().join("contexts/alice/transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let manifest = json!({
            "version": 1,
            "active_partition": "active.jsonl",
            "partitions": [
                {"file": "partitions/1000-2000.jsonl", "start_ts": 1000, "end_ts": 2000, "entry_count": 10},
                {"file": "partitions/2001-3000.jsonl", "start_ts": 2001, "end_ts": 3000, "entry_count": 5}
            ],
            "rotation_policy": {"max_entries": 1000, "max_age_seconds": 2592000}
        });
        std::fs::write(
            transcript_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let entries = backend
            .list(&VfsPath::new("/alice/transcript/partitions").unwrap())
            .await
            .unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["1000-2000.jsonl", "2001-3000.jsonl"]);
    }
}
