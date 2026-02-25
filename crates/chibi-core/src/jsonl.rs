//! JSONL read/write utilities.
//!
//! Generic functions for working with JSONL (JSON Lines) files.
//!
//! # Diagnostics
//!
//! Malformed lines are skipped with an `eprintln!` warning. This crate has no
//! logging dependency; `eprintln!` is the consistent warning mechanism used
//! throughout `chibi-core`. When a structured logging crate is adopted, all
//! such calls should be migrated together.

use serde::de::DeserializeOwned;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Read entries from a JSONL file, skipping malformed lines with warnings.
///
/// Warning messages include the file path and line number for easier debugging.
/// Returns an empty Vec if the file doesn't exist.
pub fn read_jsonl_file<T: DeserializeOwned>(path: &Path) -> io::Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!(
                    "[WARN] {}:{}: skipping malformed entry: {}",
                    path.display(),
                    line_num + 1,
                    e
                );
            }
        }
    }

    Ok(entries)
}
