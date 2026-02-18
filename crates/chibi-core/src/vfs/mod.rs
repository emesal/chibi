//! Virtual file system: sandboxed, shared storage for contexts.
//!
//! The VFS provides a permission-enforced file namespace that contexts can
//! read and write without engaging the OS-level permission system. Paths use
//! a `vfs://` URI scheme and never leak OS `PathBuf` values.
//!
//! # Architecture (Approach A — thin trait, fat router)
//!
//! ```text
//! tool code  ->  Vfs (permissions + path validation)  ->  VfsBackend (dumb storage)
//! ```
//!
//! The `Vfs` struct enforces zone-based permissions and delegates to a
//! `VfsBackend` trait implementation. Backends are trivially simple — just
//! storage, no permission logic.
//!
//! # Future evolution
//!
//! - **Multi-backend mounting**: `Vfs` maps path prefixes to different backends
//!   (e.g. `/shared/` on disk, `/remote/` on XMPP). Longest-prefix match.
//! - **Middleware layers**: Composable tower-style layers (logging, caching)
//!   wrapping backends (approach C in the design doc). Refactor when needed.

pub mod path;

pub use path::VfsPath;
