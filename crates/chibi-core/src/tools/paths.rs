//! Canonical path resolver for all file-accessing builtin tools.
//!
//! Every tool that touches the filesystem or VFS goes through
//! [`resolve_tool_path`] — there is no other path resolver.

use crate::config::ResolvedConfig;
use crate::vfs::VfsPath;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// A resolved tool path — either an OS path or a VFS path.
///
/// Returned by [`resolve_tool_path`] so callers can branch on OS vs VFS
/// without re-parsing the URI.
#[derive(Debug)]
pub(crate) enum ResolvedPath {
    Os(PathBuf),
    Vfs(VfsPath),
}

impl ResolvedPath {
    /// Extract the OS path, returning an error for VFS paths.
    ///
    /// Used by tools that operate exclusively on the OS filesystem
    /// (e.g. `dir_list`, `glob_files`, `grep_files`).
    pub(crate) fn require_os(self, tool_name: &str) -> io::Result<PathBuf> {
        match self {
            Self::Os(p) => Ok(p),
            Self::Vfs(_) => Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("{} does not support vfs:// paths", tool_name),
            )),
        }
    }
}

/// Resolve a tool path string into a validated [`ResolvedPath`].
///
/// - `vfs:///` URIs → [`ResolvedPath::Vfs`]
/// - relative paths → joined with `project_root`, then security-validated
/// - absolute paths → security-validated directly
///
/// All OS paths are checked against `config.file_tools_allowed_paths` via
/// [`security::validate_file_path`](super::security::validate_file_path).
pub(crate) fn resolve_tool_path(
    path_str: &str,
    project_root: &Path,
    config: &ResolvedConfig,
) -> io::Result<ResolvedPath> {
    if VfsPath::is_vfs_uri(path_str) {
        let vfs_path = VfsPath::from_uri(path_str)?;
        return Ok(ResolvedPath::Vfs(vfs_path));
    }

    let resolved = if Path::new(path_str).is_relative() {
        project_root.join(path_str).to_string_lossy().to_string()
    } else {
        path_str.to_string()
    };
    let validated = super::security::validate_file_path(&resolved, config)?;
    Ok(ResolvedPath::Os(validated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedConfig;
    use std::fs;
    use tempfile::TempDir;

    fn make_config(allowed: &[&str]) -> ResolvedConfig {
        let mut config = ResolvedConfig::default();
        config.file_tools_allowed_paths = allowed.iter().map(|s| s.to_string()).collect();
        config
    }

    #[test]
    fn relative_path_resolves_against_project_root() {
        let dir = TempDir::new().unwrap();
        let docs = dir.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("readme.md"), "hi").unwrap();

        let config = make_config(&[dir.path().to_str().unwrap()]);
        let result = resolve_tool_path("docs/readme.md", dir.path(), &config);
        assert!(result.is_ok(), "relative path should resolve: {:?}", result);
        match result.unwrap() {
            ResolvedPath::Os(p) => {
                assert!(p.ends_with("docs/readme.md"));
            }
            ResolvedPath::Vfs(_) => panic!("expected Os path"),
        }
    }

    #[test]
    fn absolute_path_ignores_project_root() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("abs.txt");
        fs::write(&file, "content").unwrap();

        let config = make_config(&[dir.path().to_str().unwrap()]);
        let result = resolve_tool_path(file.to_str().unwrap(), dir.path(), &config);
        assert!(
            result.is_ok(),
            "absolute allowed path should work: {:?}",
            result
        );
    }

    #[test]
    fn path_outside_allowed_dirs_is_rejected() {
        let allowed_dir = TempDir::new().unwrap();
        let other_dir = TempDir::new().unwrap();
        let file = other_dir.path().join("secret.txt");
        fs::write(&file, "secret").unwrap();

        let config = make_config(&[allowed_dir.path().to_str().unwrap()]);
        let result = resolve_tool_path(file.to_str().unwrap(), allowed_dir.path(), &config);
        assert!(result.is_err(), "path outside allowed dirs should fail");
    }

    #[test]
    fn relative_path_outside_allowed_dirs_is_rejected() {
        let project_dir = TempDir::new().unwrap();
        let allowed_dir = TempDir::new().unwrap();
        // project_root differs from the allowed dir
        fs::write(project_dir.path().join("secret.txt"), "x").unwrap();

        let config = make_config(&[allowed_dir.path().to_str().unwrap()]);
        let result = resolve_tool_path("secret.txt", project_dir.path(), &config);
        assert!(
            result.is_err(),
            "relative path outside allowed dirs should fail"
        );
    }

    #[test]
    fn vfs_uri_returns_vfs_path() {
        let config = make_config(&[]);
        let result = resolve_tool_path("vfs:///shared/notes.txt", Path::new("/unused"), &config);
        assert!(result.is_ok());
        match result.unwrap() {
            ResolvedPath::Vfs(p) => assert_eq!(p.as_str(), "/shared/notes.txt"),
            ResolvedPath::Os(_) => panic!("expected Vfs path"),
        }
    }

    #[test]
    fn invalid_vfs_uri_errors() {
        let config = make_config(&[]);
        let root = Path::new("/some/root");
        assert!(resolve_tool_path("vfs:////etc", root, &config).is_err());
        assert!(resolve_tool_path("vfs:///../etc", root, &config).is_err());
    }

    #[test]
    fn require_os_extracts_os_path() {
        let p = PathBuf::from("/some/path");
        let resolved = ResolvedPath::Os(p.clone());
        assert_eq!(resolved.require_os("test").unwrap(), p);
    }

    #[test]
    fn require_os_rejects_vfs_path() {
        let resolved = ResolvedPath::Vfs(VfsPath::from_uri("vfs:///foo").unwrap());
        assert!(resolved.require_os("test_tool").is_err());
    }
}
