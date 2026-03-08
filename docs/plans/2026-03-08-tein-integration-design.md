# tein integration — synthesised tools via VFS

**issue:** #193
**date:** 2026-03-08

## context

issue #190 landed the foundation: `ToolImpl::Synthesised`, startup scanning of
`/tools/shared/`, hot-reload on VFS writes, sandboxed tein execution, and the
convention-based tool format. this design covers the remaining items needed to
make synthesised tools fully functional.

## scope

six items in three dependency layers:

| layer | items | dependency |
|-------|-------|------------|
| 1 — foundation | `(harness tools)` module, `call-tool` bridge | none |
| 2 — multi-context | multi-zone scanning, visibility scoping | layer 1 |
| 3 — ergonomics | `define-tool` macro, tier configuration | layer 1 |

## layer 1: `(harness tools)` module & `call-tool` bridge

### the module

a scheme library registered into every synthesised tool's tein context at build
time. provides the core primitive for tool orchestration:

```scheme
(import (harness tools))

(call-tool "shell_exec" '(("command" . "ls -la")))
;; => returns a string (the tool's output)
```

`(import (harness tools))` works in sandboxed (tier 1) contexts — the module is
explicitly whitelisted regardless of sandbox tier.

### rust bridge

a foreign function registered into the tein context:

1. receives tool name (string) and args (alist) from scheme
2. converts args alist → `serde_json::Value` via tein's value-to-json conversion
3. dispatches through `ToolRegistry::dispatch_impl` — full permission + hook
   stack (PreTool, PostTool, etc.) fires as normal
4. **blocking:** uses `tokio::runtime::Handle::current().block_on()` to bridge
   sync tein → async dispatch. synthesised tools already run in a blocking
   context, so this adds no new constraint
5. returns the tool's string output as a scheme string
6. on failure, raises a scheme error (catchable with `guard`)

### caller identity

`call-tool` inherits the calling context's identity. if context "alice" runs a
synthesised tool that calls `shell_exec`, permission checks see "alice" as the
caller. tools don't get separate privileges — they act on behalf of their
invoker.

to make this work, the `ToolCallContext` (or relevant fields: context name,
config, vfs_caller) must be threaded into the tein execution. the bridge
closure captures a reference to the active `ToolCallContext` so `call-tool`
can pass it through to dispatch.

## layer 2: multi-zone scanning & visibility scoping

### startup scanning

extend `scan_and_register` to cover all three writable zones:

- `/tools/shared/` — as today
- `/tools/home/<ctx>/` — scan each context's tool directory
- `/tools/flocks/<name>/` — scan each flock's tool directory

the scan walks the context list and flock registry to discover which
subdirectories exist. hot-reload already handles all three zones — the
`is_scm_tool_path` check in `vfs.rs` already matches `/tools/home/` and
`/tools/flocks/`, so only the startup scan needs extending.

### visibility scoping in send.rs

when building the tool list for a prompt, filter synthesised tools by the
calling context's accessible zones:

| zone | visible to |
|------|-----------|
| `/tools/shared/*` | all contexts |
| `/tools/home/<ctx>/*` | context `<ctx>` only |
| `/tools/flocks/<name>/*` | members of flock `<name>` |

implementation: for each synthesised tool, check its `vfs_path` prefix against
the calling context's name and flock memberships. builtins, plugins, and MCP
tools are unaffected (always visible).

`send_prompt` already has access to context name and can resolve flock
membership via the VFS flock registry.

## layer 3: `define-tool` macro & tier configuration

### `define-tool` macro

allows multiple tool definitions per `.scm` file:

```scheme
(import (harness tools))

(define-tool summarise-diff
  (description "summarise a git diff")
  (parameters '((diff . ((type . "string") (description . "the diff")))))
  (execute (lambda (args)
    (call-tool "shell_exec" `(("command" . ,(string-append "echo " (assoc-ref "diff" args))))))))

(define-tool count-lines
  (description "count lines in text")
  (parameters '((text . ((type . "string") (description . "input text")))))
  (execute (lambda (args)
    (number->string (length (string-split (assoc-ref "text" args) #\newline))))))
```

the macro expands into an internal registration form that appends tool metadata
to a module-level list. after evaluation, the rust loader reads this list to
extract all tool definitions from the file.

**backwards compatibility:** if no `define-tool` forms are found, the loader
falls back to the current convention-based format (single `tool-name` /
`tool-description` / `tool-parameters` / `tool-execute` bindings).

**loading changes:**

- `load_tool_from_source` becomes `load_tools_from_source` (plural), returns
  `Vec<Tool>`
- convention-based single-tool format → `Vec` of one
- `define-tool` multi-tool format → `Vec` of N

**hot-reload changes:**

- on write: load all tools from the file, unregister any previously registered
  tools from that `vfs_path`, register the new set
- on delete: unregister all tools associated with that `vfs_path`
- `find_by_vfs_path` returns all matches (or we track a
  `vfs_path → Vec<tool_name>` mapping)

### tier configuration

two tiers:

| tier | environment | default |
|------|-------------|---------|
| 1 (sandboxed) | `Modules::Safe`, step limit 10M, only safe scheme modules + `(harness tools)` | yes |
| 2 (unsandboxed) | full r7rs, host access via tein IO modules, no step limit | opt-in |

**config in `chibi.toml`:**

```toml
[tools.tiers]
# per-tool path
"/tools/shared/admin-tool.scm" = 2

# per-zone (everything under this path)
"/tools/home/admin/" = 2
```

resolution rules:

- most specific path wins (per-tool overrides per-zone)
- absent from config → tier 1
- applied at load time (startup scan and hot-reload)
- changing config requires restart or re-write of the `.scm` file to trigger
  reload with new tier

**rust side:**

- `load_tools_from_source` gains a `tier: SandboxTier` parameter
- `SandboxTier` enum: `Sandboxed` (tier 1), `Unsandboxed` (tier 2)
- tier 1: `Context::builder().standard_env().sandboxed(Modules::Safe).step_limit(10_000_000)`
- tier 2: `Context::builder().standard_env()` (no sandbox, no step limit)

**`(harness tools)` availability:** both tiers get the module. tier controls
scheme-level capabilities (which scheme modules are available, step limits).
`call-tool` permission checks are always governed by the caller context's
permissions, not the tier.

## error handling

- `call-tool` errors (unknown tool, permission denied, execution failure)
  surface as scheme errors — catchable with `guard`, or propagate as the
  synthesised tool's error result to the LLM
- `define-tool` with missing required fields (description, execute) → load
  error, that tool definition skipped (other tools in the file still load)
- tier 2 tools are still constrained by chibi's permission stack on the
  `call-tool` side — tein tier only controls scheme-level capabilities

## testing

- **`call-tool` bridge:** mock registry with a simple builtin, call from scheme,
  verify args arrive correctly and result comes back
- **`define-tool` macro:** multi-tool file loading, backwards compat with
  convention format, missing field errors
- **visibility scoping:** context sees own home tools + shared + flock tools,
  doesn't see other contexts' home tools
- **multi-zone scanning:** startup scan discovers tools in all three zones
- **tier config:** tier 1 can't access host IO, tier 2 can; config resolution
  (most specific path wins)
- **integration:** write a `.scm` file with `define-tool` that uses `call-tool`
  to invoke a builtin, verify the full chain through hot-reload
