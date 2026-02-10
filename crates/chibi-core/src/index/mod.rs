//! Codebase index — schema, file walker, and query interface.
//!
//! Uses rusqlite (WAL mode) to maintain a searchable index of files, symbols, and references
//! in the project. Language plugins (`lang_<language>`) provide symbol extraction;
//! core handles all database operations.

pub mod indexer;
pub mod query;
pub mod schema;

pub use indexer::{update_index, IndexOptions, IndexStats};
pub use query::{index_status, query_refs, query_symbols, RefRow, SymbolQuery, SymbolRow};
pub use schema::open_db;
