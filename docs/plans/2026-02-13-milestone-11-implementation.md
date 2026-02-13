# Milestone 11 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete milestone 11 — config foundation, tools configurability, twelve-factor assessment, model presets, JSON mode completion, and pre-release plugin audit.

**Architecture:** Five phases in dependency order. Phase 1 (foundation) is fully specified with TDD steps. Phases 2–5 are specified at design level and will get their own detailed plans when reached, since they depend on Phase 1 outcomes and external work (ratatoskr).

**Tech Stack:** Rust, serde/toml for config, tempfile for tests.

---

## Phase 1: Foundation

### Task 1: VCS root auto-detection (#125 part 1)

**Files:**
- Create: `crates/chibi-core/src/vcs.rs`
- Modify: `crates/chibi-core/src/chibi.rs:399-410` (update `resolve_project_root`)
- Modify: `crates/chibi-core/src/lib.rs` (add `mod vcs`)

**Step 1: Write failing tests for VCS root detection**

Create `crates/chibi-core/src/vcs.rs` with tests at the bottom:

```rust
//! VCS root detection.
//!
//! Walks up from a starting directory looking for version control markers.
//! Used to auto-detect the project root when not explicitly specified.

use std::path::{Path, PathBuf};

/// VCS markers to look for when walking up the directory tree.
/// Each entry is (marker_name, is_directory). Checked in order; first match wins.
const VCS_MARKERS: &[(&str, bool)] = &[
    (".git", true),       // also matches .git file (worktrees, submodules)
    (".hg", true),        // mercurial
    (".svn", true),       // subversion
    (".bzr", true),       // bazaar
    (".pijul", true),     // pijul
    (".jj", true),        // jujutsu
    (".fslckout", false),  // fossil (file)
    ("_FOSSIL_", false),   // fossil (alt)
];

/// Detect VCS root by walking up from `start` looking for markers.
///
/// Returns the first directory containing a VCS marker, or `None` if no
/// marker is found before reaching the filesystem root. Explicit
/// `--project-root` / `CHIBI_PROJECT_ROOT` should take precedence over this.
///
/// CVS is handled specially: `CVS/` directories appear at every level of a
/// checkout, so we walk up while `CVS/` is present and return the highest
/// directory that still contains it.
pub fn detect_vcs_root(start: &Path) -> Option<PathBuf> {
    let start = start.canonicalize().ok()?;

    // First pass: check standard markers (non-CVS)
    let mut current = start.as_path();
    loop {
        for &(marker, expect_dir) in VCS_MARKERS {
            let candidate = current.join(marker);
            if expect_dir {
                // .git can be a file (worktrees/submodules) or directory
                if marker == ".git" {
                    if candidate.exists() {
                        return Some(current.to_path_buf());
                    }
                } else if candidate.is_dir() {
                    return Some(current.to_path_buf());
                }
            } else if candidate.is_file() {
                return Some(current.to_path_buf());
            }
        }
        current = current.parent()?;
    }
}

/// CVS-specific root detection. Walks up while `CVS/` is present;
/// the highest directory still containing `CVS/` is the checkout root.
pub fn detect_cvs_root(start: &Path) -> Option<PathBuf> {
    let start = start.canonicalize().ok()?;
    if !start.join("CVS").is_dir() {
        return None;
    }
    let mut root = start.clone();
    let mut current = start.as_path();
    while let Some(parent) = current.parent() {
        if parent.join("CVS").is_dir() {
            root = parent.to_path_buf();
            current = parent;
        } else {
            break;
        }
    }
    Some(root)
}

/// Detect project root from VCS markers, trying standard VCS first, then CVS.
pub fn detect_project_root(start: &Path) -> Option<PathBuf> {
    detect_vcs_root(start).or_else(|| detect_cvs_root(start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_git_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_git_file() {
        // .git as file (worktree/submodule)
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".git"), "gitdir: /somewhere").unwrap();
        let sub = root.join("child");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_fossil_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".fslckout"), "").unwrap();
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_fossil_alt_marker() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("_FOSSIL_"), "").unwrap();

        assert_eq!(detect_vcs_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_mercurial_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".hg")).unwrap();

        assert_eq!(detect_vcs_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_jujutsu_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".jj")).unwrap();
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_detect_cvs_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // CVS dirs at every level
        std::fs::create_dir(root.join("CVS")).unwrap();
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::create_dir(sub.join("CVS")).unwrap();
        let deep = sub.join("deep");
        std::fs::create_dir(&deep).unwrap();
        std::fs::create_dir(deep.join("CVS")).unwrap();

        assert_eq!(detect_cvs_root(&deep), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_no_vcs_returns_none() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("empty");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(detect_vcs_root(&sub), None);
        assert_eq!(detect_cvs_root(&sub), None);
    }

    #[test]
    fn test_detect_project_root_prefers_standard_over_cvs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir(root.join("CVS")).unwrap();

        assert_eq!(detect_project_root(root), Some(root.canonicalize().unwrap()));
    }

    #[test]
    fn test_nearest_vcs_wins() {
        let tmp = TempDir::new().unwrap();
        let outer = tmp.path();
        std::fs::create_dir(outer.join(".git")).unwrap();
        let inner = outer.join("nested");
        std::fs::create_dir(&inner).unwrap();
        std::fs::create_dir(inner.join(".hg")).unwrap();
        let deep = inner.join("src");
        std::fs::create_dir(&deep).unwrap();

        // Should find .hg (nearer) not .git (farther)
        assert_eq!(detect_vcs_root(&deep), Some(inner.canonicalize().unwrap()));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core vcs::tests -- --nocapture`
Expected: compilation error (module not yet wired)

**Step 3: Wire up the module**

Add `pub mod vcs;` to `crates/chibi-core/src/lib.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core vcs::tests -- --nocapture`
Expected: all pass

**Step 5: Integrate VCS detection into `resolve_project_root`**

In `crates/chibi-core/src/chibi.rs`, update `resolve_project_root`:

```rust
/// Resolve project root: explicit path > `CHIBI_PROJECT_ROOT` env > VCS root > cwd.
fn resolve_project_root(explicit: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root);
    }
    if let Ok(env_root) = std::env::var("CHIBI_PROJECT_ROOT")
        && !env_root.is_empty()
    {
        return Ok(PathBuf::from(env_root));
    }
    let cwd = std::env::current_dir()?;
    Ok(crate::vcs::detect_project_root(&cwd).unwrap_or(cwd))
}
```

**Step 6: Run full test suite**

Run: `cargo test -p chibi-core`
Expected: all pass

**Step 7: Commit**

```bash
git add crates/chibi-core/src/vcs.rs crates/chibi-core/src/lib.rs crates/chibi-core/src/chibi.rs
git commit -m "feat: auto-detect project root from VCS markers (#125)

Walks up from cwd looking for .git, .hg, .svn, .bzr, .pijul, .jj,
.fslckout, _FOSSIL_, and CVS/. Nearest marker wins. Explicit
--project-root and CHIBI_PROJECT_ROOT still take precedence."
```

---

### Task 2: AGENTS.md loading (#125 part 2)

**Files:**
- Create: `crates/chibi-core/src/agents_md.rs`
- Modify: `crates/chibi-core/src/lib.rs` (add `mod agents_md`)
- Modify: `crates/chibi-core/src/api/send.rs:465-591` (inject into system prompt)

**Step 1: Write failing tests for AGENTS.md discovery**

Create `crates/chibi-core/src/agents_md.rs`:

```rust
//! AGENTS.md discovery and loading.
//!
//! Loads instruction files from standard locations following the AGENTS.md
//! convention. Files are concatenated in order; later entries appear later
//! in the prompt and can effectively override earlier guidance.
//!
//! Discovery locations (in order):
//! 1. ~/AGENTS.md — user-global, tool-independent
//! 2. ~/.chibi/AGENTS.md — chibi-global
//! 3. Directory walk from project root down to cwd, checking each level

use std::fs;
use std::path::{Path, PathBuf};

/// Collect AGENTS.md content from all standard locations.
///
/// Returns the concatenated content of all found files (separated by blank
/// lines), or an empty string if none exist. Empty files are skipped.
///
/// - `home_dir`: user home directory (~)
/// - `chibi_home`: chibi home directory (~/.chibi)
/// - `project_root`: detected or explicit project root
/// - `cwd`: current working directory (may equal project_root)
pub fn load_agents_md(
    home_dir: &Path,
    chibi_home: &Path,
    project_root: &Path,
    cwd: &Path,
) -> String {
    let mut sections = Vec::new();

    // 1. ~/AGENTS.md
    try_load(home_dir.join("AGENTS.md"), &mut sections);

    // 2. ~/.chibi/AGENTS.md
    try_load(chibi_home.join("AGENTS.md"), &mut sections);

    // 3. Walk from project root down to cwd
    if let Ok(project_root) = project_root.canonicalize() {
        if let Ok(cwd) = cwd.canonicalize() {
            if let Ok(rel) = cwd.strip_prefix(&project_root) {
                // Project root itself
                try_load(project_root.join("AGENTS.md"), &mut sections);
                // Each intermediate directory down to cwd
                let mut walk = project_root.clone();
                for component in rel.components() {
                    walk = walk.join(component);
                    try_load(walk.join("AGENTS.md"), &mut sections);
                }
            } else {
                // cwd is not under project_root (unusual); just check project root
                try_load(project_root.join("AGENTS.md"), &mut sections);
            }
        }
    }

    sections.join("\n\n")
}

/// Try to read a file; if it exists and is non-empty, push its content.
fn try_load(path: PathBuf, sections: &mut Vec<String>) {
    if let Ok(content) = fs::read_to_string(&path) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_no_agents_md_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let result = load_agents_md(
            tmp.path(),
            &tmp.path().join("chibi"),
            &tmp.path().join("project"),
            &tmp.path().join("project"),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_home_agents_md() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("AGENTS.md"), "global instructions").unwrap();

        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();

        let result = load_agents_md(&home, &tmp.path().join("chibi"), &project, &project);
        assert_eq!(result, "global instructions");
    }

    #[test]
    fn test_chibi_home_agents_md() {
        let tmp = TempDir::new().unwrap();
        let chibi = tmp.path().join("chibi");
        std::fs::create_dir(&chibi).unwrap();
        std::fs::write(chibi.join("AGENTS.md"), "chibi global").unwrap();

        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();

        let result = load_agents_md(&tmp.path().join("home"), &chibi, &project, &project);
        assert_eq!(result, "chibi global");
    }

    #[test]
    fn test_project_root_agents_md() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("AGENTS.md"), "project instructions").unwrap();

        let result = load_agents_md(
            &tmp.path().join("home"),
            &tmp.path().join("chibi"),
            &project,
            &project,
        );
        assert_eq!(result, "project instructions");
    }

    #[test]
    fn test_directory_walk_concatenation() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let sub = project.join("packages/frontend");
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(project.join("AGENTS.md"), "root level").unwrap();
        // packages/ has no AGENTS.md — should be skipped
        std::fs::write(sub.join("AGENTS.md"), "frontend level").unwrap();

        let result = load_agents_md(
            &tmp.path().join("home"),
            &tmp.path().join("chibi"),
            &project,
            &sub,
        );
        assert_eq!(result, "root level\n\nfrontend level");
    }

    #[test]
    fn test_full_precedence_order() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let chibi = tmp.path().join("chibi");
        let project = tmp.path().join("project");
        let sub = project.join("src");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&chibi).unwrap();
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(home.join("AGENTS.md"), "A").unwrap();
        std::fs::write(chibi.join("AGENTS.md"), "B").unwrap();
        std::fs::write(project.join("AGENTS.md"), "C").unwrap();
        std::fs::write(sub.join("AGENTS.md"), "D").unwrap();

        let result = load_agents_md(&home, &chibi, &project, &sub);
        assert_eq!(result, "A\n\nB\n\nC\n\nD");
    }

    #[test]
    fn test_empty_files_skipped() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        std::fs::write(home.join("AGENTS.md"), "").unwrap();
        std::fs::write(project.join("AGENTS.md"), "  \n  ").unwrap();

        let result = load_agents_md(&home, &tmp.path().join("chibi"), &project, &project);
        assert!(result.is_empty());
    }

    #[test]
    fn test_dedup_when_cwd_equals_project_root() {
        // When cwd == project_root, the project root AGENTS.md should appear only once
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("AGENTS.md"), "once").unwrap();

        let result = load_agents_md(
            &tmp.path().join("home"),
            &tmp.path().join("chibi"),
            &project,
            &project,
        );
        assert_eq!(result, "once");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-core agents_md::tests -- --nocapture`
Expected: compilation error (module not wired)

**Step 3: Wire up the module**

Add `pub mod agents_md;` to `crates/chibi-core/src/lib.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chibi-core agents_md::tests -- --nocapture`
Expected: all pass

**Step 5: Commit module**

```bash
git add crates/chibi-core/src/agents_md.rs crates/chibi-core/src/lib.rs
git commit -m "feat: AGENTS.md discovery and loading (#125)

Loads instruction files from ~/AGENTS.md, ~/.chibi/AGENTS.md, and
project root down to cwd. Files concatenated in order; empty files
skipped."
```

---

### Task 3: Integrate AGENTS.md into system prompt (#125 part 3)

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:465-591` (inject agents_md content)
- Modify: `crates/chibi-core/src/chibi.rs` (expose home_dir on Chibi struct)
- Modify: `crates/chibi-core/src/state/mod.rs` (expose home_dir on AppState)

**Step 1: Make home_dir accessible**

`AppState` already stores `base_dir` (the chibi home). Check whether the user's home directory (`~`) is stored anywhere. If not, add it to `Chibi` during `load_with_options`:

```rust
// In Chibi struct, add:
pub home_dir: PathBuf,  // user home (~)

// In load_with_options, resolve it:
let home_dir = dirs::home_dir()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot determine home directory"))?;
```

Check if `dirs` crate is already a dependency; if not, use `std::env::var("HOME")` with a fallback.

**Step 2: Inject AGENTS.md into `build_full_system_prompt`**

In `send.rs`, after the base system prompt is loaded (line 476) and before the username section (line 519), add:

```rust
// Load AGENTS.md instructions from standard locations
let agents_md = crate::agents_md::load_agents_md(
    &chibi.home_dir,
    &app.base_dir,
    &chibi.project_root,
    &std::env::current_dir().unwrap_or_default(),
);
if !agents_md.is_empty() {
    full_system_prompt.push_str("\n\n--- AGENT INSTRUCTIONS ---\n");
    full_system_prompt.push_str(&agents_md);
}
```

Note: `build_full_system_prompt` currently takes `app: &AppState`. It will need access to `Chibi` (or at least the three paths). Adjust the signature to accept what's needed — either pass the paths directly, or pass `&Chibi`. Follow whatever pattern minimizes disruption.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: all pass

**Step 4: Manual smoke test**

Create `~/AGENTS.md` with test content. Run chibi with `-v`. Verify the content appears in the system prompt (check via transcript or verbose output).

**Step 5: Commit**

```bash
git add -u
git commit -m "feat: inject AGENTS.md into system prompt (#125)

Loads agent instructions from ~/AGENTS.md, ~/.chibi/AGENTS.md, and
project tree. Appears in system prompt under '--- AGENT INSTRUCTIONS ---'
section, after the base prompt and before username/context metadata."
```

---

### Task 4: Update AGENTS.md with new AGENTS.md loading docs (#125 part 4)

**Files:**
- Modify: `AGENTS.md`

**Step 1: Add documentation**

Add a section to AGENTS.md documenting:
- The AGENTS.md convention and what it's for
- Discovery locations and precedence
- VCS root auto-detection (which VCS markers are supported)
- How project root resolution now works

**Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: document AGENTS.md loading and VCS root detection (#125)"
```

---

### Task 5: Add `exclude_categories` to ToolsConfig (#132 part 1)

**Files:**
- Modify: `crates/chibi-core/src/config.rs:300-308` (extend `ToolsConfig`)
- Modify: `crates/chibi-core/src/api/send.rs:203-230` (extend `filter_tools_by_config`)

**Step 1: Write failing test for category-based filtering**

In `crates/chibi-core/src/api/send.rs`, in the existing test module, add:

```rust
#[test]
fn test_filter_tools_by_category_exclude() {
    let tools = make_test_tools(&["shell_exec", "dir_list", "file_head", "update_todos", "spawn_agent"]);
    let config = ToolsConfig {
        include: None,
        exclude: None,
        exclude_categories: Some(vec!["coding".to_string()]),
    };
    let result = filter_tools_by_config(tools, &config);
    let names = tool_names(&result);
    assert!(!names.contains(&"shell_exec"));
    assert!(!names.contains(&"dir_list"));
    assert!(names.contains(&"file_head"));
    assert!(names.contains(&"update_todos"));
    assert!(names.contains(&"spawn_agent"));
}
```

(Adapt `make_test_tools` / `tool_names` helpers to match existing test patterns in that module.)

**Step 2: Run test to verify it fails**

Run: `cargo test -p chibi-core filter_tools_by_category -- --nocapture`
Expected: FAIL (field doesn't exist)

**Step 3: Add `exclude_categories` field to `ToolsConfig`**

In `config.rs`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    /// Exclude entire tool categories: "builtin", "file", "agent", "coding"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_categories: Option<Vec<String>>,
}
```

**Step 4: Implement category filtering in `filter_tools_by_config`**

In `send.rs`, extend `filter_tools_by_config` to handle `exclude_categories`. Use the existing `classify_tool_type` function and the `BUILTIN_TOOL_NAMES` / `FILE_TOOL_NAMES` / `AGENT_TOOL_NAMES` / `CODING_TOOL_NAMES` arrays to resolve category membership. The category names in config map to `ToolType` variants.

```rust
// After existing include/exclude logic, add:
if let Some(ref categories) = config.exclude_categories {
    let plugin_names: Vec<&str> = vec![]; // plugins can't be excluded by category
    result.retain(|tool| {
        if let Some(name) = tool.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
        {
            let tool_type = classify_tool_type(name, &plugin_names);
            !categories.contains(&tool_type.as_str().to_string())
        } else {
            true
        }
    });
}
```

**Step 5: Run tests**

Run: `cargo test -p chibi-core filter_tools`
Expected: all pass

**Step 6: Commit**

```bash
git add crates/chibi-core/src/config.rs crates/chibi-core/src/api/send.rs
git commit -m "feat: exclude tool categories via tools.exclude_categories (#132)

Supports 'builtin', 'file', 'agent', 'coding' category names.
Applied after individual include/exclude filters."
```

---

### Task 6: Support `[tools]` in global config.toml (#132 part 2)

**Files:**
- Modify: `crates/chibi-core/src/config.rs` (add `tools` field to `Config`)
- Modify: `crates/chibi-core/src/state/config_resolution.rs` (merge global tools config)

**Step 1: Write failing test**

In `config.rs` tests, add a test that creates a `Config` with `tools` set and verifies it deserializes correctly from TOML.

**Step 2: Add field to Config**

```rust
// In Config struct, add:
/// Tool filtering configuration (include/exclude/exclude_categories)
#[serde(default)]
pub tools: ToolsConfig,
```

**Step 3: Merge in config resolution**

In `config_resolution.rs`, when building `ResolvedConfig`, merge the global `tools` config with the context-local one. Local takes precedence (local exclude appends to global exclude; local include overrides global include if set).

**Step 4: Run tests**

Run: `cargo test -p chibi-core`
Expected: all pass

**Step 5: Commit**

```bash
git add -u
git commit -m "feat: support [tools] config in global config.toml (#132)

Global tools config is merged with per-context local.toml tools config.
Local overrides take precedence."
```

---

### Task 7: Document tools configuration (#132 part 3)

**Files:**
- Modify: `AGENTS.md`

**Step 1: Document**

Add documentation covering:
- All available builtin tools grouped by category
- `tools.include`, `tools.exclude`, `tools.exclude_categories` config syntax
- Where config can live (config.toml global, local.toml per-context)
- Examples for common use cases

**Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: document tool categories and filtering config (#132)"
```

---

### Task 8: Verify and document zero-config coding tools (#128)

**Files:**
- Modify: `AGENTS.md`
- Possibly modify: `crates/chibi-core/src/api/send.rs` or `crates/chibi-cli/src/main.rs`

**Step 1: Manual verification**

Test with a fresh chibi home (no plugins):
1. Verify coding tools appear in API request
2. Verify interactive TTY permission works for `shell_exec`, `file_edit`, `write_file`
3. Verify read-only tools work without permission prompts
4. Test piped/headless mode — confirm fail-safe deny behavior

**Step 2: Assess headless story**

If the current fail-safe deny in headless mode is problematic, consider adding a `--trust` flag that auto-approves all permission checks. This is a simple addition:

```rust
// In Cli struct:
/// Trust mode: auto-approve all permission checks (use with caution)
#[arg(long)]
pub trust: bool,
```

Wire it into the permission handler in `main.rs`. Only implement if the assessment shows it's needed.

**Step 3: Document**

Add to AGENTS.md:
- What works out of the box (no plugins needed)
- Permission model: TTY prompt for writes, auto-allow for reads
- How to use `--trust` for headless/automation (if added)
- What plugins can enhance (custom permission policies)

**Step 4: Commit**

```bash
git add -u
git commit -m "feat: document zero-config coding tools experience (#128)"
```

---

## Phase 2: Assessment (#130)

### Task 9: Twelve-factor audit

**This is a research/documentation task, not a code task.**

**Files:**
- Create: `docs/plans/YYYY-MM-DD-twelve-factor-audit.md`

Review chibi against each of the twelve factors. For each factor, document:
- **Status:** satisfied / partially satisfied / needs work / doesn't apply
- **Current state:** how chibi handles this today
- **Gaps:** what's missing (if anything)
- **Recommendation:** follow-up issue for a future milestone, or "no action needed"

Key factors to investigate:
- III (Config): chibi uses file-based config; no env var overrides for api_key/model
- IV (Backing services): how plugins and the index DB are treated
- V (Build/release/run): cargo install is clean
- VI (Processes): context storage model
- XI (Logs): stdout/stderr split is already good

File follow-up issues for anything that warrants future work. Close #130.

---

## Phase 3: Ratatoskr Detour (#129)

### Task 10: Land ratatoskr preset support

**This happens in the ratatoskr repo, not chibi.** Fey handles this.

### Task 11: Wire up presets in chibi

**Depends on Task 10.** Detail this plan once ratatoskr work is complete, since the chibi-side API depends on what ratatoskr exposes.

Expected work:
- Add `preset` field to config (global and local)
- Resolve preset → model + API params in config resolution
- Ship a default free preset
- Make `config.toml` optional (default to the free preset when absent)

---

## Phase 4: JSON Modes (#14, #133)

### Task 12: Audit and fix --json-output coverage (#14)

**Detail this plan when Phase 1 is complete.** The work involves:

1. Audit every output path in chibi-cli (grep for `println!`, `eprintln!`, `print!`, and any direct stdout writes)
2. Inventory which should be JSON-ified vs which are incompatible
3. Route all JSON-eligible output through `OutputHandler`
4. Add `--json-output` incompatibility checks (error if combined with incompatible flags)

### Task 13: Extract chibi-json crate (#133)

**Detail this plan after Task 12.** High-level:

1. Create `crates/chibi-json/` workspace member
2. Move JSON input parsing (`ChibiInput`, `json_input.rs`) to chibi-json
3. Move `OutputHandler` JSON mode to chibi-json
4. chibi-cli depends on chibi-json; chibi-json depends on chibi-core only
5. chibi-json compiles to its own binary

---

## Phase 5: Pre-release (#131)

### Task 14: Plugin audit

**Detail this when all prior phases are complete.** Steps:

1. List all plugins in the ecosystem
2. Test each against current chibi
3. Categorize: working / broken / redundant
4. Fix or deprecate as needed
5. Document plugin status
6. Run `just pre-push` and verify clean

---

## Summary

| Task | Issue | Phase | Type |
|------|-------|-------|------|
| 1 | #125 | 1 | VCS root detection |
| 2 | #125 | 1 | AGENTS.md module |
| 3 | #125 | 1 | System prompt integration |
| 4 | #125 | 1 | Documentation |
| 5 | #132 | 1 | Category-based tool filtering |
| 6 | #132 | 1 | Global tools config |
| 7 | #132 | 1 | Documentation |
| 8 | #128 | 1 | Zero-config verification & docs |
| 9 | #130 | 2 | Twelve-factor audit |
| 10 | #129 | 3 | Ratatoskr work (external) |
| 11 | #129 | 3 | Chibi preset wiring |
| 12 | #14 | 4 | JSON output audit |
| 13 | #133 | 4 | chibi-json extraction |
| 14 | #131 | 5 | Plugin audit |
