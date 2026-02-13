//! VCS root detection.
//!
//! Walks up from a starting directory looking for version control markers.
//! Used to auto-detect the project root when not explicitly specified.

use std::path::{Path, PathBuf};

/// VCS markers to look for when walking up the directory tree.
/// Each entry is (marker_name, is_directory). Checked in order; first match wins.
const VCS_MARKERS: &[(&str, bool)] = &[
    (".git", true),       // also matches .git file (worktrees, submodules)
    (".hg", true),        // mercurial
    (".svn", true),       // subversion
    (".bzr", true),       // bazaar
    (".pijul", true),     // pijul
    (".jj", true),        // jujutsu
    (".fslckout", false),  // fossil (file)
    ("_FOSSIL_", false),   // fossil (alt)
];

/// Detect VCS root by walking up from `start` looking for markers.
///
/// Returns the first directory containing a VCS marker, or `None` if no
/// marker is found before reaching the filesystem root. Explicit
/// `--project-root` / `CHIBI_PROJECT_ROOT` should take precedence over this.
///
/// CVS is handled specially: `CVS/` directories appear at every level of a
/// checkout, so we walk up while `CVS/` is present and return the highest
/// directory that still contains it.
pub fn detect_vcs_root(start: &Path) -> Option<PathBuf> {
    let start = start.canonicalize().ok()?;

    // First pass: check standard markers (non-CVS)
    let mut current = start.as_path();
    loop {
        for &(marker, expect_dir) in VCS_MARKERS {
            let candidate = current.join(marker);
            if expect_dir {
                // .git can be a file (worktrees/submodules) or directory
                if marker == ".git" {
                    if candidate.exists() {
                        return Some(current.to_path_buf());
                    }
                } else if candidate.is_dir() {
                    return Some(current.to_path_buf());
                }
            } else if candidate.is_file() {
                return Some(current.to_path_buf());
            }
        }
        current = current.parent()?;
    }
}

/// CVS-specific root detection. Walks up while `CVS/` is present;
/// the highest directory still containing `CVS/` is the checkout root.
pub fn detect_cvs_root(start: &Path) -> Option<PathBuf> {
    let start = start.canonicalize().ok()?;
    if !start.join("CVS").is_dir() {
        return None;
    }
    let mut root = start.clone();
    let mut current = start.as_path();
    while let Some(parent) = current.parent() {
        if parent.join("CVS").is_dir() {
            root = parent.to_path_buf();
            current = parent;
        } else {
            break;
        }
    }
    Some(root)
}

/// Detect project root from VCS markers, trying standard VCS first, then CVS.
pub fn detect_project_root(start: &Path) -> Option<PathBuf> {
    detect_vcs_root(start).or_else(|| detect_cvs_root(start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_git_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_git_file() {
        // .git as file (worktree/submodule)
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".git"), "gitdir: /somewhere").unwrap();
        let sub = root.join("child");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_fossil_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".fslckout"), "").unwrap();
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_fossil_alt_marker() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("_FOSSIL_"), "").unwrap();

        assert_eq!(detect_vcs_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_mercurial_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".hg")).unwrap();

        assert_eq!(detect_vcs_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_jujutsu_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".jj")).unwrap();
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_cvs_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // CVS dirs at every level
        std::fs::create_dir(root.join("CVS")).unwrap();
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::create_dir(sub.join("CVS")).unwrap();
        let deep = sub.join("deep");
        std::fs::create_dir(&deep).unwrap();
        std::fs::create_dir(deep.join("CVS")).unwrap();

        assert_eq!(detect_cvs_root(&deep), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_no_vcs_returns_none() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("empty");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), None);
        assert_eq!(detect_cvs_root(&sub), None);
    }

    #[test]
    fn test_detect_project_root_prefers_standard_over_cvs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir(root.join("CVS")).unwrap();

        assert_eq!(detect_project_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_nearest_vcs_wins() {
        let tmp = TempDir::new().unwrap();
        let outer = tmp.path();
        std::fs::create_dir(outer.join(".git")).unwrap();
        let inner = outer.join("nested");
        std::fs::create_dir(&inner).unwrap();
        std::fs::create_dir(inner.join(".hg")).unwrap();
        let deep = inner.join("src");
        std::fs::create_dir(&deep).unwrap();

        // Should find .hg (nearer) not .git (farther)
        assert_eq!(detect_vcs_root(&deep), Some(inner.canonicalize().unwrap()));
    }
}
