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
use std::path::Path;

/// Collect AGENTS.md content from all standard locations, falling back to
/// CLAUDE.md at each location when AGENTS.md is not found.
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

    // 1. ~/AGENTS.md (or ~/CLAUDE.md)
    try_load(home_dir, &mut sections);

    // 2. ~/.chibi/AGENTS.md (or ~/.chibi/CLAUDE.md)
    // Note: if chibi_home is under home_dir (the usual ~/.chibi case) and the
    // project root also lives under ~, the walk in step 3 could theoretically
    // visit the same file again. This is benign — duplicate content is either
    // deduplicated at the consumer level or simply seen twice by the LLM.
    try_load(chibi_home, &mut sections);

    // 3. Walk from project root down to cwd
    if let Ok(project_root) = project_root.canonicalize()
        && let Ok(cwd) = cwd.canonicalize()
    {
        if let Ok(rel) = cwd.strip_prefix(&project_root) {
            // Project root itself
            try_load(&project_root, &mut sections);
            // Each intermediate directory down to cwd
            let mut walk = project_root.clone();
            for component in rel.components() {
                walk = walk.join(component);
                try_load(&walk, &mut sections);
            }
        } else {
            // cwd is not under project_root (unusual); just check project root
            try_load(&project_root, &mut sections);
        }
    }

    sections.join("\n\n")
}

/// Try to read AGENTS.md from a directory; fall back to CLAUDE.md if not found.
/// If either exists and is non-empty, push its content.
fn try_load(dir: &Path, sections: &mut Vec<String>) {
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(content) = fs::read_to_string(dir.join(filename)) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                sections.push(trimmed.to_string());
                return;
            }
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

    #[test]
    fn test_claude_md_fallback() {
        // CLAUDE.md is used when AGENTS.md is absent at a given location
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        std::fs::write(home.join("CLAUDE.md"), "home claude").unwrap();
        std::fs::write(project.join("CLAUDE.md"), "project claude").unwrap();

        let result = load_agents_md(&home, &tmp.path().join("chibi"), &project, &project);
        assert_eq!(result, "home claude\n\nproject claude");
    }

    #[test]
    fn test_agents_md_takes_precedence_over_claude_md() {
        // When both exist at the same location, only AGENTS.md is loaded
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();

        std::fs::write(project.join("AGENTS.md"), "agents wins").unwrap();
        std::fs::write(project.join("CLAUDE.md"), "claude loses").unwrap();

        let result = load_agents_md(
            &tmp.path().join("home"),
            &tmp.path().join("chibi"),
            &project,
            &project,
        );
        assert_eq!(result, "agents wins");
    }

    #[test]
    fn test_mixed_agents_and_claude_md() {
        // Different locations can independently use AGENTS.md or CLAUDE.md
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        let sub = project.join("src");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(home.join("AGENTS.md"), "home agents").unwrap();
        std::fs::write(project.join("CLAUDE.md"), "project claude").unwrap();
        std::fs::write(sub.join("AGENTS.md"), "sub agents").unwrap();

        let result = load_agents_md(&home, &tmp.path().join("chibi"), &project, &sub);
        assert_eq!(result, "home agents\n\nproject claude\n\nsub agents");
    }
}
