//! Database schema and migration system for the codebase index.
//!
//! Uses rusqlite with WAL mode. Migrations are append-only — never edit existing entries,
//! only add new ones. `open_db` is the single entry point: it opens the database, enables
//! WAL + foreign keys, and applies any pending migrations.

use rusqlite::{Connection, Result as SqlResult};
use std::path::Path;

/// A single schema migration. Migrations are applied in order and tracked in `schema_meta`.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// Append-only migration list. Never edit existing entries — only add new ones at the end.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: "
        CREATE TABLE files (
            id        INTEGER PRIMARY KEY,
            path      TEXT    NOT NULL UNIQUE,
            lang      TEXT,
            mtime     INTEGER NOT NULL,
            size      INTEGER NOT NULL,
            hash      TEXT,
            indexed_at TEXT   NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE symbols (
            id         INTEGER PRIMARY KEY,
            file_id    INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            name       TEXT    NOT NULL,
            kind       TEXT    NOT NULL,
            parent_id  INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
            line_start INTEGER NOT NULL,
            line_end   INTEGER NOT NULL,
            signature  TEXT,
            visibility TEXT
        );

        CREATE TABLE refs (
            id           INTEGER PRIMARY KEY,
            from_file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            from_line    INTEGER NOT NULL,
            to_name      TEXT    NOT NULL,
            kind         TEXT
        );

        CREATE INDEX idx_symbols_name      ON symbols(name);
        CREATE INDEX idx_symbols_file_id   ON symbols(file_id);
        CREATE INDEX idx_symbols_parent_id ON symbols(parent_id);
        CREATE INDEX idx_refs_from_file_id ON refs(from_file_id);
        CREATE INDEX idx_refs_to_name      ON refs(to_name);
    ",
}];

/// Open (or create) the index database at `path`, enable WAL mode and foreign keys,
/// and apply any pending migrations. Returns the ready-to-use connection.
pub fn open_db(path: &Path) -> SqlResult<Connection> {
    let conn = Connection::open(path)?;

    // WAL mode for concurrent reads + single writer without blocking.
    conn.pragma_update(None, "journal_mode", "wal")?;
    conn.pragma_update(None, "foreign_keys", "on")?;

    // Bootstrap the migration-tracking table (idempotent).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    apply_migrations(&conn)?;
    Ok(conn)
}

/// Apply all migrations whose version hasn't been recorded yet.
fn apply_migrations(conn: &Connection) -> SqlResult<()> {
    let max_applied: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_meta",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    for m in MIGRATIONS {
        if m.version > max_applied {
            conn.execute_batch(m.sql)?;
            conn.execute(
                "INSERT INTO schema_meta (version) VALUES (?1)",
                [m.version],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_temp_db() -> (Connection, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = open_db(&db_path).unwrap();
        (conn, dir)
    }

    #[test]
    fn open_db_creates_tables() {
        let (conn, _dir) = open_temp_db();

        // All expected tables should exist.
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
        assert!(tables.contains(&"refs".to_string()));
        assert!(tables.contains(&"schema_meta".to_string()));
    }

    #[test]
    fn wal_mode_enabled() {
        let (conn, _dir) = open_temp_db();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn foreign_keys_enabled() {
        let (conn, _dir) = open_temp_db();
        let fk: i32 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        // Open twice — second open should not fail or re-apply.
        let conn1 = open_db(&db_path).unwrap();
        drop(conn1);
        let conn2 = open_db(&db_path).unwrap();

        let version_count: u32 = conn2
            .query_row("SELECT COUNT(*) FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version_count, MIGRATIONS.len() as u32);
    }

    #[test]
    fn migration_version_recorded() {
        let (conn, _dir) = open_temp_db();
        let max_version: u32 = conn
            .query_row("SELECT MAX(version) FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(max_version, 1);
    }

    #[test]
    fn insert_and_query_file() {
        let (conn, _dir) = open_temp_db();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES (?1, ?2, ?3, ?4)",
            ("src/main.rs", "rust", 1700000000i64, 1024i64),
        )
        .unwrap();

        let path: String = conn
            .query_row("SELECT path FROM files WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(path, "src/main.rs");
    }

    #[test]
    fn insert_and_query_symbol() {
        let (conn, _dir) = open_temp_db();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES (?1, ?2, ?3, ?4)",
            ("src/lib.rs", "rust", 1700000000i64, 512i64),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (1i64, "parse", "function", 10i64, 25i64, "fn parse(input: &str) -> Result<Ast>", "public"),
        )
        .unwrap();

        let name: String = conn
            .query_row("SELECT name FROM symbols WHERE file_id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "parse");
    }

    #[test]
    fn insert_and_query_ref() {
        let (conn, _dir) = open_temp_db();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES (?1, ?2, ?3, ?4)",
            ("src/main.rs", "rust", 1700000000i64, 256i64),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO refs (from_file_id, from_line, to_name, kind)
             VALUES (?1, ?2, ?3, ?4)",
            (1i64, 42i64, "TokenStream::new", "call"),
        )
        .unwrap();

        let to_name: String = conn
            .query_row("SELECT to_name FROM refs WHERE from_file_id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(to_name, "TokenStream::new");
    }

    #[test]
    fn foreign_key_cascade_deletes_symbols() {
        let (conn, _dir) = open_temp_db();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES (?1, ?2, ?3, ?4)",
            ("src/foo.rs", "rust", 1700000000i64, 100i64),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end)
             VALUES (1, 'foo', 'function', 1, 10)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO refs (from_file_id, from_line, to_name) VALUES (1, 5, 'bar')",
            [],
        )
        .unwrap();

        // Deleting the file should cascade to symbols and refs.
        conn.execute("DELETE FROM files WHERE id = 1", []).unwrap();

        let sym_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
            .unwrap();
        let ref_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sym_count, 0);
        assert_eq!(ref_count, 0);
    }

    #[test]
    fn unique_file_path_constraint() {
        let (conn, _dir) = open_temp_db();
        conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('a.rs', 'rust', 0, 0)",
            [],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO files (path, lang, mtime, size) VALUES ('a.rs', 'rust', 0, 0)",
            [],
        );
        assert!(result.is_err());
    }
}
