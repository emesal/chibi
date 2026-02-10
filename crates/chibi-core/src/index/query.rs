//! Query interface for the codebase index.
//!
//! Provides symbol search, reference lookup, and index status reporting.
//! All queries are read-only and return structured data.

use rusqlite::Connection;
use std::fmt;
use std::path::Path;

/// A symbol row returned from a query.
#[derive(Debug)]
pub struct SymbolRow {
    pub id: i64,
    pub file_path: String,
    pub name: String,
    pub kind: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: Option<String>,
    pub visibility: Option<String>,
}

impl fmt::Display for SymbolRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}-{} {} {} {}",
            self.file_path,
            self.line_start,
            self.line_end,
            self.kind,
            self.name,
            self.signature.as_deref().unwrap_or("")
        )
    }
}

/// A reference row returned from a query.
#[derive(Debug)]
pub struct RefRow {
    pub file_path: String,
    pub from_line: i64,
    pub to_name: String,
    pub kind: Option<String>,
}

impl fmt::Display for RefRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} -> {} ({})",
            self.file_path,
            self.from_line,
            self.to_name,
            self.kind.as_deref().unwrap_or("unknown")
        )
    }
}

/// Options for querying symbols.
#[derive(Debug, Default)]
pub struct SymbolQuery {
    /// Filter by symbol name (substring match, case-insensitive).
    pub name: Option<String>,
    /// Filter by symbol kind (exact match, e.g. "function", "struct").
    pub kind: Option<String>,
    /// Filter by file path (substring match).
    pub file: Option<String>,
    /// Maximum results to return.
    pub limit: u32,
}

/// Query symbols from the index, filtering by name/kind/file.
pub fn query_symbols(conn: &Connection, opts: &SymbolQuery) -> Vec<SymbolRow> {
    let mut sql = String::from(
        "SELECT s.id, f.path, s.name, s.kind, s.line_start, s.line_end, s.signature, s.visibility
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref name) = opts.name {
        sql.push_str(" AND s.name LIKE ?");
        params.push(Box::new(format!("%{}%", name)));
    }
    if let Some(ref kind) = opts.kind {
        sql.push_str(" AND s.kind = ?");
        params.push(Box::new(kind.clone()));
    }
    if let Some(ref file) = opts.file {
        sql.push_str(" AND f.path LIKE ?");
        params.push(Box::new(format!("%{}%", file)));
    }

    let limit = if opts.limit == 0 { 50 } else { opts.limit };
    sql.push_str(&format!(" ORDER BY f.path, s.line_start LIMIT {}", limit));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        Ok(SymbolRow {
            id: row.get(0)?,
            file_path: row.get(1)?,
            name: row.get(2)?,
            kind: row.get(3)?,
            line_start: row.get(4)?,
            line_end: row.get(5)?,
            signature: row.get(6)?,
            visibility: row.get(7)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Find references to a given name in the index.
pub fn query_refs(conn: &Connection, to_name: &str, limit: u32) -> Vec<RefRow> {
    let limit = if limit == 0 { 50 } else { limit };

    let mut stmt = match conn.prepare(
        "SELECT f.path, r.from_line, r.to_name, r.kind
         FROM refs r
         JOIN files f ON r.from_file_id = f.id
         WHERE r.to_name LIKE ?1
         ORDER BY f.path, r.from_line
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map(
        rusqlite::params![format!("%{}%", to_name), limit],
        |row| {
            Ok(RefRow {
                file_path: row.get(0)?,
                from_line: row.get(1)?,
                to_name: row.get(2)?,
                kind: row.get(3)?,
            })
        },
    ) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Return a human-readable summary of the index status.
pub fn index_status(conn: &Connection, project_root: &Path) -> String {
    let file_count: u32 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap_or(0);
    let symbol_count: u32 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
        .unwrap_or(0);
    let ref_count: u32 = conn
        .query_row("SELECT COUNT(*) FROM refs", [], |row| row.get(0))
        .unwrap_or(0);

    // Language breakdown.
    let lang_stats: Vec<(String, u32)> = conn
        .prepare("SELECT COALESCE(lang, 'unknown'), COUNT(*) FROM files GROUP BY lang ORDER BY COUNT(*) DESC")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let mut out = format!(
        "Index: {} ({})\n  files: {}, symbols: {}, refs: {}",
        project_root.display(),
        if file_count == 0 { "empty" } else { "active" },
        file_count,
        symbol_count,
        ref_count,
    );

    if !lang_stats.is_empty() {
        out.push_str("\n  languages: ");
        let parts: Vec<String> = lang_stats.iter().map(|(l, c)| format!("{} ({})", l, c)).collect();
        out.push_str(&parts.join(", "));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::schema::open_db;
    use tempfile::TempDir;

    fn setup_db_with_data() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = open_db(&db_path).unwrap();

        // Insert test data.
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('src/main.rs', 'rust', 0, 100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('src/lib.rs', 'rust', 0, 200)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('app.py', 'python', 0, 50)",
            [],
        )
        .unwrap();

        // Symbols in main.rs (file_id=1).
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature, visibility)
             VALUES (1, 'main', 'function', 1, 5, 'fn main()', 'public')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature)
             VALUES (1, 'Config', 'struct', 7, 15, 'struct Config')",
            [],
        )
        .unwrap();

        // Symbols in lib.rs (file_id=2).
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature, visibility)
             VALUES (2, 'parse', 'function', 1, 20, 'fn parse(input: &str)', 'public')",
            [],
        )
        .unwrap();

        // Refs.
        conn.execute(
            "INSERT INTO refs (from_file_id, from_line, to_name, kind) VALUES (1, 3, 'parse', 'call')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO refs (from_file_id, from_line, to_name, kind) VALUES (1, 4, 'Config::new', 'call')",
            [],
        )
        .unwrap();

        (conn, dir)
    }

    #[test]
    fn query_symbols_by_name() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                name: Some("parse".into()),
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "parse");
        assert_eq!(results[0].file_path, "src/lib.rs");
    }

    #[test]
    fn query_symbols_by_kind() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                kind: Some("function".into()),
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn query_symbols_by_file() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                file: Some("main".into()),
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 2); // main + Config
    }

    #[test]
    fn query_symbols_combined_filters() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                name: Some("main".into()),
                kind: Some("function".into()),
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "main");
    }

    #[test]
    fn query_symbols_no_match() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                name: Some("nonexistent".into()),
                ..Default::default()
            },
        );
        assert!(results.is_empty());
    }

    #[test]
    fn query_symbols_respects_limit() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_symbols(
            &conn,
            &SymbolQuery {
                limit: 1,
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn query_refs_finds_matching() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_refs(&conn, "parse", 50);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].from_line, 3);
        assert_eq!(results[0].to_name, "parse");
    }

    #[test]
    fn query_refs_substring_match() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_refs(&conn, "Config", 50);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].to_name, "Config::new");
    }

    #[test]
    fn query_refs_no_match() {
        let (conn, _dir) = setup_db_with_data();
        let results = query_refs(&conn, "nonexistent", 50);
        assert!(results.is_empty());
    }

    #[test]
    fn index_status_empty_db() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = open_db(&db_path).unwrap();

        let status = index_status(&conn, Path::new("/tmp/project"));
        assert!(status.contains("empty"));
        assert!(status.contains("files: 0"));
    }

    #[test]
    fn index_status_with_data() {
        let (conn, _dir) = setup_db_with_data();
        let status = index_status(&conn, Path::new("/tmp/project"));
        assert!(status.contains("active"));
        assert!(status.contains("files: 3"));
        assert!(status.contains("symbols: 3"));
        assert!(status.contains("refs: 2"));
        assert!(status.contains("rust (2)"));
        assert!(status.contains("python (1)"));
    }

    #[test]
    fn symbol_row_display() {
        let row = SymbolRow {
            id: 1,
            file_path: "src/lib.rs".into(),
            name: "parse".into(),
            kind: "function".into(),
            line_start: 10,
            line_end: 25,
            signature: Some("fn parse()".into()),
            visibility: Some("public".into()),
        };
        let display = format!("{}", row);
        assert!(display.contains("src/lib.rs:10-25"));
        assert!(display.contains("function"));
        assert!(display.contains("parse"));
    }

    #[test]
    fn ref_row_display() {
        let row = RefRow {
            file_path: "src/main.rs".into(),
            from_line: 42,
            to_name: "Vec::new".into(),
            kind: Some("call".into()),
        };
        let display = format!("{}", row);
        assert!(display.contains("src/main.rs:42"));
        assert!(display.contains("Vec::new"));
        assert!(display.contains("call"));
    }
}
