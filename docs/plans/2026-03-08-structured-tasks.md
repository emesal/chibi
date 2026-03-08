# Structured Task Files Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace flat `todos.md` with a file-per-task VFS system using a tein synthesised plugin and ephemeral prompt injection.

**Architecture:** A tein plugin at `/tools/shared/tasks.scm` exposes five `define-tool` tools (create, update, view, list, delete) that manage `.task` files via the `call-tool` VFS bridge. A rust-side module in `send.rs` reads task metadata using `tein-sexp` and injects a table summary as an ephemeral system message before each user turn. The existing `todos.md` system is removed.

**Tech Stack:** Rust (chibi-core), tein-sexp (s-expression parser), tein/scheme (synthesised tool runtime)

**Design doc:** `docs/plans/2026-03-08-structured-tasks-design.md`

---

## Task 1: add tein-sexp dependency

**Files:**
- Modify: `crates/chibi-core/Cargo.toml:30` (dependencies section)

**Step 1: Add tein-sexp as a non-optional dependency**

In `crates/chibi-core/Cargo.toml`, add alongside the existing `tein` dep:

```toml
tein-sexp = { git = "https://github.com/emesal/tein", branch = "main" }
```

**IMPORTANT:** Do NOT mark it `optional = true`. The ephemeral injection is core functionality (not feature-gated behind `synthesised-tools`). tein-sexp lives in the same repo as tein (workspace member).

**Step 2: Verify it compiles**

Run: `cargo build -p chibi-core 2>&1 | tail -5`
Expected: successful build

**Step 3: Commit**

```
feat(deps): add tein-sexp for s-expression parsing (#186)
```

---

## Task 2: task metadata parser module

**Files:**
- Create: `crates/chibi-core/src/state/tasks.rs`
- Modify: `crates/chibi-core/src/state/mod.rs` (add `pub mod tasks;`)

This module parses `.task` files using tein-sexp and builds the ephemeral summary table. It is core infrastructure (not feature-gated) since the injection runs on every prompt.

**Step 1: Write tests for task metadata parsing**

Create `crates/chibi-core/src/state/tasks.rs`:

```rust
//! Task file parser and ephemeral summary builder.
//!
//! `.task` files contain two scheme datums: a metadata alist and a body string.
//! This module parses metadata via tein-sexp (no scheme evaluator) and builds
//! compact table summaries for ephemeral injection into the prompt.

use std::collections::HashMap;
use std::io;

/// Parsed task metadata from the first datum of a `.task` file.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskMeta {
    pub id: String,
    pub status: String,
    pub priority: String,
    pub depends_on: Vec<String>,
    pub assigned_to: Option<String>,
    pub path: String,          // VFS path relative to tasks root
    pub summary_line: String,  // first line of body
}

/// Parse a `.task` file's content into metadata.
///
/// Reads the first datum (alist) for metadata fields and optionally the
/// second datum (string) for the body summary line.
pub fn parse_task(content: &str, relative_path: &str) -> io::Result<TaskMeta> {
    todo!()
}

/// Compute which task IDs are blocked (have depends-on where dep status != done).
pub fn compute_blocked(tasks: &[TaskMeta]) -> HashMap<String, Vec<String>> {
    todo!()
}

/// Build an ephemeral summary table from a list of tasks.
///
/// Format:
/// ```text
/// --- tasks ---
/// id     status      priority  path              summary
/// a3f2   in-progress high      epic/login        implement the auth flow
/// --- 2 active (1 blocked), 1 done ---
/// ```
pub fn build_summary_table(tasks: &[TaskMeta]) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TASK: &str = r#"((id . "a3f2")
 (status . pending)
 (priority . high)
 (depends-on "b1c4" "e7d0")
 (assigned-to . "worker-1")
 (created . "20260308-1423z")
 (updated . "20260308-1445z"))

"implement the auth flow.

acceptance criteria:
- JWT tokens""#;

    const MINIMAL_TASK: &str = r#"((id . "b1c4")
 (status . done)
 (created . "20260308-1400z")
 (updated . "20260308-1500z"))
"#;

    #[test]
    fn test_parse_full_task() {
        let meta = parse_task(SAMPLE_TASK, "epic/login.task").unwrap();
        assert_eq!(meta.id, "a3f2");
        assert_eq!(meta.status, "pending");
        assert_eq!(meta.priority, "high");
        assert_eq!(meta.depends_on, vec!["b1c4", "e7d0"]);
        assert_eq!(meta.assigned_to, Some("worker-1".into()));
        assert_eq!(meta.path, "epic/login.task");
        assert_eq!(meta.summary_line, "implement the auth flow.");
    }

    #[test]
    fn test_parse_minimal_task() {
        let meta = parse_task(MINIMAL_TASK, "b1c4.task").unwrap();
        assert_eq!(meta.id, "b1c4");
        assert_eq!(meta.status, "done");
        assert_eq!(meta.priority, "medium"); // default
        assert_eq!(meta.depends_on, Vec::<String>::new());
        assert_eq!(meta.assigned_to, None);
        assert_eq!(meta.summary_line, "");
    }

    #[test]
    fn test_parse_invalid_content() {
        assert!(parse_task("not valid scheme", "bad.task").is_err());
    }

    #[test]
    fn test_compute_blocked() {
        let tasks = vec![
            TaskMeta {
                id: "a".into(), status: "done".into(), priority: "high".into(),
                depends_on: vec![], assigned_to: None,
                path: "a.task".into(), summary_line: "".into(),
            },
            TaskMeta {
                id: "b".into(), status: "pending".into(), priority: "medium".into(),
                depends_on: vec!["a".into()], assigned_to: None,
                path: "b.task".into(), summary_line: "".into(),
            },
            TaskMeta {
                id: "c".into(), status: "pending".into(), priority: "high".into(),
                depends_on: vec!["b".into()], assigned_to: None,
                path: "c.task".into(), summary_line: "".into(),
            },
        ];
        let blocked = compute_blocked(&tasks);
        // b depends on a which is done → not blocked
        assert!(!blocked.contains_key("b"));
        // c depends on b which is pending → blocked
        assert_eq!(blocked["c"], vec!["b"]);
    }

    #[test]
    fn test_build_summary_table() {
        let tasks = vec![
            TaskMeta {
                id: "a3f2".into(), status: "in-progress".into(), priority: "high".into(),
                depends_on: vec![], assigned_to: None,
                path: "epic/login.task".into(), summary_line: "implement the auth flow".into(),
            },
            TaskMeta {
                id: "b1c4".into(), status: "done".into(), priority: "medium".into(),
                depends_on: vec![], assigned_to: None,
                path: "ui/nav.task".into(), summary_line: "redesign nav".into(),
            },
        ];
        let table = build_summary_table(&tasks);
        assert!(table.contains("--- tasks ---"));
        assert!(table.contains("a3f2"));
        assert!(table.contains("in-progress"));
        assert!(table.contains("--- 1 active, 1 done ---"));
    }

    #[test]
    fn test_build_summary_empty() {
        let table = build_summary_table(&[]);
        assert!(table.is_empty(), "no tasks = no injection");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core state::tasks::tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL (todo! panics)

**Step 3: Implement parse_task**

Replace the `parse_task` todo with an implementation that:
1. Calls `tein_sexp::parser::parse_all(content)` to get a `Vec<Sexp>`
2. First datum must be a `SexpKind::List` — iterate pairs to extract metadata fields
3. For each alist entry `(key . value)` (a `DottedList` or `List` of length 2+), match on key symbol name
4. `id`, `assigned-to` → extract string value
5. `status`, `priority` → extract symbol name (as string)
6. `depends-on` → collect tail elements as strings (it's `(depends-on "id1" "id2")`, a flat list not a dotted pair)
7. Second datum (if present and `SexpKind::String`) → take first line as summary
8. Default `priority` to `"medium"` if absent

**Step 4: Implement compute_blocked**

Build a `HashMap<String, String>` of id→status from the task list, then for each task with non-empty `depends_on`, collect dep IDs whose status is not `"done"`. Return map of task_id → vec of blocking dep IDs (only entries with non-empty blockers).

**Step 5: Implement build_summary_table**

Return empty string if tasks is empty (no injection). Otherwise:
1. Call `compute_blocked` to get blocker map
2. Build header: `"--- tasks ---\n"`
3. Build column header: `"id     status      priority  path              summary\n"`
4. For each task, format one row. Append `" (blocked by: x, y)"` to summary if in blocker map.
5. Build footer counting active (not done) tasks, blocked count, done count.

**Step 6: Register module**

In `crates/chibi-core/src/state/mod.rs`, add `pub mod tasks;` alongside existing module declarations.

**Step 7: Run tests to verify they pass**

Run: `cargo test -p chibi-core state::tasks -- --nocapture 2>&1 | tail -15`
Expected: all pass

**Step 8: Commit**

```
feat: task metadata parser and summary table builder (#186)

tein-sexp based parser for .task files, blocked-status computation,
and ephemeral table summary generation.
```

---

## Task 3: ephemeral injection in send.rs

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:1958` (after message assembly, before tools prep)
- Modify: `crates/chibi-core/src/api/send.rs:420,492-496` (remove todos injection)

**Step 1: Write a test for ephemeral injection**

The test should verify that:
- A system message containing the task summary appears before the last user message in the API request
- The transcript does NOT contain the ephemeral message
- When no tasks exist, no message is injected

Add test in `crates/chibi-core/src/state/tasks.rs` (or a new integration test if send.rs testing patterns require it — check existing send.rs tests for patterns).

**Step 2: Add task collection helper**

In `crates/chibi-core/src/state/tasks.rs`, add an async function:

```rust
/// Collect task metadata from all accessible task directories.
///
/// Reads `/home/<ctx>/tasks/` and `/flocks/<flock>/tasks/` for each
/// flock the context belongs to. Parses metadata only (first datum).
pub async fn collect_tasks(
    vfs: &crate::vfs::Vfs,
    context_name: &str,
) -> Vec<TaskMeta> {
    // ...
}
```

This function:
1. Lists flocks for context via `vfs.flock_list_for(context_name)`
2. Walks `/home/{context_name}/tasks/` recursively (VFS list + read)
3. Walks `/flocks/{flock}/tasks/` for each flock
4. Parses each `.task` file, skips unreadable/unparseable
5. Returns all collected `TaskMeta` entries

Use the same recursive walking pattern as `scan_zone` in `synthesised.rs:625-676` — list entries, recurse into directories, filter for `.task` extension, read content, parse metadata.

For the `path` field in TaskMeta, use the path relative to the tasks root (e.g. `"epic/login.task"`) and annotate flock tasks with the source (e.g. `"deploy.task (flock:infra)"`).

**Step 3: Wire injection into send_prompt**

In `send.rs`, after the message loop (line 1958) and before tools preparation (line 1960):

```rust
// === Ephemeral task injection ===
{
    let tasks = crate::state::tasks::collect_tasks(&app.vfs, context_name).await;
    let summary = crate::state::tasks::build_summary_table(&tasks);
    if !summary.is_empty() {
        // Insert system message before the last user message
        let inject = serde_json::json!({
            "role": "system",
            "content": summary,
        });
        // Find the last user message position and insert before it
        if let Some(pos) = messages.iter().rposition(|m| m["role"] == "user") {
            messages.insert(pos, inject);
        } else {
            messages.push(inject);
        }
    }
}
```

**Note on mid-array system messages:** This is safe. `to_ratatoskr_message` in `gateway.rs` handles `"role": "system"` anywhere in the array, converting it to `Message::system(...)`. Ratatoskr's `to_llm_messages` then extracts system messages from the array (last one wins if multiple) and passes them as a top-level `system_prompt` to the provider. The mid-array position is fully supported — ratatoskr does not require system messages to be first.

This message is never persisted — it's constructed at request time and only exists in the local `messages` vec that gets sent to the API.

**Step 4: Run tests**

Run: `cargo test -p chibi-core 2>&1 | tail -10`
Expected: all pass

**Step 5: Commit**

```
feat: ephemeral task injection in prompt (#186)

collects task metadata from VFS at request time and injects a
summary table as a system message before the current user turn.
never persisted to transcript — cache-friendly.
```

---

## Task 4: remove todos system

**Files:**
- Modify: `crates/chibi-core/src/state/prompts.rs:199-218` (remove load_todos, save_todos)
- Modify: `crates/chibi-core/src/tools/memory.rs:13,32-43,162-167` (remove TODOS_TOOL_NAME, def, handler)
- Modify: `crates/chibi-core/src/api/send.rs:420,492-496` (remove todos loading and injection)
- Modify: any tests referencing todos

**Step 1: Remove todos from send.rs**

- Remove `let todos = app.load_todos(context_name)?;` at line 420
- Remove the todos injection block at lines 492-496
- Remove `"todos": todos` from `pre_sys_hook_data` at line 427 and from the second hook payload at line 513 (`post_system_prompt`)
- **Breaking change:** plugins consuming `pre_system_prompt` or `post_system_prompt` hook payloads that read the `todos` key will break. Tracked in #214 — audit `chibi/plugins` before merging.

**Step 2: Remove update_todos tool from memory.rs**

- Remove `TODOS_TOOL_NAME` constant (line 13)
- Remove its `BuiltinToolDef` entry from `MEMORY_TOOL_DEFS` (lines 32-43)
- Remove its handler branch from `execute_memory_tool` (lines 162-167)
- Remove `load_todos_for` call in `execute_read_context` (line 272) — replace the `todos` field in the returned JSON with a task summary: call `crate::state::tasks::collect_tasks` (blocking via `vfs_block_on`) and `build_summary_table`, insert as `"tasks"` key. The `execute_read_context` function is not async so use `vfs_block_on` consistently with the surrounding code pattern.
- Update `test_todos_tool_api_format` test (lines 368-377) — delete it
- Update the tool count assertion `assert_eq!(MEMORY_TOOL_DEFS.len(), 6)` (line 401) → `5`

**Step 3: Remove load_todos / save_todos from prompts.rs**

- Remove `load_todos` (lines 199-209) and `save_todos` (lines 212-218)
- Remove any imports/references these functions use

**Step 4: Fix compilation — chase all references**

Run: `cargo build -p chibi-core 2>&1 | head -40`

Fix any remaining references to `load_todos`, `save_todos`, `TODOS_TOOL_NAME`, or the removed functions. Common locations:
- Hook payload construction in `send.rs` (around lines 420-431)
- `AppState` impl if load/save are methods there
- Doc comments referencing todos

**Step 5: Run tests**

Run: `cargo test -p chibi-core 2>&1 | tail -10`
Expected: all pass

**Step 6: Commit**

```
refactor: remove flat todos.md system (#186)

replaced by structured task files with ephemeral injection.
removes update_todos tool, load/save_todos, and system prompt injection.
```

---

## Task 5: tein task plugin

**Files:**
- Create: the `.scm` plugin file (placed in the repo, installed to VFS at startup or by user)

This is the tein synthesised tool plugin that exposes task CRUD to the LLM. It uses `define-tool` for multi-tool-per-file and `call-tool` for VFS operations.

**Step 1: Write the plugin**

Create a file (e.g. `plugins/tasks.scm` in the repo, to be copied to `/tools/shared/tasks.scm` in VFS):

```scheme
(import (scheme base))
(import (scheme char))
(import (harness tools))

;;; --- helpers ---

;; generate a 4-char hex id — provided by harness (task 7)
;; (generate-id) is defined as a foreign function in build_tein_context
;; do not call it here until task 7 is complete; leave as a forward reference

;; format timestamp as YYYYMMDD-HHMMz (placeholder — no clock access in sandbox)
(define (timestamp)
  "00000000-0000z")

;; escape a string for writing as a scheme datum
(define (escape-string s)
  (let loop ((i 0) (out '()))
    (if (= i (string-length s))
        (list->string (reverse out))
        (let ((c (string-ref s i)))
          (cond
            ((char=? c #\") (loop (+ i 1) (cons #\" (cons #\\ out))))
            ((char=? c #\\) (loop (+ i 1) (cons #\\ (cons #\\ out))))
            (else (loop (+ i 1) (cons c out))))))))

;; serialise a task to file content (two datums)
(define (serialise-task meta body)
  (string-append
    "(" (meta->sexp meta) ")\n\n"
    "\"" (escape-string body) "\"\n"))

;; convert metadata alist to s-expression string (inner pairs)
(define (meta->sexp meta)
  (let loop ((pairs meta) (out ""))
    (if (null? pairs)
        out
        (let* ((pair (car pairs))
               (key (car pair))
               (val (cdr pair))
               (sep (if (string=? out "") "" "\n "))
               (entry
                 (cond
                   ;; depends-on is a flat list: (depends-on "id1" "id2")
                   ((string=? key "depends-on")
                    (string-append "(" key
                      (let dep-loop ((deps val) (s ""))
                        (if (null? deps) s
                            (dep-loop (cdr deps)
                                      (string-append s " \"" (car deps) "\""))))
                      ")"))
                   ;; symbol values: status, priority
                   ((or (string=? key "status") (string=? key "priority"))
                    (string-append "(" key " . " val ")"))
                   ;; string values: everything else
                   (else
                    (string-append "(" key " . \"" (escape-string val) "\")")))))
          (loop (cdr pairs) (string-append out sep entry))))))

;; determine the base VFS path for task operations
;; if path starts with "flock:" use /flocks/<name>/tasks/, else /home/<ctx>/tasks/
(define (resolve-task-base path context-name)
  (if (and (> (string-length path) 6)
           (string=? (substring path 0 6) "flock:"))
      (let* ((rest (substring path 6 (string-length path)))
             (slash (let scan ((i 0))
                      (cond ((= i (string-length rest)) i)
                            ((char=? (string-ref rest i) #\/) i)
                            (else (scan (+ i 1))))))
             (flock (substring rest 0 slash))
             (sub (if (= slash (string-length rest)) ""
                      (substring rest (+ slash 1) (string-length rest)))))
        (values (string-append "/flocks/" flock "/tasks") sub))
      (values (string-append "/home/" context-name "/tasks") path)))

;;; --- tools ---

(define-tool task_create
  (description "Create a new task. Returns the task ID and VFS path.")
  (parameters '((path . ((type . "string")
                          (description . "task path relative to tasks root, e.g. 'epic/login' or 'flock:infra/deploy'. directories auto-created.")))
                (body . ((type . "string")
                         (description . "task description (plain text, can be multi-line)")))
                (priority . ((type . "string")
                             (description . "low, medium, or high (default: medium)")))
                (assigned-to . ((type . "string")
                                (description . "context name to assign this task to")))
                (depends-on . ((type . "string")
                               (description . "comma-separated task IDs this depends on")))))
  (execute (lambda (args)
    (let* ((path-arg (cdr (assoc "path" args)))
           (body (let ((b (assoc "body" args))) (if b (cdr b) "")))
           (priority (let ((p (assoc "priority" args))) (if p (cdr p) "medium")))
           (assigned (assoc "assigned-to" args))
           (deps-str (assoc "depends-on" args))
           (context-name (call-tool "vfs_info" '(("path" . "/sys/contexts"))))
           ;; TODO: extract actual context name from response
           (id (generate-id))
           (ts (timestamp))
           (meta `(("id" . ,id)
                   ("status" . "pending")
                   ("priority" . ,priority)
                   ,@(if assigned `(("assigned-to" . ,(cdr assigned))) '())
                   ,@(if deps-str
                         ;; parse comma-separated into list
                         `(("depends-on" . ,(string-split (cdr deps-str) #\,)))
                         '())
                   ("created" . ,ts)
                   ("updated" . ,ts))))
      ;; resolve base + ensure directory exists
      ;; write the task file
      ;; return id + path
      (let-values (((base sub) (resolve-task-base path-arg "TODO-ctx")))
        (let ((full-path (string-append base "/" sub ".task")))
          (call-tool "write_file"
            `(("path" . ,(string-append "vfs:///" full-path))
              ("content" . ,(serialise-task meta body))))
          (string-append "created task " id " at " full-path)))))))

;; ... remaining define-tool entries follow the same pattern
```

**Important:** This is a starting scaffold. The actual implementation will need iteration — especially around:
- Context name discovery (how does a synth tool know who's calling?)
- ID generation (needs a better entropy source, or accept sequential/timestamp-based)
- `let-values` requires `(scheme base)` which should include it in R7RS

**Confirmed blockers to resolve before/during task 5:**
- `string-split` is **not** in `(scheme base)` and `(scheme string)` does not exist in tein's VFS registry. A manual implementation is required. Add it to the plugin file. A simple version splitting on a char:

```scheme
(define (string-split str ch)
  (let loop ((i 0) (start 0) (acc '()))
    (cond
      ((= i (string-length str))
       (reverse (cons (substring str start i) acc)))
      ((char=? (string-ref str i) ch)
       (loop (+ i 1) (+ i 1) (cons (substring str start i) acc)))
      (else
       (loop (+ i 1) start acc)))))
```

These are implementation details to resolve during coding, not design questions.

**Step 2: Write integration tests**

In `crates/chibi-core/src/tools/synthesised.rs` (alongside existing synth tool tests), add tests that:
1. Load the tasks.scm plugin into a test VFS
2. Execute `task_create` via the registry
3. Verify the `.task` file was written to the correct VFS path
4. Execute `task_list` and verify the created task appears
5. Execute `task_update` to change status, verify file updated
6. Execute `task_view` to read back the task
7. Execute `task_delete` to remove it, verify file gone

Follow the pattern from existing tests (lines 915-991 of synthesised.rs) using `make_test_vfs()` and `make_registry()`.

**Step 3: Iterate on the plugin until tests pass**

This is the most exploratory task — the scheme code will need debugging and refinement. Key things to watch:
- `call-tool` argument format must match what the rust tools expect
- `write_file` expects a `vfs:///` URI prefix for VFS paths
- VFS directory auto-creation behaviour (may need explicit `vfs_mkdir`)
- Sandboxed tier module restrictions (stick to `(scheme base)`, `(scheme char)`)

**Step 4: Commit**

```
feat: tein task plugin with CRUD tools (#186)

synthesised tool plugin exposing task_create, task_update, task_view,
task_list, task_delete. uses call-tool bridge for VFS storage.
```

---

## Task 6: context name in synthesised tool calls

The task plugin needs to know which context is calling it (to resolve `/home/<ctx>/tasks/`). Check how the call context is passed to synthesised tools.

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (call context setup)
- Possibly modify: the `BRIDGE_CALL_CTX` thread-local

**Step 1: Investigate current call context**

Read `CallContextGuard` and `BRIDGE_CALL_CTX` in `synthesised.rs:167-204`. Determine what fields are available. The plugin needs at minimum the context name and the list of flocks.

**Step 2: Expose context name to scheme**

If not already available, add a way for the tein plugin to discover the calling context name. Options:
- A `(call-tool "context_name" '())` pseudo-tool
- A pre-defined variable `%context-name%` injected before evaluation
- Read from `/sys/contexts/` (but which one is "me"?)

Choose the simplest option that doesn't require a new tool. A pre-defined variable is cleanest — inject `(define %context-name% "ctx-name")` into the preamble before the plugin source evaluates.

**Step 3: Implement and test**

**Step 4: Commit**

```
feat: expose context name to synthesised tools (#186)
```

---

## Task 7: ID generation

The task plugin needs to generate unique-ish short hex IDs. In sandboxed mode there's no `(scheme time)` or random source.

**Files:**
- Modify: the tasks.scm plugin
- Possibly: expose a helper from rust side

**Step 1: Evaluate options**

- Option A: Add `%generate-id%` as a foreign function in the harness (rust-side, uses `rand` or timestamp)
- Option B: Use `call-tool` to read something with entropy (e.g. `vfs_info` on a path, hash it)
- Option C: Add `(scheme time)` to the Modules::Safe allowlist

Option A is cleanest — a small foreign fn in `build_tein_context` that returns a 4-hex-char string.

**Step 2: Implement**

Add to `build_tein_context` (after `call-tool` registration):

```rust
ctx.define_fn("generate-id", |_args| {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let hash = t.as_nanos() as u32;
    Ok(Value::from(format!("{:04x}", hash % 0x10000)))
})?;
```

Also expose `(current-timestamp)` returning `"YYYYMMDD-HHMMz"` format.

**Step 3: Test, commit**

```
feat: generate-id and current-timestamp harness helpers (#186)
```

---

## Task 8: update documentation

**Files:**
- Modify: `docs/plugins.md` (add task plugin documentation)
- Modify: `docs/hooks.md` (if any hook changes)
- Modify: `docs/vfs.md` (document `/home/<ctx>/tasks/` and `/flocks/<name>/tasks/`)
- Modify: `docs/configuration.md` (if any config changes)
- Modify: `docs/agentic.md` (reference task system for agentic workflows)
- Modify: `AGENTS.md` (add quirks/gotchas from implementation)

**Step 1: Update docs**

- `docs/vfs.md`: add task directories to the namespace layout
- `docs/plugins.md`: add section on the task plugin, its tools, and the `.task` file format
- `docs/agentic.md`: reference task tools for multi-agent coordination
- `AGENTS.md`: add any quirks discovered during implementation

**Step 2: Commit**

```
docs: document structured task system (#186)
```

---

## Task 9: final cleanup and lint

**Step 1: Run linter**

Run: `just lint`

**Step 2: Fix any issues**

**Step 3: Run full test suite**

Run: `cargo test -p chibi-core 2>&1 | tail -20`

**Step 4: Final commit closing the issue**

```
chore: lint and cleanup

closes #186
```

**Step 5: Remember `just pre-push` before pushing and `just merge-to-dev` for merging.**

---

## Execution Notes

- Tasks 1-4 are straightforward rust work with clear insertion points
- Task 5 (tein plugin) is the most exploratory — expect iteration
- Task 6 and 7 may be discovered as blockers during task 5 — they're broken out for clarity but may be tackled inline
- The ephemeral injection (task 3) can be tested independently of the tein plugin using manually-written `.task` files in the VFS
- Order flexibility: tasks 3 and 4 can be swapped (remove todos before or after adding injection)

---

## Progress (as of 2026-03-08)

### Completed
- **Task 1**: tein-sexp dep added (commit 99397bcc)
- **Task 2**: `state/tasks.rs` — parse_task, compute_blocked, build_summary_table, collect_tasks. All tests pass (commit c316dce0)
- **Task 3**: ephemeral injection wired into send.rs + injection-logic tests (commit 532d69b8)
- **Task 4**: todos system fully removed — prompts.rs, memory.rs, send.rs, compact.rs, execution.rs, chibi.rs, cli.rs, state/tests.rs (commit e523e3b2)

### In Progress — Task 5+6+7 (combined)

**What's done:**
- `generate-id` and `current-timestamp` harness fns added to `build_tein_context` in `synthesised.rs` (via `#[tein::tein_fn]` macro, NOT `ctx.define_fn` — only `define_fn_variadic` exists)
- `%context-name%` added to HARNESS_PREAMBLE (as mutable binding, default "")
- `execute_synthesised` injects per-call context name via `(set! %context-name% "...")` before calling
- `secs_to_ymdhmz` UTC decomposition helper added
- `plugins/tasks.scm` written — 5 define-tool tools (task_create, task_update, task_view, task_list, task_delete)
- Integration tests added at end of synthesised.rs tests block:
  - `test_tasks_plugin_loads` — PASSES (5 tools registered)
  - `test_harness_helpers_generate_id_and_timestamp` — PASSES
  - `test_context_name_injection` — PASSES
  - `test_task_crud_integration` — FAILS

**Critical bug — BRIDGE_CALL_CTX thread mismatch:**

`ThreadLocalContext` (tein) runs scheme on a DEDICATED WORKER THREAD. C FFI functions like `call-tool` are called FROM that worker thread. But `CallContextGuard::set(call.context)` sets `BRIDGE_CALL_CTX` on the CALLER thread (the tokio thread calling `execute_synthesised`). Result: `BRIDGE_CALL_CTX` is always `None` on the tein thread → "no active call context" error whenever a synthesised tool calls `call-tool`.

Same issue applies to `BRIDGE_REGISTRY` — but it works because... actually it may be broken too and the tests that pass just don't hit it.

**Fix needed in `execute_synthesised`:**

Instead of using `thread_local! BRIDGE_CALL_CTX` (which is per-thread), use a global `Mutex<Option<ActiveCallContext>>` or use a per-context-ID approach. The simplest fix: replace `BRIDGE_CALL_CTX` (thread-local) with a `Mutex<HashMap<ThreadId, ActiveCallContext>>` keyed by the tein worker thread ID, OR use a global `OnceLock<Mutex<Option<ActiveCallContext>>>`.

Actually the simplest fix: change `BRIDGE_CALL_CTX` from `thread_local!` to a global `static Mutex` (or `RwLock`) since all scheme calls from one instance are serialised anyway (the `ThreadLocalContext` has a `Mutex<...>` on the channel, so concurrent calls can't interleave). Use `Arc<Mutex<Option<ActiveCallContext>>>` stored inside `ThreadLocalContext` or as a global.

**Simplest concrete fix:** In `execute_synthesised`, after setting `BRIDGE_CALL_CTX` on the current thread, also pass the context data TO the tein thread via a tein evaluate call that stores it in a scheme variable, which `call_tool_fn` can read by reconstructing from scheme. OR: make `BRIDGE_CALL_CTX` a global `Mutex<Option<ActiveCallContext>>` (safe since tein channels are serialized per context).

**Recommended fix:** Change `BRIDGE_CALL_CTX` from `thread_local!` to:
```rust
static BRIDGE_CALL_CTX: std::sync::Mutex<Option<ActiveCallContext>> = std::sync::Mutex::new(None);
```
Update `CallContextGuard::set` to lock and set it, `Drop` to clear it, and `call_tool_fn` to use `BRIDGE_CALL_CTX.lock().unwrap()` instead of `.with(...)`.

**CAVEAT:** `ActiveCallContext` contains raw pointers (`*const AppState` etc.) which are only valid for the duration of `execute_synthesised`. Making it global (not thread-local) is safe as long as it's only accessed while the guard is alive. This is guaranteed by `CallContextGuard` RAII. The global mutex also serializes access, which matches the serialized nature of tein channel calls.

**After fixing this**, the full CRUD test should pass. Also need to commit tasks 5+6+7.

### Not Started
- Task 8: documentation
- Task 9: lint + final commit

### Files modified since last commit (unstaged/uncommitted)
- `crates/chibi-core/src/tools/synthesised.rs` — %context-name%, generate-id, current-timestamp, tests
- `plugins/tasks.scm` — new file

### Run before committing
```
cargo test -p chibi-core --lib "synthesised::tests" -- --nocapture
```
All tests should pass including test_task_crud_integration.
