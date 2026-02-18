//! VFS path newtype: the only way to address VFS content.
//!
//! `VfsPath` is an opaque string validated on construction. It rejects `..`,
//! `//`, null bytes, and anything that could escape the VFS sandbox. OS
//! `PathBuf` values never appear in the VFS API — backends translate `VfsPath`
//! to their native addressing internally.

use std::fmt;
use std::io::{self, ErrorKind};

/// Opaque path within the virtual file system.
///
/// Invariants (enforced at construction):
/// - Starts with `/`
/// - No `.` or `..` components
/// - No `//` sequences
/// - No null bytes
/// - No trailing `/` (except root `/`)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsPath(String);

/// URI scheme prefix for VFS paths in tool arguments.
const VFS_URI_PREFIX: &str = "vfs://";

impl VfsPath {
    /// Create a new VFS path, validating all invariants.
    pub fn new(path: &str) -> io::Result<Self> {
        if path.is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "VFS path cannot be empty",
            ));
        }
        if !path.starts_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path must start with '/': {}", path),
            ));
        }
        if path.contains('\0') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "VFS path cannot contain null bytes",
            ));
        }
        if path != "/" && path.ends_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path cannot have trailing slash: {}", path),
            ));
        }
        if path.contains("//") {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("VFS path cannot contain '//': {}", path),
            ));
        }
        for component in path.split('/') {
            if component == "." || component == ".." {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("VFS path cannot contain '.' or '..': {}", path),
                ));
            }
        }
        Ok(Self(path.to_string()))
    }

    /// The path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parent path, or `None` if this is the root.
    pub fn parent(&self) -> Option<VfsPath> {
        if self.0 == "/" {
            return None;
        }
        // Safety: slicing a validated VfsPath always produces a valid VfsPath,
        // so we use the private constructor directly to avoid redundant validation.
        match self.0.rfind('/') {
            Some(0) => Some(VfsPath("/".to_string())),
            Some(pos) => Some(VfsPath(self.0[..pos].to_string())),
            None => None,
        }
    }

    /// Final component of the path, or `None` for root.
    pub fn file_name(&self) -> Option<&str> {
        if self.0 == "/" {
            return None;
        }
        self.0.rsplit('/').next()
    }

    /// Join a relative path onto this path.
    ///
    /// The segment must not be empty, start with `/`, or contain `.`/`..`.
    pub fn join(&self, segment: &str) -> io::Result<VfsPath> {
        if segment.starts_with('/') {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "join segment must be relative",
            ));
        }
        if segment.split('/').any(|c| c == ".." || c == ".") {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "join segment cannot contain '.' or '..'",
            ));
        }
        let combined = if self.0 == "/" {
            format!("/{}", segment)
        } else {
            format!("{}/{}", self.0, segment)
        };
        VfsPath::new(&combined)
    }

    /// Parse a `vfs:///path` URI into a `VfsPath`.
    pub fn from_uri(uri: &str) -> io::Result<Self> {
        if !Self::is_vfs_uri(uri) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("not a vfs:/// URI (requires three slashes): {}", uri),
            ));
        }
        VfsPath::new(&uri[VFS_URI_PREFIX.len()..])
    }

    /// Check whether a string is a `vfs://` URI.
    ///
    /// Requires `vfs:///` (three slashes) so the path component starts with `/`.
    pub fn is_vfs_uri(s: &str) -> bool {
        s.starts_with("vfs:///")
    }
}

impl fmt::Display for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_paths() {
        assert!(VfsPath::new("/shared/foo.txt").is_ok());
        assert!(VfsPath::new("/home/planner/notes.md").is_ok());
        assert!(VfsPath::new("/sys/info").is_ok());
        assert!(VfsPath::new("/").is_ok());
    }

    #[test]
    fn test_rejects_dotdot() {
        assert!(VfsPath::new("/shared/../etc/passwd").is_err());
        assert!(VfsPath::new("/home/ctx/../../secret").is_err());
    }

    #[test]
    fn test_rejects_single_dot() {
        assert!(VfsPath::new("/shared/./file.txt").is_err());
        assert!(VfsPath::new("/./shared").is_err());
    }

    #[test]
    fn test_rejects_double_slash() {
        assert!(VfsPath::new("/shared//foo").is_err());
    }

    #[test]
    fn test_rejects_no_leading_slash() {
        assert!(VfsPath::new("shared/foo").is_err());
        assert!(VfsPath::new("").is_err());
    }

    #[test]
    fn test_rejects_null_bytes() {
        assert!(VfsPath::new("/shared/\0bad").is_err());
    }

    #[test]
    fn test_rejects_trailing_slash_except_root() {
        assert!(VfsPath::new("/shared/").is_err());
        assert!(VfsPath::new("/").is_ok());
    }

    #[test]
    fn test_as_str() {
        let p = VfsPath::new("/shared/foo.txt").unwrap();
        assert_eq!(p.as_str(), "/shared/foo.txt");
    }

    #[test]
    fn test_parent() {
        let p = VfsPath::new("/shared/sub/foo.txt").unwrap();
        assert_eq!(p.parent().unwrap().as_str(), "/shared/sub");
        let root = VfsPath::new("/").unwrap();
        assert!(root.parent().is_none());
    }

    #[test]
    fn test_file_name() {
        let p = VfsPath::new("/shared/foo.txt").unwrap();
        assert_eq!(p.file_name(), Some("foo.txt"));
        let root = VfsPath::new("/").unwrap();
        assert_eq!(root.file_name(), None);
    }

    #[test]
    fn test_join() {
        let base = VfsPath::new("/shared").unwrap();
        let joined = base.join("sub/file.txt").unwrap();
        assert_eq!(joined.as_str(), "/shared/sub/file.txt");
    }

    #[test]
    fn test_join_rejects_absolute() {
        let base = VfsPath::new("/shared").unwrap();
        assert!(base.join("/etc/passwd").is_err());
    }

    #[test]
    fn test_join_rejects_dotdot() {
        let base = VfsPath::new("/shared").unwrap();
        assert!(base.join("../etc/passwd").is_err());
    }

    #[test]
    fn test_join_rejects_single_dot() {
        let base = VfsPath::new("/shared").unwrap();
        assert!(base.join("./file.txt").is_err());
        assert!(base.join("sub/./file.txt").is_err());
    }

    #[test]
    fn test_from_vfs_uri() {
        let p = VfsPath::from_uri("vfs:///shared/foo.txt").unwrap();
        assert_eq!(p.as_str(), "/shared/foo.txt");
        let root = VfsPath::from_uri("vfs:///").unwrap();
        assert_eq!(root.as_str(), "/");
    }

    #[test]
    fn test_from_vfs_uri_rejects_non_vfs() {
        assert!(VfsPath::from_uri("/shared/foo.txt").is_err());
        assert!(VfsPath::from_uri("file:///shared/foo.txt").is_err());
    }

    #[test]
    fn test_is_vfs_uri() {
        assert!(VfsPath::is_vfs_uri("vfs:///shared/foo.txt"));
        assert!(!VfsPath::is_vfs_uri("/shared/foo.txt"));
        assert!(!VfsPath::is_vfs_uri("vfs://shared")); // only two slashes
    }
}
