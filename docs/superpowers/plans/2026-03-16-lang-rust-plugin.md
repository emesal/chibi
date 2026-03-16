# lang_rust Plugin Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build chibi's first language plugin — a standalone rust binary using tree-sitter to extract symbols and references from rust source files.

**Architecture:** Standalone binary (`lang_rust`) in `chibi/plugins` repo, communicating with chibi-core via the existing JSON language plugin protocol. One core-side patch in `chibi/chibi` to wire up `parent_id` resolution using line-range containment. TDD throughout.

**Tech Stack:** Rust, tree-sitter, tree-sitter-rust, serde/serde_json

**Spec:** `docs/superpowers/specs/2026-03-16-lang-rust-plugin-design.md`

---

## Chunk 1: Core-Side Patch — Parent Resolution

This chunk modifies `chibi/chibi` (the main repo). It patches `insert_symbols` in `indexer.rs` to resolve `parent` names from plugin output into `parent_id` foreign keys using line-range containment.

### Task 1: Test parent_id resolution

**Repo:** `chibi/chibi`
**Files:**
- Modify: `crates/chibi-core/src/index/indexer.rs` (tests section, before the closing `}` of `mod tests` at line 580)

- [ ] **Step 1: Write failing test — parent_id resolved from plugin output**

Add to the existing `mod tests` block in `indexer.rs`:

```rust
#[test]
fn insert_symbols_resolves_parent_id() {
    let (conn, dir) = setup_temp_project();
    let _ = dir;

    conn.execute(
        "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
        [],
    )
    .unwrap();

    let output = serde_json::json!({
        "symbols": [
            {"name": "Parser", "kind": "struct", "line_start": 1, "line_end": 10},
            {"name": "input", "kind": "field", "line_start": 2, "line_end": 2, "parent": "Parser"},
            {"name": "Parser", "kind": "impl", "line_start": 12, "line_end": 25},
            {"name": "new", "kind": "function", "line_start": 13, "line_end": 20, "parent": "Parser"}
        ]
    });

    insert_symbols(&conn, 1, &output);

    // "input" (field at line 2) should have parent_id pointing to "Parser" (struct at lines 1-10).
    let field_parent: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM symbols WHERE name = 'input' AND kind = 'field'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let struct_id: i64 = conn
        .query_row(
            "SELECT id FROM symbols WHERE name = 'Parser' AND kind = 'struct'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(field_parent, Some(struct_id));

    // "new" (function at line 13) should have parent_id pointing to "Parser" (impl at lines 12-25),
    // NOT the struct at lines 1-10 (which doesn't contain line 13).
    let fn_parent: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM symbols WHERE name = 'new' AND kind = 'function'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let impl_id: i64 = conn
        .query_row(
            "SELECT id FROM symbols WHERE name = 'Parser' AND kind = 'impl'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fn_parent, Some(impl_id));
}

#[test]
fn insert_symbols_no_parent_still_works() {
    let (conn, dir) = setup_temp_project();
    let _ = dir;

    conn.execute(
        "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
        [],
    )
    .unwrap();

    let output = serde_json::json!({
        "symbols": [
            {"name": "main", "kind": "function", "line_start": 1, "line_end": 5}
        ]
    });

    let count = insert_symbols(&conn, 1, &output);
    assert_eq!(count, 1);

    let parent_id: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM symbols WHERE name = 'main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent_id, None);
}

#[test]
fn insert_symbols_unresolvable_parent_stays_null() {
    let (conn, dir) = setup_temp_project();
    let _ = dir;

    conn.execute(
        "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
        [],
    )
    .unwrap();

    // Parent "Nonexistent" doesn't match any symbol — should gracefully leave parent_id NULL.
    let output = serde_json::json!({
        "symbols": [
            {"name": "orphan", "kind": "function", "line_start": 1, "line_end": 5, "parent": "Nonexistent"}
        ]
    });

    let count = insert_symbols(&conn, 1, &output);
    assert_eq!(count, 1);

    let parent_id: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM symbols WHERE name = 'orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent_id, None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core insert_symbols_resolves_parent_id insert_symbols_no_parent_still_works insert_symbols_unresolvable_parent_stays_null -- --nocapture`

Expected: `insert_symbols_resolves_parent_id` FAILS (parent_id is always NULL). The other two pass with current code (they both assert NULL parent_id, which is the current default behaviour). Only `resolves_parent_id` is the actual failing test driving the implementation.

### Task 2: Implement parent_id resolution

**Repo:** `chibi/chibi`
**Files:**
- Modify: `crates/chibi-core/src/index/indexer.rs:281-311` (the `insert_symbols` function)

- [ ] **Step 3: Implement two-pass insert with line-range containment**

Replace the `insert_symbols` function body:

```rust
/// Insert symbols from plugin output into the database. Returns count of symbols added.
///
/// Uses a two-pass approach for parent resolution:
/// 1. Insert all symbols with parent_id NULL, collecting (id, name, line_start, line_end, parent_name).
/// 2. For each symbol with a parent name, find the matching parent by name + line-range containment
///    and UPDATE parent_id.
fn insert_symbols(conn: &Connection, file_id: i64, output: &serde_json::Value) -> u32 {
    let symbols = match output.get("symbols").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return 0,
    };

    // First pass: insert all symbols, collect metadata for parent resolution.
    struct SymMeta {
        id: i64,
        name: String,
        line_start: i64,
        line_end: i64,
        parent_name: Option<String>,
    }
    let mut metas: Vec<SymMeta> = Vec::new();
    let mut count = 0u32;

    for sym in symbols {
        let name = sym.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = sym.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let line_start = sym.get("line_start").and_then(|v| v.as_i64()).unwrap_or(0);
        let line_end = sym.get("line_end").and_then(|v| v.as_i64()).unwrap_or(0);
        let signature = sym.get("signature").and_then(|v| v.as_str());
        let visibility = sym.get("visibility").and_then(|v| v.as_str());
        let parent_name = sym.get("parent").and_then(|v| v.as_str()).map(|s| s.to_string());

        let result = conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![file_id, name, kind, line_start, line_end, signature, visibility],
        );

        if result.is_ok() {
            let id = conn.last_insert_rowid();
            metas.push(SymMeta {
                id,
                name: name.to_string(),
                line_start,
                line_end,
                parent_name,
            });
            count += 1;
        }
    }

    // Second pass: resolve parent_id via line-range containment.
    for meta in &metas {
        if let Some(ref parent_name) = meta.parent_name {
            // Find the nearest enclosing parent: name matches AND parent's line range contains child.
            let parent_id = metas
                .iter()
                .filter(|p| {
                    p.name == *parent_name
                        && p.line_start <= meta.line_start
                        && p.line_end >= meta.line_end
                        && p.id != meta.id
                })
                // Nearest enclosing = smallest containing range.
                .min_by_key(|p| p.line_end - p.line_start)
                .map(|p| p.id);

            if let Some(pid) = parent_id {
                let _ = conn.execute(
                    "UPDATE symbols SET parent_id = ?1 WHERE id = ?2",
                    rusqlite::params![pid, meta.id],
                );
            } else {
                eprintln!(
                    "index: unresolved parent \"{}\" for symbol \"{}\" at line {}",
                    parent_name, meta.name, meta.line_start
                );
            }
        }
    }

    count
}
```

- [ ] **Step 4: Run all indexer tests**

Run: `cargo test -p chibi-core -- index::indexer --nocapture`

Expected: ALL tests pass, including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/chibi-core/src/index/indexer.rs
git commit -m "feat(index): resolve parent_id via line-range containment

Two-pass insert_symbols: first pass inserts with NULL parent_id and
collects metadata, second pass resolves parent names by matching
name + smallest enclosing line range. Backwards-compatible — plugins
that don't emit parent field work exactly as before.

Prepares chibi-core for language plugins that emit parent-child
symbol hierarchies."
```

---

## Chunk 2: Plugin Scaffolding & Types

This chunk and all subsequent chunks work in the `chibi/plugins` repo at `/home/fey/projects/chibi/plugins`.

### Task 3: Scaffold the lang_rust crate

**Repo:** `chibi/plugins`
**Files:**
- Create: `lang_rust/Cargo.toml`
- Create: `lang_rust/src/main.rs`
- Create: `lang_rust/src/types.rs`
- Create: `lang_rust/src/extract.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "lang_rust"
version = "0.1.0"
edition = "2021"
description = "Chibi language plugin: extracts symbols and references from Rust source files using tree-sitter"

[dependencies]
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**Important:** After creating this file, run `cargo check -p lang_rust` from the `lang_rust/` directory. If tree-sitter version mismatch occurs (tree-sitter-rust 0.23 may depend on tree-sitter 0.23, not 0.24), adjust both versions to be compatible. The correct pair is whatever `tree-sitter-rust`'s latest release depends on. Check with `cargo tree -p lang_rust` after initial build.

- [ ] **Step 2: Create `src/types.rs`**

```rust
//! Serde types for the language plugin JSON protocol.
//!
//! Input: `{"files": [{"path": "...", "content": "..."}]}`
//! Output: `{"symbols": [...], "refs": [...]}`

use serde::{Deserialize, Serialize};

/// Top-level input from the indexer.
#[derive(Debug, Deserialize)]
pub struct Input {
    pub files: Vec<FileEntry>,
}

/// A single file to extract symbols from.
#[derive(Debug, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub content: String,
}

/// Top-level output to the indexer.
#[derive(Debug, Serialize, Default)]
pub struct Output {
    pub symbols: Vec<Symbol>,
    pub refs: Vec<Ref>,
}

/// An extracted symbol.
#[derive(Debug, Serialize, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line_start: usize,
    pub line_end: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// An extracted reference (use statement).
#[derive(Debug, Serialize, PartialEq)]
pub struct Ref {
    pub from_line: usize,
    pub to_name: String,
    pub kind: String,
}
```

- [ ] **Step 3: Create stub `src/extract.rs`**

```rust
//! AST walk over tree-sitter parse tree to extract symbols and references.
//!
//! Strategy: depth-first traversal with parent stack. See spec for full details:
//! `chibi/chibi/docs/superpowers/specs/2026-03-16-lang-rust-plugin-design.md`

use crate::types::{Output, Ref, Symbol};

/// Extract symbols and references from rust source code.
pub fn extract(source: &str) -> Output {
    let _ = source;
    Output::default()
}
```

- [ ] **Step 4: Create `src/main.rs`**

```rust
//! lang_rust — chibi language plugin for Rust.
//!
//! Extracts symbols and references from Rust source files using tree-sitter.
//! Conforms to chibi's language plugin protocol:
//! - `--schema`: print tool schema JSON and exit 0
//! - stdin: `{"files": [{"path": "...", "content": "..."}]}`
//! - stdout: `{"symbols": [...], "refs": [...]}`

mod extract;
mod types;

use types::{Input, Output};

fn main() {
    // Schema mode: print tool schema and exit.
    if std::env::args().any(|a| a == "--schema") {
        print_schema();
        return;
    }

    // Execution mode: read input from stdin, extract, write output to stdout.
    let input: Input = match serde_json::from_reader(std::io::stdin()) {
        Ok(input) => input,
        Err(e) => {
            eprintln!("lang_rust: failed to parse input: {}", e);
            std::process::exit(1);
        }
    };

    let mut combined = Output::default();
    for file in &input.files {
        let result = extract::extract(&file.content);
        combined.symbols.extend(result.symbols);
        combined.refs.extend(result.refs);
    }

    serde_json::to_writer(std::io::stdout(), &combined).unwrap_or_else(|e| {
        eprintln!("lang_rust: failed to write output: {}", e);
        std::process::exit(1);
    });
}

fn print_schema() {
    let schema = serde_json::json!({
        "name": "lang_rust",
        "description": "Extracts symbols and references from Rust source files using tree-sitter",
        "parameters": {
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" }
                        },
                        "required": ["path", "content"]
                    }
                }
            },
            "required": ["files"]
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
```

- [ ] **Step 5: Verify it compiles and schema mode works**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo build`
Run: `cargo run -- --schema`

Expected: compiles clean, schema JSON printed to stdout.

If tree-sitter version mismatch: adjust `Cargo.toml` versions so `tree-sitter` and `tree-sitter-rust` are compatible (check `cargo tree`).

- [ ] **Step 6: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/
git commit -m "feat: scaffold lang_rust plugin with types and schema mode

Standalone binary for chibi's language plugin protocol. Uses tree-sitter
for rust source parsing. Extract stub returns empty output for now.

Dependencies: tree-sitter, tree-sitter-rust, serde, serde_json."
```

---

## Chunk 3: Symbol Extraction — Core Items

### Task 4: Extract top-level symbols (fn, struct, enum, trait, mod, type, const, static, macro, union)

**Repo:** `chibi/plugins`
**Files:**
- Modify: `lang_rust/src/extract.rs`

- [ ] **Step 1: Write failing test — basic function extraction**

Add to `extract.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_function() {
        let src = "pub fn hello(name: &str) -> String {\n    format!(\"hi {}\", name)\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 1);
        let sym = &out.symbols[0];
        assert_eq!(sym.name, "hello");
        assert_eq!(sym.kind, "function");
        assert_eq!(sym.line_start, 1);
        assert_eq!(sym.line_end, 3);
        assert_eq!(sym.visibility.as_deref(), Some("public"));
        assert_eq!(
            sym.signature.as_deref(),
            Some("pub fn hello(name: &str) -> String")
        );
        assert_eq!(sym.parent, None);
    }

    #[test]
    fn extract_struct() {
        let src = "pub struct Config {\n    name: String,\n    value: i32,\n}";
        let out = extract(src);
        // struct + 2 fields
        assert_eq!(out.symbols.len(), 3);
        assert_eq!(out.symbols[0].name, "Config");
        assert_eq!(out.symbols[0].kind, "struct");
        assert_eq!(out.symbols[0].visibility.as_deref(), Some("public"));
        assert_eq!(out.symbols[0].signature.as_deref(), Some("pub struct Config"));
    }

    #[test]
    fn extract_enum_with_variants() {
        let src = "pub enum Color {\n    Red,\n    Green,\n    Blue,\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 4); // enum + 3 variants
        assert_eq!(out.symbols[0].name, "Color");
        assert_eq!(out.symbols[0].kind, "enum");
        assert_eq!(out.symbols[1].kind, "variant");
        assert_eq!(out.symbols[1].parent.as_deref(), Some("Color"));
    }

    #[test]
    fn extract_trait() {
        let src = "pub trait Display {\n    fn fmt(&self) -> String;\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 2); // trait + method signature
        assert_eq!(out.symbols[0].name, "Display");
        assert_eq!(out.symbols[0].kind, "trait");
        assert_eq!(out.symbols[1].name, "fmt");
        assert_eq!(out.symbols[1].kind, "function");
        assert_eq!(out.symbols[1].parent.as_deref(), Some("Display"));
    }

    #[test]
    fn extract_const_and_static() {
        let src = "pub const MAX: usize = 100;\nstatic COUNTER: i32 = 0;";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 2);
        assert_eq!(out.symbols[0].name, "MAX");
        assert_eq!(out.symbols[0].kind, "constant");
        assert_eq!(out.symbols[1].name, "COUNTER");
        assert_eq!(out.symbols[1].kind, "static");
    }

    #[test]
    fn extract_type_alias() {
        let src = "pub type Result<T> = std::result::Result<T, Error>;";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 1);
        assert_eq!(out.symbols[0].name, "Result");
        assert_eq!(out.symbols[0].kind, "type");
    }

    #[test]
    fn extract_mod() {
        let src = "pub mod parser {\n    pub fn parse() {}\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 2); // mod + fn
        assert_eq!(out.symbols[0].name, "parser");
        assert_eq!(out.symbols[0].kind, "module");
        assert_eq!(out.symbols[1].name, "parse");
        assert_eq!(out.symbols[1].parent.as_deref(), Some("parser"));
    }

    #[test]
    fn extract_macro_definition() {
        let src = "macro_rules! my_macro {\n    () => {};\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 1);
        assert_eq!(out.symbols[0].name, "my_macro");
        assert_eq!(out.symbols[0].kind, "macro");
    }

    #[test]
    fn extract_union() {
        let src = "pub union MyUnion {\n    f: f32,\n    i: i32,\n}";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 3); // union + 2 fields
        assert_eq!(out.symbols[0].name, "MyUnion");
        assert_eq!(out.symbols[0].kind, "union");
        assert_eq!(out.symbols[1].kind, "field");
        assert_eq!(out.symbols[1].parent.as_deref(), Some("MyUnion"));
    }

    #[test]
    fn extract_private_function() {
        let src = "fn helper() {}";
        let out = extract(src);
        assert_eq!(out.symbols[0].visibility.as_deref(), Some("private"));
    }

    #[test]
    fn extract_pub_crate_visibility() {
        let src = "pub(crate) fn internal() {}";
        let out = extract(src);
        assert_eq!(out.symbols[0].visibility.as_deref(), Some("pub(crate)"));
    }

    #[test]
    fn extract_pub_super_visibility() {
        let src = "pub(super) fn parent_visible() {}";
        let out = extract(src);
        assert_eq!(out.symbols[0].visibility.as_deref(), Some("pub(super)"));
    }

    #[test]
    fn extract_tuple_struct_no_fields() {
        // Tuple struct fields (ordered_field_declaration_list) are not extracted in v1.
        // Only the struct itself should appear.
        let src = "pub struct Pair(i32, i32);";
        let out = extract(src);
        assert_eq!(out.symbols.len(), 1);
        assert_eq!(out.symbols[0].name, "Pair");
        assert_eq!(out.symbols[0].kind, "struct");
    }

    #[test]
    fn extract_empty_file() {
        let out = extract("");
        assert!(out.symbols.is_empty());
        assert!(out.refs.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: FAIL — `extract` returns empty `Output`.

- [ ] **Step 3: Implement the AST walk in `extract.rs`**

Replace the `extract` function and add helper functions. The full implementation should:

1. Parse with `tree_sitter::Parser` + `tree_sitter_rust::LANGUAGE`
2. Depth-first walk using a cursor, maintaining a parent name stack
3. Match on `node.kind()` for each symbol kind in the spec's table
4. Extract name via `child_by_field_name("name")` (or appropriate child for each node kind)
5. Extract visibility via `visibility_modifier` child
6. Extract signature by taking source text from node start up to first body child (node kind ending in `_list` or equal to `block`), trimmed and collapsed to single line
7. Push/pop parent stack for parent-capable nodes

Key implementation details:
- `impl_item` naming: extract type name from `type` field; if `trait` field present, format as `"TraitName for TypeName"`. Strip generic parameters (just use the identifier, not `Foo<T>`).
- `function_signature_item`: same as `function_item` but has no `block` child (trait methods without default bodies).
- `enum_variant`: the name child is just an `identifier`.
- `field_declaration`: name is the first `field_identifier` child.
- `macro_definition`: name is the `identifier` child.
- Use `node.start_position().row + 1` for 1-indexed line numbers.

- [ ] **Step 4: Run tests**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: ALL tests pass.

- [ ] **Step 5: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/src/extract.rs
git commit -m "feat(lang_rust): symbol extraction via tree-sitter AST walk

Depth-first traversal with parent stack. Extracts: function,
function_signature, struct, enum, variant, union, trait, impl,
module, type, constant, static, macro, field. Includes visibility,
signature, and parent-child nesting."
```

---

## Chunk 4: Impl Blocks & Edge Cases

### Task 5: Impl block naming and nesting

**Repo:** `chibi/plugins`
**Files:**
- Modify: `lang_rust/src/extract.rs` (add tests, potentially adjust impl extraction)

- [ ] **Step 1: Write tests for impl block edge cases**

Add to the tests module:

```rust
#[test]
fn extract_impl_block_methods() {
    let src = "struct Foo {}\n\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n    fn helper(&self) {}\n}";
    let out = extract(src);
    // struct + impl + 2 methods
    let impl_sym = out.symbols.iter().find(|s| s.kind == "impl").unwrap();
    assert_eq!(impl_sym.name, "Foo");
    assert_eq!(impl_sym.signature.as_deref(), Some("impl Foo"));

    // Both methods have parent "Foo" (a name string). Disambiguation between
    // struct Foo and impl Foo happens core-side via line-range containment, not here.
    let methods: Vec<_> = out.symbols.iter().filter(|s| s.kind == "function" && s.parent.as_deref() == Some("Foo")).collect();
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].name, "new");
    assert_eq!(methods[0].visibility.as_deref(), Some("public"));
    assert_eq!(methods[1].name, "helper");
    assert_eq!(methods[1].visibility.as_deref(), Some("private"));
}

#[test]
fn extract_impl_trait_for_type() {
    let src = "impl Display for Foo {\n    fn fmt(&self) -> String { String::new() }\n}";
    let out = extract(src);
    let impl_sym = out.symbols.iter().find(|s| s.kind == "impl").unwrap();
    assert_eq!(impl_sym.name, "Display for Foo");

    let method = out.symbols.iter().find(|s| s.kind == "function").unwrap();
    assert_eq!(method.parent.as_deref(), Some("Display for Foo"));
}

#[test]
fn extract_generic_impl_strips_params() {
    let src = "impl<T> Foo<T> {\n    fn bar(&self) {}\n}";
    let out = extract(src);
    let impl_sym = out.symbols.iter().find(|s| s.kind == "impl").unwrap();
    assert_eq!(impl_sym.name, "Foo");
}

#[test]
fn extract_generic_trait_for_type_strips_params() {
    let src = "impl<T: Display> ToString for T {\n    fn to_string(&self) -> String { String::new() }\n}";
    let out = extract(src);
    let impl_sym = out.symbols.iter().find(|s| s.kind == "impl").unwrap();
    assert_eq!(impl_sym.name, "ToString for T");
}

#[test]
fn extract_multiple_impl_blocks_same_type() {
    let src = "struct S {}\nimpl S {\n    fn a(&self) {}\n}\nimpl S {\n    fn b(&self) {}\n}";
    let out = extract(src);
    let methods: Vec<_> = out.symbols.iter().filter(|s| s.kind == "function").collect();
    assert_eq!(methods.len(), 2);
    // Both should have parent "S" but mapped to the correct (containing) impl block.
    assert!(methods.iter().all(|m| m.parent.as_deref() == Some("S")));
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: All pass (if impl extraction was implemented correctly in Task 4). If any fail, fix the extraction logic.

- [ ] **Step 3: Commit (if changes were needed)**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/src/extract.rs
git commit -m "test(lang_rust): impl block edge cases — trait impls, generics, multiple impls"
```

### Task 6: Syntax error resilience and edge cases

**Repo:** `chibi/plugins`
**Files:**
- Modify: `lang_rust/src/extract.rs` (add tests)

- [ ] **Step 1: Write edge case tests**

```rust
#[test]
fn extract_syntax_error_partial_tree() {
    // tree-sitter produces a partial tree for incomplete code.
    let src = "pub fn valid() {}\n\npub fn broken( {}\n\npub struct Good {}";
    let out = extract(src);
    // Should extract valid and Good; broken may or may not appear depending on tree-sitter error recovery.
    let names: Vec<&str> = out.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"valid"));
    assert!(names.contains(&"Good"));
}

#[test]
fn extract_deeply_nested_modules() {
    let src = "mod a {\n    mod b {\n        pub fn deep() {}\n    }\n}";
    let out = extract(src);
    let deep = out.symbols.iter().find(|s| s.name == "deep").unwrap();
    assert_eq!(deep.parent.as_deref(), Some("b"));
    let b = out.symbols.iter().find(|s| s.name == "b").unwrap();
    assert_eq!(b.parent.as_deref(), Some("a"));
}

#[test]
fn extract_struct_fields_as_children() {
    let src = "pub struct Point {\n    pub x: f64,\n    pub y: f64,\n}";
    let out = extract(src);
    let fields: Vec<_> = out.symbols.iter().filter(|s| s.kind == "field").collect();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "x");
    assert_eq!(fields[0].parent.as_deref(), Some("Point"));
    assert_eq!(fields[0].visibility.as_deref(), Some("public"));
    assert_eq!(fields[1].name, "y");
}

#[test]
fn extract_trait_with_default_and_signature_methods() {
    let src = "trait MyTrait {\n    fn required(&self);\n    fn optional(&self) { }\n}";
    let out = extract(src);
    let methods: Vec<_> = out.symbols.iter().filter(|s| s.kind == "function").collect();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|m| m.parent.as_deref() == Some("MyTrait")));
}

#[test]
fn extract_multiline_signature() {
    let src = "pub fn complex(\n    a: i32,\n    b: String,\n) -> Result<(), Error> {\n}";
    let out = extract(src);
    let sig = out.symbols[0].signature.as_deref().unwrap();
    // Signature should be collapsed to single line, trimmed.
    assert!(sig.contains("pub fn complex("));
    assert!(sig.contains("-> Result<(), Error>"));
    assert!(!sig.contains('{'));
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: All pass. Fix any that don't — these are the edge cases that validate robustness.

- [ ] **Step 3: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/src/extract.rs
git commit -m "test(lang_rust): edge cases — syntax errors, nesting, multiline signatures"
```

---

## Chunk 5: Reference Extraction

### Task 7: Extract use statement references

**Repo:** `chibi/plugins`
**Files:**
- Modify: `lang_rust/src/extract.rs`

- [ ] **Step 1: Write failing tests for use statements**

```rust
#[test]
fn extract_simple_use() {
    let src = "use std::collections::HashMap;";
    let out = extract(src);
    assert_eq!(out.refs.len(), 1);
    assert_eq!(out.refs[0].to_name, "std::collections::HashMap");
    assert_eq!(out.refs[0].kind, "import");
    assert_eq!(out.refs[0].from_line, 1);
}

#[test]
fn extract_grouped_use() {
    let src = "use crate::parser::{Parser, Token};";
    let out = extract(src);
    assert_eq!(out.refs.len(), 2);
    let names: Vec<&str> = out.refs.iter().map(|r| r.to_name.as_str()).collect();
    assert!(names.contains(&"crate::parser::Parser"));
    assert!(names.contains(&"crate::parser::Token"));
}

#[test]
fn extract_nested_grouped_use() {
    let src = "use std::{collections::{HashMap, BTreeMap}, io::Read};";
    let out = extract(src);
    assert_eq!(out.refs.len(), 3);
    let names: Vec<&str> = out.refs.iter().map(|r| r.to_name.as_str()).collect();
    assert!(names.contains(&"std::collections::HashMap"));
    assert!(names.contains(&"std::collections::BTreeMap"));
    assert!(names.contains(&"std::io::Read"));
}

#[test]
fn extract_glob_use() {
    let src = "use std::collections::*;";
    let out = extract(src);
    assert_eq!(out.refs.len(), 1);
    assert_eq!(out.refs[0].to_name, "std::collections::*");
}

#[test]
fn extract_aliased_use() {
    let src = "use std::io::Result as IoResult;";
    let out = extract(src);
    assert_eq!(out.refs.len(), 1);
    assert_eq!(out.refs[0].to_name, "std::io::Result");
}

#[test]
fn extract_bare_use_list() {
    // Rare but legal: `use {std, core};` with no path prefix.
    let src = "use {std, core};";
    let out = extract(src);
    assert_eq!(out.refs.len(), 2);
    let names: Vec<&str> = out.refs.iter().map(|r| r.to_name.as_str()).collect();
    assert!(names.contains(&"std"));
    assert!(names.contains(&"core"));
}

#[test]
fn extract_multiple_use_statements() {
    let src = "use std::io;\nuse std::fmt;\n\npub fn f() {}";
    let out = extract(src);
    assert_eq!(out.refs.len(), 2);
    assert_eq!(out.refs[0].from_line, 1);
    assert_eq!(out.refs[1].from_line, 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: ref tests FAIL (extract doesn't produce refs yet).

- [ ] **Step 3: Implement use-statement reference extraction**

Add to `extract.rs` a function that walks `use_declaration` nodes recursively:

1. When the walker encounters a `use_declaration` node, call a helper `extract_use_refs(node, source, line, &mut refs)`
2. The helper recursively walks `use_list` / `scoped_use_list` children, building up a path prefix
3. At leaf nodes (`scoped_identifier`, `identifier`, `use_wildcard`), emit a `Ref` with the full path
4. For `use_as_clause`, extract the original path (not the alias)

Key tree-sitter node kinds inside use trees:
- `scoped_identifier` → path like `std::collections::HashMap`
- `scoped_use_list` → `path::{items}`
- `use_list` → `{items}` (the braced list)
- `use_wildcard` → `*`
- `use_as_clause` → `path as alias`
- `identifier` → bare name

- [ ] **Step 4: Run tests**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: ALL tests pass.

- [ ] **Step 5: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/src/extract.rs
git commit -m "feat(lang_rust): extract use-statement references

Recursively walks use_declaration nodes to emit import refs.
Handles: simple paths, grouped imports (including nested), glob
imports, aliased imports. Each leaf becomes a Ref with full path."
```

---

## Chunk 6: Integration Tests & Final Polish

### Task 8: Fixture-based integration tests

**Repo:** `chibi/plugins`
**Files:**
- Create: `lang_rust/tests/fixtures/basic.rs`
- Create: `lang_rust/tests/integration.rs`

- [ ] **Step 1: Create test fixture `tests/fixtures/basic.rs`**

```rust
use std::collections::HashMap;
use crate::utils::{Helper, Config};

pub struct Parser {
    input: String,
    tokens: Vec<Token>,
}

pub enum Token {
    Word(String),
    Number(i64),
    Eof,
}

pub trait Parseable {
    fn parse(&self) -> Result<(), Error>;
    fn validate(&self) -> bool { true }
}

impl Parseable for Parser {
    fn parse(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl Parser {
    pub fn new(input: String) -> Self {
        Self { input, tokens: Vec::new() }
    }

    fn tokenize(&mut self) {}
}

pub const MAX_DEPTH: usize = 100;
static INSTANCE_COUNT: i32 = 0;
pub type ParseResult<T> = Result<T, Error>;

macro_rules! parse_assert {
    ($e:expr) => {};
}

mod internal {
    pub fn helper() {}
}
```

- [ ] **Step 2: Create `tests/integration.rs`**

Validates key invariants structurally (not exact JSON comparison): correct symbol names/kinds present, parent relationships correct, ref paths correct.

```rust
use std::process::Command;

/// Run lang_rust as a subprocess (the way chibi's indexer does) and verify the output.
#[test]
fn integration_basic_fixture() {
    let fixture = std::fs::read_to_string("tests/fixtures/basic.rs")
        .expect("fixture file missing");

    let input = serde_json::json!({
        "files": [{"path": "tests/fixtures/basic.rs", "content": fixture}]
    });

    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .expect("failed to run lang_rust");

    assert!(output.status.success(), "lang_rust exited with error: {}", String::from_utf8_lossy(&output.stderr));

    let result: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("invalid JSON output");

    let symbols = result["symbols"].as_array().unwrap();
    let refs = result["refs"].as_array().unwrap();

    // Verify key symbols exist.
    let sym_names: Vec<&str> = symbols.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(sym_names.contains(&"Parser"));
    assert!(sym_names.contains(&"Token"));
    assert!(sym_names.contains(&"Parseable"));
    assert!(sym_names.contains(&"new"));
    assert!(sym_names.contains(&"tokenize"));
    assert!(sym_names.contains(&"MAX_DEPTH"));
    assert!(sym_names.contains(&"INSTANCE_COUNT"));
    assert!(sym_names.contains(&"ParseResult"));
    assert!(sym_names.contains(&"parse_assert"));
    assert!(sym_names.contains(&"internal"));
    assert!(sym_names.contains(&"helper"));

    // Verify refs from use statements.
    let ref_names: Vec<&str> = refs.iter().map(|r| r["to_name"].as_str().unwrap()).collect();
    assert!(ref_names.contains(&"std::collections::HashMap"));
    assert!(ref_names.contains(&"crate::utils::Helper"));
    assert!(ref_names.contains(&"crate::utils::Config"));
    assert_eq!(refs.len(), 3);

    // Verify parent relationships.
    let find_sym = |name: &str, kind: &str| {
        symbols.iter().find(|s| s["name"].as_str().unwrap() == name && s["kind"].as_str().unwrap() == kind)
    };
    assert_eq!(find_sym("input", "field").unwrap()["parent"].as_str(), Some("Parser"));
    assert_eq!(find_sym("Word", "variant").unwrap()["parent"].as_str(), Some("Token"));
    assert_eq!(find_sym("helper", "function").unwrap()["parent"].as_str(), Some("internal"));
}

#[test]
fn integration_schema_mode() {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--", "--schema"])
        .output()
        .expect("failed to run lang_rust --schema");

    assert!(output.status.success());

    let schema: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("invalid schema JSON");

    assert_eq!(schema["name"].as_str(), Some("lang_rust"));
    assert!(schema["parameters"]["properties"]["files"].is_object());
}

#[test]
fn integration_empty_input() {
    let input = serde_json::json!({"files": [{"path": "empty.rs", "content": ""}]});

    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .expect("failed to run lang_rust");

    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["symbols"].as_array().unwrap().len(), 0);
    assert_eq!(result["refs"].as_array().unwrap().len(), 0);
}
```

- [ ] **Step 4: Run integration tests**

Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test -- --include-ignored`

Expected: ALL pass.

- [ ] **Step 5: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add lang_rust/tests/
git commit -m "test(lang_rust): integration tests — subprocess invocation, fixture validation

Tests lang_rust as a real subprocess (matching chibi's indexer dispatch).
Validates: schema mode, basic fixture with all symbol kinds + refs +
parent nesting, empty input handling."
```

### Task 9: Update plugins repo README

**Repo:** `chibi/plugins`
**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add lang_rust to the plugins table**

Add to the table in README.md:

```
| `lang_rust` | Rust | Language plugin: extracts symbols/refs from Rust source files (tree-sitter) |
```

- [ ] **Step 2: Commit**

```bash
cd /home/fey/projects/chibi/plugins
git add README.md
git commit -m "docs: add lang_rust to plugins table"
```

### Task 10: Final verification

- [ ] **Step 1: Run full test suite in both repos**

Run: `cd /home/fey/projects/chibi/chibi && cargo test -p chibi-core -- index`
Run: `cd /home/fey/projects/chibi/plugins/lang_rust && cargo test`

Expected: ALL pass in both repos.

- [ ] **Step 2: Test end-to-end with chibi (manual smoke test)**

Install the plugin and verify chibi picks it up:
```bash
cd /home/fey/projects/chibi/plugins/lang_rust && cargo build --release
cp target/release/lang_rust ~/.chibi/plugins/
```

Then in a chibi context with a rust project, run index_update and verify symbols appear in index_query results. (This is a manual verification step — document results but don't block on it.)

- [ ] **Step 3: Collect AGENTS.md notes**

Review the implementation for any quirks or gotchas that should be added to `chibi/chibi/AGENTS.md` or `chibi/plugins/README.md`. Examples might include:
- tree-sitter version pairing gotcha
- Any node kind surprises discovered during implementation
- Plugin installation path requirements
