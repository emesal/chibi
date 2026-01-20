# Additional Features Implementation Plan

## Overview

This plan implements all remaining TODO items related to:
- Context lockfiles for multi-process safety
- Noop recursion tool replacing built-in continue_processing
- Per-context configuration via local.toml
- JSON transcripts with from/to fields and unique IDs
- Inter-context communication (coffee-table, username)
- Agentic workflow prompts
- Reasoning tokens support

**Skipped:** Tandem Goals (deferred as requested)

---

## 1. Context Lockfiles

### Overview
Each chibi process acquires a lockfile on its context directory to prevent concurrent write access.

### File Structure

```
~/.chibi/contexts/<name>/.lock/
└── lockfile  # Contains "PID timestamp"
```

### Implementation

#### 1.1 Lockfile Module (`src/lock.rs` - new file)

```rust
use std::fs::{self, File};
use std::io::{self, Write, BufWriter};
use std::path::PathBuf;

const LOCK_DIR_NAME: &str = ".lock";
const LOCK_FILE_NAME: &str = "lockfile";

pub struct ContextLock {
    lock_dir: PathBuf,
}

impl ContextLock {
    pub fn acquire(context_dir: &PathBuf, heartbeat_secs: u64, timeout_secs: u64) -> io::Result<Self> {
        let lock_dir = context_dir.join(LOCK_DIR_NAME);
        let lock_file = lock_dir.join(LOCK_FILE_NAME);

        let start = std::time::Instant::now();
        let pid = std::process::id();
        let stale_threshold_secs = (heartbeat_secs as f64 * 1.5) as u64;

        loop {
            match fs::create_dir(&lock_dir) {
                Ok(_) => {
                    Self::write_lockfile(&lock_file, pid)?;
                    return Ok(ContextLock { lock_dir });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if Self::is_stale(&lock_dir, stale_threshold_secs) {
                        let _ = fs::remove_dir_all(&lock_dir);
                        continue;
                    }

                    if start.elapsed().as_secs() >= timeout_secs {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            format!("Could not acquire lock after {}s", timeout_secs),
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn write_lockfile(lock_file: &PathBuf, pid: u32) -> io::Result<()> {
        let timestamp = now_timestamp();
        let mut file = File::create(lock_file)?;
        writeln!(file, "{} {}", pid, timestamp)?;
        Ok(())
    }

    fn is_stale(lock_dir: &PathBuf, stale_threshold_secs: u64) -> bool {
        let lock_file = lock_dir.join(LOCK_FILE_NAME);
        if let Ok(content) = fs::read_to_string(&lock_file) {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(timestamp) = parts[1].parse::<u64>() {
                    let elapsed = now_timestamp() - timestamp;
                    if elapsed > stale_threshold_secs {
                        return true;
                    }
                    if let Ok(pid) = parts[0].parse::<u32>() {
                        return !Self::is_process_alive(pid);
                    }
                }
            }
        }
        true
    }

    fn is_process_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            use std::process::Command;
            Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        { false }
    }

    pub fn touch(&self) -> io::Result<()> {
        let lock_file = self.lock_dir.join(LOCK_FILE_NAME);
        Self::write_lockfile(&lock_file, std::process::id())
    }

    pub fn release(self) -> io::Result<()> {
        fs::remove_dir_all(&self.lock_dir).map_err(|e| {
            io::Error::new(e.kind(), format!("Failed to release lock: {}", e))
        })
    }

    pub fn get_status(context_dir: &PathBuf, heartbeat_secs: u64) -> Option<&'static str> {
        let lock_dir = context_dir.join(LOCK_DIR_NAME);
        if !lock_dir.exists() {
            return None;
        }
        let stale_threshold_secs = (heartbeat_secs as f64 * 1.5) as u64;
        if Self::is_stale(&lock_dir, stale_threshold_secs) {
            Some("[stale]")
        } else {
            Some("[active]")
        }
    }
}

impl Drop for ContextLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.lock_dir);
    }
}
```

#### 1.2 Background Heartbeat Thread

```rust
// In main.rs, after acquiring lock
if app.resolved_config.lock_heartbeat_seconds > 0 {
    let lock_for_thread = ContextLock { lock_dir: lock.lock_dir.clone() };
    let heartbeat_secs = app.resolved_config.lock_heartbeat_seconds;

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(heartbeat_secs));
            let _ = lock_for_thread.touch();
        }
    });
}
```

---

## 2. Recursion as Noop Tool

### Create External Tool (`examples/tools/recurse`)

```bash
#!/bin/bash
if [[ "$1" == "--schema" ]]; then
  cat <<'EOF'
{
  "name": "recurse",
  "description": "Continue processing without returning to user. Provide a note about what to do next.",
  "parameters": {
    "type": "object",
    "properties": {
      "note": {
        "type": "string",
        "description": "Note to self about what to do next"
      }
    },
    "required": ["note"]
  }
}
EOF
  exit 0
fi

note=$(echo "$CHIBI_TOOL_ARGS" | jq -r '.note')
echo "{\"note\": \"$note\"}"
```

### Remove Built-in Tool

Remove `CONTINUE_TOOL_NAME`, `continue_tool_to_api_format()`, `ContinueSignal`, `check_continue_signal()` from `src/tools.rs`.

---

## 3. Local.toml Per-Context Overrides

### New Config Structures

```rust
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct LocalConfig {
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub username: Option<String>,
    pub auto_compact: Option<bool>,
    pub max_recursion_depth: Option<usize>,
}

pub struct ResolvedConfig {
    pub api_key: String,
    pub model: String,
    pub context_window: usize,
    pub base_url: String,
    pub auto_compact: bool,
    pub reflection_enabled: bool,
    pub reflection_character_limit: usize,
    pub username: String,
    pub max_recursion_depth: usize,
    pub warn_threshold_percent: f32,
    pub lock_heartbeat_seconds: u64,
}
```

### CLI Username Flags

Add `-u/--username` (persistent) and `-U/--temp-username` (this invocation only) to `src/cli.rs`.

---

## 4. JSON Transcripts

### Transcript Entry Structure

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TranscriptEntry {
    pub id: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub content: String,
    pub entry_type: String,
}
```

### JSONL File

`~/.chibi/contexts/<name>/transcript.jsonl` - one JSON per line.

---

## 5. Inter-Context Communication

### Coffee-Table Tool

Create `examples/tools/coffee-table`:
- Auto-creates `coffee-table` context
- Uses `on_start` hook to inject system prompt notice
- Returns usage instructions

---

## 6. Agentic Workflow Prompts

### Create Example Prompts

- `examples/prompts/researcher.md`
- `examples/prompts/coder.md`
- `examples/prompts/reviewer.md`

---

## 7. Reasoning Tokens

### Research OpenRouter API

Investigate how to enable reasoning tokens and add config options.

---

## Implementation Order

1. Context Lockfiles
2. Local.toml Overrides
3. JSON Transcripts
4. Recursion Tool
5. Inter-Context Communication
6. Agentic Workflow Prompts
7. Reasoning Tokens

