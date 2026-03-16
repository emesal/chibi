# lang_rust — Tree-sitter Language Plugin for Chibi

**Date:** 2026-03-16
**Status:** Draft
**Repo:** chibi/plugins (standalone binary)

## Overview

`lang_rust` is chibi's first language plugin — a standalone rust binary that uses tree-sitter to extract symbols and references from rust source files. It conforms to chibi's existing language plugin protocol (JSON stdin → JSON stdout) and sets the pattern for all future `lang_*` plugins.

## Motivation

Chibi's indexer can walk source trees and store file metadata, but without language plugins it has zero symbol awareness. An LLM working with an indexed codebase can find files but not navigate to structs, functions, or traits. `lang_rust` fills that gap for rust codebases and exercises the full plugin pipeline (discovery → dispatch → symbol storage → query) for the first time.

## Plugin Contract

### Schema Mode

`lang_rust --schema` outputs:

```json
{
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
}
```

### Execution Mode

Input (JSON on stdin):

```json
{"files": [{"path": "src/parser.rs", "content": "pub struct Parser {\n    input: String,\n}\n\nimpl Parser {\n    pub fn new(input: String) -> Self {\n        Self { input }\n    }\n}"}]}
```

Output (JSON on stdout):

```json
{
  "symbols": [
    {
      "name": "Parser",
      "kind": "struct",
      "line_start": 1,
      "line_end": 3,
      "signature": "pub struct Parser",
      "visibility": "public",
      "parent": null
    },
    {
      "name": "input",
      "kind": "field",
      "line_start": 2,
      "line_end": 2,
      "signature": "input: String",
      "visibility": "private",
      "parent": "Parser"
    },
    {
      "name": "Parser",
      "kind": "impl",
      "line_start": 5,
      "line_end": 9,
      "signature": "impl Parser",
      "visibility": null,
      "parent": null
    },
    {
      "name": "new",
      "kind": "function",
      "line_start": 6,
      "line_end": 8,
      "signature": "pub fn new(input: String) -> Self",
      "visibility": "public",
      "parent": "Parser"
    }
  ],
  "refs": []
}
```

## Parsing Strategy

**Approach:** AST walk — tree-sitter parse followed by depth-first traversal with rust match on `node.kind()`.

**Rationale:** Full control over signature extraction, parent tracking, and visibility mapping. More testable and debuggable than query-based approaches. The hybrid (queries for discovery + walk for extraction) adds indirection without meaningful savings for a single-language plugin.

## Symbol Extraction

### Symbol Kinds

| tree-sitter node kind | emitted symbol kind | can be parent? |
|---|---|---|
| `function_item` | `function` | no |
| `struct_item` | `struct` | yes (fields) |
| `enum_item` | `enum` | yes (variants) |
| `enum_variant` | `variant` | no |
| `union_item` | `union` | yes (fields) |
| `trait_item` | `trait` | yes (methods, types, consts) |
| `impl_item` | `impl` | yes (methods, types, consts) |
| `mod_item` | `module` | yes (nested items) |
| `type_item` | `type` | no |
| `const_item` | `constant` | no |
| `static_item` | `static` | no |
| `macro_definition` | `macro` | no |
| `field_declaration` | `field` | no |

### Parent Stack

When we enter a parent-capable node, push its name onto a stack. Children emitted while inside that node set `parent` to the stack top. Pop on exit. Handles arbitrary nesting:

```rust
mod outer {
    struct Foo {
        x: i32,  // parent: "Foo"
    }           // parent: "outer"
}
```

### Impl Block Naming

`impl` blocks don't have a single name — they're `impl Type` or `impl Trait for Type`. The symbol name is the type name (or `Trait for Type`). Methods inside set their `parent` to this name.

### Visibility Extraction

tree-sitter-rust exposes a `visibility_modifier` child node:

| modifier | emitted value |
|---|---|
| absent | `"private"` |
| `pub` | `"public"` |
| `pub(crate)` | `"pub(crate)"` |
| `pub(super)` | `"pub(super)"` |
| `pub(in path)` | literal text |

### Signature Extraction

The signature is the declaration line without the body. For a function: everything from visibility through the return type, excluding the `{ ... }` block. Extracted by taking the source text from node start up to (but not including) the `block` / `declaration_list` / `field_declaration_list` child, trimmed.

## Reference Extraction

v1 extracts references from `use` statements only.

```rust
use std::collections::HashMap;
//  → { from_line: 1, to_name: "std::collections::HashMap", kind: "import" }

use crate::parser::{Parser, Token};
//  → { from_line: 1, to_name: "crate::parser::Parser", kind: "import" }
//  → { from_line: 1, to_name: "crate::parser::Token", kind: "import" }

use std::io::Result as IoResult;
//  → { from_line: 1, to_name: "std::io::Result", kind: "import" }
```

For grouped imports (`{A, B}`), one ref per leaf. For glob imports (`use foo::*`), a single ref to `foo::*`.

### Not in v1

- References inside function bodies (calls, type mentions, field access)
- References in type signatures (parameter types, return types, where clauses)
- Macro invocations

## Project Structure

```
plugins/
  lang_rust/
    Cargo.toml
    src/
      main.rs       — CLI entry point (--schema, stdin/stdout dispatch)
      extract.rs    — AST walk, symbol + ref extraction
      types.rs      — serde structs (Input, Output, Symbol, Ref)
    tests/
      fixtures/     — .rs files for testing
      integration.rs
```

### Dependencies

```toml
[dependencies]
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Minimal — no async, no tokio, no chibi-core dependency. Communication is purely through the JSON contract.

### Installation

`cargo install --path plugins/lang_rust` places the binary on `$PATH`. User symlinks or copies it into `~/.chibi/plugins/`. A `just install-plugins` recipe is a documentation/convenience item, not a v1 requirement.

## Core-Side Patch: Parent Resolution

Currently `insert_symbols` in `chibi-core/src/index/indexer.rs` writes `parent_id: NULL` regardless of the `parent` field in plugin output.

### Two-Pass Insert

1. **First pass:** Insert all symbols for the file, collecting a `name → id` map.
2. **Second pass:** For any symbol with a non-null `parent` field, `UPDATE symbols SET parent_id = ? WHERE id = ?` using the map lookup.

Scoped to a single file's symbols within the same transaction. If a `parent` name doesn't resolve, log a warning and leave `parent_id` NULL — same graceful degradation pattern as the rest of the indexer.

**Why two-pass:** Symbols in plugin output aren't guaranteed parent-before-child order. Two-pass handles any ordering without needing a topological sort.

**Size:** ~15-20 lines in `insert_symbols`. Backwards-compatible — plugins that don't emit `parent` work exactly as before.

## Testing Strategy

- **Unit tests** in `extract.rs`: parse known rust snippets, assert on emitted symbols/refs.
- **Fixture-based integration tests**: `.rs` files in `tests/fixtures/` with expected `.json` sidecars. Test reads the fixture, runs extraction, compares output.
- **Edge cases:**
  - Empty files
  - Syntax errors (tree-sitter produces partial trees — extract what we can)
  - Deeply nested modules
  - Multiple impl blocks for the same type
  - `macro_rules!` with unusual syntax
  - Grouped and glob use statements
  - All visibility modifiers
  - `impl Trait for Type` naming

## Explicit Non-Goals

- Cross-file reference resolution
- Doc comment extraction
- Attribute/derive parsing
- Procedural macro expansion
- LSP/rust-analyzer integration
- Auto-installation mechanism

## What This Enables

- An LLM can ask "what symbols are in this file", "show me the methods on `Parser`", "what does this struct look like"
- The index becomes meaningfully navigable for rust codebases
- The pattern is set for `lang_python`, `lang_typescript`, and other future language plugins
- The `parent_id` column in the index schema is exercised for the first time
