# Synth Tool Discoverability Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make synthesised tool APIs (`define-tool`, `call-tool`) self-discoverable via runtime introspection so an LLM can find and understand them without looping.

**Architecture:** Three targeted fixes: (1) inject a `harness-tools-docs` alist into `HARNESS_PREAMBLE` so `(describe harness-tools-docs)` surfaces the full harness API; (2) improve `describe` error message in tein's `docs.scm` for non-alist input; (3) update `chibi.md` system prompt to point LLMs at `harness-tools-docs`. Tests in `eval.rs` verify both `harness-tools-docs` presence and `describe` behaviour in `scheme_eval` contexts (the actual LLM-facing path).

**Tech Stack:** Rust (`synthesised.rs`, `eval.rs`), Scheme (`tein/docs.scm`, `HARNESS_PREAMBLE`), Markdown (`chibi.md`)

**tein dependency:** git dep at `git = "https://github.com/emesal/tein", branch = "main"`. Local fork is `~/forks/chibi-scheme`. Changes to `docs.scm` require push to remote + `cargo update -p tein` in chibi.

**`tein::Value::String` inner type:** wraps a plain `String`. `.contains()` works directly.

---

## Chunk 1: harness-tools-docs alist + describe error fix

### Task 1: Add `harness-tools-docs` alist to `HARNESS_PREAMBLE`

**Files:**
- Modify: `crates/chibi-core/src/tools/synthesised.rs` (the `HARNESS_PREAMBLE` const, lines ~311–347)
- Test: `crates/chibi-core/src/tools/eval.rs`

**Background:** `HARNESS_PREAMBLE` is a Scheme string evaluated at the start of every synthesised-tool and `scheme_eval` context. `binding-info` and `describe-environment/text` don't surface `define-tool` because it's not in tein's build-time `MODULE_EXPORTS` table — it's a top-level syntax form, not a library export. `(harness tools)` is runtime-registered so `(module-exports '(harness tools))` errors. The fix: add a docs alist named `harness-tools-docs` in the preamble, following the same pattern as `introspect-docs`. The LLM can then call `(describe harness-tools-docs)` or `(module-doc harness-tools-docs 'define-tool)`. Only public-facing bindings go in the alist — no `%`-prefixed internals.

Tests go in `eval.rs` using `build_sandboxed_harness_context()` directly (same path `scheme_eval` uses) — this correctly exercises the binding at runtime rather than only at load time.

- [ ] **Step 1.1: Write failing tests in `eval.rs`**

In `crates/chibi-core/src/tools/eval.rs`, inside the `#[cfg(test)]` block, add:

```rust
#[test]
fn test_harness_tools_docs_is_alist() {
    // harness-tools-docs must be a non-empty pair in every scheme_eval context
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session
        .evaluate("(pair? harness-tools-docs)")
        .expect("evaluate");
    assert_eq!(result, tein::Value::Boolean(true));
}

#[test]
fn test_harness_tools_docs_has_define_tool_entry() {
    // must have a 'define-tool key with a non-empty string doc
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session
        .evaluate("(let ((e (assq 'define-tool harness-tools-docs))) (and (pair? e) (string? (cdr e)) (not (string=? \"\" (cdr e)))))")
        .expect("evaluate");
    assert_eq!(result, tein::Value::Boolean(true));
}

#[test]
fn test_harness_tools_docs_has_call_tool_entry() {
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session
        .evaluate("(let ((e (assq 'call-tool harness-tools-docs))) (and (pair? e) (string? (cdr e))))")
        .expect("evaluate");
    assert_eq!(result, tein::Value::Boolean(true));
}

#[test]
fn test_describe_harness_tools_docs_mentions_define_tool() {
    // (describe harness-tools-docs) must return a string mentioning define-tool
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session
        .evaluate("(describe harness-tools-docs)")
        .expect("evaluate");
    match result {
        tein::Value::String(s) => assert!(
            s.contains("define-tool"),
            "expected define-tool in describe output, got: {s}"
        ),
        other => panic!("expected string, got: {other:?}"),
    }
}
```

- [ ] **Step 1.2: Run tests to confirm they fail**

```bash
cargo test -p chibi-core --features synthesised-tools test_harness_tools_docs test_describe_harness_tools_docs 2>&1 | grep -E "FAILED|error\[|panicked"
```

Expected: compile error or test failure — `harness-tools-docs` is not yet defined.

- [ ] **Step 1.3: Add `harness-tools-docs` to `HARNESS_PREAMBLE`**

In `crates/chibi-core/src/tools/synthesised.rs`, update the `HARNESS_PREAMBLE` const. Add the alist after `%context-name%` and before `define-tool`. Keep only public-facing bindings (no `%`-prefixed internals):

```rust
pub(crate) const HARNESS_PREAMBLE: &str = r#"
(import (scheme base))

;; accumulates define-tool entries. each entry is a list:
;; (name-string description-string params-value execute-procedure)
(define %tool-registry% '())

;; accumulates hook registrations. each entry is a list:
;; (hook-name-string handler-procedure)
;; rust reads %hook-registry% after evaluation to populate Tool.hooks.
(define %hook-registry% '())

;; name of the calling context — mutated by execute_synthesised before each call.
;; plugins read this to resolve /home/<ctx>/... VFS paths.
(define %context-name% "")

;; docs alist for public harness APIs — use (describe harness-tools-docs) or
;; (module-doc harness-tools-docs 'define-tool) to look up usage.
;; follows the same convention as introspect-docs, json-docs, etc.
;; note: (describe X) takes an alist directly, NOT a symbol.
(define harness-tools-docs
  '((__module__ . "harness tools")
    (define-tool . "macro: (define-tool name (description DESC) (parameters PARAMS-ALIST) (execute (lambda (args) ...))) — registers a persistent tool; args is ((\"key\" . val) ...) alist")
    (call-tool . "procedure: (call-tool NAME ARGS-ALIST) -> string — invoke another registered tool; NAME is a string, ARGS-ALIST is ((\"key\" . \"val\") ...)")
    (register-hook . "procedure: (register-hook HOOK-SYMBOL HANDLER) — register a hook callback; HOOK-SYMBOL e.g. 'pre_vfs_write, HANDLER is (lambda (payload) ...)")))

;; registers a tool: appends to %tool-registry% in definition order (LIFO via cons).
;; rust reads %tool-registry% after evaluation; non-empty → multi-tool mode.
(define-syntax define-tool
  (syntax-rules (description parameters execute)
    ((define-tool name
       (description desc)
       (parameters params)
       (execute handler))
     (set! %tool-registry%
       (cons (list (symbol->string 'name) desc params handler)
             %tool-registry%)))))

;; registers a hook handler for a given hook point.
;; hook-name is a symbol (e.g. 'pre_vfs_write).
;; handler is a procedure taking one argument (the hook payload as an alist)
;; and returning an alist (or '() for no-op).
(define (register-hook hook-name handler)
  (set! %hook-registry%
    (cons (list (symbol->string hook-name) handler)
          %hook-registry%)))
"#;
```

- [ ] **Step 1.4: Update the doc comment on `HARNESS_PREAMBLE`**

Replace the existing `///` comment block above `HARNESS_PREAMBLE`:

```rust
/// Top-level scheme preamble evaluated in every synthesised tool context.
///
/// Defines `%tool-registry%`, `%hook-registry%`, `%context-name%`, `define-tool`,
/// `register-hook`, and `harness-tools-docs` at the top level.
///
/// `harness-tools-docs` is a docs alist (same convention as `introspect-docs`) covering
/// the public harness API. Call `(describe harness-tools-docs)` or
/// `(module-doc harness-tools-docs 'define-tool)` to retrieve usage docs.
/// Note: `(describe X)` takes an alist directly, not a symbol.
///
/// `define-tool` must be top-level (not inside a library) so its `set!` of
/// `%tool-registry%` affects the top-level binding that rust reads post-evaluation.
///
/// Mutation site: if `define-tool` syntax changes, update `extract_multi_tools`
/// which parses `%tool-registry%` entries. If `harness-tools-docs` entries change,
/// update `chibi.md` accordingly.
```

- [ ] **Step 1.5: Run tests to confirm they pass**

```bash
cargo test -p chibi-core --features synthesised-tools test_harness_tools_docs test_describe_harness_tools_docs 2>&1 | grep -E "ok|FAILED"
```

Expected: all four tests pass.

- [ ] **Step 1.6: Run full test suite**

```bash
cargo test -p chibi-core --features synthesised-tools 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 1.7: Commit**

```bash
git add crates/chibi-core/src/tools/synthesised.rs crates/chibi-core/src/tools/eval.rs
git commit -m "feat: add harness-tools-docs alist to HARNESS_PREAMBLE for LLM introspection"
```

---

### Task 2: Fix `describe` error message for non-alist input

**Files:**
- Modify: `/home/fey/forks/chibi-scheme/lib/tein/docs.scm`
- Test: `crates/chibi-core/src/tools/eval.rs`

**Background:** `describe` from `(tein docs)` takes a docs alist and formats it. When called with a non-pair (e.g. a quoted symbol like `'define-tool`), it crashes inside `module-doc` with `car: not a pair: <symbol>` — an opaque chibi-scheme internal error that gives the LLM no guidance. The fix: guard the input at the top of `describe` and return a helpful message if the argument is not a pair.

`tein` is a git dep — changes to `docs.scm` require push + `cargo update -p tein`.

- [ ] **Step 2.1: Write failing tests in `eval.rs`**

In `crates/chibi-core/src/tools/eval.rs` tests, add after the Task 1 tests:

```rust
#[test]
fn test_describe_symbol_gives_helpful_error() {
    // Before fix: (describe 'define-tool) crashes with "car: not a pair"
    // After fix: must return a helpful string, not raise an exception
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session.evaluate("(describe 'define-tool)").expect("evaluate");
    match result {
        tein::Value::String(s) => {
            assert!(
                s.contains("alist") || s.contains("harness-tools-docs"),
                "expected helpful guidance mentioning alist or harness-tools-docs, got: {s}"
            );
        }
        other => panic!("expected string result from describe, got: {other:?}"),
    }
}

#[test]
fn test_describe_number_gives_helpful_error() {
    // non-symbol non-pair input must also be handled gracefully
    let (session, _tid) = super::synthesised::build_sandboxed_harness_context()
        .expect("build context");
    let result = session.evaluate("(describe 42)").expect("evaluate");
    match result {
        tein::Value::String(s) => assert!(
            s.contains("alist") || s.contains("non-list"),
            "expected helpful guidance, got: {s}"
        ),
        other => panic!("expected string, got: {other:?}"),
    }
}
```

- [ ] **Step 2.2: Run failing tests to confirm they fail before the fix**

```bash
cargo test -p chibi-core --features synthesised-tools test_describe_symbol_gives_helpful_error test_describe_number_gives_helpful_error 2>&1 | grep -E "FAILED|panicked|ok"
```

Expected: FAILED or panicked — `describe` crashes with `car: not a pair` or raises a scheme error that surfaces as a Rust panic.

- [ ] **Step 2.3: Update `describe` in `tein/docs.scm`**

In `/home/fey/forks/chibi-scheme/lib/tein/docs.scm`, replace the `describe` function:

```scheme
(define (describe alist)
  (if (not (pair? alist))
      (string-append
        "error: describe expects a docs alist (e.g. introspect-docs, harness-tools-docs), got: "
        (if (symbol? alist)
            (string-append "symbol '" (symbol->string alist)
              " — did you mean (describe " (symbol->string alist) "-docs)?")
            "a non-list value"))
      (let ((mod (module-doc alist '__module__))
            (entries (module-docs alist)))
        (apply string-append
          (append
            (if mod (list "(" mod ")\n") '())
            (map (lambda (p)
                   (string-append
                     "  " (symbol->string (car p))
                     (if (string=? (cdr p) "")
                         "\n"
                         (string-append " — " (cdr p) "\n"))))
                 entries))))))
```

- [ ] **Step 2.4: Commit and push the tein fork**

```bash
git -C /home/fey/forks/chibi-scheme add lib/tein/docs.scm
git -C /home/fey/forks/chibi-scheme commit -m "fix: describe gives helpful error for non-alist input"
git -C /home/fey/forks/chibi-scheme push
```

- [ ] **Step 2.5: Update tein in chibi's Cargo.lock**

```bash
cargo update -p tein
```

- [ ] **Step 2.6: Run failing tests to confirm they now pass**

```bash
cargo test -p chibi-core --features synthesised-tools test_describe_symbol_gives_helpful_error test_describe_number_gives_helpful_error 2>&1 | grep -E "ok|FAILED"
```

Expected: both pass.

- [ ] **Step 2.7: Run full test suite**

```bash
cargo test -p chibi-core --features synthesised-tools 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 2.8: Commit**

```bash
git add crates/chibi-core/src/tools/eval.rs Cargo.lock
git commit -m "fix: update tein for describe non-alist error message improvement"
```

---

## Chunk 2: system prompt + wrap up

### Task 3: Update `chibi.md` system prompt

**Files:**
- Modify: `crates/chibi-core/prompts/chibi.md`

**Background:** The current system prompt has the right examples but doesn't tell the LLM how to discover the API at runtime. Adding a pointer to `harness-tools-docs` and an explicit note that `describe` takes an alist (not a symbol) prevents the loop seen in the transcript.

- [ ] **Step 3.1: Replace the `**synthesised tools**` section in `chibi.md`**

In `crates/chibi-core/prompts/chibi.md`, replace the entire `**synthesised tools**` block (from `**synthesised tools**` through the final `- deploy with:` line) with:

```markdown
**synthesised tools**
- write a .scm file to the VFS under /tools/ to create a persistent tool callable by the LLM
  - /tools/shared/ for tools available to all contexts
  - /tools/home/<context>/ private to this context
- same prelude as scheme_eval — all standard modules pre-imported, no explicit (import ...) needed
- for single-tool format, define: tool-name, tool-description, tool-parameters, (tool-execute args)
- multi-tool format: use (import (harness tools)) and the define-tool macro
- (assoc "key" args) extracts call arguments; keys are strings, not symbols
- call-tool invokes other registered tools: (call-tool "name" '(("arg" . "val")))
- tools register automatically on write — no restart needed, live on next turn
- runtime API docs available in every context: (describe harness-tools-docs)
  - or: (module-doc harness-tools-docs 'define-tool) for a specific entry
  - important: (describe X) takes a docs alist directly — NOT a symbol
- single-tool example:
  ```scheme
  (define tool-name        "greet")
  (define tool-description "greets someone by name")
  (define tool-parameters
    '((name . ((type . "string") (description . "the name to greet")))))
  (define (tool-execute args)
    (string-append "hello, " (cdr (assoc "name" args)) "!"))
  ```
- multi-tool example (define-tool):
  ```scheme
  (import (harness tools))
  (define-tool greet
    (description "greets someone")
    (parameters '((name . ((type . "string") (description . "the name")))))
    (execute (lambda (args)
      (string-append "hello, " (cdr (assoc "name" args)) "!"))))
  ```
- deploy with: write_file {"path": "vfs:///tools/shared/my_tool.scm", "content": "..."}
```

- [ ] **Step 3.2: Verify the file looks correct**

```bash
cat crates/chibi-core/prompts/chibi.md
```

- [ ] **Step 3.3: Commit**

```bash
git add crates/chibi-core/prompts/chibi.md
git commit -m "docs: add harness-tools-docs runtime discovery to chibi.md system prompt"
```

---

### Task 4: Lint and wrap up

- [ ] **Step 4.1: Run lint**

```bash
just lint 2>&1 | tail -30
```

Fix any clippy warnings in touched files.

- [ ] **Step 4.2: Commit lint fixes if any**

```bash
git add -p
git commit -m "chore: lint fixes for synth tool discoverability changes"
```

- [ ] **Step 4.3: Add AGENTS.md notes**

Add to the `# Quirks / Gotchas / etc` section of `AGENTS.md`:

```
- `harness-tools-docs` is a docs alist (same convention as `introspect-docs`) available in every synthesised-tool and `scheme_eval` context. Use `(describe harness-tools-docs)` to list the full public harness API, or `(module-doc harness-tools-docs 'define-tool)` for a specific entry. `describe` takes an alist directly — NOT a symbol.
- `(module-exports '(harness tools))` errors — `(harness tools)` is runtime-registered and absent from tein's build-time `MODULE_EXPORTS` table. Use `harness-tools-docs` for API discovery instead.
```

- [ ] **Step 4.4: Final commit**

```bash
git add AGENTS.md
git commit -m "docs: add harness-tools-docs discoverability notes to AGENTS.md"
```
