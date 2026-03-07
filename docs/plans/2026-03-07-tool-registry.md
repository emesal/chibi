# ToolRegistry Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace chibi's ad-hoc tool dispatch with a unified `ToolRegistry` that owns all tool lookup and execution, backed by a VFS `/tools/` namespace for lazy discovery and synthesised scheme tools via tein.

**Architecture:** The registry is a concrete `ToolRegistry` struct holding an `IndexMap<String, Tool>`. Each tool carries a `ToolImpl` enum variant that describes how to execute it, and a `ToolCategory` for filtering. Builtin handlers are closures that capture their own dependencies at registration time. The registry provides pure dispatch (no hooks/permissions); policy stays in `send.rs` as middleware. A `ToolsBackend` implements `VfsBackend` for the virtual `/tools/sys/` namespace; writable zones use `LocalBackend`.

**Tech Stack:** Rust, tokio (async), indexmap, serde_json, tein (optional, for synthesised tools)

**Design doc:** `docs/plans/2026-03-07-tool-registry-design.md`

---

## Progress

### Phase 1 — COMPLETE (Tasks 1–4, commit `cad1fa59`)

**What was done:**
- Task 1: `indexmap = { version = "2", features = ["serde"] }` added to chibi-core/Cargo.toml
- Task 2: Created `tools/registry.rs` with `ToolHandler`, `ToolCallContext<'a>`, `ToolCall<'a>`, `ToolImpl` (Builtin/Plugin/Mcp), `ToolCategory` (11 variants), manual `Clone` for `ToolImpl`
- Task 3: Updated `Tool` struct — added `r#impl: ToolImpl`, `category: ToolCategory`, manual `Debug` impl (ToolHandler not Debug); `path` kept for migration; all construction sites updated; added `Tool::from_builtin_def()` helper and `BuiltinToolDef::to_json_schema()`
- Task 4: `ToolRegistry` struct with `IndexMap` backend, `register/unregister/get/all/filter/dispatch_with_context` (Plugin/Mcp arms are stubs, wired in Task 6); all tests pass

**Implementation notes:**
- `ToolImpl::Clone` is manual because `ToolHandler = Arc<dyn Fn...>` doesn't derive Clone (but `Arc::clone` works fine)
- `ToolMetadata::default()` gives `parallel: false` — always use `ToolMetadata::new()` for regular tools (parallel: true)
- `ToolCallContext<'_>` and `VfsCaller<'_>` share the same lifetime `'a` — this works cleanly
- Plugin/MCP dispatch stubs in registry return `Err("not yet wired")` — Task 6 replaces these

### Execution instructions (for Claude)

- After completing each task, update this plan file to reflect progress (mark complete, add notes).
- Use the task list (TaskCreate/TaskUpdate) to track progress so the user can follow along.

### Phase 2 — IN PROGRESS (Tasks 5–10)

#### Task 5 — COMPLETE (commit `cde36bb9`)

**What was done:**
- Added `register_*_tools()` to all 8 builtin modules (memory, fs_read, fs_write, shell, network, index, flow, vfs_tools)
- Added re-exports to `tools/mod.rs`
- Added `test_register_all_builtins` to `registry.rs` tests — 784 tests pass

**Implementation notes:**
- `BoxFuture` is intentionally `!Send` — `AppState` and `Vfs` contain `RefCell` fields that are `!Sync`. Tool dispatch runs on a single tokio task via `join_all` (no `tokio::spawn`), so `Send` is not required. Plan had `+ Send` in `BoxFuture` — removed.
- Sync handlers (memory, fs_read, fs_write, index) extract `io::Result<String>` before the `Box::pin(async move { result })` to avoid holding `!Sync` refs across `.await` points.
- Async handlers (vfs_tools, flow) rely on `!Send` future staying on the same thread; bind refs to local vars then move into async block.
- `flow.rs`: tools handled by send.rs middleware (send_message, call_user, model_info) have a handler that returns an error if dispatched directly through registry — they should be intercepted by send.rs before reaching dispatch.
- `flow.rs`: per-tool metadata uses manual `Tool { ... }` construction (not `Tool::from_builtin_def`) to override `ToolMetadata`.
- `index.rs`: `tools: &[]` passed — wired to registry in Task 7+.

#### Task 6 — COMPLETE (commit `68d3dca0`)

**What was done:**
- Added `execute_tool_by_path(path, name, args)` to `plugins.rs` — standalone, no `&Tool`
- Added `execute_mcp_call(server, tool_name, args, home)` to `mcp.rs` — standalone, no `&Tool`
- Wired both into `ToolRegistry::dispatch_with_context` Plugin/Mcp arms (replacing the stub errors)
- Re-exported `execute_tool_by_path` from `tools/mod.rs`

#### Task 7 — COMPLETE (commit `99d56cfe`)

**What was done:**
- `Chibi.tools: Vec<Tool>` → `registry: Arc<RwLock<ToolRegistry>>`
- `load_with_options`: builds registry via `register_*_tools()`, loads plugins and MCP tools
- All `self.tools` refs updated: init/shutdown collect plugin tools, send_prompt_streaming snapshots all tools, execute_tool now uses `dispatch_with_context`
- `for_test()` registers all builtins (test `test_tool_count_empty` → `test_tool_count_has_builtins`)
- `execution.rs:295` RunPlugin: registry.get() instead of find_tool()

**Implementation notes:**
- `for_test()` now registers all builtins so tests via `create_test_chibi` can call any builtin tool

#### Task 8 — COMPLETE (commit `5a05a9e6`)

**What was done:**
- `execute_tool` in chibi.rs replaced with `dispatch_with_context`
- Builds `ToolCallContext` from runtime values, calls `ensure_project_root_allowed` first

#### Task 9 — COMPLETE (commit TBD)

**What was done:**
- Step 1: Updated `send_prompt_streaming` call site in `chibi.rs` — passes `Arc::clone(&self.registry)` directly
- Step 2: Added `ToolCategory::as_str()` to `registry.rs` (all 11 variants)
- Step 3: Changed `send_prompt` signature to accept `registry: Arc<RwLock<ToolRegistry>>` instead of `tools: &[Tool]`
- Step 4: Extracted `plugin_tools: Vec<Tool>` from registry at top of `send_prompt`; all hook calls use `&plugin_tools`
- Step 5: Updated `execute_tool_pure`, `execute_single_tool`, `process_tool_calls`, `handle_final_response` signatures — all use `plugin_tools: &[Tool]` + `registry: &Arc<RwLock<ToolRegistry>>`
- Step 6: Updated `get_tool_metadata` and `tool_call_summary` in `tools/mod.rs` to take `&ToolRegistry`; updated all call sites including `config_resolution.rs::validate_config`
- Step 7: Replaced the if/else dispatch chain in `execute_tool_pure` with category-based permission middleware + `registry.dispatch_with_context`. Removed `unwrap_tool_dispatch`. Preserved VFS bypass, SEND_MESSAGE, MODEL_INFO, flow_control, and URL policy gating.
- Step 8: Removed `ToolType` enum, `classify_tool_type()`, all their tests. Updated `build_tool_info_list` and `filter_tools_by_config` to use `&Arc<RwLock<ToolRegistry>>`. Tests updated to use `make_test_registry()`.
- Step 9: Added `Tool::to_api_format()`. Replaced per-category `all_*_to_api_format()` calls with single registry iteration (Flow tools still added separately for dynamic preset_capabilities). Added `ToolRegistry::dispatch_impl(tool_impl, ...)` as a lock-free dispatch entry point.
- Step 10: All 772 tests pass, `just lint` clean.

**Implementation notes:**
- `plugin_tools` vec is extracted once per `send_prompt` call (filter by `ToolCategory::Plugin`). All hooks use this slice.
- `tool_category` and `tool_metadata` are both looked up from registry in one lock acquisition at top of `execute_tool_pure` (lock dropped before any `.await`)
- `dispatch_impl` is the preferred dispatch path when the caller holds an `Arc<RwLock<ToolRegistry>>` — clone `ToolImpl` while locked, drop guard, then call `dispatch_impl`. `dispatch_with_context` remains available for callers that own the registry directly.
- VFS bypass tests (`test_vfs_*_bypasses_*`) require `make_test_registry()` — the bypass fires permission gating but still routes through the registry for actual execution.
- `ToolMetadata` moved to `#[cfg(test)]` import in `registry.rs` (only used in tests).
- Flow tools added separately after registry iteration (spawn_agent needs dynamic preset_capabilities injected).

---

#### Task 9 — detailed execution plan

**Files to modify:**
- `crates/chibi-core/src/api/send.rs` — major refactor (all steps below)
- `crates/chibi-core/src/chibi.rs` — update call site (step 1)
- `crates/chibi-core/src/tools/registry.rs` — add `ToolCategory::as_str()` (step 2)
- `crates/chibi-core/src/tools/mod.rs` — update `get_tool_metadata` + `tool_call_summary` (step 6)

**Step 1: Update `send_prompt_streaming` call site in `chibi.rs`**

Currently (`chibi.rs` around line 360):
```rust
let tools_snap: Vec<Tool> = self.registry.read().unwrap().all().cloned().collect();
send_prompt(&self.app, context_name, prompt.to_string(), &tools_snap, config, options, sink, ...)
```

Change to:
```rust
send_prompt(&self.app, context_name, prompt.to_string(), Arc::clone(&self.registry), config, options, sink, ...)
```

Also update import in `chibi.rs` — `use crate::tools::{self, Tool, ToolCategory, ToolRegistry}` — remove `Tool` if no longer needed (or keep if used elsewhere).

**Step 2: Add `ToolCategory::as_str()` to `registry.rs`**

In `crates/chibi-core/src/tools/registry.rs`, after the `ToolCategory` enum:
```rust
impl ToolCategory {
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
        }
    }
}
```

**Step 3: Change `send_prompt` signature**

Old (line 1923):
```rust
pub async fn send_prompt<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    initial_prompt: String,
    tools: &[Tool],
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
    sink: &mut S,
    permission_handler: Option<&PermissionHandler>,
    home_dir: &Path,
    project_root: &Path,
) -> io::Result<()>
```

New:
```rust
pub async fn send_prompt<S: ResponseSink>(
    app: &AppState,
    context_name: &str,
    initial_prompt: String,
    registry: Arc<RwLock<ToolRegistry>>,
    resolved_config: &ResolvedConfig,
    options: &PromptOptions<'_>,
    sink: &mut S,
    permission_handler: Option<&PermissionHandler>,
    home_dir: &Path,
    project_root: &Path,
) -> io::Result<()>
```

Add imports at top of send.rs:
```rust
use std::sync::{Arc, RwLock};
use crate::tools::{ToolCategory, ToolRegistry};
```
Remove `use crate::tools::{self, Tool};` → `use crate::tools::{self, Tool};` (keep `Tool` for now, remove after functions updated).

**Step 4: Extract plugin tools once at top of `send_prompt`**

Right after the `let mut resolved_config = ...` / `let fuel_total = ...` setup lines:
```rust
// Plugin tools: used for hook execution throughout this call.
// Builtins don't have hooks; only plugin tools register hook scripts.
let plugin_tools: Vec<Tool> = registry.read().unwrap()
    .filter(|t| t.category == ToolCategory::Plugin)
    .into_iter()
    .cloned()
    .collect();
```

Then replace every `tools` reference in `send_prompt`'s body (the outermost `loop`) that is used for hooks with `&plugin_tools`. References used for **dispatch** will be handled in step 7.

Concretely, these hook call sites in the outer loop use `tools` purely for hook execution and should become `&plugin_tools`:
- `app.validate_config(&resolved_config, tools)?` → `app.validate_config(&resolved_config, &plugin_tools)?`
- `tools::execute_hook(tools, HookPoint::PreMessage, ...)` → `tools::execute_hook(&plugin_tools, ...)`
- `tools::execute_hook(tools, HookPoint::PreApiTools, ...)` → `tools::execute_hook(&plugin_tools, ...)`
- `tools::execute_hook(tools, HookPoint::PreApiRequest, ...)` → `tools::execute_hook(&plugin_tools, ...)`
- `tools::execute_hook(tools, HookPoint::PreAgenticLoop, ...)` → `tools::execute_hook(&plugin_tools, ...)`
- in `build_full_system_prompt(...)` call: `tools` arg → `&plugin_tools`
- `process_tool_calls(... tools, ...)` → see step 5

**Step 5: Update `process_tool_calls`, `execute_single_tool`, `execute_tool_pure` signatures**

These functions all take `tools: &[Tool]`. Change them all to take both:
- `plugin_tools: &[Tool]` — for hooks
- `registry: &Arc<RwLock<ToolRegistry>>` — for dispatch and metadata

```rust
async fn execute_tool_pure(
    app: &AppState,
    context_name: &str,
    tool_call: &ratatoskr::ToolCall,
    plugin_tools: &[Tool],          // ← was: tools: &[Tool]
    registry: &Arc<RwLock<ToolRegistry>>,  // ← new
    use_reflection: bool,
    resolved_config: &ResolvedConfig,
    permission_handler: Option<&PermissionHandler>,
    project_root: &Path,
) -> io::Result<ToolExecutionResult>

async fn execute_single_tool<S: ResponseSink>(
    ...same changes...
)

async fn process_tool_calls<S: ResponseSink>(
    ...same changes...
)
```

Update `handle_final_response` similarly (uses `tools` for `PostMessage` hook only):
- `tools: &[Tool]` → `plugin_tools: &[Tool]`

**Step 6: Update `get_tool_metadata` and `tool_call_summary` in `tools/mod.rs`**

These currently take `tools: &[Tool]` and check if a tool is in the slice. With the registry they should look up by name:

`get_tool_metadata` (line 292):
```rust
pub fn get_tool_metadata(registry: &ToolRegistry, name: &str) -> ToolMetadata {
    if let Some(tool) = registry.get(name) {
        return tool.metadata.clone();
    }
    flow_tool_metadata(name)  // fallback for unregistered flow tools
}
```

`tool_call_summary` (line 322):
```rust
pub fn tool_call_summary(registry: &ToolRegistry, name: &str, args_json: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;
    // get summary_params from registry, fall back to builtin lookup
    let summary_params: Vec<String> = if let Some(tool) = registry.get(name) {
        tool.summary_params.clone()
    } else {
        builtin_summary_params(name).iter().map(|s| s.to_string()).collect()
    };
    // ... rest unchanged
}
```

All call sites in send.rs: `tools::get_tool_metadata(tools, name)` → `tools::get_tool_metadata(&registry.read().unwrap(), name)` (lock, get, drop — all synchronous, no await).

`tool_call_summary` call sites similarly.

**Step 7: Replace the `execute_tool_pure` dispatch chain with registry dispatch**

The current if/else chain (lines 992–1281) looks like:
```
if let Some(memory_result) = execute_memory_tool(...)
else if tool_call.name == SEND_MESSAGE_TOOL_NAME
else if tool_call.name == MODEL_INFO_TOOL_NAME
else if is_fs_read_tool(...)
else if is_fs_write_tool(...)
else if is_vfs_tool(...)
else if is_shell_tool(...)
else if is_network_tool(...)
else if is_index_tool(...)
else if is_flow_tool(...)
else if let Some(tool) = find_tool(tools, name)  // plugin/mcp
else { "unknown tool" }
```

Replace with:

```rust
// Look up category for middleware routing. Drop lock before any await.
let (category, tool_metadata) = {
    let reg = registry.read().unwrap();
    let tool = reg.get(&tool_call.name);
    let cat = tool.map(|t| t.category).unwrap_or(ToolCategory::Plugin);
    let meta = tool.map(|t| t.metadata.clone()).unwrap_or_else(|| tools::flow_tool_metadata(&tool_call.name));
    (cat, meta)
};

let tool_result = if tool_metadata.flow_control {
    // ... same as before (handoff handling, no dispatch needed)
} else if tool_call.name == tools::SEND_MESSAGE_TOOL_NAME {
    // ... same as before
} else if tool_call.name == tools::MODEL_INFO_TOOL_NAME {
    // ... same as before
} else {
    // Permission middleware on category, then dispatch
    let permission_denied: Option<String> = match category {
        ToolCategory::FsRead => {
            // ... same VFS path bypass + classify_file_path logic as before
        }
        ToolCategory::FsWrite => {
            // ... same VFS path bypass + PreFileWrite check as before
        }
        ToolCategory::Shell => {
            // ... same PreShellExec check as before
        }
        ToolCategory::Network => {
            // ... same URL policy + PreFetchUrl check as before
        }
        _ => None,
    };

    if let Some(reason) = permission_denied {
        format!("Error: {}", reason)
    } else {
        // Build ToolCallContext and dispatch
        let mut config_for_dispatch = resolved_config.clone();
        tools::ensure_project_root_allowed(&mut config_for_dispatch, project_root);
        let call_ctx = tools::ToolCallContext {
            app,
            context_name,
            config: &config_for_dispatch,
            project_root,
            vfs: &app.vfs,
            vfs_caller: crate::vfs::VfsCaller::Context(context_name),
        };
        match registry.read().unwrap()
            .dispatch_with_context(&tool_call.name, &args, &call_ctx)
            .await
        {
            Ok(r) => r,
            Err(e) => format!("Error: {}", e),
        }
    }
};
```

**Important:** the VFS read special-case (bypass OS permission for `vfs://` paths) must be preserved inside the `FsRead` arm. The existing logic at lines 1022–1083 handles this already — keep it, just restructure it as a `match category` arm.

**Step 8: Remove `ToolType`, `classify_tool_type`, update `build_tool_info_list` and `filter_tools_by_config`**

`build_tool_info_list` (line 181): currently uses `classify_tool_type(name, plugin_tools)`. Replace with registry lookup:
```rust
fn build_tool_info_list(
    all_tools: &[serde_json::Value],
    registry: &Arc<RwLock<ToolRegistry>>,
) -> Vec<serde_json::Value> {
    let reg = registry.read().unwrap();
    all_tools.iter().filter_map(|tool| {
        let name = tool.get("function")?.get("name")?.as_str()?;
        let category = reg.get(name)
            .map(|t| t.category.as_str())
            .unwrap_or("plugin");  // unknown tools default to plugin
        Some(json!({ "name": name, "type": category }))
    }).collect()
}
```

`filter_tools_by_config` (line 225): uses `classify_tool_type` only in the `exclude_categories` arm. Replace:
```rust
fn filter_tools_by_config(
    tools: Vec<serde_json::Value>,
    config: &ToolsConfig,
    registry: &Arc<RwLock<ToolRegistry>>,
) -> Vec<serde_json::Value> {
    // ... include/exclude filters unchanged ...
    if let Some(ref categories) = config.exclude_categories {
        let reg = registry.read().unwrap();
        result.retain(|tool| {
            tool.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str())
                .map(|name| {
                    let cat = reg.get(name).map(|t| t.category.as_str()).unwrap_or("plugin");
                    !categories.contains(&cat.to_string())
                })
                .unwrap_or(true)
        });
    }
    result
}
```

Delete the `ToolType` enum (lines 65–93) and `classify_tool_type` function (lines 96–128).

**Step 9: Update tool API format building in `send_prompt`**

The current code (lines 2067–2100) builds `all_tools` from separate per-category functions:
```rust
let mut all_tools = tools::tools_to_api_format(tools);      // plugin tools
all_tools.extend(tools::all_memory_tools_to_api_format());
all_tools.extend(tools::all_fs_read_tools_to_api_format());
// ... etc
```

With the registry, all tools are in one place. The API format is already on each `Tool` via `parameters` field. Replace with:
```rust
let reg = registry.read().unwrap();
let mut all_tools: Vec<serde_json::Value> = reg.all()
    .filter(|t| {
        // exclude reflection tool if not enabled
        t.name != tools::REFLECTION_TOOL_NAME || use_reflection
    })
    .map(|t| t.to_api_format())           // ← needs Tool::to_api_format() — see note below
    .collect();
drop(reg);
```

**Note:** `Tool::to_api_format()` doesn't exist yet — add it to the `Tool` impl in `tools/mod.rs`:
```rust
impl Tool {
    pub fn to_api_format(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}
```

The `all_flow_tools_to_api_format(&preset_cap_refs)` call passes `preset_capabilities` to the spawn_agent tool description — this is dynamic. The spawn_agent tool in the registry has the *base* description without the preset list. For now, keep the flow tools API format call separate:
```rust
let reg = registry.read().unwrap();
let mut all_tools: Vec<serde_json::Value> = reg.all()
    .filter(|t| t.category != ToolCategory::Flow)  // flow tools added separately below
    .filter(|t| t.name != tools::REFLECTION_TOOL_NAME || use_reflection)
    .map(|t| t.to_api_format())
    .collect();
drop(reg);
// Flow tools need dynamic preset_capabilities in spawn_agent description
all_tools.extend(tools::all_flow_tools_to_api_format(&preset_cap_refs));
annotate_fallback_tool(&mut all_tools, &resolved_config.fallback_tool);
all_tools = filter_tools_by_config(all_tools, &resolved_config.tools, &registry);
```

**Step 10: Verify and commit**

Run: `cargo test -p chibi-core`
Expected: all tests pass.

Run: `just lint`
Expected: clean (no clippy warnings).

Commit:
```
refactor: replace send.rs dispatch chain with registry + middleware (#190)

removes ToolType enum, classify_tool_type(), and the if/else
dispatch chain. permission gating now routes on tool.category.
send_prompt accepts Arc<RwLock<ToolRegistry>> directly.
```

---

## Phase 1: Core Types and Registry

### Task 1: Add indexmap dependency

**Files:**
- Modify: `crates/chibi-core/Cargo.toml`

**Step 1: Add indexmap to dependencies**

Add under `[dependencies]`:
```toml
indexmap = { version = "2", features = ["serde"] }
```

**Step 2: Verify it compiles**

Run: `cargo check -p chibi-core`
Expected: compiles cleanly

**Step 3: Commit**

```
feat: add indexmap dependency for ToolRegistry (#190)
```

---

### Task 2: Define ToolImpl, ToolCategory, ToolCall, ToolHandler types

**Files:**
- Modify: `crates/chibi-core/src/tools/mod.rs:190-201` (Tool struct)
- Create: `crates/chibi-core/src/tools/registry.rs`

**Step 1: Write tests for ToolCategory and ToolCall**

In `crates/chibi-core/src/tools/registry.rs`, write:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_carries_name_and_args() {
        // ToolCallContext has no sensible default in tests; use a helper or
        // construct only the fields the assertion needs.
        // This test is a compile-time check that the struct fields exist.
        let args = serde_json::json!({"text": "hello"});
        // Full construction tested in integration tests where AppState is available.
        // Here we verify field names compile:
        let _ = std::mem::size_of::<ToolCall>();      // type exists
        let _ = std::mem::size_of::<ToolCallContext>(); // type exists
        let _ = args["text"].as_str().unwrap() == "hello";
    }

    #[test]
    fn test_tool_category_debug() {
        // ensure all variants exist and are debuggable
        let cats = [
            ToolCategory::Memory, ToolCategory::FsRead, ToolCategory::FsWrite,
            ToolCategory::Shell, ToolCategory::Network, ToolCategory::Index,
            ToolCategory::Flow, ToolCategory::Vfs, ToolCategory::Plugin,
            ToolCategory::Mcp, ToolCategory::Synthesised,
        ];
        for cat in &cats {
            let _ = format!("{:?}", cat);
        }
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core -- tools::registry::tests`
Expected: FAIL (module doesn't exist yet)

**Step 3: Implement the types**

In `crates/chibi-core/src/tools/registry.rs`:

```rust
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

/// Async future type for tool handlers.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Uniform async handler. Captures its own state at registration time.
pub type ToolHandler =
    Arc<dyn Fn(ToolCall<'_>) -> BoxFuture<'_, io::Result<String>> + Send + Sync>;

/// Runtime context passed per-call. Handlers receive this through ToolCall.
/// Captures values that aren't known at registration time (active context,
/// resolved config, project root, etc.).
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
pub enum ToolImpl {
    /// Built-in Rust handler. The closure captures its own dependencies.
    Builtin(ToolHandler),
    /// OS-path plugin executable (spawned as subprocess).
    Plugin(PathBuf),
    /// MCP bridge tool (JSON-over-TCP to mcp-bridge daemon).
    Mcp { server: String, tool_name: String },
    // Synthesised variant added in Phase 4 (tein integration).
}

/// Tool category for filtering and permission routing.
/// Replaces all is_*_tool() predicates and ToolType enum in send.rs.
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
```

Add `pub mod registry;` to `tools/mod.rs` and re-export:
```rust
pub use registry::{ToolCall, ToolCallContext, ToolCategory, ToolHandler, ToolImpl, ToolRegistry};
```

Note: `ToolCall` carries `context` from day one, so Task 5 handlers don't require a
signature change. `AppState`, `ResolvedConfig`, `Vfs`, and `VfsCaller` imports are
resolved when the module integrates with the rest of chibi-core; use placeholder
`todo!()` type aliases or forward declarations if they aren't in scope yet.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core -- tools::registry::tests`
Expected: PASS

**Step 5: Commit**

```
feat: define ToolImpl, ToolCategory, ToolCall, ToolHandler types (#190)
```

---

### Task 3: Update Tool struct — add `impl` and `category`, keep `path` temporarily

**Files:**
- Modify: `crates/chibi-core/src/tools/mod.rs:190-201`

**Step 1: Write a test for the new Tool fields**

In `registry.rs` tests:
```rust
#[test]
fn test_tool_has_impl_and_category() {
    let handler: ToolHandler = Arc::new(|_call| Box::pin(async { Ok("ok".into()) }));
    let tool = super::super::Tool {
        name: "test_tool".into(),
        description: "a test".into(),
        parameters: serde_json::json!({}),
        path: std::path::PathBuf::new(),
        hooks: vec![],
        metadata: super::super::ToolMetadata::default(),
        summary_params: vec![],
        r#impl: ToolImpl::Builtin(handler),
        category: ToolCategory::Memory,
    };
    assert_eq!(tool.category, ToolCategory::Memory);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core -- tools::registry::tests::test_tool_has_impl_and_category`
Expected: FAIL (fields don't exist)

**Step 3: Add the fields to Tool**

In `tools/mod.rs`, update the Tool struct (around line 190):
```rust
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub path: PathBuf,                    // kept temporarily for migration
    pub hooks: Vec<HookPoint>,
    pub metadata: ToolMetadata,
    pub summary_params: Vec<String>,
    pub r#impl: ToolImpl,                 // NEW
    pub category: ToolCategory,           // NEW
}
```

Remove `derive(Debug)` from Tool (ToolHandler is a trait object, not Debug). Add a
manual `impl Debug for Tool` that prints name, category, and a placeholder for `r#impl`
(e.g. `"<handler>"`). This avoids breaking any `{:?}` uses of Tool elsewhere.

Add a `Default` or placeholder impl for `ToolImpl` to avoid breaking existing Tool construction sites. A helper:
```rust
impl ToolImpl {
    /// Placeholder for migration. Will be removed once all construction sites set impl explicitly.
    pub fn placeholder() -> Self {
        ToolImpl::Plugin(PathBuf::new())
    }
}
```

Update ALL existing Tool construction sites to add the two new fields with placeholder values:
- `plugins.rs:parse_single_tool_schema` → `r#impl: ToolImpl::Plugin(path.to_path_buf()), category: ToolCategory::Plugin`
- `mcp.rs:mcp_tool_from_info` → `r#impl: ToolImpl::Mcp { server: server.into(), tool_name: name.into() }, category: ToolCategory::Mcp`
- Any test sites → use `ToolImpl::placeholder()` and `ToolCategory::Plugin`

Search for all struct-literal `Tool {` constructions:
Run: `grep -rn 'Tool {' crates/chibi-core/src/tools/`
and update each one.

**Step 4: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: PASS (all existing tests still work)

**Step 5: Commit**

```
feat: add ToolImpl and ToolCategory fields to Tool struct (#190)

path field kept temporarily for migration. all construction sites
updated with typed impl and category values.
```

---

### Task 4: Implement ToolRegistry struct

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

**Step 1: Write tests for ToolRegistry**

```rust
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
        Box::pin(async move { Ok(format!("called {}", name)) })
    });
    reg.register(make_test_tool("echo", ToolCategory::Shell, ToolImpl::Builtin(handler)));
    let ctx = test_call_context();
    let result = reg.dispatch_with_context("echo", &serde_json::json!({}), &ctx).await.unwrap();
    assert_eq!(result, "called echo");
}

#[tokio::test]
async fn test_registry_dispatch_unknown_tool() {
    let reg = ToolRegistry::new();
    let ctx = test_call_context();
    let err = reg.dispatch_with_context("nope", &serde_json::json!({}), &ctx).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
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
    // shift_remove (not swap_remove) is required to keep insertion order stable
    // after a removal, which matters for deterministic tool lists sent to the LLM.
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
    reg.register(make_test_tool("read2", ToolCategory::FsRead, ToolImpl::Builtin(handler.clone())));
    let reads: Vec<&str> = reg.filter(|t| t.category == ToolCategory::FsRead)
        .iter().map(|t| t.name.as_str()).collect();
    assert_eq!(reads, vec!["read1", "read2"]);
}

// test helper
fn make_test_tool(name: &str, category: ToolCategory, r#impl: ToolImpl) -> Tool {
    Tool {
        name: name.into(),
        description: format!("test tool {}", name),
        parameters: serde_json::json!({}),
        path: std::path::PathBuf::new(),
        hooks: vec![],
        metadata: ToolMetadata::default(),
        summary_params: vec![],
        r#impl,
        category,
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core -- tools::registry::tests`
Expected: FAIL (ToolRegistry doesn't exist)

**Step 3: Implement ToolRegistry**

```rust
/// Single source of truth for all tools at runtime.
pub struct ToolRegistry {
    tools: IndexMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: IndexMap::new() }
    }

    /// Register a tool. Replaces if name already exists (hot-reload).
    pub fn register(&mut self, tool: Tool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Remove a tool by name. Uses shift_remove (not swap_remove) to preserve
    /// insertion order for remaining tools — order is deterministic for the LLM
    /// tool list.
    pub fn unregister(&mut self, name: &str) -> Option<Tool> {
        self.tools.shift_remove(name)
    }

    /// Look up by name.
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
    /// no permissions. Policy stays in send.rs as middleware.
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
                // delegate to existing execute_tool in plugins.rs
                // wired up in Phase 2
                Err(io::Error::new(io::ErrorKind::Other, "plugin dispatch not yet wired"))
            }
            ToolImpl::Mcp { .. } => {
                // delegate to existing execute_mcp_tool in mcp.rs
                // wired up in Phase 2
                Err(io::Error::new(io::ErrorKind::Other, "mcp dispatch not yet wired"))
            }
        }
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core -- tools::registry::tests`
Expected: PASS

**Step 5: Commit**

```
feat: implement ToolRegistry with register/dispatch_with_context/filter (#190)

IndexMap-backed, O(1) lookup, insertion-ordered. shift_remove keeps
order stable after unregistration. dispatch_with_context is the single
dispatch API; plugin and MCP arms are stubs for Phase 2.
```

---

## Phase 2: Builtin Registration and Dispatch Wiring

### Task 5: Add register_*_tools() to each builtin module

**Files:**
- Modify: `crates/chibi-core/src/tools/memory.rs`
- Modify: `crates/chibi-core/src/tools/fs_read.rs`
- Modify: `crates/chibi-core/src/tools/fs_write.rs`
- Modify: `crates/chibi-core/src/tools/shell.rs`
- Modify: `crates/chibi-core/src/tools/network.rs`
- Modify: `crates/chibi-core/src/tools/index.rs`
- Modify: `crates/chibi-core/src/tools/flow.rs`
- Modify: `crates/chibi-core/src/tools/vfs_tools.rs`

This task adds a `register_*_tools()` function to each module. The function takes a `&mut ToolRegistry` plus whatever dependencies that module needs, constructs a `ToolHandler` closure capturing those dependencies, and registers each tool from the module's `*_TOOL_DEFS`.

Each module's registration function must match its `execute_*_tool()` signature. Study the execute function's parameters to determine what to capture. The key references:

- `memory.rs:129` — `execute_memory_tool(app, context_name, name, args, config)` — needs `AppState`, context_name (runtime), config (runtime)
- `fs_read.rs:240` — `execute_fs_read_tool(app, context_name, tool_name, args, config, project_root)` — needs `AppState`, context_name, config, project_root (all runtime)
- `fs_write.rs:117` — `execute_fs_write_tool(tool_name, args, project_root, config, vfs, caller)` — needs project_root, config, vfs, caller (runtime)
- `shell.rs:60` — `execute_shell_tool(tool_name, args, project_root)` — needs project_root (runtime)
- `network.rs:65` — `execute_network_tool(tool_name, args)` — no deps
- `index.rs:106` — `execute_index_tool(tool_name, args, project_root, config, tools)` — needs project_root, config, tools ref
- `flow.rs:461` — `execute_flow_tool(config, tool_name, args, tools)` — needs config, tools ref
- `vfs_tools.rs:246` — `execute_vfs_tool(vfs, caller, tool_name, args)` — needs vfs, caller (runtime)

**Important observation:** Many execute functions need runtime values (context_name, caller, config) that aren't known at registration time. The handler closure can't capture these — they must come through `ToolCall`.

`ToolCall` already carries `context: &ToolCallContext` as of Task 2. Handlers use `call.context.*` for all runtime values. Most builtins capture nothing at registration time.

**Step 1: Verify ToolCall and ToolCallContext are in scope**

`ToolCallContext` was introduced in Task 2. Confirm `AppState`, `ResolvedConfig`, `Vfs`, and `VfsCaller` imports resolve in `registry.rs`. No type changes needed here.

**Step 2: Add register functions to each module**

Pattern (using network as the simplest example):

```rust
// in network.rs
pub fn register_network_tools(registry: &mut ToolRegistry) {
    let handler: ToolHandler = Arc::new(|call| {
        Box::pin(async move {
            execute_network_tool(call.name, call.args)
                .await
                .unwrap_or_else(|| {
                    Err(io::Error::new(io::ErrorKind::NotFound,
                        format!("unknown network tool: {}", call.name)))
                })
        })
    });

    for def in NETWORK_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(def, handler.clone(), ToolCategory::Network));
    }
}
```

Add `Tool::from_builtin_def()` helper to reduce boilerplate across 8 modules:

```rust
impl Tool {
    pub fn from_builtin_def(
        def: &BuiltinToolDef,
        handler: ToolHandler,
        category: ToolCategory,
    ) -> Self {
        Self {
            name: def.name.to_string(),
            description: def.description.to_string(),
            parameters: def.to_json_schema(),
            path: PathBuf::new(),
            hooks: vec![],
            metadata: ToolMetadata::default(),
            summary_params: def.summary_params.iter().map(|s| s.to_string()).collect(),
            r#impl: ToolImpl::Builtin(handler),
            category,
        }
    }
}
```

For modules needing runtime context (most of them), the handler uses `call.context`:

```rust
// in fs_read.rs
pub fn register_fs_read_tools(registry: &mut ToolRegistry) {
    let handler: ToolHandler = Arc::new(|call| {
        Box::pin(async move {
            let ctx = call.context;
            execute_fs_read_tool(ctx.app, ctx.context_name, call.name, call.args, ctx.config, ctx.project_root)
                .unwrap_or_else(|| {
                    Err(io::Error::new(io::ErrorKind::NotFound,
                        format!("unknown fs_read tool: {}", call.name)))
                })
        })
    });
    for def in FS_READ_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(def, handler.clone(), ToolCategory::FsRead));
    }
}
```

Repeat for all 8 modules. Handle special cases:
- `memory.rs` — `execute_memory_tool` returns `Option<Result>`, unwrap with NotFound
- `flow.rs` — `all_flow_tools_to_api_format` takes `preset_capabilities` — handle during tool-list building, not registration
- `flow.rs` — `flow_tool_metadata` sets non-default `ToolMetadata` (flow_control, ends_turn) — set per-tool during registration
- `index.rs` — needs `tools: &[Tool]` — will need registry ref or tool list; defer to Phase 2 wiring

**Step 3: Write a test that registers all builtins and verifies count**

```rust
#[test]
fn test_register_all_builtins() {
    let mut reg = ToolRegistry::new();
    register_memory_tools(&mut reg);
    register_fs_read_tools(&mut reg);
    register_fs_write_tools(&mut reg);
    register_shell_tools(&mut reg);
    register_network_tools(&mut reg);
    register_index_tools(&mut reg);
    register_flow_tools(&mut reg);
    register_vfs_tools(&mut reg);

    // verify expected tool counts
    let total = reg.all().count();
    assert!(total > 20, "expected 20+ builtin tools, got {}", total);

    // verify categories are set
    assert!(reg.get("file_head").unwrap().category == ToolCategory::FsRead);
    assert!(reg.get("shell_exec").unwrap().category == ToolCategory::Shell);
    assert!(reg.get("fetch_url").unwrap().category == ToolCategory::Network);
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core -- tools::registry::tests`
Expected: PASS

**Step 5: Commit**

```
feat: add register_*_tools() to all builtin modules (#190)

each module registers its tools with typed ToolImpl::Builtin handlers
and ToolCategory. handlers receive runtime context via ToolCall.
```

---

### Task 6: Wire plugin and MCP dispatch into ToolRegistry

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs` (dispatch match arms)
- Modify: `crates/chibi-core/src/tools/plugins.rs` (adapt execute_tool)
- Modify: `crates/chibi-core/src/tools/mcp.rs` (adapt execute_mcp_tool)

**Step 1: Update registry dispatch to call existing plugin/MCP executors**

In `registry.rs`, update the `Plugin` and `Mcp` match arms:

```rust
ToolImpl::Plugin(path) => {
    super::plugins::execute_tool_by_path(path, args).await
}
ToolImpl::Mcp { server, tool_name } => {
    // needs chibi_dir from ToolCallContext
    super::mcp::execute_mcp_call(server, tool_name, args, call.context.app.chibi_dir()).await
}
```

Extract the core execution logic from `plugins::execute_tool` and `mcp::execute_mcp_tool` into standalone functions that don't need a `&Tool` (since the registry already has the path/server info in the `ToolImpl` variant).

**Step 2: Write tests**

Test plugin dispatch with a mock (or test that the error path works for a nonexistent plugin path). Test MCP dispatch similarly.

**Step 3: Run tests**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 4: Commit**

```
feat: wire plugin and MCP dispatch into ToolRegistry (#190)
```

---

### Task 7: Build registry at Chibi init, replace Chibi.tools

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs:89-99` (struct), `:160-209` (init)
- Modify: `crates/chibi-core/src/tools/mod.rs` (re-exports)

**Step 1: Replace `tools: Vec<Tool>` with `registry: Arc<RwLock<ToolRegistry>>` on Chibi**

Use `Arc<RwLock<>>` from the start — `ToolsBackend` (Phase 3) needs a shared reference
to the registry without requiring a mutable borrow of `Chibi`. Taking the lock at this
task costs nothing and avoids a type change in Task 14.

In `chibi.rs`:
```rust
pub struct Chibi {
    pub app: AppState,
    pub registry: Arc<RwLock<ToolRegistry>>,
    pub project_root: PathBuf,
    permission_handler: Option<PermissionHandler>,
}
```

**Step 2: Build registry in `load_with_options`**

Replace the tool loading section (lines 160-179) with:
```rust
let mut reg = ToolRegistry::new();

// register builtins
tools::register_memory_tools(&mut reg);
tools::register_fs_read_tools(&mut reg);
tools::register_fs_write_tools(&mut reg);
tools::register_shell_tools(&mut reg);
tools::register_network_tools(&mut reg);
tools::register_index_tools(&mut reg);
tools::register_flow_tools(&mut reg);
tools::register_vfs_tools(&mut reg);

// register plugins
for tool in tools::load_tools(&app.plugins_dir)? {
    reg.register(tool);
}

// register MCP tools
if let Ok(mcp_tools) = tools::mcp::load_mcp_tools(&app.chibi_dir) {
    for tool in mcp_tools {
        reg.register(tool);
    }
}

let registry = Arc::new(RwLock::new(reg));
```

**Step 3: Update all `self.tools` references in chibi.rs**

Search for `self.tools` and replace with `self.registry` equivalents:
- `find_tool(&self.tools, name)` → `self.registry.get(name)`
- `&self.tools` passed to functions → pass `&self.registry` or adapt

**Step 4: Run full test suite**

Run: `cargo test`
Expected: PASS (may need to fix compilation errors in other crates that reference `Chibi.tools`)

**Step 5: Commit**

```
feat: build ToolRegistry at Chibi init, replace Vec<Tool> (#190)
```

---

### Task 8: Replace dispatch in chibi.rs::execute_tool

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs:351-460`

**Step 1: Replace the if/else dispatch chain with registry.dispatch()**

The existing `execute_tool` method (lines 351-460) has the full if/else chain. Replace with:

```rust
pub async fn execute_tool(
    &self,
    context_name: &str,
    name: &str,
    args: serde_json::Value,
) -> io::Result<String> {
    // build runtime context
    let config = self.app.resolve_config(context_name)?;
    let call_ctx = ToolCallContext {
        app: &self.app,
        context_name,
        config: &config,
        project_root: &self.project_root,
        vfs: self.app.vfs(),
        vfs_caller: VfsCaller::Context(context_name),
    };
    self.registry.read().unwrap().dispatch_with_context(name, &args, &call_ctx).await
}
```

`dispatch_with_context` already exists on `ToolRegistry` from Task 4. By this task
`Chibi.registry` is `Arc<RwLock<ToolRegistry>>` (introduced in Task 7 — see note
there), so acquire a read lock before dispatching.

**Step 2: Run existing integration tests**

Run: `cargo test`
Expected: PASS

**Step 3: Commit**

```
refactor: replace chibi.rs dispatch chain with registry.dispatch (#190)
```

---

### Task 9: Replace dispatch in send.rs::execute_tool_pure

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:65-128` (remove ToolType, classify_tool_type)
- Modify: `crates/chibi-core/src/api/send.rs:940-1281` (execute_tool_pure)

**Step 1: Refactor execute_tool_pure to use registry + middleware pattern**

The middleware (hooks, permissions, caching) stays in `execute_tool_pure`. The actual tool execution delegates to `registry.dispatch()`. Replace the classify/match chain with:

```rust
let reg = registry.read().unwrap();
let tool = reg.get(tool_name).ok_or_else(|| ...)?;

// permission middleware based on tool.category
match tool.category {
    ToolCategory::FsRead => { /* PreFileRead hook gating */ }
    ToolCategory::FsWrite => { /* PreFileWrite hook gating */ }
    ToolCategory::Shell => { /* PreShellExec hook gating */ }
    ToolCategory::Network => { /* URL policy + PreFetchUrl gating */ }
    _ => {}
}

// dispatch (drop read guard first if dispatch needs to re-acquire)
drop(reg);
let result = registry.read().unwrap().dispatch_with_context(tool_name, args, &call_ctx).await?;
```

**Step 2: Remove ToolType enum and classify_tool_type()**

These are now fully replaced by `tool.category`.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: PASS

**Step 4: Commit**

```
refactor: replace send.rs dispatch chain with registry + middleware (#190)

removes ToolType enum, classify_tool_type(), and the if/else
dispatch chain. permission gating now routes on tool.category.
```

---

### Task 10: Remove is_*_tool() predicates, find_tool(), Tool.path

**Files:**
- Modify: all 8 tool modules (remove `is_*_tool()`)
- Modify: `crates/chibi-core/src/tools/plugins.rs` (remove `find_tool`)
- Modify: `crates/chibi-core/src/tools/mod.rs` (remove re-exports, remove `path` field)
- Modify: all Tool construction sites (remove `path` field)

**Step 1: Remove all is_*_tool() functions**

Search: `grep -rn 'is_.*_tool' crates/chibi-core/src/tools/`
Remove each function. Update any remaining callers to use `registry.get(name).map(|t| t.category)`.

**Step 2: Remove find_tool from plugins.rs**

No longer needed — `registry.get(name)` replaces it.

**Step 3: Remove Tool.path field**

Remove `pub path: PathBuf` from Tool struct. Fix all construction sites. Plugin path is now in `ToolImpl::Plugin(path)`. MCP server/tool_name is in `ToolImpl::Mcp { .. }`.

**Step 4: Remove stale re-exports from tools/mod.rs**

Remove re-exports of `find_tool`, `is_*_tool`, and `all_*_tools_to_api_format` functions that are no longer the primary interface.

**Step 5: Run full test suite**

Run: `cargo test`
Expected: PASS

**Step 6: Run linter**

Run: `just lint`
Expected: clean

**Step 7: Commit**

```
refactor: remove is_*_tool(), find_tool(), Tool.path (#190)

all tool identification now goes through ToolRegistry + ToolCategory.
the polymorphic path field is replaced by typed ToolImpl variants.
```

---

## Phase 3: VFS Tool Namespace

### Task 11: Add /tools/ zones to VFS permissions

**Files:**
- Modify: `crates/chibi-core/src/vfs/permissions.rs`

**Step 1: Write permission tests for /tools/ zones**

```rust
#[test]
fn test_tools_sys_read_allowed() {
    let path = VfsPath::new("/tools/sys/shell_exec").unwrap();
    assert!(check_read(VfsCaller::Context("any"), &path).is_ok());
}

#[test]
fn test_tools_sys_write_denied() {
    let path = VfsPath::new("/tools/sys/shell_exec").unwrap();
    assert!(check_write(VfsCaller::Context("any"), &path, None).is_err());
}

#[test]
fn test_tools_sys_system_write_denied() {
    // even SYSTEM can't write to /tools/sys/ — it's virtual
    let path = VfsPath::new("/tools/sys/anything").unwrap();
    assert!(check_write(VfsCaller::System, &path, None).is_err());
}

#[test]
fn test_tools_shared_writable() {
    let path = VfsPath::new("/tools/shared/my_tool.scm").unwrap();
    assert!(check_write(VfsCaller::Context("any"), &path, None).is_ok());
}

#[test]
fn test_tools_home_owner_writable() {
    let path = VfsPath::new("/tools/home/alice/my_tool.scm").unwrap();
    assert!(check_write(VfsCaller::Context("alice"), &path, None).is_ok());
    assert!(check_write(VfsCaller::Context("bob"), &path, None).is_err());
}

#[test]
fn test_tools_flocks_member_writable() {
    let mut reg = FlockRegistry::default();
    reg.add_member("devteam", "alice", "site:abc");
    let path = VfsPath::new("/tools/flocks/devteam/shared_tool.scm").unwrap();
    assert!(check_write(VfsCaller::Context("alice"), &path, Some((&reg, "abc"))).is_ok());
    assert!(check_write(VfsCaller::Context("bob"), &path, Some((&reg, "abc"))).is_err());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core -- vfs::permissions::tests`
Expected: FAIL

**Step 3: Add /tools/ zone rules to check_write**

In `permissions.rs`, add before the final deny:

```rust
// /tools/sys/ — never writable (virtual read-only)
if p == "/tools/sys" || p.starts_with("/tools/sys/") {
    return Err(io::Error::new(
        ErrorKind::PermissionDenied,
        format!("'{}' is read-only (virtual tool registry)", path),
    ));
}

// /tools/shared/ — world-writable
if p == "/tools/shared" || p.starts_with("/tools/shared/") {
    return Ok(());
}

// /tools/home/<ctx>/ — owner-writable
if let Some(rest) = p.strip_prefix("/tools/home/") {
    let owner = rest.split('/').next().unwrap_or("");
    if owner == name {
        return Ok(());
    }
    return Err(io::Error::new(
        ErrorKind::PermissionDenied,
        format!("context '{}' cannot write to /tools/home/{}/", name, owner),
    ));
}

// /tools/flocks/<name>/ — flock members only
if let Some(rest) = p.strip_prefix("/tools/flocks/") {
    let flock_name = rest.split('/').next().unwrap_or("");
    if !flock_name.is_empty() {
        if let Some((registry, site_id)) = flock_ctx
            && registry.is_member(flock_name, name, &site_flock_name(site_id))
        {
            return Ok(());
        }
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("context '{}' is not a member of flock '{}'", name, flock_name),
        ));
    }
}
```

Note: the `/tools/sys/` deny must come before the System early-return at the top of `check_write`. Move the System check down, or add the `/tools/sys/` check at the very top before the System bypass.

**Step 4: Run tests**

Run: `cargo test -p chibi-core -- vfs::permissions::tests`
Expected: PASS

**Step 5: Commit**

```
feat: add /tools/ zone permissions to VFS (#190)

/tools/sys/ is read-only (even for SYSTEM — virtual).
/tools/shared/ is world-writable.
/tools/home/<ctx>/ is owner-writable.
/tools/flocks/<name>/ follows flock membership.
```

---

### Task 12: Implement multi-backend VFS mounting

**Files:**
- Modify: `crates/chibi-core/src/vfs/vfs.rs`

The VFS currently wraps a single `Box<dyn VfsBackend>`. We need prefix-based routing so `/tools/sys/` can go to `ToolsBackend` while everything else goes to `LocalBackend`.

**Step 1: Write tests for multi-backend routing**

```rust
#[tokio::test]
async fn test_multi_backend_routing() {
    // setup: two LocalBackends mounted at different prefixes
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();
    let backend1 = LocalBackend::new(dir1.path().to_path_buf());
    let backend2 = LocalBackend::new(dir2.path().to_path_buf());

    let vfs = Vfs::builder("test-site-0000")
        .mount("/", Box::new(backend1))
        .mount("/tools/shared", Box::new(backend2))
        .build();

    // write to /tools/shared/ goes to backend2
    let path = VfsPath::new("/tools/shared/test.txt").unwrap();
    vfs.write(VfsCaller::System, &path, b"hello").await.unwrap();
    assert_eq!(vfs.read(VfsCaller::System, &path).await.unwrap(), b"hello");

    // write to /shared/ goes to backend1 (root mount)
    let path2 = VfsPath::new("/shared/other.txt").unwrap();
    vfs.write(VfsCaller::System, &path2, b"world").await.unwrap();
    assert_eq!(vfs.read(VfsCaller::System, &path2).await.unwrap(), b"world");
}
```

**Step 2: Run tests to verify they fail**

Expected: FAIL (Vfs::builder doesn't exist)

**Step 3: Implement multi-backend mounting**

Add a `VfsBuilder` and update `Vfs` to hold `Vec<(VfsPath, Box<dyn VfsBackend>)>` sorted by prefix length (longest first). The `resolve_backend` method does longest-prefix match and strips the mount prefix from the path before delegating.

Keep `Vfs::new()` as a convenience that mounts a single backend at `/`.

```rust
pub struct VfsBuilder {
    mounts: Vec<(VfsPath, Box<dyn VfsBackend>)>,
    site_id: String,
}

impl VfsBuilder {
    pub fn mount(mut self, prefix: &str, backend: Box<dyn VfsBackend>) -> Self {
        self.mounts.push((VfsPath::new(prefix).unwrap(), backend));
        self
    }

    pub fn build(mut self) -> Vfs {
        // sort by prefix length descending (longest prefix first)
        self.mounts.sort_by(|a, b| b.0.as_str().len().cmp(&a.0.as_str().len()));
        Vfs {
            mounts: self.mounts,
            site_id: self.site_id,
            registry_cache: RefCell::new(None),
        }
    }
}
```

Update all Vfs methods to use `resolve_backend()` instead of `self.backend`.

**Step 4: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: PASS (existing single-backend tests still work via `Vfs::new()`)

**Step 5: Commit**

```
feat: multi-backend VFS mounting with longest-prefix match (#190)

Vfs now supports mounting different backends at path prefixes.
Vfs::new() remains as convenience for single-backend usage.
Vfs::builder() enables multi-mount configurations.
```

---

### Task 13: Implement ToolsBackend for /tools/sys/

**Files:**
- Create: `crates/chibi-core/src/vfs/tools_backend.rs`
- Modify: `crates/chibi-core/src/vfs/mod.rs` (add module)

**Step 1: Write tests**

```rust
#[tokio::test]
async fn test_tools_backend_list_root() {
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    // register a test tool
    {
        let mut reg = registry.write().unwrap();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        reg.register(make_test_tool("shell_exec", ToolCategory::Shell, ToolImpl::Builtin(handler)));
    }
    let backend = ToolsBackend::new(Arc::clone(&registry));
    let root = VfsPath::new("/").unwrap();
    let entries = backend.list(&root).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "shell_exec");
}

#[tokio::test]
async fn test_tools_backend_read_tool_schema() {
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    {
        let mut reg = registry.write().unwrap();
        let handler: ToolHandler = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        let mut tool = make_test_tool("my_tool", ToolCategory::Network, ToolImpl::Builtin(handler));
        tool.description = "fetch a URL".into();
        reg.register(tool);
    }
    let backend = ToolsBackend::new(Arc::clone(&registry));
    let path = VfsPath::new("/my_tool").unwrap();
    let data = backend.read(&path).await.unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&data).unwrap();
    assert_eq!(schema["name"], "my_tool");
    assert_eq!(schema["description"], "fetch a URL");
    assert_eq!(schema["category"], "network");
}

#[tokio::test]
async fn test_tools_backend_write_rejected() {
    let registry = Arc::new(RwLock::new(ToolRegistry::new()));
    let backend = ToolsBackend::new(registry);
    let path = VfsPath::new("/anything").unwrap();
    let err = backend.write(&path, b"nope").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
}
```

**Step 2: Run tests to verify they fail**

Expected: FAIL

**Step 3: Implement ToolsBackend**

```rust
use std::sync::{Arc, RwLock};

pub struct ToolsBackend {
    registry: Arc<RwLock<ToolRegistry>>,
}

impl ToolsBackend {
    pub fn new(registry: Arc<RwLock<ToolRegistry>>) -> Self {
        Self { registry }
    }
}

impl VfsBackend for ToolsBackend {
    fn read<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            let name = path.as_str().trim_start_matches('/');
            let reg = self.registry.read().map_err(|_| io::Error::other("lock"))?;
            let tool = reg.get(name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no tool: {name}"))
            })?;
            let schema = serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "category": format!("{:?}", tool.category).to_lowercase(),
                "parameters": tool.parameters,
            });
            Ok(serde_json::to_vec_pretty(&schema).unwrap())
        })
    }

    fn list<'a>(&'a self, path: &'a VfsPath) -> BoxFuture<'a, io::Result<Vec<VfsEntry>>> {
        Box::pin(async move {
            let reg = self.registry.read().map_err(|_| io::Error::other("lock"))?;
            Ok(reg.all().map(|t| VfsEntry {
                name: t.name.clone(),
                kind: VfsEntryKind::File,
            }).collect())
        })
    }

    fn write<'a>(&'a self, _: &'a VfsPath, _: &'a [u8]) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async { Err(io::Error::new(io::ErrorKind::PermissionDenied, "/tools/sys/ is read-only")) })
    }

    // ... other methods return PermissionDenied or NotSupported
}
```

**Step 4: Run tests**

Run: `cargo test -p chibi-core -- vfs::tools_backend::tests`
Expected: PASS

**Step 5: Commit**

```
feat: ToolsBackend — virtual VFS backend for /tools/sys/ (#190)

synthesises tool schema JSON on read, directory listings from
the registry. all write operations rejected (read-only).
```

---

### Task 14: Mount /tools/ backends at Chibi init

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs` (VFS init)
- Modify: wherever the `Vfs` is currently constructed

**Step 1: Update VFS construction to use builder with mounts**

`Chibi.registry` is already `Arc<RwLock<ToolRegistry>>` from Task 7. Pass a clone
of that Arc to `ToolsBackend`:

```rust
// registry is already Arc<RwLock<ToolRegistry>> from the init block above

let vfs = Vfs::builder(&site_id)
    .mount("/", Box::new(LocalBackend::new(vfs_root)))
    .mount("/tools/sys", Box::new(ToolsBackend::new(Arc::clone(&registry))))
    .build();
```

No type change required — this task is purely about wiring the mount.

**Step 2: Verify VFS tool browsing works end-to-end**

Write an integration test or manually verify:
```
vfs_list vfs:///tools/sys/
file_head vfs:///tools/sys/shell_exec
```

**Step 3: Run full test suite**

Run: `cargo test`
Expected: PASS

**Step 4: Commit**

```
feat: mount /tools/sys/ backend at VFS init (#190)

tools are now browsable via vfs_list and file_head on
vfs:///tools/sys/. registry shared via Arc<RwLock<>>.
```

---

## Phase 4: Synthesised Tools (tein integration)

> **Before starting Phase 4:** Verify tein's public API matches the assumptions in
> Tasks 15-17. Run:
> ```
> cargo doc --open -p tein
> ```
> and confirm these types/methods exist: `Context::builder()`, `Modules::Safe`,
> `ThreadLocalContext`, `step_limit`, `ctx.call(fn, &[args])`, `Value::as_string()`,
> `Value::is_procedure()`. If the API differs, update Tasks 16-17 before writing any
> code. This is the riskiest phase — a mis-assumed API means rewriting handlers.

### Task 15: Add tein dependency (feature-gated)

**Files:**
- Modify: `crates/chibi-core/Cargo.toml`

**Step 1: Add tein as optional dependency**

```toml
[features]
default = ["synthesised-tools"]
synthesised-tools = ["tein"]

[dependencies]
tein = { git = "https://github.com/emesal/tein", branch = "main", optional = true }
```

**Step 2: Verify it compiles with and without the feature**

Run: `cargo check -p chibi-core`
Run: `cargo check -p chibi-core --no-default-features`
Expected: both compile

**Step 3: Commit**

```
feat: add tein as optional dependency for synthesised tools (#190)
```

---

### Task 16: Add ToolImpl::Synthesised variant

**Files:**
- Modify: `crates/chibi-core/src/tools/registry.rs`

**Step 1: Add the variant behind cfg**

```rust
pub enum ToolImpl {
    Builtin(ToolHandler),
    Plugin(PathBuf),
    Mcp { server: String, tool_name: String },
    #[cfg(feature = "synthesised-tools")]
    Synthesised {
        vfs_path: VfsPath,
        context: Arc<tein::ThreadLocalContext>,
    },
}
```

**Step 2: Update dispatch match**

```rust
#[cfg(feature = "synthesised-tools")]
ToolImpl::Synthesised { context, .. } => {
    super::synthesised::execute_synthesised(context, &call).await
}
```

**Step 3: Commit**

```
feat: add ToolImpl::Synthesised variant (#190)
```

---

### Task 17: Implement synthesised tool loader

**Files:**
- Create: `crates/chibi-core/src/tools/synthesised.rs`

**Step 1: Write tests for scheme tool loading**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_TOOL: &str = r#"
(define tool-name "word_count")
(define tool-description "count words in text")
(define tool-parameters
  '((text . ((type . "string") (description . "text to count")))))
(define (tool-execute args)
  (let ((text (cdr (assoc "text" args))))
    (number->string (length (string-split text #\space)))))
"#;

    #[test]
    fn test_load_synthesised_tool_schema() {
        let tool = load_tool_from_source(SIMPLE_TOOL, &VfsPath::new("/tools/shared/word_count.scm").unwrap()).unwrap();
        assert_eq!(tool.name, "word_count");
        assert_eq!(tool.description, "count words in text");
        assert_eq!(tool.category, ToolCategory::Synthesised);
    }

    #[tokio::test]
    async fn test_execute_synthesised_tool() {
        let tool = load_tool_from_source(SIMPLE_TOOL, &VfsPath::new("/tools/shared/word_count.scm").unwrap()).unwrap();
        let args = serde_json::json!({"text": "hello world foo"});
        if let ToolImpl::Synthesised { ref context, .. } = tool.r#impl {
            let result = execute_synthesised(context, &ToolCall {
                name: "word_count",
                args: &args,
                context: &test_call_context(),
            }).await.unwrap();
            assert_eq!(result, "3");
        } else {
            panic!("expected Synthesised impl");
        }
    }

    #[test]
    fn test_load_tool_missing_bindings() {
        let bad = "(define tool-name \"oops\")"; // missing other bindings
        let result = load_tool_from_source(bad, &VfsPath::new("/tools/shared/bad.scm").unwrap());
        assert!(result.is_err());
    }
}
```

**Step 2: Run tests to verify they fail**

Expected: FAIL

**Step 3: Implement the loader**

```rust
#[cfg(feature = "synthesised-tools")]
use tein::{Context, Value, ThreadLocalContext, sandbox::Modules};
use std::sync::Arc;

/// Load a synthesised tool from scheme source.
pub fn load_tool_from_source(source: &str, vfs_path: &VfsPath) -> io::Result<Tool> {
    // create sandboxed tein context
    let ctx = Context::builder()
        .standard_env()
        .sandboxed(Modules::Safe)
        .step_limit(100_000)
        .build_managed(|ctx| {
            ctx.evaluate(source).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("scheme eval error: {e}"))
            })?;
            Ok(())
        })
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tein init error: {e}")))?;

    // extract bindings
    let name = extract_string(&ctx, "tool-name")?;
    let description = extract_string(&ctx, "tool-description")?;
    let params_val = ctx.evaluate("tool-parameters")
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("missing tool-parameters: {e}")))?;
    let parameters = params_alist_to_json_schema(&params_val)?;

    // verify tool-execute is a procedure
    let exec_val = ctx.evaluate("tool-execute")
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("missing tool-execute: {e}")))?;
    if !exec_val.is_procedure() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "tool-execute is not a procedure"));
    }

    let context = Arc::new(ctx);

    Ok(Tool {
        name: name.clone(),
        description,
        parameters,
        hooks: vec![],
        metadata: ToolMetadata::default(),
        summary_params: vec![],
        r#impl: ToolImpl::Synthesised {
            vfs_path: vfs_path.clone(),
            context,
        },
        category: ToolCategory::Synthesised,
    })
}

/// Execute a synthesised tool.
pub async fn execute_synthesised(
    context: &ThreadLocalContext,
    call: &ToolCall<'_>,
) -> io::Result<String> {
    let args_alist = json_args_to_scheme_alist(call.args);
    let exec_fn = context.evaluate("tool-execute")
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("resolve tool-execute: {e}")))?;
    let result = context.call(&exec_fn, &[args_alist])
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("tool execution error: {e}")))?;
    match result.as_string() {
        Some(s) => Ok(s.to_string()),
        None => Ok(result.to_string()),
    }
}

// helper: extract a string binding from the context
fn extract_string(ctx: &ThreadLocalContext, name: &str) -> io::Result<String> {
    let val = ctx.evaluate(name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}: {e}")))?;
    val.as_string()
        .map(|s| s.to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{name} is not a string")))
}

// helper: convert scheme params alist to JSON schema
fn params_alist_to_json_schema(val: &Value) -> io::Result<serde_json::Value> {
    // convert ((name . ((type . "string") ...)) ...) to JSON schema
    // implementation details depend on tein's Value layout
    todo!("implement alist → JSON schema conversion")
}

// helper: convert JSON args to scheme alist
fn json_args_to_scheme_alist(args: &serde_json::Value) -> Value {
    // convert {"key": "value", ...} to ((key . value) ...)
    todo!("implement JSON → scheme alist conversion")
}
```

**Step 4: Implement the conversion helpers and run tests**

Run: `cargo test -p chibi-core -- tools::synthesised::tests`
Expected: PASS

**Step 5: Commit**

```
feat: synthesised tool loader — scheme source → registered tool (#190)

loads .scm files into sandboxed tein contexts, extracts tool-name,
tool-description, tool-parameters, tool-execute bindings.
convention-based: homoiconic schema as scheme data.
```

---

### Task 18: Scan writable VFS zones on startup

**Files:**
- Modify: `crates/chibi-core/src/chibi.rs` (init)
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

**Step 1: Implement scan function**

```rust
/// Scan VFS zones for .scm files and register synthesised tools.
pub async fn scan_and_register(
    vfs: &Vfs,
    registry: &mut ToolRegistry,
) -> io::Result<()> {
    let zones = ["/tools/shared"];
    // also scan /tools/home/*/ and /tools/flocks/*/
    for zone in &zones {
        let zone_path = VfsPath::new(zone)?;
        if !vfs.exists(VfsCaller::System, &zone_path).await.unwrap_or(false) {
            continue;
        }
        let entries = vfs.list(VfsCaller::System, &zone_path).await?;
        for entry in entries {
            if entry.name.ends_with(".scm") {
                let file_path = VfsPath::new(&format!("{}/{}", zone, entry.name))?;
                let source = vfs.read(VfsCaller::System, &file_path).await?;
                let source_str = String::from_utf8(source).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, e)
                })?;
                match load_tool_from_source(&source_str, &file_path) {
                    Ok(tool) => {
                        log::info!("registered synthesised tool: {}", tool.name);
                        registry.register(tool);
                    }
                    Err(e) => {
                        log::warn!("failed to load {}: {}", file_path, e);
                    }
                }
            }
        }
    }
    Ok(())
}
```

**Step 2: Call from Chibi init after registry and VFS are constructed**

**Step 3: Write integration test**

Write a `.scm` file to the VFS shared zone, then verify the tool appears in the registry and can be dispatched.

**Step 4: Run tests**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 5: Commit**

```
feat: scan VFS zones for synthesised tools on startup (#190)
```

---

### Task 19: Hot-reload on VFS writes

**Files:**
- Modify: `crates/chibi-core/src/vfs/vfs.rs` (post-write callback)
- Modify: `crates/chibi-core/src/tools/synthesised.rs`

**Step 1: Add a post-write hook mechanism to Vfs**

When a write completes to `/tools/shared/`, `/tools/home/*/`, or `/tools/flocks/*/` and the file ends in `.scm`, trigger a reload callback.

Options:
- A closure stored on Vfs: `on_tool_write: Option<Box<dyn Fn(&VfsPath) + Send + Sync>>`
- A channel-based notification

Use the closure approach for simplicity. The closure calls into the synthesised module to reload the tool.

**Step 2: Implement reload logic**

```rust
pub fn reload_tool(
    vfs: &Vfs,
    registry: &Arc<RwLock<ToolRegistry>>,
    path: &VfsPath,
) {
    // read source, load tool, register (replacing old if exists)
}
```

**Step 3: Handle deletion**

When a `.scm` file is deleted, unregister the tool. This requires either:
- Tracking vfs_path → tool_name mapping
- Or deriving tool name from the path (filename minus `.scm`)

Use the convention: filename (minus `.scm`) is checked against registered tools' `vfs_path`.

**Step 4: Write tests**

Test: write a .scm → tool appears. Overwrite → tool updated. Delete → tool gone.

**Step 5: Run tests**

Run: `cargo test -p chibi-core`
Expected: PASS

**Step 6: Commit**

```
feat: hot-reload synthesised tools on VFS writes (#190)

.scm files in writable /tools/ zones are automatically loaded,
updated, or unregistered when written or deleted.
```

---

## Deferred: Synthesised Tool Visibility Scoping

After Phase 4, all synthesised tools are globally visible in the registry — a context
can see tools from another context's `/tools/home/` zone. The design doc explicitly
defers this: "the registry stays dumb, visibility is policy."

A future task should add scoping to `send.rs`'s tool-list building: filter out
`ToolCategory::Synthesised` tools whose `ToolImpl::Synthesised { vfs_path, .. }`
falls outside the active context's accessible zones (checked against VFS permissions).
This is a `send.rs` / middleware concern, not a registry concern.

Track as a follow-up issue before merging Phase 4 to main.

---

## Phase 5: Cleanup and Documentation

### Task 20: Remove all_*_tools_to_api_format() functions

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (tool list building)
- Modify: all tool modules

**Step 1: Replace tool-list building in send.rs**

The current code (lines 2067-2100) calls `all_*_tools_to_api_format()` per module. Replace with:

```rust
// registry: &Arc<RwLock<ToolRegistry>> accessible via chibi or send.rs context
let all_tools: Vec<serde_json::Value> = registry.read().unwrap()
    .all()
    .map(tool_to_api_format)
    .collect();
```

Add `tool_to_api_format(tool: &Tool) -> serde_json::Value` that serialises a Tool for the API.

**Step 2: Remove the per-module `all_*_tools_to_api_format()` functions**

They're no longer called.

**Step 3: Run tests**

Run: `cargo test`
Expected: PASS

**Step 4: Commit**

```
refactor: unified tool-to-API serialisation from registry (#190)

removes 8 per-module all_*_tools_to_api_format() functions.
tool list for LLM now built from registry.all() with a single
serialisation function.
```

---

### Task 21: Update documentation

**Files:**
- Modify: `docs/architecture.md`
- Modify: `docs/plugins.md`
- Modify: `docs/vfs.md`
- Modify: `AGENTS.md`

**Step 1: Update architecture.md**

- Add ToolRegistry to the architecture overview
- Document the `/tools/` VFS namespace
- Update the tool dispatch flow description

**Step 2: Update plugins.md**

- Document how plugin tools are registered in the registry
- Document synthesised tools: authoring, conventions, lifecycle

**Step 3: Update vfs.md**

- Document `/tools/` zones and their permissions
- Document `ToolsBackend` for `/tools/sys/`

**Step 4: Update AGENTS.md**

- Add any quirks or gotchas discovered during implementation
- Update the architecture section if needed

**Step 5: Commit**

```
docs: update architecture, plugins, vfs docs for ToolRegistry (#190)
```

---

### Task 22: Final verification and lint

**Step 1: Run full test suite**

Run: `cargo test`
Expected: all pass

**Step 2: Run linter**

Run: `just lint`
Expected: clean

**Step 3: Run without synthesised-tools feature**

Run: `cargo test -p chibi-core --no-default-features`
Expected: all pass (synthesised tests skipped)

**Step 4: Commit any fixes**

**Step 5: Run `just pre-push`**

---

## Notes for AGENTS.md

Collect during implementation:
- Any quirks with `Arc<RwLock<ToolRegistry>>` sharing patterns
- ToolCall lifetime constraints
- tein context threading model gotchas
- VFS multi-backend mount prefix stripping behaviour
