//! File walker and index updater for the codebase index.
//!
//! Walks the project tree (respecting `.gitignore`), detects changes via mtime + size,
//! dispatches to language plugins for symbol extraction, and writes everything to the database.
//! Core handles all DB writes — plugins never touch sqlite directly.

use crate::tools::{HookPoint, Tool, execute_hook, execute_tool};
use ignore::WalkBuilder;
use rusqlite::Connection;
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Statistics returned after an index update.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub files_scanned: u32,
    pub files_indexed: u32,
    pub files_skipped: u32,
    pub files_removed: u32,
    pub symbols_added: u32,
    pub refs_added: u32,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scanned: {}, indexed: {}, skipped (unchanged): {}, removed: {}, symbols: {}, refs: {}",
            self.files_scanned,
            self.files_indexed,
            self.files_skipped,
            self.files_removed,
            self.symbols_added,
            self.refs_added,
        )
    }
}

/// Options controlling index behaviour.
#[derive(Default)]
pub struct IndexOptions {
    /// Re-index all files regardless of mtime/size.
    pub force: bool,
    /// Print progress to stderr.
    pub verbose: bool,
}

/// Extension-to-language mapping. Hardcoded for now; configurable later.
const LANG_MAP: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("js", "javascript"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("jsx", "javascript"),
    ("rb", "ruby"),
    ("go", "go"),
    ("java", "java"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
    ("cc", "cpp"),
    ("zig", "zig"),
    ("lua", "lua"),
    ("sh", "shell"),
    ("bash", "shell"),
    ("toml", "toml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("json", "json"),
    ("md", "markdown"),
];

/// Detect language from file extension.
fn detect_language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    LANG_MAP
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, lang)| *lang)
}

/// Walk the project, detect changes, dispatch to plugins, update the database.
///
/// Language plugins follow the convention: tool named `lang_<language>` (e.g. `lang_rust`).
/// They receive `{"files": [{"path": "...", "content": "..."}]}` on stdin and return
/// `{"symbols": [...], "refs": [...]}` on stdout. Core handles all DB writes.
pub fn update_index(
    conn: &Connection,
    project_root: &Path,
    options: &IndexOptions,
    tools: &[Tool],
) -> io::Result<IndexStats> {
    let mut stats = IndexStats::default();

    // Collect all indexed paths so we can detect removals.
    let mut existing_paths: HashMap<String, (i64, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT path, mtime, size FROM files")
            .map_err(|e| io::Error::other(format!("failed to query files: {}", e)))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| io::Error::other(format!("failed to iterate files: {}", e)))?;
        for row in rows {
            let (path, mtime, size) =
                row.map_err(|e| io::Error::other(format!("row error: {}", e)))?;
            existing_paths.insert(path, (mtime, size));
        }
    }

    // Walk the project tree, respecting .gitignore.
    let walker = WalkBuilder::new(project_root)
        .hidden(true) // skip hidden files by default
        .git_ignore(true)
        .build();

    // Group files by language for batched plugin dispatch.
    let mut files_by_lang: HashMap<String, Vec<(String, std::fs::Metadata)>> = HashMap::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        // Skip directories.
        if entry.file_type().map(|t| !t.is_file()).unwrap_or(true) {
            continue;
        }

        let abs_path = entry.path();
        let rel_path = abs_path
            .strip_prefix(project_root)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .into_owned();

        // Skip the .chibi directory itself.
        if rel_path.starts_with(".chibi") {
            continue;
        }

        stats.files_scanned += 1;
        seen_paths.insert(rel_path.clone());

        let meta = match std::fs::metadata(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let size = meta.len() as i64;

        // Change detection: skip if mtime and size unchanged (unless force).
        if !options.force
            && let Some(&(old_mtime, old_size)) = existing_paths.get(&rel_path)
            && old_mtime == mtime
            && old_size == size
        {
            stats.files_skipped += 1;
            continue;
        }

        let lang = detect_language(abs_path).unwrap_or("unknown").to_string();

        files_by_lang
            .entry(lang)
            .or_default()
            .push((rel_path, meta));
    }

    // Process each language batch.
    for (lang, files) in &files_by_lang {
        // Find language plugin (tool named `lang_<language>`).
        let plugin_name = format!("lang_{}", lang);
        let plugin = tools.iter().find(|t| t.name == plugin_name);

        for (rel_path, meta) in files {
            let abs_path = project_root.join(rel_path);
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size = meta.len() as i64;

            // Upsert the file record.
            conn.execute(
                "INSERT INTO files (path, lang, mtime, size)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(path) DO UPDATE SET
                     lang = excluded.lang,
                     mtime = excluded.mtime,
                     size = excluded.size,
                     indexed_at = datetime('now')",
                rusqlite::params![rel_path, lang, mtime, size],
            )
            .map_err(|e| io::Error::other(format!("failed to upsert file: {}", e)))?;

            let file_id: i64 = conn
                .query_row("SELECT id FROM files WHERE path = ?1", [rel_path], |row| {
                    row.get(0)
                })
                .map_err(|e| io::Error::other(format!("failed to get file id: {}", e)))?;

            // Clear old symbols and refs for this file (cascade would handle it,
            // but explicit delete is clearer for partial re-index).
            conn.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])
                .map_err(|e| io::Error::other(format!("failed to delete symbols: {}", e)))?;
            conn.execute("DELETE FROM refs WHERE from_file_id = ?1", [file_id])
                .map_err(|e| io::Error::other(format!("failed to delete refs: {}", e)))?;

            // If a language plugin exists, dispatch to it for symbol extraction.
            if let Some(plugin) = plugin {
                let content = std::fs::read_to_string(&abs_path).unwrap_or_default();
                let input = serde_json::json!({
                    "files": [{"path": rel_path, "content": content}]
                });

                match execute_tool(plugin, &input) {
                    Ok(output) => {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output) {
                            stats.symbols_added += insert_symbols(conn, file_id, &parsed);
                            stats.refs_added += insert_refs(conn, file_id, &parsed);
                        }
                        // Malformed output → graceful fallback (file indexed without symbols).
                    }
                    Err(_) => {
                        // Plugin failure → file still indexed, just without symbols.
                    }
                }
            }

            stats.files_indexed += 1;

            // Fire PostIndexFile hook (observe only — errors are non-fatal).
            let hook_data = serde_json::json!({
                "path": rel_path,
                "lang": lang,
                "symbol_count": stats.symbols_added,
                "ref_count": stats.refs_added,
            });
            let _ = execute_hook(tools, HookPoint::PostIndexFile, &hook_data);
        }
    }

    // Remove files that no longer exist on disk.
    for path in existing_paths.keys() {
        if !seen_paths.contains(path) {
            conn.execute("DELETE FROM files WHERE path = ?1", [path])
                .map_err(|e| io::Error::other(format!("failed to remove stale file: {}", e)))?;
            stats.files_removed += 1;
        }
    }

    Ok(stats)
}

/// Insert symbols from plugin output into the database. Returns count of symbols added.
fn insert_symbols(conn: &Connection, file_id: i64, output: &serde_json::Value) -> u32 {
    let symbols = match output.get("symbols").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return 0,
    };

    let mut count = 0u32;
    for sym in symbols {
        let name = sym.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = sym.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let line_start = sym.get("line_start").and_then(|v| v.as_i64()).unwrap_or(0);
        let line_end = sym.get("line_end").and_then(|v| v.as_i64()).unwrap_or(0);
        let signature = sym.get("signature").and_then(|v| v.as_str());
        let visibility = sym.get("visibility").and_then(|v| v.as_str());

        // Note: parent_id resolution (matching parent name → id) deferred to phase 6
        // when we have a proper protocol. For now, parent_id is NULL.
        let result = conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                file_id, name, kind, line_start, line_end, signature, visibility
            ],
        );

        if result.is_ok() {
            count += 1;
        }
    }
    count
}

/// Insert refs from plugin output into the database. Returns count of refs added.
fn insert_refs(conn: &Connection, file_id: i64, output: &serde_json::Value) -> u32 {
    let refs = match output.get("refs").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return 0,
    };

    let mut count = 0u32;
    for r in refs {
        let from_line = r.get("from_line").and_then(|v| v.as_i64()).unwrap_or(0);
        let to_name = r.get("to_name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = r.get("kind").and_then(|v| v.as_str());

        let result = conn.execute(
            "INSERT INTO refs (from_file_id, from_line, to_name, kind)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![file_id, from_line, to_name, kind],
        );

        if result.is_ok() {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::schema::open_db;
    use std::fs;
    use tempfile::TempDir;

    fn setup_temp_project() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join(".chibi").join("codebase.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = open_db(&db_path).unwrap();
        (conn, dir)
    }

    #[test]
    fn detect_language_known_extensions() {
        assert_eq!(detect_language(Path::new("src/main.rs")), Some("rust"));
        assert_eq!(detect_language(Path::new("app.py")), Some("python"));
        assert_eq!(detect_language(Path::new("index.tsx")), Some("typescript"));
        assert_eq!(detect_language(Path::new("README.md")), Some("markdown"));
    }

    #[test]
    fn detect_language_unknown_extension() {
        assert_eq!(detect_language(Path::new("file.xyz")), None);
        assert_eq!(detect_language(Path::new("Makefile")), None);
    }

    #[test]
    fn update_index_empty_project() {
        let (conn, dir) = setup_temp_project();
        let opts = IndexOptions::default();
        let stats = update_index(&conn, dir.path(), &opts, &[]).unwrap();
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.files_indexed, 0);
    }

    #[test]
    fn update_index_indexes_files() {
        let (conn, dir) = setup_temp_project();

        // Create some source files.
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("lib.py"), "def hello(): pass").unwrap();
        fs::write(dir.path().join("notes.txt"), "not code").unwrap();

        let opts = IndexOptions::default();
        let stats = update_index(&conn, dir.path(), &opts, &[]).unwrap();

        assert!(stats.files_scanned >= 3);
        assert!(stats.files_indexed >= 3);
        assert_eq!(stats.files_skipped, 0);

        // Verify files are in the database.
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        assert!(count >= 3);
    }

    #[test]
    fn update_index_incremental_skips_unchanged() {
        let (conn, dir) = setup_temp_project();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let opts = IndexOptions::default();

        // First run: indexes the file.
        let stats1 = update_index(&conn, dir.path(), &opts, &[]).unwrap();
        assert!(stats1.files_indexed >= 1);

        // Second run: should skip unchanged file.
        let stats2 = update_index(&conn, dir.path(), &opts, &[]).unwrap();
        assert!(stats2.files_skipped >= 1);
        assert_eq!(stats2.files_indexed, 0);
    }

    #[test]
    fn update_index_force_reindexes_all() {
        let (conn, dir) = setup_temp_project();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let default_opts = IndexOptions::default();
        update_index(&conn, dir.path(), &default_opts, &[]).unwrap();

        let force_opts = IndexOptions {
            force: true,
            ..Default::default()
        };
        let stats = update_index(&conn, dir.path(), &force_opts, &[]).unwrap();
        assert!(stats.files_indexed >= 1);
        assert_eq!(stats.files_skipped, 0);
    }

    #[test]
    fn update_index_removes_deleted_files() {
        let (conn, dir) = setup_temp_project();
        let file_path = dir.path().join("temp.rs");
        fs::write(&file_path, "fn temp() {}").unwrap();

        let opts = IndexOptions::default();
        update_index(&conn, dir.path(), &opts, &[]).unwrap();

        // File should be indexed.
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE path = 'temp.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Delete the file and re-index.
        fs::remove_file(&file_path).unwrap();
        let stats = update_index(&conn, dir.path(), &opts, &[]).unwrap();
        assert_eq!(stats.files_removed, 1);

        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE path = 'temp.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn update_index_skips_chibi_dir() {
        let (conn, dir) = setup_temp_project();

        // The .chibi directory already exists from setup. Add a file inside it.
        fs::write(dir.path().join(".chibi").join("config.toml"), "key = 'val'").unwrap();
        fs::write(dir.path().join("real.rs"), "fn real() {}").unwrap();

        let opts = IndexOptions::default();
        let stats = update_index(&conn, dir.path(), &opts, &[]).unwrap();

        // Only real.rs should be indexed, not .chibi/config.toml.
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM files")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(paths.contains(&"real.rs".to_string()));
        assert!(!paths.iter().any(|p| p.starts_with(".chibi")));
        assert!(stats.files_indexed >= 1);
    }

    #[test]
    fn insert_symbols_from_plugin_output() {
        let (conn, dir) = setup_temp_project();
        let _ = dir; // keep alive

        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
            [],
        )
        .unwrap();

        let output = serde_json::json!({
            "symbols": [
                {"name": "parse", "kind": "function", "line_start": 1, "line_end": 10,
                 "signature": "fn parse()", "visibility": "public"},
                {"name": "Ast", "kind": "struct", "line_start": 12, "line_end": 20}
            ]
        });

        let count = insert_symbols(&conn, 1, &output);
        assert_eq!(count, 2);

        let sym_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE file_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sym_count, 2);
    }

    #[test]
    fn insert_refs_from_plugin_output() {
        let (conn, dir) = setup_temp_project();
        let _ = dir;

        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
            [],
        )
        .unwrap();

        let output = serde_json::json!({
            "refs": [
                {"from_line": 5, "to_name": "Vec::new", "kind": "call"},
                {"from_line": 10, "to_name": "String", "kind": "type"}
            ]
        });

        let count = insert_refs(&conn, 1, &output);
        assert_eq!(count, 2);
    }

    #[test]
    fn insert_symbols_malformed_output_is_graceful() {
        let (conn, dir) = setup_temp_project();
        let _ = dir;

        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('test.rs', 'rust', 0, 0)",
            [],
        )
        .unwrap();

        // No symbols key.
        let count = insert_symbols(&conn, 1, &serde_json::json!({"other": 42}));
        assert_eq!(count, 0);

        // Symbols is not an array.
        let count = insert_symbols(&conn, 1, &serde_json::json!({"symbols": "bad"}));
        assert_eq!(count, 0);
    }

    #[test]
    fn language_detection_maps_correctly() {
        // Spot-check a few mappings.
        for (ext, expected) in &[
            ("rs", "rust"),
            ("py", "python"),
            ("go", "go"),
            ("zig", "zig"),
        ] {
            let path = format!("file.{}", ext);
            assert_eq!(detect_language(Path::new(&path)), Some(*expected));
        }
    }
}
