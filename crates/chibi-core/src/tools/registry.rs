//! ToolRegistry — single source of truth for all tools at runtime.
//!
//! The registry owns all tool dispatch. Policy (hooks, permissions, caching)
//! stays in `send.rs` as middleware wrapping `dispatch_with_context`.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use indexmap::IndexMap;
use serde_json::Value;

use crate::config::ResolvedConfig;
use crate::state::AppState;
use crate::vfs::Vfs;
use crate::vfs::caller::VfsCaller;

use super::Tool;
#[cfg(test)]
use super::ToolMetadata;

/// Async future type for tool handlers.
///
/// Not `Send` — tool dispatch runs on a single tokio task via `join_all` (see
/// `send.rs`), so futures never cross thread boundaries. `AppState` and `Vfs`
/// contain `RefCell` fields that are `!Sync`, which would prevent `Send` anyway.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Uniform async handler. Captures its own state at registration time.
/// All runtime values come through `ToolCall.context`.
///
/// `Send + Sync` on the closure itself allows the `Arc<ToolHandler>` to be
/// shared across threads (e.g. during registry reads). The *future* returned
/// is intentionally `!Send` — see `BoxFuture`.
pub type ToolHandler = Arc<dyn Fn(ToolCall<'_>) -> BoxFuture<'_, io::Result<String>> + Send + Sync>;

/// Runtime context passed per-call. Carries values not known at registration time.
pub struct ToolCallContext<'a> {
    pub app: &'a AppState,
    pub context_name: &'a str,
    pub config: &'a ResolvedConfig,
    pub project_root: &'a Path,
    pub vfs: &'a Vfs,
    pub vfs_caller: VfsCaller<'a>,
}

/// Input to a tool handler.
pub struct ToolCall<'a> {
    pub name: &'a str,
    pub args: &'a Value,
    pub context: &'a ToolCallContext<'a>,
}

/// How a tool is implemented — the registry's dispatch discriminant.
///
/// Replaces `Tool.path: PathBuf`. Each variant carries only the info needed
/// to execute that tool type.
pub enum ToolImpl {
    /// Built-in Rust handler. The closure captures its own dependencies.
    Builtin(ToolHandler),
    /// OS-path plugin executable (spawned as subprocess).
    Plugin(PathBuf),
    /// MCP bridge tool (JSON-over-TCP to mcp-bridge daemon).
    Mcp { server: String, tool_name: String },
    /// Scheme tool loaded from VFS source via tein. Context is shared across
    /// all tools in the same `.scm` file (Arc). `exec_binding` names the
    /// scheme binding to call: `"tool-execute"` for single-tool convention
    /// format, `"%tool-execute-{name}%"` for `define-tool` multi-tool files.
    /// `registry` is the owning registry, passed to `execute_synthesised` so
    /// `call-tool` uses a per-call registry instead of a global static.
    ///
    /// Mutation site: if exec_binding format changes, update `extract_single_tool`
    /// and `extract_multi_tools` in synthesised.rs.
    #[cfg(feature = "synthesised-tools")]
    Synthesised {
        vfs_path: crate::vfs::VfsPath,
        exec_binding: String,
        context: std::sync::Arc<super::synthesised::TeinSession>,
        registry: Arc<RwLock<ToolRegistry>>,
        /// The tein worker thread's `ThreadId`, captured at context init time.
        /// Used as the key in `BRIDGE_CALL_CTX` so concurrent synthesised tool
        /// calls from different tein contexts never overwrite each other's entry.
        worker_thread_id: std::thread::ThreadId,
        /// Maps hook points to scheme binding names for hook callbacks.
        /// Populated from `%hook-registry%` during tool loading.
        hook_bindings: std::collections::HashMap<super::hooks::HookPoint, String>,
    },
}

impl Clone for ToolImpl {
    fn clone(&self) -> Self {
        match self {
            ToolImpl::Builtin(h) => ToolImpl::Builtin(h.clone()),
            ToolImpl::Plugin(p) => ToolImpl::Plugin(p.clone()),
            ToolImpl::Mcp { server, tool_name } => ToolImpl::Mcp {
                server: server.clone(),
                tool_name: tool_name.clone(),
            },
            #[cfg(feature = "synthesised-tools")]
            ToolImpl::Synthesised {
                vfs_path,
                exec_binding,
                context,
                registry,
                worker_thread_id,
                hook_bindings,
            } => ToolImpl::Synthesised {
                vfs_path: vfs_path.clone(),
                exec_binding: exec_binding.clone(),
                context: context.clone(),
                registry: Arc::clone(registry),
                worker_thread_id: *worker_thread_id,
                hook_bindings: hook_bindings.clone(),
            },
        }
    }
}

/// Tool category for filtering and permission routing.
///
/// Set once at registration time. Replaces all `is_*_tool()` predicates and
/// the `ToolType` enum in `send.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    Memory,
    FsRead,
    FsWrite,
    Shell,
    Network,
    Index,
    Flow,
    Vfs,
    Plugin,
    Mcp,
    Synthesised,
    Eval,
}

impl ToolCategory {
    /// String key used in hook payloads and config `exclude_categories`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolCategory::Memory => "memory",
            ToolCategory::FsRead => "fs_read",
            ToolCategory::FsWrite => "fs_write",
            ToolCategory::Shell => "shell",
            ToolCategory::Network => "network",
            ToolCategory::Index => "index",
            ToolCategory::Flow => "flow",
            ToolCategory::Vfs => "vfs",
            ToolCategory::Plugin => "plugin",
            ToolCategory::Mcp => "mcp",
            ToolCategory::Synthesised => "synthesised",
            ToolCategory::Eval => "eval",
        }
    }
}

/// Single source of truth for all tools at runtime.
///
/// - `IndexMap` — O(1) lookup, preserves insertion order for deterministic
///   tool lists sent to the LLM.
/// - Pure dispatch: `dispatch_with_context` finds the tool and calls its handler.
///   Hooks, permissions, and caching stay in `send.rs` as middleware.
pub struct ToolRegistry {
    tools: IndexMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: IndexMap::new(),
        }
    }

    /// Register a tool. Replaces any existing tool with the same name (hot-reload).
    pub fn register(&mut self, tool: Tool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Remove a tool by name. Uses `shift_remove` (not `swap_remove`) to keep
    /// insertion order stable for remaining tools — order is deterministic for
    /// the LLM tool list.
    pub fn unregister(&mut self, name: &str) -> Option<Tool> {
        self.tools.shift_remove(name)
    }

    /// Find all synthesised tool names whose VFS path matches `path`.
    ///
    /// Returns a `Vec<String>` of tool names so the caller can unregister or
    /// replace them. Handles multi-tool files (multiple tools per `.scm` file).
    #[cfg(feature = "synthesised-tools")]
    pub fn find_all_by_vfs_path(&self, path: &crate::vfs::VfsPath) -> Vec<String> {
        self.tools
            .values()
            .filter(
                |t| matches!(&t.r#impl, ToolImpl::Synthesised { vfs_path, .. } if vfs_path == path),
            )
            .map(|t| t.name.clone())
            .collect()
    }

    /// Check if a synthesised tool is visible to the given context.
    ///
    /// Visibility rules (non-synthesised tools always visible):
    /// - `/tools/shared/*` — visible to all contexts
    /// - `/tools/home/<ctx>/*` — visible only to context `<ctx>`
    /// - `/tools/flocks/<flock>/*` — visible to contexts that are members of `<flock>`
    /// - Unknown zone — visible by default (forward-compat)
    ///
    /// `flock_memberships` is the list of flock names the context belongs to.
    /// Returns `true` if tool is not found (caller's filter can then skip it).
    #[cfg(feature = "synthesised-tools")]
    pub fn is_tool_visible(
        &self,
        tool_name: &str,
        context_name: &str,
        flock_memberships: &[String],
    ) -> bool {
        let tool = match self.get(tool_name) {
            Some(t) => t,
            None => return true, // not found — let other filters handle it
        };
        match &tool.r#impl {
            ToolImpl::Synthesised { vfs_path, .. } => {
                let path = vfs_path.as_str();
                if path.starts_with("/tools/shared/") {
                    true
                } else if let Some(rest) = path.strip_prefix("/tools/home/") {
                    // /tools/home/alice/foo.scm → owner is "alice"
                    rest.split('/').next() == Some(context_name)
                } else if let Some(rest) = path.strip_prefix("/tools/flocks/") {
                    // /tools/flocks/dev-team/foo.scm → flock is "dev-team"
                    rest.split('/')
                        .next()
                        .map(|flock| flock_memberships.iter().any(|f| f == flock))
                        .unwrap_or(false)
                } else {
                    true // unknown zone — visible by default
                }
            }
            _ => true, // non-synthesised tools always visible
        }
    }

    /// Look up by name. O(1).
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// All tools in registration order.
    pub fn all(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values()
    }

    /// All tools matching a predicate.
    pub fn filter(&self, pred: impl Fn(&Tool) -> bool) -> Vec<&Tool> {
        self.tools.values().filter(|t| pred(t)).collect()
    }

    /// Dispatch an already-cloned `ToolImpl` without holding `&self`.
    ///
    /// Use this when the caller needs to release an `RwLockReadGuard` before
    /// the `.await` — clone the `ToolImpl` while holding the lock, drop the
    /// guard, then call `dispatch_impl`.
    pub async fn dispatch_impl(
        tool_impl: ToolImpl,
        name: &str,
        args: &Value,
        ctx: &ToolCallContext<'_>,
    ) -> io::Result<String> {
        let call = ToolCall {
            name,
            args,
            context: ctx,
        };
        match tool_impl {
            ToolImpl::Builtin(handler) => handler(call).await,
            ToolImpl::Plugin(path) => super::plugins::execute_tool_by_path(&path, name, args),
            ToolImpl::Mcp { server, tool_name } => {
                let home = ctx.app.chibi_dir.clone();
                super::mcp::execute_mcp_call(&server, &tool_name, args, &home)
            }
            #[cfg(feature = "synthesised-tools")]
            ToolImpl::Synthesised {
                context,
                exec_binding,
                registry,
                worker_thread_id,
                ..
            } => {
                super::synthesised::execute_synthesised(
                    context.as_ref(),
                    &exec_binding,
                    &call,
                    registry,
                    worker_thread_id,
                )
                .await
            }
        }
    }

    /// Dispatch a tool call with runtime context. Pure dispatch — no hooks,
    /// no permissions. Policy stays in `send.rs` as middleware.
    ///
    /// Clones `ToolImpl` before any `.await` so no borrow of `self` crosses
    /// an async suspension point, then delegates to `dispatch_impl`.
    pub async fn dispatch_with_context(
        &self,
        name: &str,
        args: &Value,
        ctx: &ToolCallContext<'_>,
    ) -> io::Result<String> {
        let tool_impl = self
            .get(name)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("unknown tool: {name}"))
            })?
            .r#impl
            .clone();
        Self::dispatch_impl(tool_impl, name, args, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_type_exists() {
        // compile-time check that the struct fields exist
        let args = serde_json::json!({"text": "hello"});
        let _ = std::mem::size_of::<ToolCall>();
        let _ = std::mem::size_of::<ToolCallContext>();
        let _ = args["text"].as_str().unwrap() == "hello";
    }

    #[test]
    fn test_tool_category_debug() {
        // ensure all variants exist and are debuggable
        let cats = [
            ToolCategory::Memory,
            ToolCategory::FsRead,
            ToolCategory::FsWrite,
            ToolCategory::Shell,
            ToolCategory::Network,
            ToolCategory::Index,
            ToolCategory::Flow,
            ToolCategory::Vfs,
            ToolCategory::Plugin,
            ToolCategory::Mcp,
            ToolCategory::Synthesised,
            ToolCategory::Eval,
        ];
        for cat in &cats {
            let _ = format!("{cat:?}");
        }
    }

    fn make_test_tool(name: &str, category: ToolCategory, r#impl: ToolImpl) -> Tool {
        Tool {
            name: name.into(),
            description: format!("test tool {name}"),
            parameters: serde_json::json!({}),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl,
            category,
        }
    }

    #[tokio::test]
    async fn test_registry_register_and_get() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("result".into()) }));
        reg.register(make_test_tool(
            "my_tool",
            ToolCategory::Shell,
            ToolImpl::Builtin(handler),
        ));
        assert!(reg.get("my_tool").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_registry_dispatch_builtin() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|call| {
            let name = call.name.to_string();
            Box::pin(async move { Ok(format!("called {name}")) })
        });
        reg.register(make_test_tool(
            "echo",
            ToolCategory::Shell,
            ToolImpl::Builtin(handler),
        ));
        // dispatch_with_context needs a ToolCallContext — tested via integration tests.
        // here we verify registration and lookup only.
        assert_eq!(reg.get("echo").unwrap().name, "echo");
    }

    #[test]
    fn test_registry_unregister() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        reg.register(make_test_tool(
            "rm_me",
            ToolCategory::Plugin,
            ToolImpl::Builtin(handler),
        ));
        assert!(reg.get("rm_me").is_some());
        let removed = reg.unregister("rm_me");
        assert!(removed.is_some());
        assert!(reg.get("rm_me").is_none());
    }

    #[test]
    fn test_registry_preserves_insertion_order() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        for name in ["charlie", "alice", "bob"] {
            reg.register(make_test_tool(
                name,
                ToolCategory::Plugin,
                ToolImpl::Builtin(handler.clone()),
            ));
        }
        let names: Vec<&str> = reg.all().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["charlie", "alice", "bob"]);
    }

    #[test]
    fn test_registry_unregister_preserves_order_of_remaining() {
        // shift_remove (not swap_remove) required to keep insertion order stable
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        for name in ["a", "b", "c", "d"] {
            reg.register(make_test_tool(
                name,
                ToolCategory::Plugin,
                ToolImpl::Builtin(handler.clone()),
            ));
        }
        reg.unregister("b");
        let names: Vec<&str> = reg.all().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c", "d"]);
    }

    #[test]
    fn test_registry_filter() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        reg.register(make_test_tool(
            "read1",
            ToolCategory::FsRead,
            ToolImpl::Builtin(handler.clone()),
        ));
        reg.register(make_test_tool(
            "write1",
            ToolCategory::FsWrite,
            ToolImpl::Builtin(handler.clone()),
        ));
        reg.register(make_test_tool(
            "read2",
            ToolCategory::FsRead,
            ToolImpl::Builtin(handler),
        ));
        let reads: Vec<&str> = reg
            .filter(|t| t.category == ToolCategory::FsRead)
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(reads, vec!["read1", "read2"]);
    }

    // --- visibility tests (synthesised-tools feature) ---

    #[cfg(feature = "synthesised-tools")]
    fn make_synth_tool(name: &str, vfs_path: &str) -> Tool {
        // minimal synthesised tool stub for visibility tests
        // uses a fresh context that evaluates to a trivial single-tool
        let path = crate::vfs::VfsPath::new(vfs_path).unwrap();
        let registry = Arc::new(std::sync::RwLock::new(ToolRegistry::new()));
        let source = format!(
            r#"(import (scheme base))
(define tool-name "{name}")
(define tool-description "stub")
(define tool-parameters '())
(define (tool-execute args) "stub")"#
        );
        crate::tools::synthesised::load_tool_from_source(&source, &path, &registry).unwrap()
    }

    #[cfg(feature = "synthesised-tools")]
    fn registry_with_synth_tool(name: &str, vfs_path: &str) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(make_synth_tool(name, vfs_path));
        reg
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_visibility_shared_visible_to_all() {
        let reg = registry_with_synth_tool("shared_tool", "/tools/shared/tool.scm");
        assert!(reg.is_tool_visible("shared_tool", "alice", &[]));
        assert!(reg.is_tool_visible("shared_tool", "bob", &[]));
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_visibility_home_only_owner() {
        let reg = registry_with_synth_tool("alice_tool", "/tools/home/alice/tool.scm");
        assert!(reg.is_tool_visible("alice_tool", "alice", &[]));
        assert!(!reg.is_tool_visible("alice_tool", "bob", &[]));
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_visibility_flock_members_only() {
        let reg = registry_with_synth_tool("flock_tool", "/tools/flocks/dev/tool.scm");
        assert!(reg.is_tool_visible("flock_tool", "alice", &["dev".to_string()]));
        assert!(!reg.is_tool_visible("flock_tool", "bob", &[]));
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_visibility_builtin_always_visible() {
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        let reg = {
            let mut r = ToolRegistry::new();
            r.register(make_test_tool(
                "builtin_tool",
                ToolCategory::Shell,
                ToolImpl::Builtin(handler),
            ));
            r
        };
        assert!(reg.is_tool_visible("builtin_tool", "anyone", &[]));
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_visibility_missing_tool_returns_true() {
        let reg = ToolRegistry::new();
        assert!(reg.is_tool_visible("nonexistent", "alice", &[]));
    }

    #[test]
    fn test_register_all_builtins() {
        use super::super::{
            register_eval_tools, register_flow_tools, register_fs_read_tools,
            register_fs_write_tools, register_index_tools, register_memory_tools,
            register_network_tools, register_shell_tools, register_vfs_tools,
        };

        let mut reg = ToolRegistry::new();
        register_memory_tools(&mut reg);
        register_fs_read_tools(&mut reg);
        register_fs_write_tools(&mut reg);
        register_shell_tools(&mut reg);
        register_network_tools(&mut reg);
        register_index_tools(&mut reg);
        register_flow_tools(&mut reg);
        register_vfs_tools(&mut reg);

        let reg_arc = std::sync::Arc::new(std::sync::RwLock::new(reg));
        register_eval_tools(&reg_arc);
        let reg = reg_arc.read().unwrap();

        let total = reg.all().count();
        assert!(total > 20, "expected 20+ builtin tools, got {total}");

        // spot-check categories
        assert_eq!(reg.get("file_head").unwrap().category, ToolCategory::FsRead);
        assert_eq!(reg.get("shell_exec").unwrap().category, ToolCategory::Shell);
        assert_eq!(
            reg.get("fetch_url").unwrap().category,
            ToolCategory::Network
        );
        assert_eq!(reg.get("vfs_list").unwrap().category, ToolCategory::Vfs);
        assert_eq!(reg.get("spawn_agent").unwrap().category, ToolCategory::Flow);
        assert_eq!(
            reg.get("update_reflection").unwrap().category,
            ToolCategory::Memory
        );
        assert_eq!(reg.get("scheme_eval").unwrap().category, ToolCategory::Eval);
        assert!(
            !reg.get("scheme_eval").unwrap().metadata.parallel,
            "scheme_eval must not be parallel"
        );
    }
}
