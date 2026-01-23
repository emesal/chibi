//! JSONL read/write utilities.
//!
//! Generic functions for working with JSONL (JSON Lines) files.

use serde::de::DeserializeOwned;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Read entries from a JSONL file, skipping malformed lines with warnings.
/// Returns an empty Vec if the file doesn't exist.
pub fn read_jsonl_file<T: DeserializeOwned>(path: &Path, warn_prefix: &str) -> io::Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!("[WARN] Skipping malformed {} entry: {}", warn_prefix, e);
            }
        }
    }

    Ok(entries)
}
