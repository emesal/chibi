//! Flock context loading for prompt composition.
//!
//! A context may belong to the implicit site flock (`site:<site_id>`) plus any
//! number of explicit named flocks. This module loads per-flock prompts and
//! goals from the VFS and formats them for injection into the system prompt.

use std::io;

use crate::tools::vfs_block_on;
use crate::vfs::{Vfs, VfsCaller, VfsPath, flock::resolve_flock_vfs_root};

/// A single flock's prompt and goals, loaded from the VFS.
pub struct FlockContext {
    pub flock_name: String,
    pub prompt: Option<String>,
    pub goals: Option<String>,
}

/// Load all flock contexts for a given context (site flock first, then explicit).
pub fn load_flock_contexts(vfs: &Vfs, context_name: &str) -> io::Result<Vec<FlockContext>> {
    let site_flock_name = format!("site:{}", vfs.site_id());
    let explicit = vfs_block_on(vfs.flock_list_for(context_name))?;

    let mut result = Vec::with_capacity(1 + explicit.len());
    result.push(load_single_flock(vfs, &site_flock_name)?);
    for flock_name in explicit {
        result.push(load_single_flock(vfs, &flock_name)?);
    }
    Ok(result)
}

fn load_single_flock(vfs: &Vfs, flock_name: &str) -> io::Result<FlockContext> {
    let root = resolve_flock_vfs_root(flock_name, vfs.site_id())?;
    let goals = read_optional(vfs, &format!("{}/goals.md", root.as_str()))?;
    let prompt = read_optional(vfs, &format!("{}/prompt.md", root.as_str()))?;
    Ok(FlockContext {
        flock_name: flock_name.to_string(),
        prompt,
        goals,
    })
}

fn read_optional(vfs: &Vfs, path: &str) -> io::Result<Option<String>> {
    let p = VfsPath::new(path)?;
    match vfs_block_on(vfs.read(VfsCaller::System, &p)) {
        Ok(data) => {
            let s = String::from_utf8_lossy(&data).into_owned();
            Ok(if s.is_empty() { None } else { Some(s) })
        }
        Err(_) => Ok(None),
    }
}

/// Format flock contexts for injection into the system prompt.
///
/// Prompts are listed first (all flocks), then goals (all flocks), each
/// attributed by flock name.
pub fn format_flock_sections(contexts: &[FlockContext]) -> String {
    let mut out = String::new();
    for fc in contexts {
        if let Some(prompt) = &fc.prompt {
            out.push_str(&format!("\n\n--- PROMPT [{}] ---\n{}", fc.flock_name, prompt));
        }
    }
    for fc in contexts {
        if let Some(goals) = &fc.goals {
            out.push_str(&format!("\n\n--- GOALS [{}] ---\n{}", fc.flock_name, goals));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{LocalBackend, Vfs, VfsCaller, VfsPath};
    use tempfile::TempDir;

    fn setup(site_id: &str) -> (TempDir, Vfs) {
        let dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        (dir, Vfs::new(Box::new(backend), site_id))
    }

    fn write(vfs: &Vfs, path: &str, content: &str) {
        let p = VfsPath::new(path).unwrap();
        vfs_block_on(vfs.write(VfsCaller::System, &p, content.as_bytes())).unwrap();
    }

    fn mkdir(vfs: &Vfs, path: &str) {
        let p = VfsPath::new(path).unwrap();
        vfs_block_on(vfs.mkdir(VfsCaller::System, &p)).unwrap();
    }

    #[test]
    fn test_load_flock_contexts_includes_site() {
        let (_dir, vfs) = setup("test-site");
        mkdir(&vfs, "/site");
        write(&vfs, "/site/goals.md", "site goals here");

        let contexts = load_flock_contexts(&vfs, "ctx1").unwrap();
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].flock_name, "site:test-site");
        assert_eq!(contexts[0].goals.as_deref(), Some("site goals here"));
        assert!(contexts[0].prompt.is_none());
    }

    #[test]
    fn test_load_flock_contexts_includes_explicit_flocks() {
        let (_dir, vfs) = setup("test-site");
        mkdir(&vfs, "/site");
        mkdir(&vfs, "/flocks");
        mkdir(&vfs, "/flocks/myteam");
        write(&vfs, "/site/goals.md", "site goals");
        write(&vfs, "/flocks/myteam/goals.md", "team goals");

        // Join ctx1 to myteam
        vfs_block_on(vfs.flock_join("myteam", "ctx1")).unwrap();

        let contexts = load_flock_contexts(&vfs, "ctx1").unwrap();
        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].flock_name, "site:test-site");
        assert_eq!(contexts[1].flock_name, "myteam");
        assert_eq!(contexts[1].goals.as_deref(), Some("team goals"));
    }

    #[test]
    fn test_format_flock_goals_for_prompt() {
        let contexts = vec![
            FlockContext {
                flock_name: "site:s1".to_string(),
                prompt: Some("site prompt".to_string()),
                goals: Some("site goal".to_string()),
            },
            FlockContext {
                flock_name: "team".to_string(),
                prompt: None,
                goals: Some("team goal".to_string()),
            },
        ];
        let formatted = format_flock_sections(&contexts);
        assert!(formatted.contains("--- PROMPT [site:s1] ---"));
        assert!(formatted.contains("--- GOALS [site:s1] ---"));
        assert!(formatted.contains("--- GOALS [team] ---"));
        // no prompt section for team
        assert!(!formatted.contains("--- PROMPT [team] ---"));
    }

    #[test]
    fn test_format_flock_sections_empty() {
        let contexts = vec![FlockContext {
            flock_name: "site:s1".to_string(),
            prompt: None,
            goals: None,
        }];
        assert!(format_flock_sections(&contexts).is_empty());
    }
}
