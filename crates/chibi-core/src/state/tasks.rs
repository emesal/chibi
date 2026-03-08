//! Task file parser and ephemeral summary builder.
//!
//! `.task` files contain two scheme datums: a metadata alist and a body string.
//! This module parses metadata via tein-sexp (no scheme evaluator) and builds
//! compact table summaries for ephemeral injection into the prompt.

use std::collections::HashMap;
use std::io;

use tein_sexp::{SexpKind, parser};

use crate::vfs::{VfsCaller, VfsEntryKind, VfsPath};

/// Parsed task metadata from the first datum of a `.task` file.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskMeta {
    pub id: String,
    pub status: String,
    pub priority: String,
    pub depends_on: Vec<String>,
    pub assigned_to: Option<String>,
    /// VFS path relative to tasks root (e.g. `"epic/login.task"`).
    /// Flock tasks annotated with `" (flock:name)"` suffix.
    pub path: String,
    /// First line of body datum, empty string if body absent or empty.
    pub summary_line: String,
}

/// Parse a `.task` file's content into metadata.
///
/// Reads the first datum (alist) for metadata fields and optionally the
/// second datum (string) for the body summary line.
pub fn parse_task(content: &str, relative_path: &str) -> io::Result<TaskMeta> {
    let datums = parser::parse_all(content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let alist = datums
        .first()
        .and_then(|s| s.as_list())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "first datum must be a list"))?;

    let mut id = String::new();
    let mut status = String::new();
    let mut priority = String::from("medium");
    let mut depends_on: Vec<String> = Vec::new();
    let mut assigned_to: Option<String> = None;

    for entry in alist {
        // Each entry is either a dotted pair `(key . val)` (DottedList with one head)
        // or a flat list `(key val...)` (List).
        let (key_sexp, values): (&tein_sexp::Sexp, Vec<&tein_sexp::Sexp>) = match &entry.kind {
            SexpKind::DottedList(heads, tail) => {
                if heads.len() != 1 {
                    continue;
                }
                (&heads[0], vec![tail.as_ref()])
            }
            SexpKind::List(items) if items.len() >= 2 => (&items[0], items[1..].iter().collect()),
            _ => continue,
        };

        let key = match key_sexp.as_symbol() {
            Some(k) => k,
            None => continue,
        };

        match key {
            "id" => {
                if let Some(v) = values.first().and_then(|s| s.as_string()) {
                    id = v.to_owned();
                }
            }
            "status" => {
                if let Some(v) = values.first().and_then(|s| s.as_symbol()) {
                    status = v.to_owned();
                }
            }
            "priority" => {
                if let Some(v) = values.first().and_then(|s| s.as_symbol()) {
                    priority = v.to_owned();
                }
            }
            "assigned-to" => {
                if let Some(v) = values.first().and_then(|s| s.as_string()) {
                    assigned_to = Some(v.to_owned());
                }
            }
            "depends-on" => {
                // Flat list: (depends-on "id1" "id2") → values = ["id1", "id2"]
                depends_on = values
                    .iter()
                    .filter_map(|s| s.as_string())
                    .map(|s| s.to_owned())
                    .collect();
            }
            _ => {}
        }
    }

    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task missing id field",
        ));
    }
    if status.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task missing status field",
        ));
    }

    let summary_line = datums
        .get(1)
        .and_then(|s| s.as_string())
        .map(|s| s.lines().next().unwrap_or("").to_owned())
        .unwrap_or_default();

    Ok(TaskMeta {
        id,
        status,
        priority,
        depends_on,
        assigned_to,
        path: relative_path.to_owned(),
        summary_line,
    })
}

/// Compute which task IDs are blocked (have depends-on where dep status != done).
///
/// Returns a map of task_id → vec of blocking dep IDs (only entries with blockers).
pub fn compute_blocked(tasks: &[TaskMeta]) -> HashMap<String, Vec<String>> {
    let status_map: HashMap<&str, &str> = tasks
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();

    let mut blocked = HashMap::new();
    for task in tasks {
        if task.depends_on.is_empty() {
            continue;
        }
        let blockers: Vec<String> = task
            .depends_on
            .iter()
            .filter(|dep_id| {
                // Blocked if dep status is not "done" (including unknown/missing deps)
                status_map
                    .get(dep_id.as_str())
                    .copied()
                    .unwrap_or("unknown")
                    != "done"
            })
            .cloned()
            .collect();
        if !blockers.is_empty() {
            blocked.insert(task.id.clone(), blockers);
        }
    }
    blocked
}

/// Build an ephemeral summary table from a list of tasks.
///
/// Returns empty string if tasks is empty (no injection).
///
/// Format:
/// ```text
/// --- tasks ---
/// id     status      priority  path              summary
/// a3f2   in-progress high      epic/login        implement the auth flow
/// --- 2 active (1 blocked), 1 done ---
/// ```
pub fn build_summary_table(tasks: &[TaskMeta]) -> String {
    if tasks.is_empty() {
        return String::new();
    }

    let blocked = compute_blocked(tasks);

    let mut out = String::from("--- tasks ---\n");
    out.push_str(&format!(
        "{:<8} {:<12} {:<10} {:<20} {}\n",
        "id", "status", "priority", "path", "summary"
    ));

    let mut active_count = 0usize;
    let mut done_count = 0usize;

    for task in tasks {
        let is_done = task.status == "done";
        if is_done {
            done_count += 1;
        } else {
            active_count += 1;
        }

        let summary = if let Some(blockers) = blocked.get(&task.id) {
            format!(
                "{} (blocked by: {})",
                task.summary_line,
                blockers.join(", ")
            )
        } else {
            task.summary_line.clone()
        };

        // Truncate path for display (max 20 chars)
        let path_display = if task.path.len() > 20 {
            format!("{}…", &task.path[..19])
        } else {
            task.path.clone()
        };

        out.push_str(&format!(
            "{:<8} {:<12} {:<10} {:<20} {}\n",
            task.id, task.status, task.priority, path_display, summary
        ));
    }

    let blocked_count = blocked.len();
    if blocked_count > 0 {
        out.push_str(&format!(
            "--- {} active ({} blocked), {} done ---\n",
            active_count, blocked_count, done_count
        ));
    } else {
        out.push_str(&format!(
            "--- {} active, {} done ---\n",
            active_count, done_count
        ));
    }

    out
}

/// Collect task metadata from all accessible task directories.
///
/// Reads `/home/<ctx>/tasks/` and `/flocks/<flock>/tasks/` for each
/// flock the context belongs to. Parses metadata only (first datum).
/// Skips unreadable or unparseable files silently.
pub async fn collect_tasks(vfs: &crate::vfs::Vfs, context_name: &str) -> Vec<TaskMeta> {
    let mut tasks = Vec::new();

    // Collect from context-local tasks dir
    let local_root = format!("/home/{}/tasks", context_name);
    collect_from_dir(vfs, &local_root, "", &mut tasks).await;

    // Collect from flock task dirs
    if let Ok(flocks) = vfs.flock_list_for(context_name).await {
        for flock_name in flocks {
            let flock_root = format!("/flocks/{}/tasks", flock_name);
            let flock_tag = format!(" (flock:{})", flock_name);
            collect_from_dir(vfs, &flock_root, &flock_tag, &mut tasks).await;
        }
    }

    tasks
}

/// Recursively collect `.task` files from a VFS directory.
async fn collect_from_dir(
    vfs: &crate::vfs::Vfs,
    dir_path: &str,
    path_suffix: &str,
    tasks: &mut Vec<TaskMeta>,
) {
    let Ok(zone_path) = VfsPath::new(dir_path) else {
        return;
    };
    if !vfs
        .exists(VfsCaller::System, &zone_path)
        .await
        .unwrap_or(false)
    {
        return;
    }
    let entries = match vfs.list(VfsCaller::System, &zone_path).await {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries {
        let full_path = format!("{}/{}", dir_path.trim_end_matches('/'), entry.name);
        if entry.kind == VfsEntryKind::Directory {
            // Recurse — Box::pin to handle async recursion
            Box::pin(collect_from_dir(vfs, &full_path, path_suffix, tasks)).await;
        } else if entry.name.ends_with(".task") {
            let Ok(file_path) = VfsPath::new(&full_path) else {
                continue;
            };
            let bytes = match vfs.read(VfsCaller::System, &file_path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };
            // relative_path = strip zone dir prefix (and leading slash), append suffix
            let root_prefix = dir_path.trim_end_matches('/');
            let rel = full_path
                .strip_prefix(root_prefix)
                .unwrap_or(entry.name.as_str())
                .trim_start_matches('/')
                .to_owned();
            let display_path = format!("{}{}", rel, path_suffix);
            if let Ok(meta) = parse_task(&content, &display_path) {
                tasks.push(meta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TASK: &str = r#"((id . "a3f2")
 (status . pending)
 (priority . high)
 (depends-on "b1c4" "e7d0")
 (assigned-to . "worker-1")
 (created . "20260308-1423z")
 (updated . "20260308-1445z"))

"implement the auth flow.

acceptance criteria:
- JWT tokens""#;

    const MINIMAL_TASK: &str = r#"((id . "b1c4")
 (status . done)
 (created . "20260308-1400z")
 (updated . "20260308-1500z"))
"#;

    #[test]
    fn test_parse_full_task() {
        let meta = parse_task(SAMPLE_TASK, "epic/login.task").unwrap();
        assert_eq!(meta.id, "a3f2");
        assert_eq!(meta.status, "pending");
        assert_eq!(meta.priority, "high");
        assert_eq!(meta.depends_on, vec!["b1c4", "e7d0"]);
        assert_eq!(meta.assigned_to, Some("worker-1".into()));
        assert_eq!(meta.path, "epic/login.task");
        assert_eq!(meta.summary_line, "implement the auth flow.");
    }

    #[test]
    fn test_parse_minimal_task() {
        let meta = parse_task(MINIMAL_TASK, "b1c4.task").unwrap();
        assert_eq!(meta.id, "b1c4");
        assert_eq!(meta.status, "done");
        assert_eq!(meta.priority, "medium"); // default
        assert_eq!(meta.depends_on, Vec::<String>::new());
        assert_eq!(meta.assigned_to, None);
        assert_eq!(meta.summary_line, "");
    }

    #[test]
    fn test_parse_invalid_content() {
        assert!(parse_task("not valid scheme", "bad.task").is_err());
    }

    #[test]
    fn test_parse_missing_id() {
        let content = r#"((status . pending) (created . "x") (updated . "x"))"#;
        assert!(parse_task(content, "x.task").is_err());
    }

    #[test]
    fn test_parse_missing_status() {
        let content = r#"((id . "ab12") (created . "x") (updated . "x"))"#;
        assert!(parse_task(content, "x.task").is_err());
    }

    #[test]
    fn test_compute_blocked() {
        let tasks = vec![
            TaskMeta {
                id: "a".into(),
                status: "done".into(),
                priority: "high".into(),
                depends_on: vec![],
                assigned_to: None,
                path: "a.task".into(),
                summary_line: "".into(),
            },
            TaskMeta {
                id: "b".into(),
                status: "pending".into(),
                priority: "medium".into(),
                depends_on: vec!["a".into()],
                assigned_to: None,
                path: "b.task".into(),
                summary_line: "".into(),
            },
            TaskMeta {
                id: "c".into(),
                status: "pending".into(),
                priority: "high".into(),
                depends_on: vec!["b".into()],
                assigned_to: None,
                path: "c.task".into(),
                summary_line: "".into(),
            },
        ];
        let blocked = compute_blocked(&tasks);
        // b depends on a which is done → not blocked
        assert!(!blocked.contains_key("b"));
        // c depends on b which is pending → blocked
        assert_eq!(blocked["c"], vec!["b"]);
    }

    #[test]
    fn test_build_summary_table() {
        let tasks = vec![
            TaskMeta {
                id: "a3f2".into(),
                status: "in-progress".into(),
                priority: "high".into(),
                depends_on: vec![],
                assigned_to: None,
                path: "epic/login.task".into(),
                summary_line: "implement the auth flow".into(),
            },
            TaskMeta {
                id: "b1c4".into(),
                status: "done".into(),
                priority: "medium".into(),
                depends_on: vec![],
                assigned_to: None,
                path: "ui/nav.task".into(),
                summary_line: "redesign nav".into(),
            },
        ];
        let table = build_summary_table(&tasks);
        assert!(table.contains("--- tasks ---"));
        assert!(table.contains("a3f2"));
        assert!(table.contains("in-progress"));
        assert!(table.contains("--- 1 active, 1 done ---"));
    }

    #[test]
    fn test_build_summary_empty() {
        let table = build_summary_table(&[]);
        assert!(table.is_empty(), "no tasks = no injection");
    }

    /// Verify that the injection logic (rposition + insert) places the task
    /// summary system message directly before the last user message.
    /// This mirrors the logic in send.rs without requiring AppState.
    #[test]
    fn test_injection_position() {
        use serde_json::json;

        let tasks = vec![TaskMeta {
            id: "a3f2".into(),
            status: "in-progress".into(),
            priority: "high".into(),
            depends_on: vec![],
            assigned_to: None,
            path: "epic/login.task".into(),
            summary_line: "implement the auth flow".into(),
        }];

        let summary = build_summary_table(&tasks);
        assert!(!summary.is_empty());

        let mut messages = vec![
            json!({"role": "user", "content": "first turn"}),
            json!({"role": "assistant", "content": "first reply"}),
            json!({"role": "user", "content": "current turn"}),
        ];

        // Replicate the injection logic from send.rs
        let inject = json!({"role": "system", "content": summary});
        if let Some(pos) = messages.iter().rposition(|m| m["role"] == "user") {
            messages.insert(pos, inject);
        }

        // System message should be at index 2 (before the last user message)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2]["role"], "system");
        assert!(messages[2]["content"].as_str().unwrap().contains("a3f2"));
        assert_eq!(messages[3]["role"], "user");
        assert_eq!(messages[3]["content"], "current turn");
    }

    /// When no tasks exist, build_summary_table returns empty — no injection.
    #[test]
    fn test_no_injection_when_no_tasks() {
        let summary = build_summary_table(&[]);
        assert!(summary.is_empty(), "empty summary → no injection happens");
    }
}
