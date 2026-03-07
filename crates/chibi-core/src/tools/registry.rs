//! ToolRegistry — single source of truth for all tools at runtime.
//!
//! The registry owns all tool dispatch. Policy (hooks, permissions, caching)
//! stays in `send.rs` as middleware wrapping `dispatch_with_context`.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use crate::config::ResolvedConfig;
use crate::state::AppState;
use crate::vfs::caller::VfsCaller;
use crate::vfs::Vfs;

use super::{Tool, ToolMetadata};

/// Async future type for tool handlers.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Uniform async handler. Captures its own state at registration time.
/// All runtime values come through `ToolCall.context`.
pub type ToolHandler =
    Arc<dyn Fn(ToolCall<'_>) -> BoxFuture<'_, io::Result<String>> + Send + Sync>;

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
    // Synthesised variant added in Phase 4 (tein integration).
}

impl ToolImpl {
    /// Placeholder used during migration at construction sites not yet
    /// converted to typed variants. Will be removed once migration is complete.
    pub fn placeholder() -> Self {
        ToolImpl::Plugin(PathBuf::new())
    }
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
        Self { tools: IndexMap::new() }
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

    /// Dispatch a tool call with runtime context. Pure dispatch — no hooks,
    /// no permissions. Policy stays in `send.rs` as middleware.
    pub async fn dispatch_with_context(
        &self,
        name: &str,
        args: &Value,
        ctx: &ToolCallContext<'_>,
    ) -> io::Result<String> {
        let tool = self.get(name).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("unknown tool: {name}"))
        })?;
        let call = ToolCall { name, args, context: ctx };
        match &tool.r#impl {
            ToolImpl::Builtin(handler) => handler(call).await,
            ToolImpl::Plugin(_path) => {
                // wired to plugins::execute_tool_by_path in Task 6
                Err(io::Error::new(io::ErrorKind::Other, "plugin dispatch not yet wired"))
            }
            ToolImpl::Mcp { .. } => {
                // wired to mcp::execute_mcp_call in Task 6
                Err(io::Error::new(io::ErrorKind::Other, "mcp dispatch not yet wired"))
            }
        }
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
            path: PathBuf::new(),
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
        reg.register(make_test_tool("my_tool", ToolCategory::Shell, ToolImpl::Builtin(handler)));
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
        reg.register(make_test_tool("echo", ToolCategory::Shell, ToolImpl::Builtin(handler)));
        // dispatch_with_context needs a ToolCallContext — tested via integration tests.
        // here we verify registration and lookup only.
        assert_eq!(reg.get("echo").unwrap().name, "echo");
    }

    #[test]
    fn test_registry_unregister() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        reg.register(make_test_tool("rm_me", ToolCategory::Plugin, ToolImpl::Builtin(handler)));
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
            reg.register(make_test_tool(name, ToolCategory::Plugin, ToolImpl::Builtin(handler.clone())));
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
            reg.register(make_test_tool(name, ToolCategory::Plugin, ToolImpl::Builtin(handler.clone())));
        }
        reg.unregister("b");
        let names: Vec<&str> = reg.all().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c", "d"]);
    }

    #[test]
    fn test_registry_filter() {
        let mut reg = ToolRegistry::new();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        reg.register(make_test_tool("read1", ToolCategory::FsRead, ToolImpl::Builtin(handler.clone())));
        reg.register(make_test_tool("write1", ToolCategory::FsWrite, ToolImpl::Builtin(handler.clone())));
        reg.register(make_test_tool("read2", ToolCategory::FsRead, ToolImpl::Builtin(handler)));
        let reads: Vec<&str> = reg
            .filter(|t| t.category == ToolCategory::FsRead)
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(reads, vec!["read1", "read2"]);
    }
}
