//! Typed caller identity for VFS operations.

use std::fmt;

/// Typed caller identity for VFS operations.
///
/// Replaces the raw `&str` caller parameter. `System` has unrestricted write
/// access; `Context` is subject to zone-based permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsCaller<'a> {
    /// Internal system caller — unrestricted write access to all zones.
    System,
    /// A named context — subject to zone-based permission checks.
    Context(&'a str),
}

impl<'a> fmt::Display for VfsCaller<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VfsCaller::System => write!(f, "SYSTEM"),
            VfsCaller::Context(name) => write!(f, "{}", name),
        }
    }
}
