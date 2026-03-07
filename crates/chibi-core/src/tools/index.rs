//!
//! index tools: codebase index management.
//! index_update, index_query, index_status.

use std::io::{self, ErrorKind};
use std::path::Path;

use super::{BuiltinToolDef, ToolPropertyDef};
use crate::config::ResolvedConfig;
use crate::json_ext::JsonExt;

// === Tool Name Constants ===

pub const INDEX_UPDATE_TOOL_NAME: &str = "index_update";
pub const INDEX_QUERY_TOOL_NAME: &str = "index_query";
pub const INDEX_STATUS_TOOL_NAME: &str = "index_status";

// === Tool Definition Registry ===

/// All index tool definitions
pub static INDEX_TOOL_DEFS: &[BuiltinToolDef] = &[
    BuiltinToolDef {
        name: INDEX_UPDATE_TOOL_NAME,
        description: "Trigger a codebase index update. Walks the project tree, detects changed files, and dispatches to language plugins for symbol extraction. Returns a summary of what was indexed.",
        properties: &[ToolPropertyDef {
            name: "force",
            prop_type: "string",
            description: "Force re-index of all files, ignoring change detection (\"true\"/\"false\", default: \"false\")",
            default: None,
        }],
        required: &[],
        summary_params: &[],
    },
    BuiltinToolDef {
        name: INDEX_QUERY_TOOL_NAME,
        description: "Search the codebase index for symbols or references. Use `name`/`kind`/`file` to query symbols, or `refs_to` to find references to a name. Returns formatted results.",
        properties: &[
            ToolPropertyDef {
                name: "name",
                prop_type: "string",
                description: "Filter symbols by name (substring match, case-insensitive)",
                default: None,
            },
            ToolPropertyDef {
                name: "kind",
                prop_type: "string",
                description: "Filter symbols by kind (exact match, e.g. \"function\", \"struct\")",
                default: None,
            },
            ToolPropertyDef {
                name: "file",
                prop_type: "string",
                description: "Filter symbols by file path (substring match)",
                default: None,
            },
            ToolPropertyDef {
                name: "refs_to",
                prop_type: "string",
                description: "Find references to this name (substring match). When set, name/kind/file are ignored.",
                default: None,
            },
            ToolPropertyDef {
                name: "limit",
                prop_type: "integer",
                description: "Maximum number of results to return (default: 50)",
                default: Some(50),
            },
        ],
        required: &[],
        summary_params: &["name", "kind"],
    },
    BuiltinToolDef {
        name: INDEX_STATUS_TOOL_NAME,
        description: "Show a summary of the codebase index: file counts, language breakdown, symbol and reference totals.",
        properties: &[],
        required: &[],
        summary_params: &[],
    },
];

// === Registry Helpers ===

/// Register all index tools into the registry.
///
/// Note: the `tools` slice passed to `execute_index_tool` is `&[]` here.
/// Full registry wiring (for language plugin dispatch) happens in Task 7+.
pub fn register_index_tools(registry: &mut super::registry::ToolRegistry) {
    use std::sync::Arc;
    use super::registry::{ToolCategory, ToolHandler};
    use super::Tool;

    let handler: ToolHandler = Arc::new(|call| {
        // execute_index_tool is sync — extract result before the async block so
        // no !Sync references cross an .await point.
        let ctx = call.context;
        let result = execute_index_tool(call.name, call.args, ctx.project_root, ctx.config, &[])
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("unknown index tool: {}", call.name),
                ))
            });
        Box::pin(async move { result })
    });

    for def in INDEX_TOOL_DEFS {
        registry.register(Tool::from_builtin_def(def, handler.clone(), ToolCategory::Index));
    }
}

/// Convert all index tools to API format
pub fn all_index_tools_to_api_format() -> Vec<serde_json::Value> {
    INDEX_TOOL_DEFS
        .iter()
        .map(|def| def.to_api_format())
        .collect()
}

/// Check if a tool name belongs to the index group
pub fn is_index_tool(name: &str) -> bool {
    matches!(
        name,
        INDEX_UPDATE_TOOL_NAME | INDEX_QUERY_TOOL_NAME | INDEX_STATUS_TOOL_NAME
    )
}

// === Tool Execution ===

/// Execute an index tool by name.
///
/// Returns `Some(result)` when the tool name is recognised, `None` otherwise.
/// The `tools` slice is forwarded to `index_update` for language plugin dispatch;
/// pass `&[]` when the plugin list is unavailable.
pub fn execute_index_tool(
    tool_name: &str,
    args: &serde_json::Value,
    project_root: &Path,
    _config: &ResolvedConfig,
    tools: &[super::Tool],
) -> Option<io::Result<String>> {
    match tool_name {
        INDEX_UPDATE_TOOL_NAME => Some(execute_index_update(args, project_root, tools)),
        INDEX_QUERY_TOOL_NAME => Some(execute_index_query(args, project_root)),
        INDEX_STATUS_TOOL_NAME => Some(execute_index_status(project_root)),
        _ => None,
    }
}

// === index_update ===

/// Execute index_update: walk the project, detect changes, dispatch to language plugins.
fn execute_index_update(
    args: &serde_json::Value,
    project_root: &Path,
    tools: &[super::Tool],
) -> io::Result<String> {
    use crate::index::{self, IndexOptions, open_db};

    let force = args.get_str_or("force", "false") == "true";
    let db_path = crate::project_index_db_path(project_root);

    // Ensure .chibi directory exists
    crate::project_chibi_dir(project_root)?;

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    let options = IndexOptions {
        force,
        verbose: false,
    };

    let stats = index::update_index(&conn, project_root, &options, tools)?;
    Ok(stats.to_string())
}

// === index_query ===

/// Execute index_query: search for symbols or references in the codebase index.
fn execute_index_query(args: &serde_json::Value, project_root: &Path) -> io::Result<String> {
    use crate::index::{SymbolQuery, open_db, query_refs, query_symbols};

    let db_path = crate::project_index_db_path(project_root);
    if !db_path.exists() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            "No codebase index found. Run index_update first.",
        ));
    }

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    let limit = args.get_u64_or("limit", 50) as u32;

    // If refs_to is set, query references instead of symbols
    if let Some(refs_to) = args.get_str("refs_to") {
        let rows = query_refs(&conn, refs_to, limit);
        if rows.is_empty() {
            return Ok(format!("No references found matching: {}", refs_to));
        }
        let formatted: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
        return Ok(formatted.join("\n"));
    }

    // Otherwise, query symbols
    let opts = SymbolQuery {
        name: args.get_str("name").map(String::from),
        kind: args.get_str("kind").map(String::from),
        file: args.get_str("file").map(String::from),
        limit,
    };

    let rows = query_symbols(&conn, &opts);
    if rows.is_empty() {
        return Ok("No symbols found matching query.".to_string());
    }
    let formatted: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    Ok(formatted.join("\n"))
}

// === index_status ===

/// Execute index_status: return a human-readable summary of the codebase index.
fn execute_index_status(project_root: &Path) -> io::Result<String> {
    use crate::index::{index_status, open_db};

    let db_path = crate::project_index_db_path(project_root);
    if !db_path.exists() {
        return Ok("No codebase index found. Run index_update to create one.".to_string());
    }

    let conn = open_db(&db_path)
        .map_err(|e| io::Error::other(format!("Failed to open index database: {}", e)))?;

    Ok(index_status(&conn, project_root))
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn args(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    #[test]
    fn test_index_tool_defs_api_format() {
        for def in INDEX_TOOL_DEFS {
            let api = def.to_api_format();
            assert_eq!(api["type"], "function");
            assert!(api["function"]["name"].is_string());
        }
    }

    #[test]
    fn test_is_index_tool() {
        assert!(is_index_tool(INDEX_UPDATE_TOOL_NAME));
        assert!(is_index_tool(INDEX_QUERY_TOOL_NAME));
        assert!(is_index_tool(INDEX_STATUS_TOOL_NAME));
        assert!(!is_index_tool("shell_exec"));
        assert!(!is_index_tool("file_head"));
    }

    #[test]
    fn test_tool_constants() {
        assert_eq!(INDEX_UPDATE_TOOL_NAME, "index_update");
        assert_eq!(INDEX_QUERY_TOOL_NAME, "index_query");
        assert_eq!(INDEX_STATUS_TOOL_NAME, "index_status");
    }

    #[test]
    fn test_index_status_no_db() {
        let dir = tempfile::tempdir().unwrap();
        let result = execute_index_status(dir.path()).unwrap();
        assert!(result.contains("No codebase index found"));
    }

    #[test]
    fn test_index_query_no_db() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&[("name", serde_json::json!("foo"))]);
        let result = execute_index_query(&a, dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No codebase index")
        );
    }

    #[test]
    fn test_index_update_creates_db_and_indexes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("lib.py"), "def hello(): pass\n").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("util.rs"), "pub fn util() {}\n").unwrap();

        let a = args(&[]);
        let result = execute_index_update(&a, dir.path(), &[]).unwrap();

        assert!(result.contains("scanned:"));

        let db_path = crate::project_index_db_path(dir.path());
        assert!(db_path.exists());
    }

    #[test]
    fn test_index_update_then_status() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        let result = execute_index_status(dir.path()).unwrap();
        assert!(result.contains("file"));
    }

    #[test]
    fn test_index_update_then_query_no_symbols() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        let q = args(&[("name", serde_json::json!("main"))]);
        let result = execute_index_query(&q, dir.path()).unwrap();
        assert!(result.contains("No symbols found"));
    }

    #[test]
    fn test_index_query_refs_no_refs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        let q = args(&[("refs_to", serde_json::json!("nonexistent"))]);
        let result = execute_index_query(&q, dir.path()).unwrap();
        assert!(result.contains("No references found"));
    }

    #[test]
    fn test_index_update_force_reindex() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

        // First index
        let a = args(&[]);
        execute_index_update(&a, dir.path(), &[]).unwrap();

        // Second index without force should skip unchanged files
        let result = execute_index_update(&a, dir.path(), &[]).unwrap();
        assert!(result.contains("skipped (unchanged): 1"));

        // Force re-index should re-process all files
        let a_force = args(&[("force", serde_json::json!("true"))]);
        let result = execute_index_update(&a_force, dir.path(), &[]).unwrap();
        assert!(result.contains("indexed: 1"));
    }
}
