//! Hook system for plugin lifecycle events.
//!
//! Hooks allow plugins to be notified at specific points during chibi's execution,
//! such as before/after messages, tool calls, context switches, and compaction.

use super::Tool;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use strum::{AsRefStr, EnumIter, EnumString};

#[cfg(feature = "synthesised-tools")]
use std::cell::RefCell;
#[cfg(feature = "synthesised-tools")]
use std::collections::HashSet;
#[cfg(feature = "synthesised-tools")]
use std::sync::{Arc, RwLock};

// Tracks which hook points are currently being dispatched to tein callbacks.
// Prevents re-entrancy: if a tein hook callback triggers an action that fires
// the same hook point, tein callbacks are skipped on the recursive call.
// Subprocess hooks still fire normally regardless.
#[cfg(feature = "synthesised-tools")]
thread_local! {
    static TEIN_HOOK_GUARD: RefCell<HashSet<HookPoint>> = RefCell::new(HashSet::new());
}

/// Hook points where tools can register to be called
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, EnumIter, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum HookPoint {
    PreMessage,
    PostMessage,
    PreTool,
    PostTool,
    PreToolOutput,  // Before tool output is processed (can modify/block output)
    PostToolOutput, // After tool output is processed (observe only)
    PreClear,
    PostClear,
    PreCompact,
    PostCompact,
    PreRollingCompact,
    PostRollingCompact,
    OnStart,
    OnEnd,
    PreSystemPrompt,  // Can inject content before system prompt sections
    PostSystemPrompt, // Can inject content after all system prompt sections
    PreSendMessage,   // Can intercept delivery (return {"delivered": true, "via": "..."})
    PostSendMessage,  // Observe delivery (read-only)
    PreCacheOutput,   // Before caching large tool output (can provide custom summary)
    PostCacheOutput,  // After output is cached (notification only)
    PreApiTools,      // Before tools are sent to API (can filter tools)
    PreApiRequest,    // Before API request is sent (can modify full request body)
    PreAgenticLoop,   // Before entering the tool loop (can override fallback)
    PostToolBatch,    // After processing a batch of tool calls (can override fallback)
    PreFileRead, // Before reading a file outside allowed paths (can approve/deny, fail-safe deny)
    PreFileWrite, // Before file write/patch (can approve/deny/modify operation)
    PreShellExec, // Before shell command execution (can approve/deny, fail-safe deny)
    PreFetchUrl, // Before fetching a sensitive URL (can approve/deny, fail-safe deny)
    PreSpawnAgent, // Before sub-agent call (can intercept/replace with {"response": "..."} or block)
    PostSpawnAgent, // After sub-agent call (observe only)
    PostIndexFile, // After a file is indexed (observe: path, lang, symbol_count, ref_count)
    PreVfsWrite,   // Before a VFS file write (advisory, non-blocking; observe-and-snapshot)
    PostVfsWrite,  // After a successful VFS file write (observe only)
}

// --- hook metadata for discoverability ---

/// Describes one field in a hook's payload or return value.
#[cfg(feature = "synthesised-tools")]
pub(crate) struct FieldMeta {
    pub name: &'static str,
    /// Schematic type string: "string", "number", "bool", "object", "array"
    pub typ: &'static str,
    pub description: &'static str,
}

/// Metadata for one hook point — the canonical single source of truth for
/// hook contracts. Used to generate `hooks-docs` (scheme alist) and
/// `docs/hooks.md` (markdown reference section).
#[cfg(feature = "synthesised-tools")]
pub(crate) struct HookMeta {
    pub point: HookPoint,
    /// Grouping category: "session", "message", "system_prompt", "tool",
    /// "api", "agentic", "file_permission", "url_security", "agent",
    /// "cache", "message_delivery", "index", "vfs_write", "context"
    pub category: &'static str,
    /// One-line description of when the hook fires.
    pub description: &'static str,
    /// Whether the hook callback can return values that modify behaviour.
    pub can_modify: bool,
    pub payload_fields: &'static [FieldMeta],
    /// Empty slice for observe-only hooks.
    pub return_fields: &'static [FieldMeta],
    /// Extra context or caveats; empty string if none.
    pub notes: &'static str,
}

#[cfg(feature = "synthesised-tools")]
pub(crate) const HOOK_METADATA: &[HookMeta] = &[
    HookMeta {
        point: HookPoint::OnStart,
        category: "session",
        description: "fires when chibi starts, before any processing",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "chibi_home",
                typ: "string",
                description: "chibi home directory path",
            },
            FieldMeta {
                name: "project_root",
                typ: "string",
                description: "project root directory path",
            },
            FieldMeta {
                name: "tool_count",
                typ: "number",
                description: "number of loaded tools",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::OnEnd,
        category: "session",
        description: "fires when chibi exits, after all processing",
        can_modify: false,
        payload_fields: &[],
        return_fields: &[],
        notes: "receives empty payload",
    },
    HookMeta {
        point: HookPoint::PreMessage,
        category: "message",
        description: "fires before sending a prompt to the LLM",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "prompt",
                typ: "string",
                description: "the user's prompt",
            },
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
        ],
        return_fields: &[FieldMeta {
            name: "prompt",
            typ: "string",
            description: "modified prompt",
        }],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostMessage,
        category: "message",
        description: "fires after receiving the LLM response",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "prompt",
                typ: "string",
                description: "original prompt",
            },
            FieldMeta {
                name: "response",
                typ: "string",
                description: "LLM's response",
            },
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreSystemPrompt,
        category: "system_prompt",
        description: "fires before building the system prompt; can inject content",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
            FieldMeta {
                name: "flock_goals",
                typ: "array",
                description: "array of {flock, goals} objects",
            },
        ],
        return_fields: &[FieldMeta {
            name: "inject",
            typ: "string",
            description: "content to add to system prompt",
        }],
        notes: "flock_goals replaced the old goals field; todos field removed (use VFS task files)",
    },
    HookMeta {
        point: HookPoint::PostSystemPrompt,
        category: "system_prompt",
        description: "fires after building the system prompt; can inject content",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
            FieldMeta {
                name: "flock_goals",
                typ: "array",
                description: "array of {flock, goals} objects",
            },
        ],
        return_fields: &[FieldMeta {
            name: "inject",
            typ: "string",
            description: "content to add to system prompt",
        }],
        notes: "same payload/return as pre_system_prompt",
    },
    HookMeta {
        point: HookPoint::PreTool,
        category: "tool",
        description: "fires before executing a tool; can modify arguments or block",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "name of the tool being called",
            },
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "tool arguments object",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "modified arguments",
            },
            FieldMeta {
                name: "block",
                typ: "bool",
                description: "set true to block execution",
            },
            FieldMeta {
                name: "message",
                typ: "string",
                description: "message shown when blocked",
            },
        ],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostTool,
        category: "tool",
        description: "fires after executing a tool; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "name of the tool that ran",
            },
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "tool arguments object",
            },
            FieldMeta {
                name: "result",
                typ: "string",
                description: "tool output",
            },
            FieldMeta {
                name: "cached",
                typ: "bool",
                description: "true if output was cached due to size",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreToolOutput,
        category: "tool",
        description: "fires after tool returns, before caching decisions; can modify or block output",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "name of the tool that ran",
            },
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "tool arguments object",
            },
            FieldMeta {
                name: "output",
                typ: "string",
                description: "raw tool output",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "output",
                typ: "string",
                description: "modified output",
            },
            FieldMeta {
                name: "block",
                typ: "bool",
                description: "set true to replace output entirely",
            },
            FieldMeta {
                name: "message",
                typ: "string",
                description: "replacement message shown to LLM when blocked",
            },
        ],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostToolOutput,
        category: "tool",
        description: "fires after tool output processing and caching; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "name of the tool that ran",
            },
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "tool arguments object",
            },
            FieldMeta {
                name: "output",
                typ: "string",
                description: "original output after pre_tool_output modifications",
            },
            FieldMeta {
                name: "final_output",
                typ: "string",
                description: "what the LLM will see (may be truncated if cached)",
            },
            FieldMeta {
                name: "cached",
                typ: "bool",
                description: "true if output was cached",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreApiTools,
        category: "api",
        description: "fires before tools are sent to the API; can filter tools",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "tools",
                typ: "array",
                description: "array of {name, type} tool objects",
            },
            FieldMeta {
                name: "fuel_remaining",
                typ: "number",
                description: "remaining tool-call budget",
            },
            FieldMeta {
                name: "fuel_total",
                typ: "number",
                description: "total fuel budget",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "exclude",
                typ: "array",
                description: "tool names to remove (union across hooks)",
            },
            FieldMeta {
                name: "include",
                typ: "array",
                description: "allowlist: only these tools remain (intersection across hooks)",
            },
        ],
        notes: "include/exclude are mutually exclusive per response; excludes union, includes intersect across multiple hooks",
    },
    HookMeta {
        point: HookPoint::PreApiRequest,
        category: "api",
        description: "fires after tool filtering, before HTTP request; can modify request body",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "request_body",
                typ: "object",
                description: "full request body (model, messages, tools, etc.)",
            },
            FieldMeta {
                name: "fuel_remaining",
                typ: "number",
                description: "remaining tool-call budget",
            },
            FieldMeta {
                name: "fuel_total",
                typ: "number",
                description: "total fuel budget",
            },
        ],
        return_fields: &[FieldMeta {
            name: "request_body",
            typ: "object",
            description: "fields to merge into request body (partial override)",
        }],
        notes: "returned fields are merged, not replaced; cache_prompt and exclude_from_output are chibi-internal field names",
    },
    HookMeta {
        point: HookPoint::PreAgenticLoop,
        category: "agentic",
        description: "fires before each agentic loop iteration; can override fallback and fuel",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "fuel_remaining",
                typ: "number",
                description: "remaining tool-call budget",
            },
            FieldMeta {
                name: "fuel_total",
                typ: "number",
                description: "total fuel budget",
            },
            FieldMeta {
                name: "current_fallback",
                typ: "string",
                description: "current fallback target (call_agent or call_user)",
            },
            FieldMeta {
                name: "message",
                typ: "string",
                description: "user message for this loop",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "fallback",
                typ: "string",
                description: "override fallback: call_agent or call_user",
            },
            FieldMeta {
                name: "fuel",
                typ: "number",
                description: "set fuel_remaining to this value",
            },
        ],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostToolBatch,
        category: "agentic",
        description: "fires after processing a batch of tool calls; can override fallback and adjust fuel",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "fuel_remaining",
                typ: "number",
                description: "remaining tool-call budget",
            },
            FieldMeta {
                name: "fuel_total",
                typ: "number",
                description: "total fuel budget",
            },
            FieldMeta {
                name: "current_fallback",
                typ: "string",
                description: "current fallback target",
            },
            FieldMeta {
                name: "tool_calls",
                typ: "array",
                description: "array of {name, arguments} for tools that ran",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "fallback",
                typ: "string",
                description: "override fallback: call_agent or call_user",
            },
            FieldMeta {
                name: "fuel_delta",
                typ: "number",
                description: "adjust fuel by this amount (positive adds, negative consumes, saturating)",
            },
        ],
        notes: "post_tool_batch output > pre_agentic_loop output > config fallback; last hook to set fallback wins",
    },
    HookMeta {
        point: HookPoint::PreFileRead,
        category: "file_permission",
        description: "fires before reading a file outside allowed paths; deny-only permission protocol",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "file_head, file_tail, or file_lines",
            },
            FieldMeta {
                name: "path",
                typ: "string",
                description: "absolute path being read",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "denied",
                typ: "bool",
                description: "set true to block the read",
            },
            FieldMeta {
                name: "reason",
                typ: "string",
                description: "reason shown when denied",
            },
        ],
        notes: "fail-safe deny if no handler; empty {} response falls through to frontend handler",
    },
    HookMeta {
        point: HookPoint::PreFileWrite,
        category: "file_permission",
        description: "fires before write_file or file_edit; deny-only permission protocol",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "write_file or file_edit",
            },
            FieldMeta {
                name: "path",
                typ: "string",
                description: "absolute path being written",
            },
            FieldMeta {
                name: "content",
                typ: "string",
                description: "file content (null for file_edit)",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "denied",
                typ: "bool",
                description: "set true to block the write",
            },
            FieldMeta {
                name: "reason",
                typ: "string",
                description: "reason shown when denied",
            },
        ],
        notes: "fail-safe deny if no permission handler configured",
    },
    HookMeta {
        point: HookPoint::PreShellExec,
        category: "file_permission",
        description: "fires before shell_exec; deny-only permission protocol",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "shell_exec",
            },
            FieldMeta {
                name: "command",
                typ: "string",
                description: "shell command string",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "denied",
                typ: "bool",
                description: "set true to block execution",
            },
            FieldMeta {
                name: "reason",
                typ: "string",
                description: "reason shown when denied",
            },
        ],
        notes: "same deny-only protocol as pre_file_read and pre_file_write",
    },
    HookMeta {
        point: HookPoint::PreFetchUrl,
        category: "url_security",
        description: "fires before fetching a sensitive URL or invoking a network-category tool without a URL; deny-only",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "name of the tool making the network call",
            },
            FieldMeta {
                name: "url",
                typ: "string",
                description: "URL being fetched (absent when safety is \"no_url\")",
            },
            FieldMeta {
                name: "safety",
                typ: "string",
                description: "\"sensitive\" for URL-based calls, \"no_url\" for network tools without a URL parameter",
            },
            FieldMeta {
                name: "reason",
                typ: "string",
                description: "classification reason (absent when safety is \"no_url\")",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "human-readable summary from summary_params (present only when safety is \"no_url\")",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "denied",
                typ: "bool",
                description: "set true to block the fetch",
            },
            FieldMeta {
                name: "reason",
                typ: "string",
                description: "reason shown when denied",
            },
        ],
        notes: "only fires when no url_policy is configured; url_policy is authoritative when set",
    },
    HookMeta {
        point: HookPoint::PreSpawnAgent,
        category: "agent",
        description: "fires before a sub-agent LLM call; can intercept/replace or block",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "system_prompt",
                typ: "string",
                description: "system prompt for sub-agent",
            },
            FieldMeta {
                name: "input",
                typ: "string",
                description: "input content to process",
            },
            FieldMeta {
                name: "model",
                typ: "string",
                description: "model identifier",
            },
            FieldMeta {
                name: "temperature",
                typ: "number",
                description: "sampling temperature",
            },
            FieldMeta {
                name: "max_tokens",
                typ: "number",
                description: "max tokens for response",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "response",
                typ: "string",
                description: "pre-computed response to use instead of LLM call",
            },
            FieldMeta {
                name: "block",
                typ: "bool",
                description: "set true to block the sub-agent call",
            },
            FieldMeta {
                name: "message",
                typ: "string",
                description: "message shown when blocked",
            },
        ],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostSpawnAgent,
        category: "agent",
        description: "fires after sub-agent returns; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "system_prompt",
                typ: "string",
                description: "system prompt used",
            },
            FieldMeta {
                name: "input",
                typ: "string",
                description: "input content",
            },
            FieldMeta {
                name: "model",
                typ: "string",
                description: "model identifier",
            },
            FieldMeta {
                name: "response",
                typ: "string",
                description: "sub-agent's response",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreCacheOutput,
        category: "cache",
        description: "fires before caching a large tool output; can provide custom summary",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "tool whose output is being cached",
            },
            FieldMeta {
                name: "arguments",
                typ: "object",
                description: "tool arguments",
            },
            FieldMeta {
                name: "content",
                typ: "string",
                description: "full output content",
            },
            FieldMeta {
                name: "char_count",
                typ: "number",
                description: "character count of content",
            },
            FieldMeta {
                name: "line_count",
                typ: "number",
                description: "line count of content",
            },
        ],
        return_fields: &[FieldMeta {
            name: "summary",
            typ: "string",
            description: "custom summary to show LLM instead of full content",
        }],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostCacheOutput,
        category: "cache",
        description: "fires after output is cached; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "tool whose output was cached",
            },
            FieldMeta {
                name: "cache_id",
                typ: "string",
                description: "filename under vfs:///sys/tool_cache/<context>/",
            },
            FieldMeta {
                name: "output_size",
                typ: "number",
                description: "size of cached output in bytes",
            },
            FieldMeta {
                name: "preview_size",
                typ: "number",
                description: "size of preview shown to LLM",
            },
        ],
        return_fields: &[],
        notes: "access cached content with file_head/file_tail/file_lines using full vfs:// URI",
    },
    HookMeta {
        point: HookPoint::PreSendMessage,
        category: "message_delivery",
        description: "fires before delivering an inter-context message; can claim delivery",
        can_modify: true,
        payload_fields: &[
            FieldMeta {
                name: "from",
                typ: "string",
                description: "sending context name",
            },
            FieldMeta {
                name: "to",
                typ: "string",
                description: "recipient context name",
            },
            FieldMeta {
                name: "content",
                typ: "string",
                description: "message content",
            },
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
        ],
        return_fields: &[
            FieldMeta {
                name: "delivered",
                typ: "bool",
                description: "set true to claim delivery was handled",
            },
            FieldMeta {
                name: "via",
                typ: "string",
                description: "delivery mechanism name (for logging)",
            },
        ],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostSendMessage,
        category: "message_delivery",
        description: "fires after message delivery; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "from",
                typ: "string",
                description: "sending context name",
            },
            FieldMeta {
                name: "to",
                typ: "string",
                description: "recipient context name",
            },
            FieldMeta {
                name: "content",
                typ: "string",
                description: "message content",
            },
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "active context name",
            },
            FieldMeta {
                name: "delivery_result",
                typ: "string",
                description: "delivery outcome description",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostIndexFile,
        category: "index",
        description: "fires after a file is indexed by the code indexer; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "path",
                typ: "string",
                description: "relative path of indexed file",
            },
            FieldMeta {
                name: "lang",
                typ: "string",
                description: "detected language",
            },
            FieldMeta {
                name: "symbol_count",
                typ: "number",
                description: "number of symbols indexed",
            },
            FieldMeta {
                name: "ref_count",
                typ: "number",
                description: "number of references indexed",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreVfsWrite,
        category: "vfs_write",
        description: "fires before a VFS file write via tool dispatch; advisory, non-blocking",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "write_file or file_edit",
            },
            FieldMeta {
                name: "path",
                typ: "string",
                description: "VFS path being written",
            },
            FieldMeta {
                name: "content",
                typ: "string",
                description: "new content (null for file_edit)",
            },
            FieldMeta {
                name: "caller",
                typ: "string",
                description: "context initiating the write",
            },
        ],
        return_fields: &[],
        notes: "only fires for context-initiated writes via send.rs; VfsCaller::System and (harness io) bypass this hook",
    },
    HookMeta {
        point: HookPoint::PostVfsWrite,
        category: "vfs_write",
        description: "fires after a successful VFS file write via tool dispatch; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "tool_name",
                typ: "string",
                description: "write_file or file_edit",
            },
            FieldMeta {
                name: "path",
                typ: "string",
                description: "VFS path that was written",
            },
            FieldMeta {
                name: "caller",
                typ: "string",
                description: "context that initiated the write",
            },
        ],
        return_fields: &[],
        notes: "same caller restriction as pre_vfs_write",
    },
    HookMeta {
        point: HookPoint::PreClear,
        category: "context",
        description: "fires before clearing a context; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context being cleared",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "number of messages before clear",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "existing conversation summary",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostClear,
        category: "context",
        description: "fires after clearing a context; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context that was cleared",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "message count before clear",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "summary before clear",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreCompact,
        category: "context",
        description: "fires before full compaction; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context being compacted",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "number of messages before compact",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostCompact,
        category: "context",
        description: "fires after full compaction; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context that was compacted",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "message count before compact",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PreRollingCompact,
        category: "context",
        description: "fires before rolling compaction; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context being compacted",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "total message count",
            },
            FieldMeta {
                name: "non_system_count",
                typ: "number",
                description: "non-system message count",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "conversation summary",
            },
        ],
        return_fields: &[],
        notes: "",
    },
    HookMeta {
        point: HookPoint::PostRollingCompact,
        category: "context",
        description: "fires after rolling compaction; observe only",
        can_modify: false,
        payload_fields: &[
            FieldMeta {
                name: "context_name",
                typ: "string",
                description: "context that was compacted",
            },
            FieldMeta {
                name: "message_count",
                typ: "number",
                description: "message count after archiving",
            },
            FieldMeta {
                name: "messages_archived",
                typ: "number",
                description: "number of messages archived",
            },
            FieldMeta {
                name: "summary",
                typ: "string",
                description: "updated summary",
            },
        ],
        return_fields: &[],
        notes: "",
    },
];

#[cfg(feature = "synthesised-tools")]
/// Generate a scheme alist string for `hooks-docs` injected into the tein runtime.
///
/// Follows the same convention as `introspect-docs`, `harness-tools-docs`, etc.
/// Each key is the snake_case hook point name (as a symbol); the value is a
/// multi-line human-readable string describing category, payload, and returns.
///
/// Mutation site: if `HookMeta` structure changes, update the format string below.
pub(crate) fn generate_hooks_docs_alist() -> String {
    let mut entries = String::new();
    entries.push_str(
        r#"'((__module__ . "hook points — lifecycle hooks for plugins and synthesised tools")"#,
    );
    entries.push('\n');

    for meta in HOOK_METADATA {
        let key = meta.point.as_ref(); // snake_case name from strum

        let can_modify_str = if meta.can_modify { "yes" } else { "no" };

        let payload_str: String = if meta.payload_fields.is_empty() {
            "  payload: (none)".to_string()
        } else {
            let fields: Vec<String> = meta
                .payload_fields
                .iter()
                .map(|f| format!("{} ({}): {}", f.name, f.typ, f.description))
                .collect();
            format!("  payload: {}", fields.join(", "))
        };

        let returns_str: String = if meta.return_fields.is_empty() {
            "  returns: (observe only)".to_string()
        } else {
            let fields: Vec<String> = meta
                .return_fields
                .iter()
                .map(|f| format!("{} ({}): {}", f.name, f.typ, f.description))
                .collect();
            format!("  returns: {}", fields.join(", "))
        };

        let notes_str = if meta.notes.is_empty() {
            String::new()
        } else {
            format!("\n  note: {}", meta.notes)
        };

        // Escape backslashes and double-quotes for scheme string literals.
        let value = format!(
            "category: {} | {} | can modify: {}\n{}\n{}{}",
            meta.category, meta.description, can_modify_str, payload_str, returns_str, notes_str,
        );
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        entries.push_str(&format!("    ({key} . \"{escaped}\")\n"));
    }

    entries.push(')');
    entries
}

#[cfg(feature = "synthesised-tools")]
/// Generate the hook reference section for `docs/hooks.md`.
///
/// Produces the content that belongs between the `BEGIN GENERATED` and
/// `END GENERATED` markers. Category-grouped tables followed by per-hook
/// payload/return details (JSON blocks).
///
/// Mutation site: if table format or per-hook detail format changes, update
/// the freshness test in the test suite and re-run `just generate-docs`.
pub fn generate_hooks_markdown() -> String {
    use std::collections::BTreeMap;

    // Ordered category display names
    let category_order = [
        ("session", "Session Lifecycle"),
        ("message", "Message Lifecycle"),
        ("system_prompt", "System Prompt Lifecycle"),
        ("tool", "Tool Lifecycle"),
        ("api", "API Request Lifecycle"),
        ("agentic", "Agentic Loop Lifecycle"),
        ("file_permission", "File Permission"),
        ("url_security", "URL Security"),
        ("agent", "Sub-Agent Lifecycle"),
        ("cache", "Tool Output Caching"),
        ("message_delivery", "Message Delivery"),
        ("index", "Index Lifecycle"),
        ("vfs_write", "VFS Write Lifecycle"),
        ("context", "Context Lifecycle"),
    ];

    // Group hooks by category preserving HOOK_METADATA order within each group
    let mut by_category: BTreeMap<&str, Vec<&HookMeta>> = BTreeMap::new();
    for meta in HOOK_METADATA {
        by_category.entry(meta.category).or_default().push(meta);
    }

    let mut out = String::new();
    out.push_str("## Hook Points\n");

    // Tables per category
    for (cat_key, cat_display) in &category_order {
        let Some(hooks) = by_category.get(cat_key) else {
            continue;
        };
        out.push('\n');
        out.push_str(&format!("### {cat_display}\n\n"));
        out.push_str("| Hook | When | Can Modify |\n");
        out.push_str("|------|------|------------|\n");
        for meta in hooks {
            let key = meta.point.as_ref();
            let can_modify = if meta.can_modify { "Yes" } else { "No" };
            out.push_str(&format!(
                "| `{key}` | {} | {can_modify} |\n",
                meta.description
            ));
        }
    }

    // Per-hook detail sections
    out.push_str("\n## Hook Data by Type\n");
    for meta in HOOK_METADATA {
        let key = meta.point.as_ref();
        out.push('\n');
        out.push_str(&format!("### {key}\n\n"));

        // Emit a JSON block for a slice of FieldMeta entries.
        // Commas appear after the value (before the // comment) so the output
        // is syntactically valid JSON once comments are stripped. The last
        // field has no comma. String fields use "..." as a placeholder;
        // the description comment provides the semantic context.
        fn json_block(fields: &[FieldMeta]) -> String {
            let last = fields.len().saturating_sub(1);
            let lines: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let example = match f.typ {
                        "string" => "\"...\"".to_string(),
                        "number" => "0".to_string(),
                        "bool" => "false".to_string(),
                        "array" => "[]".to_string(),
                        "object" => "{}".to_string(),
                        _ => "null".to_string(),
                    };
                    let comma = if i < last { "," } else { "" };
                    format!(
                        "  \"{}\": {}{}  // {}",
                        f.name, example, comma, f.description
                    )
                })
                .collect();
            format!("```json\n{{\n{}\n}}\n```\n", lines.join("\n"))
        }

        // Payload JSON block
        if meta.payload_fields.is_empty() {
            out.push_str("Payload: (empty)\n");
        } else {
            out.push_str(&json_block(meta.payload_fields));
        }

        if !meta.return_fields.is_empty() {
            out.push_str("\n**Can return:**\n");
            out.push_str(&json_block(meta.return_fields));
        }

        if !meta.notes.is_empty() {
            out.push_str(&format!("\n> **Note:** {}\n", meta.notes));
        }
    }

    out
}

/// Context needed to set up `BRIDGE_CALL_CTX` during tein hook dispatch,
/// enabling tein hook callbacks to use `call-tool` and `(harness io)`.
///
/// Pass `Some(...)` from async contexts that have the full app state.
/// Pass `None` from contexts without a tokio runtime (sync lifecycle hooks)
/// or tests — tein callbacks still dispatch but cannot use IO or `call-tool`.
///
/// When the `synthesised-tools` feature is disabled this is an empty struct;
/// the 4th `execute_hook` parameter is always present so call sites compile
/// uniformly with `, None` regardless of feature state.
pub struct TeinHookContext<'a> {
    #[cfg(feature = "synthesised-tools")]
    pub app: &'a crate::state::AppState,
    #[cfg(feature = "synthesised-tools")]
    pub context_name: &'a str,
    #[cfg(feature = "synthesised-tools")]
    pub config: &'a crate::config::ResolvedConfig,
    #[cfg(feature = "synthesised-tools")]
    pub project_root: &'a std::path::Path,
    #[cfg(feature = "synthesised-tools")]
    pub vfs: &'a crate::vfs::Vfs,
    #[cfg(feature = "synthesised-tools")]
    pub registry: Arc<RwLock<super::registry::ToolRegistry>>,
    /// Zero-sized phantom to keep the lifetime parameter valid when feature is off.
    #[cfg(not(feature = "synthesised-tools"))]
    _phantom: std::marker::PhantomData<&'a ()>,
}

/// Execute a hook on all tools that registered for it
/// Returns a vector of (tool_name, result) for tools that returned non-empty output
///
/// Hook data is passed via stdin (JSON). The CHIBI_HOOK env var identifies which hook is firing.
///
/// `tein_ctx` (synthesised-tools feature only): when `Some`, sets `BRIDGE_CALL_CTX` per tein tool
/// during dispatch, enabling `call-tool` and `(harness io)` from tein hook callbacks.
/// Pass `None` from sync contexts or tests that lack a tokio runtime.
pub fn execute_hook(
    tools: &[Tool],
    hook: HookPoint,
    data: &serde_json::Value,
    _tein_ctx: Option<&TeinHookContext<'_>>,
) -> io::Result<Vec<(String, serde_json::Value)>> {
    let mut results = Vec::new();
    let data_str = data.to_string();

    for tool in tools {
        if !tool.hooks.contains(&hook) {
            continue;
        }

        // Only plugin tools can register hooks; extract the executable path.
        let plugin_path = match &tool.r#impl {
            super::ToolImpl::Plugin(p) => p.clone(),
            _ => continue, // non-plugin tools cannot spawn hooks
        };
        let mut child = Command::new(&plugin_path)
            .env("CHIBI_HOOK", hook.as_ref())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                io::Error::other(format!(
                    "Failed to spawn hook {} on {}: {}",
                    hook.as_ref(),
                    tool.name,
                    e
                ))
            })?;

        // Write hook data to stdin (ignore BrokenPipe — child may exit before reading)
        if let Some(mut stdin) = child.stdin.take() {
            match stdin.write_all(data_str.as_bytes()) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                Err(e) => return Err(e),
            }
            // stdin is dropped here, closing the pipe and signaling EOF
        }

        let timeout = std::time::Duration::from_secs(super::PLUGIN_TIMEOUT_SECS);
        let context = format!("hook {} on {}", hook.as_ref(), tool.name);
        let output = super::wait_with_timeout(child, timeout, &context).map_err(|e| {
            io::Error::other(format!(
                "Failed to execute hook {} on {}: {}",
                hook.as_ref(),
                tool.name,
                e
            ))
        })?;

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Try to parse as JSON, otherwise wrap as string
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|_| serde_json::Value::String(trimmed.to_string()));

        results.push((tool.name.clone(), value));
    }

    // --- synthesised tein hooks ---
    #[cfg(feature = "synthesised-tools")]
    {
        let should_dispatch = TEIN_HOOK_GUARD.with(|guard| !guard.borrow().contains(&hook));

        if should_dispatch {
            TEIN_HOOK_GUARD.with(|guard| {
                guard.borrow_mut().insert(hook);
            });

            // Deduplicate: tools from the same .scm file share a tein context
            // and produce the same (worker_thread_id, binding) pair. Only fire
            // each unique (context instance, binding) once per hook event.
            let mut dispatched: std::collections::HashSet<(std::thread::ThreadId, String)> =
                std::collections::HashSet::new();

            for tool in tools {
                if !tool.hooks.contains(&hook) {
                    continue;
                }
                let (context, hook_bindings, worker_thread_id) = match &tool.r#impl {
                    super::ToolImpl::Synthesised {
                        context,
                        hook_bindings,
                        worker_thread_id,
                        ..
                    } => (context, hook_bindings, *worker_thread_id),
                    _ => continue,
                };

                // Set call context guard if tein_ctx available — enables call-tool
                // and (harness io) from tein hook callbacks.
                // Guard drops at end of each loop iteration, clearing the bridge context.
                let _bridge_guard = _tein_ctx.map(|ctx| {
                    super::synthesised::CallContextGuard::set_from_hook_ctx(ctx, worker_thread_id)
                });

                let Some(binding) = hook_bindings.get(&hook) else {
                    continue;
                };

                // Skip if this (context, binding) was already dispatched this event.
                if !dispatched.insert((worker_thread_id, binding.clone())) {
                    continue;
                }

                let payload = match super::synthesised::json_args_to_scheme_alist(data) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {}: payload conversion: {e}",
                            hook.as_ref()
                        );
                        continue;
                    }
                };

                let hook_fn = match context.evaluate(binding) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {} on {}: resolve {binding}: {e}",
                            hook.as_ref(),
                            tool.name
                        );
                        continue;
                    }
                };

                let result = match context.call(&hook_fn, &[payload]) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[WARN] tein hook {} on {}: {e}", hook.as_ref(), tool.name);
                        continue;
                    }
                };

                // empty list or nil → no-op, don't push a result
                if result.is_nil() {
                    continue;
                }
                if matches!(&result, tein::Value::List(items) if items.is_empty()) {
                    continue;
                }

                match super::synthesised::scheme_value_to_json(&result) {
                    Ok(value) => results.push((tool.name.clone(), value)),
                    Err(e) => {
                        eprintln!(
                            "[WARN] tein hook {} on {}: result conversion: {e}",
                            hook.as_ref(),
                            tool.name
                        );
                    }
                }
            }

            TEIN_HOOK_GUARD.with(|guard| {
                guard.borrow_mut().remove(&hook);
            });
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `ToolsConfig` that maps `vfs_path` to the given tier.
    #[cfg(feature = "synthesised-tools")]
    fn config_with_tier(vfs_path: &str, tier: u8) -> crate::config::ToolsConfig {
        let mut tiers = std::collections::HashMap::new();
        tiers.insert(vfs_path.to_string(), tier);
        crate::config::ToolsConfig {
            tiers: Some(tiers),
            ..Default::default()
        }
    }

    // All 31 hook points for testing
    const ALL_HOOKS: &[(&str, HookPoint)] = &[
        ("pre_message", HookPoint::PreMessage),
        ("post_message", HookPoint::PostMessage),
        ("pre_tool", HookPoint::PreTool),
        ("post_tool", HookPoint::PostTool),
        ("pre_tool_output", HookPoint::PreToolOutput),
        ("post_tool_output", HookPoint::PostToolOutput),
        ("pre_clear", HookPoint::PreClear),
        ("post_clear", HookPoint::PostClear),
        ("pre_compact", HookPoint::PreCompact),
        ("post_compact", HookPoint::PostCompact),
        ("pre_rolling_compact", HookPoint::PreRollingCompact),
        ("post_rolling_compact", HookPoint::PostRollingCompact),
        ("on_start", HookPoint::OnStart),
        ("on_end", HookPoint::OnEnd),
        ("pre_system_prompt", HookPoint::PreSystemPrompt),
        ("post_system_prompt", HookPoint::PostSystemPrompt),
        ("pre_send_message", HookPoint::PreSendMessage),
        ("post_send_message", HookPoint::PostSendMessage),
        ("pre_cache_output", HookPoint::PreCacheOutput),
        ("post_cache_output", HookPoint::PostCacheOutput),
        ("pre_api_tools", HookPoint::PreApiTools),
        ("pre_api_request", HookPoint::PreApiRequest),
        ("pre_agentic_loop", HookPoint::PreAgenticLoop),
        ("post_tool_batch", HookPoint::PostToolBatch),
        ("pre_file_read", HookPoint::PreFileRead),
        ("pre_file_write", HookPoint::PreFileWrite),
        ("pre_shell_exec", HookPoint::PreShellExec),
        ("pre_fetch_url", HookPoint::PreFetchUrl),
        ("pre_spawn_agent", HookPoint::PreSpawnAgent),
        ("post_spawn_agent", HookPoint::PostSpawnAgent),
        ("post_index_file", HookPoint::PostIndexFile),
        ("pre_vfs_write", HookPoint::PreVfsWrite),
        ("post_vfs_write", HookPoint::PostVfsWrite),
    ];

    #[test]
    fn test_hook_point_from_str_valid() {
        for (s, expected) in ALL_HOOKS {
            let result = s.parse::<HookPoint>();
            assert!(result.is_ok(), "parse failed for '{}'", s);
            assert_eq!(result.unwrap(), *expected);
        }
    }

    #[test]
    fn test_hook_point_from_str_invalid() {
        assert!("".parse::<HookPoint>().is_err());
        assert!("unknown".parse::<HookPoint>().is_err());
        assert!("PreMessage".parse::<HookPoint>().is_err()); // wrong case
        assert!("pre-message".parse::<HookPoint>().is_err()); // wrong separator
    }

    #[test]
    fn test_hook_point_as_str() {
        for (expected_str, hook) in ALL_HOOKS {
            assert_eq!(hook.as_ref(), *expected_str);
        }
    }

    #[test]
    fn test_hook_point_round_trip() {
        for (s, _) in ALL_HOOKS {
            let hook = s.parse::<HookPoint>().unwrap();
            assert_eq!(hook.as_ref(), *s);
        }
    }

    // --- hook metadata tests ---

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_hook_metadata_completeness() {
        // Every HookPoint variant must have an entry in HOOK_METADATA.
        // Adding a variant without metadata will fail this test.
        use strum::IntoEnumIterator;

        let meta_points: std::collections::HashSet<HookPoint> =
            HOOK_METADATA.iter().map(|m| m.point).collect();

        let mut missing = Vec::new();
        for variant in HookPoint::iter() {
            if !meta_points.contains(&variant) {
                missing.push(variant.as_ref().to_string());
            }
        }

        assert!(
            missing.is_empty(),
            "HookPoint variants missing from HOOK_METADATA: {missing:?}"
        );
        assert_eq!(
            HOOK_METADATA.len(),
            HookPoint::iter().count(),
            "HOOK_METADATA has duplicate entries or wrong count"
        );
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_hook_metadata_categories_valid() {
        // Every HOOK_METADATA entry's category must appear in generate_hooks_markdown's
        // category_order. Without this, a mismatched category string is silently
        // dropped from the generated markdown.
        let category_order = [
            "session",
            "message",
            "system_prompt",
            "tool",
            "api",
            "agentic",
            "file_permission",
            "url_security",
            "agent",
            "cache",
            "message_delivery",
            "index",
            "vfs_write",
            "context",
        ];
        let valid: std::collections::HashSet<&str> = category_order.iter().copied().collect();
        let mut bad = Vec::new();
        for meta in HOOK_METADATA {
            if !valid.contains(meta.category) {
                bad.push(format!("{}: {:?}", meta.point.as_ref(), meta.category));
            }
        }
        assert!(
            bad.is_empty(),
            "HOOK_METADATA entries with unknown category (will be silently dropped from generated docs): {bad:?}"
        );
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_generate_hooks_docs_alist_contains_all_hooks() {
        use strum::IntoEnumIterator;

        let alist = generate_hooks_docs_alist();
        // Must start with the standard alist form
        assert!(
            alist.starts_with("'("),
            "alist must start with quote+paren: {alist}"
        );

        // Every hook's snake_case name must appear as a key
        for variant in HookPoint::iter() {
            let key = variant.as_ref();
            assert!(
                alist.contains(&format!("({key} . ")),
                "hooks-docs alist missing key: {key}"
            );
        }
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_hooks_docs_in_scheme_context() {
        let (session, _) =
            crate::tools::synthesised::build_sandboxed_harness_context().expect("build context");

        // hooks-docs must be a non-empty pair
        let is_pair = session
            .evaluate("(pair? hooks-docs)")
            .expect("evaluate pair?");
        assert_eq!(is_pair, tein::Value::Boolean(true));

        // (describe hooks-docs) must return a string mentioning pre_message
        let described = session
            .evaluate("(describe hooks-docs)")
            .expect("evaluate describe");
        match described {
            tein::Value::String(s) => {
                assert!(
                    s.contains("pre_message"),
                    "describe hooks-docs should mention pre_message: {s}"
                );
            }
            other => panic!("expected string from describe, got: {other:?}"),
        }
    }

    #[cfg(feature = "synthesised-tools")]
    #[test]
    fn test_module_doc_hooks_docs_pre_message() {
        let (session, _) =
            crate::tools::synthesised::build_sandboxed_harness_context().expect("build context");
        let result = session
            .evaluate("(module-doc hooks-docs 'pre_message)")
            .expect("evaluate");
        match result {
            tein::Value::String(s) => {
                assert!(
                    s.contains("message") || s.contains("prompt"),
                    "expected pre_message doc string, got: {s}"
                );
            }
            other => panic!("expected string, got: {other:?}"),
        }
    }

    use super::super::ToolMetadata;
    use super::super::test_helpers::create_test_script;

    /// Execute a hook with retry on ETXTBSY (text file busy).
    fn execute_hook_with_retry(
        tools: &[Tool],
        hook: HookPoint,
        data: &serde_json::Value,
    ) -> io::Result<Vec<(String, serde_json::Value)>> {
        for attempt in 0..5 {
            match execute_hook(tools, hook, data, None) {
                Ok(result) => return Ok(result),
                Err(e) if e.to_string().contains("Text file busy") && attempt < 4 => {
                    std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1) as u64));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    #[test]
    fn test_execute_hook_receives_stdin_data() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            b"#!/bin/bash\ncat\n", // Echo stdin to stdout
        );

        let tools = vec![Tool {
            name: "hook_tool".to_string(),
            description: "Hook tester".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let data = serde_json::json!({"event": "start", "context": "test"});
        let results = execute_hook_with_retry(&tools, HookPoint::OnStart, &data).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "hook_tool");
        assert_eq!(results[0].1["event"], "start");
        assert_eq!(results[0].1["context"], "test");
    }

    #[test]
    fn test_execute_hook_env_var() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook_env.sh",
            b"#!/bin/bash\ncat > /dev/null\necho \"hook=$CHIBI_HOOK\"\n",
        );

        let tools = vec![Tool {
            name: "env_hook".to_string(),
            description: "Env checker".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::PreMessage],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 1);
        // Result is string since it's not valid JSON
        let output = results[0].1.as_str().unwrap();
        assert!(output.contains("hook=pre_message"));
    }

    #[test]
    fn test_execute_hook_no_hook_data_env() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "verify_env.sh",
            br#"#!/bin/bash
cat > /dev/null
if [ -n "$CHIBI_HOOK_DATA" ]; then
  echo 'ERROR: CHIBI_HOOK_DATA should not be set'
  exit 1
fi
echo 'OK'
"#,
        );

        let tools = vec![Tool {
            name: "verify_hook".to_string(),
            description: "Env verifier".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnEnd],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnEnd, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.as_str().unwrap(), "OK");
    }

    #[test]
    fn test_execute_hook_skips_non_registered() {
        let dir = tempfile::tempdir().unwrap();
        let script_path =
            create_test_script(dir.path(), "skip.sh", b"#!/bin/bash\necho 'CALLED'\n");

        let tools = vec![Tool {
            name: "skip_tool".to_string(),
            description: "Should be skipped".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart], // Registered for OnStart only
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        // Call with OnEnd - should not execute the tool
        let results =
            execute_hook_with_retry(&tools, HookPoint::OnEnd, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_execute_hook_skips_failures() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = create_test_script(dir.path(), "fail.sh", b"#!/bin/bash\nexit 1\n");

        let tools = vec![Tool {
            name: "fail_hook".to_string(),
            description: "Always fails".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script_path),
            category: crate::tools::ToolCategory::Plugin,
        }];

        // Failed hooks should be skipped (not error)
        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_execute_hook_multiple_tools() {
        let dir = tempfile::tempdir().unwrap();
        let script1 = create_test_script(
            dir.path(),
            "hook1.sh",
            b"#!/bin/bash\ncat > /dev/null\necho 'first'\n",
        );
        let script2 = create_test_script(
            dir.path(),
            "hook2.sh",
            b"#!/bin/bash\ncat > /dev/null\necho 'second'\n",
        );

        let tools = vec![
            Tool {
                name: "first_hook".to_string(),
                description: "First".to_string(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(script1),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "second_hook".to_string(),
                description: "Second".to_string(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::OnStart],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(script2),
                category: crate::tools::ToolCategory::Plugin,
            },
        ];

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "first_hook");
        assert_eq!(results[0].1.as_str().unwrap(), "first");
        assert_eq!(results[1].0, "second_hook");
        assert_eq!(results[1].1.as_str().unwrap(), "second");
    }

    #[test]
    #[cfg(unix)]
    fn test_execute_hook_failure_cascade() {
        // Middle hook fails (exit 1) — first and third should still produce results
        let dir = tempfile::tempdir().unwrap();

        let ok1 = create_test_script(dir.path(), "ok1.sh", b"#!/bin/bash\necho '{\"order\": 1}'");
        let fail = create_test_script(dir.path(), "fail.sh", b"#!/bin/bash\nexit 1");
        let ok2 = create_test_script(dir.path(), "ok2.sh", b"#!/bin/bash\necho '{\"order\": 3}'");

        let tools = vec![
            Tool {
                name: "ok1".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(ok1),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "fail".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(fail),
                category: crate::tools::ToolCategory::Plugin,
            },
            Tool {
                name: "ok2".to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(ok2),
                category: crate::tools::ToolCategory::Plugin,
            },
        ];

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 2, "failed hook should be skipped silently");
        assert_eq!(results[0].0, "ok1");
        assert_eq!(results[0].1["order"], 1);
        assert_eq!(results[1].0, "ok2");
        assert_eq!(results[1].1["order"], 3);
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_execute_hook_dispatches_to_synthesised() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start
  (lambda (payload)
    (list (cons "saw_event" (cdr (assoc "event" payload))))))

(define tool-name "tein-hook-test")
(define tool-description "Hook tester")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/hook-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        let data = serde_json::json!({"event": "start"});
        let results = execute_hook(&tools, HookPoint::OnStart, &data, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "tein-hook-test");
        assert_eq!(results[0].1["saw_event"], "start");
    }

    #[test]
    #[cfg(unix)]
    fn test_execute_hook_ordering() {
        // Results must arrive in tool registration order
        let dir = tempfile::tempdir().unwrap();

        let scripts: Vec<_> = (1..=3)
            .map(|i| {
                create_test_script(
                    dir.path(),
                    &format!("hook{i}.sh"),
                    format!("#!/bin/bash\necho '{{\"order\": {i}}}'").as_bytes(),
                )
            })
            .collect();

        let tools: Vec<_> = scripts
            .into_iter()
            .enumerate()
            .map(|(i, path)| Tool {
                name: format!("hook{}", i + 1),
                description: String::new(),
                parameters: serde_json::json!({}),
                hooks: vec![HookPoint::PreMessage],
                metadata: ToolMetadata::new(),
                summary_params: vec![],
                r#impl: crate::tools::ToolImpl::Plugin(path),
                category: crate::tools::ToolCategory::Plugin,
            })
            .collect();

        let results =
            execute_hook_with_retry(&tools, HookPoint::PreMessage, &serde_json::json!({})).unwrap();

        assert_eq!(results.len(), 3);
        for (i, (name, value)) in results.iter().enumerate() {
            assert_eq!(*name, format!("hook{}", i + 1));
            assert_eq!(value["order"], (i + 1) as u64);
        }
    }

    // --- tein (synthesised) hook dispatch tests ---

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_empty_list_return_is_noop() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) '()))
(define tool-name "noop-hook")
(define tool-description "Returns empty list")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/noop.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        let results =
            execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(
            results.len(),
            0,
            "empty list return should be treated as no-op"
        );
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_json_object_return() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'pre_message
  (lambda (payload)
    (list (cons "prompt" "modified prompt"))))
(define tool-name "modify-hook")
(define tool-description "Modifies prompt")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/modify.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        let results = execute_hook(
            &tools,
            HookPoint::PreMessage,
            &serde_json::json!({"prompt": "hello"}),
            None,
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1["prompt"], "modified prompt");
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_skips_unregistered_hook_point() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "selective-hook")
(define tool-description "Only fires on on_start")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/selective.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        // fire on_end — tool is registered for on_start only
        let results = execute_hook(&tools, HookPoint::OnEnd, &serde_json::json!({}), None).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_error_in_callback_skipped() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (error "boom")))
(define tool-name "error-hook")
(define tool-description "Errors in hook")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/error.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        // should not error — failed hooks are skipped silently
        let results =
            execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    #[cfg(all(feature = "synthesised-tools", unix))]
    fn test_mixed_plugin_and_tein_hooks() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        // subprocess plugin hook
        let dir = tempfile::tempdir().unwrap();
        let script = create_test_script(
            dir.path(),
            "plugin.sh",
            b"#!/bin/bash\ncat > /dev/null\necho '{\"from\": \"plugin\"}'",
        );
        let plugin_tool = Tool {
            name: "plugin-hook".to_string(),
            description: "Plugin".to_string(),
            parameters: serde_json::json!({}),
            hooks: vec![HookPoint::OnStart],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
            r#impl: crate::tools::ToolImpl::Plugin(script),
            category: crate::tools::ToolCategory::Plugin,
        };

        // tein synthesised hook
        let source = r#"
(import (harness hooks))
(register-hook 'on_start
  (lambda (payload) (list (cons "from" "tein"))))
(define tool-name "tein-hook")
(define tool-description "Tein hook")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/tein.scm").unwrap();
        let mut tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();
        tools.insert(0, plugin_tool);

        let results =
            execute_hook_with_retry(&tools, HookPoint::OnStart, &serde_json::json!({})).unwrap();

        // plugin first (subprocess loop), then tein (synthesised loop)
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1["from"], "plugin");
        assert_eq!(results[1].1["from"], "tein");
    }

    // --- re-entrancy guard tests ---

    /// Verify that when a hook point is already in TEIN_HOOK_GUARD (simulating
    /// a recursive call from within a tein hook callback), tein callbacks are
    /// skipped entirely while the guard is held.
    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_reentrancy_guard_skips_tein_callbacks() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "reentrancy-guard-test")
(define tool-description "Should be skipped under guard")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/reentrancy.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        // Simulate re-entrancy: mark on_start as already-in-progress
        TEIN_HOOK_GUARD.with(|guard| {
            guard.borrow_mut().insert(HookPoint::OnStart);
        });

        let results =
            execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();

        // Clean up guard state so other tests in this thread aren't affected
        TEIN_HOOK_GUARD.with(|guard| {
            guard.borrow_mut().remove(&HookPoint::OnStart);
        });

        assert_eq!(
            results.len(),
            0,
            "tein callbacks must be skipped when guard is held (re-entrancy)"
        );
    }

    /// Verify that the guard is cleared after execute_hook completes, so
    /// a subsequent call on the same thread dispatches normally.
    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_reentrancy_guard_cleared_after_dispatch() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(register-hook 'on_start (lambda (payload) (list (cons "fired" #t))))
(define tool-name "guard-cleanup-test")
(define tool-description "Checks guard is cleared post-dispatch")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/guard-cleanup.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &crate::config::ToolsConfig::default(),
        )
        .unwrap();

        // First call — fires normally
        let r1 = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(r1.len(), 1, "first call should fire normally");

        // Second call on the same thread — guard must be cleared; fires again
        let r2 = execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();
        assert_eq!(r2.len(), 1, "guard must be cleared; second call must fire");
    }

    // --- TeinHookContext / CallContextGuard in hook dispatch ---

    /// Helper: build a minimal `(AppState, ResolvedConfig, TempDir)` for tein hook tests.
    ///
    /// Returns `(app, resolved_config, _tmp)` — `_tmp` must outlive `app`.
    #[cfg(feature = "synthesised-tools")]
    fn make_test_tein_env() -> (
        crate::state::AppState,
        crate::config::ResolvedConfig,
        tempfile::TempDir,
    ) {
        use crate::config::{ApiParams, Config, ToolsConfig, VfsConfig};
        use crate::partition::StorageConfig;
        let temp = tempfile::TempDir::new().unwrap();
        let config = Config {
            api_key: None,
            model: None,
            context_window_limit: None,
            warn_threshold_percent: 75.0,
            no_tool_calls: false,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            reflection_enabled: false,
            reflection_character_limit: 10000,
            fuel: 0,
            fuel_empty_response_cost: 0,
            username: "test".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
            tool_output_cache_threshold: 4000,
            tool_cache_max_age_days: 7,
            auto_cleanup_cache: false,
            tool_cache_preview_chars: 500,
            file_tools_allowed_paths: vec![],
            api: ApiParams::default(),
            storage: StorageConfig::default(),
            fallback_tool: "call_user".to_string(),
            tools: ToolsConfig::default(),
            vfs: VfsConfig::default(),
            url_policy: None,
            subagent_cost_tier: "free".to_string(),
            models: Default::default(),
            site: None,
        };
        let app = crate::state::AppState::from_dir(temp.path().to_path_buf(), config).unwrap();
        (app, crate::config::ResolvedConfig::default(), temp)
    }

    /// Verify that when TeinHookContext is provided, BRIDGE_CALL_CTX is populated
    /// during tein hook dispatch, enabling call-tool from hook callbacks.
    ///
    /// The hook calls `(call-tool "nonexistent-tool" '())`. With the guard set, it
    /// should fail with "not found" (tool registry lookup error), NOT with
    /// "no active call context" (bridge not set). This proves the guard is live.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_tein_hook_call_tool_with_tein_ctx_sets_bridge() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        // Tein tool with an on_start hook that tries call-tool.
        // Uses with-exception-handler to capture the error message string.
        // R7RS: error-object-message works for (error ...) conditions.
        let source = r#"
(import (harness hooks))
(import (harness tools))
(register-hook 'on_start
  (lambda (payload)
    ;; Try to call a nonexistent tool. Capture whether it errors with "no active
    ;; call context" (bridge not set) vs anything else (bridge set, tool not found).
    (define result "no-error")
    (call-with-current-continuation
      (lambda (k)
        (with-exception-handler
          (lambda (exn)
            (set! result
              (if (error-object? exn)
                  (error-object-message exn)
                  "unknown-error"))
            (k #f))
          (lambda ()
            (call-tool "nonexistent-tool-xyzzy" '())))))
    (list (cons "error" result))))
(define tool-name "bridge-test")
(define tool-description "Tests bridge")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/bridge-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &config_with_tier(path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        let results = execute_hook(
            &tools,
            HookPoint::OnStart,
            &serde_json::json!({}),
            Some(&tein_ctx),
        )
        .unwrap();

        assert_eq!(results.len(), 1, "hook must return a result");
        let error_msg = results[0].1["error"].as_str().unwrap_or("");
        // With bridge set: error is about tool not found, NOT about missing context
        assert!(
            !error_msg.contains("no active call context"),
            "bridge should be set; got error: {error_msg}"
        );
        assert!(
            !error_msg.contains("called outside tool execute"),
            "bridge should be set; got error: {error_msg}"
        );
    }

    /// Verify that without TeinHookContext, call-tool in a hook callback fails
    /// with "no active call context" (bridge not set).
    #[test]
    #[cfg(feature = "synthesised-tools")]
    fn test_tein_hook_call_tool_without_tein_ctx_no_bridge() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::VfsPath;
        use std::sync::{Arc, RwLock};

        let source = r#"
(import (harness hooks))
(import (harness tools))
(register-hook 'on_start
  (lambda (payload)
    (define result "no-error")
    (call-with-current-continuation
      (lambda (k)
        (with-exception-handler
          (lambda (exn)
            (set! result
              (if (error-object? exn)
                  (error-object-message exn)
                  "unknown-error"))
            (k #f))
          (lambda ()
            (call-tool "any-tool" '())))))
    (list (cons "error" result))))
(define tool-name "no-bridge-test")
(define tool-description "Tests no bridge")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/no-bridge-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &config_with_tier(path.as_str(), 2),
        )
        .unwrap();

        // No TeinHookContext → bridge not set
        let results =
            execute_hook(&tools, HookPoint::OnStart, &serde_json::json!({}), None).unwrap();

        assert_eq!(results.len(), 1, "hook must return a result");
        let error_msg = results[0].1["error"].as_str().unwrap_or("");
        // Without bridge: must fail with "no active call context"
        assert!(
            error_msg.contains("no active call context")
                || error_msg.contains("called outside tool execute"),
            "expected 'no active call context' error, got: {error_msg}"
        );
    }

    /// Full chain: execute_hook → CallContextGuard → tein callback → (harness io)
    /// → VFS write. Verifies io-write in a hook callback actually persists.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_tein_hook_harness_io_vfs_write() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        let source = r#"
(import (harness hooks))
(import (harness io))
(register-hook 'on_start
  (lambda (payload)
    (io-write "vfs:///shared/hook-output.txt" "hello from hook")
    '()))
(define tool-name "io-hook-test")
(define tool-description "Hook that writes via io")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/io-hook-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &config_with_tier(path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        execute_hook(
            &tools,
            HookPoint::OnStart,
            &serde_json::json!({}),
            Some(&tein_ctx),
        )
        .unwrap();

        // Verify the VFS write happened
        let written_path = VfsPath::new("/shared/hook-output.txt").unwrap();
        let content = app
            .vfs
            .read(VfsCaller::System, &written_path)
            .await
            .expect("hook should have written vfs:///shared/hook-output.txt");
        assert_eq!(
            String::from_utf8_lossy(&content),
            "hello from hook",
            "io-write from hook should persist to VFS"
        );
    }

    /// IO from a tein hook callback bypasses the tool layer, so no hooks fire
    /// from the IO write. This confirms no re-entrancy / infinite loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_tein_hook_io_does_not_trigger_hooks() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        // A hook that writes a counter file. If io-write triggered hooks, the
        // on_start hook would call itself recursively. TEIN_HOOK_GUARD should
        // prevent this; and io-write doesn't go through hook dispatch at all.
        // We just verify: (a) the hook runs, (b) the write succeeds, (c) no panic.
        let source = r#"
(import (harness hooks))
(import (harness io))
(register-hook 'on_start
  (lambda (payload)
    (let ((prev (io-read "vfs:///shared/counter.txt")))
      (let ((count (if (string? prev) (+ (string->number prev) 1) 1)))
        (io-write "vfs:///shared/counter.txt" (number->string count))
        (list (cons "count" (number->string count)))))))
(define tool-name "counter-hook-test")
(define tool-description "Hook writes counter")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let path = VfsPath::new("/tools/shared/counter-hook-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &path,
            &registry,
            &config_with_tier(path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        // Fire the hook once
        let results = execute_hook(
            &tools,
            HookPoint::OnStart,
            &serde_json::json!({}),
            Some(&tein_ctx),
        )
        .unwrap();

        // Hook returned count=1
        assert_eq!(results.len(), 1);
        let count_val = &results[0].1["count"];
        assert_eq!(
            count_val.as_str(),
            Some("1"),
            "counter should be 1: {count_val:?}"
        );

        // VFS counter file should be "1" (not "2" or higher — no re-entrancy)
        let counter_path = VfsPath::new("/shared/counter.txt").unwrap();
        let content = app
            .vfs
            .read(VfsCaller::System, &counter_path)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&content),
            "1",
            "counter should be 1 — no recursive hook dispatch from io-write"
        );
    }

    // --- history.scm integration tests ---

    const HISTORY_PLUGIN: &str = include_str!("../../../../plugins/history.scm");

    /// Sanity check: a bare pre_vfs_write hook that writes a file works.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_pre_vfs_write_hook_io_write_works() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        let source = r#"
(import (scheme base))
(import (harness io))
(import (harness hooks))
(import (harness tools))
(register-hook 'pre_vfs_write
  (lambda (payload)
    (io-write "vfs:///shared/hook-fired.txt" "hook ran")
    '()))
(define tool-name "dummy")
(define tool-description "dummy")
(define tool-parameters '())
(define (tool-execute args) "ok")
"#;
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let plugin_path = VfsPath::new("/tools/shared/hook-test.scm").unwrap();
        let tools = load_tools_from_source(
            source,
            &plugin_path,
            &registry,
            &config_with_tier(plugin_path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        let hook_data = serde_json::json!({
            "path": "vfs:///shared/test.txt",
            "content": "new",
        });
        let _ = execute_hook(&tools, HookPoint::PreVfsWrite, &hook_data, Some(&tein_ctx)).unwrap();

        let fired_path = VfsPath::new("/shared/hook-fired.txt").unwrap();
        let content = app.vfs.read(VfsCaller::System, &fired_path).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&content), "hook ran");
    }

    /// Full flow: write a file, fire pre_vfs_write hook, verify snapshot created.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_history_snapshot_on_write() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        // Write initial file content
        let path = VfsPath::new("/shared/test.txt").unwrap();
        app.vfs
            .write(VfsCaller::System, &path, b"version 1")
            .await
            .unwrap();

        // Load history plugin
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let plugin_path = VfsPath::new("/tools/shared/history.scm").unwrap();
        let tools = load_tools_from_source(
            HISTORY_PLUGIN,
            &plugin_path,
            &registry,
            &config_with_tier(plugin_path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        // Fire pre_vfs_write hook (simulates what send.rs does before a write)
        let hook_data = serde_json::json!({
            "path": "vfs:///shared/test.txt",
            "content": "version 2",
            "caller": "test-ctx",
        });
        let _ = execute_hook(&tools, HookPoint::PreVfsWrite, &hook_data, Some(&tein_ctx)).unwrap();

        // Snapshot should exist at revision 1
        let snapshot_path = VfsPath::new("/shared/.chibi/history/test.txt/1").unwrap();
        let snapshot = app
            .vfs
            .read(VfsCaller::System, &snapshot_path)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&snapshot),
            "version 1",
            "snapshot should contain pre-write content"
        );

        // Meta should exist and contain 'next'
        let meta_path = VfsPath::new("/shared/.chibi/history/test.txt/meta").unwrap();
        let meta = app.vfs.read(VfsCaller::System, &meta_path).await.unwrap();
        let meta_str = String::from_utf8_lossy(&meta);
        assert!(meta_str.contains("next"), "meta should contain next field");
    }

    /// Pruning: fire hook 12 times, verify only 10 most recent snapshots remain.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_history_pruning() {
        use crate::tools::registry::ToolRegistry;
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let plugin_path = VfsPath::new("/tools/shared/history.scm").unwrap();
        let tools = load_tools_from_source(
            HISTORY_PLUGIN,
            &plugin_path,
            &registry,
            &config_with_tier(plugin_path.as_str(), 2),
        )
        .unwrap();

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        let path = VfsPath::new("/shared/prune-test.txt").unwrap();

        // Fire hook 12 times, updating file content before each hook
        for i in 1u32..=12 {
            let content = format!("v{}", i - 1);
            app.vfs
                .write(VfsCaller::System, &path, content.as_bytes())
                .await
                .unwrap();

            let hook_data = serde_json::json!({
                "path": "vfs:///shared/prune-test.txt",
                "content": format!("v{}", i),
                "caller": "test-ctx",
            });
            let _ =
                execute_hook(&tools, HookPoint::PreVfsWrite, &hook_data, Some(&tein_ctx)).unwrap();
        }

        // Should have exactly 10 revisions (3..=12), not 12
        let history_dir = VfsPath::new("/shared/.chibi/history/prune-test.txt").unwrap();
        let entries = app.vfs.list(VfsCaller::System, &history_dir).await.unwrap();
        let rev_count = entries.iter().filter(|e| e.name != "meta").count();
        assert_eq!(rev_count, 10, "should prune to keep=10 revisions");

        // Oldest remaining should be revision 3 (revisions 1 and 2 pruned)
        let oldest = VfsPath::new("/shared/.chibi/history/prune-test.txt/3").unwrap();
        assert!(
            app.vfs.exists(VfsCaller::System, &oldest).await.unwrap(),
            "revision 3 should exist"
        );

        let pruned = VfsPath::new("/shared/.chibi/history/prune-test.txt/1").unwrap();
        assert!(
            !app.vfs.exists(VfsCaller::System, &pruned).await.unwrap(),
            "revision 1 should be pruned"
        );
    }

    /// Integration: file_history_diff shows correct unified diff output.
    ///
    /// Fires PreVfsWrite to snapshot the original content, updates the file,
    /// then calls the diff tool and asserts the output reflects the change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg(feature = "synthesised-tools")]
    async fn test_history_diff_tool() {
        use crate::tools::registry::{ToolCallContext, ToolRegistry};
        use crate::tools::synthesised::load_tools_from_source;
        use crate::vfs::{VfsCaller, VfsPath};
        use std::sync::{Arc, RwLock};

        let (app, resolved_config, _tmp) = make_test_tein_env();
        let project_root = _tmp.path();

        // Write initial file content
        let path = VfsPath::new("/shared/diff-test.txt").unwrap();
        app.vfs
            .write(VfsCaller::System, &path, b"line1\nline2\nline3\n")
            .await
            .unwrap();

        // Load history plugin
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let plugin_path = VfsPath::new("/tools/shared/history.scm").unwrap();
        let tools = load_tools_from_source(
            HISTORY_PLUGIN,
            &plugin_path,
            &registry,
            &config_with_tier(plugin_path.as_str(), 2),
        )
        .unwrap();

        // Register tools so dispatch_impl can find them
        {
            let mut reg = registry.write().unwrap();
            for t in &tools {
                reg.register(t.clone());
            }
        }

        let tein_ctx = TeinHookContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            registry: Arc::clone(&registry),
        };

        // Fire pre_vfs_write hook — snapshots "line1\nline2\nline3\n" as revision 1
        let hook_data = serde_json::json!({
            "path": "vfs:///shared/diff-test.txt",
            "content": "line1\nmodified\nline3\n",
            "caller": "test-ctx",
        });
        execute_hook(&tools, HookPoint::PreVfsWrite, &hook_data, Some(&tein_ctx)).unwrap();

        // Update file to the modified version
        app.vfs
            .write(VfsCaller::System, &path, b"line1\nmodified\nline3\n")
            .await
            .unwrap();

        // Execute file_history_diff via dispatch_impl
        let diff_tool_impl = {
            let reg = registry.read().unwrap();
            reg.get("file_history_diff").map(|t| t.r#impl.clone())
        }
        .expect("file_history_diff should be registered");

        let call_ctx = ToolCallContext {
            app: &app,
            context_name: "test-ctx",
            config: &resolved_config,
            project_root,
            vfs: &app.vfs,
            vfs_caller: crate::vfs::VfsCaller::Context("test-ctx"),
        };

        let args = serde_json::json!({
            "path": "vfs:///shared/diff-test.txt",
            "revision": 1,
        });
        let result =
            ToolRegistry::dispatch_impl(diff_tool_impl, "file_history_diff", &args, &call_ctx)
                .await
                .unwrap();

        // Diff output should mention the changed line
        assert!(
            result.contains("line2") || result.contains("-line2"),
            "diff should show removed 'line2': {result}"
        );
        assert!(
            result.contains("modified") || result.contains("+modified"),
            "diff should show added 'modified': {result}"
        );
    }
}
