//! Lightweight checks for the repository's hand-written agent context and tasks.

use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_FILE_LINES: usize = 300;
const MAX_CONTEXT_LINES: usize = 1_200;

pub struct Task {
    pub status: String,
    pub priority: String,
    pub title: String,
    pub path: String,
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("guardrails crate lives in tools/")
        .to_path_buf()
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = vec![root.join("AGENTS.md"), root.join("README.md")];
    paths.push(root.join(".agents/skills/create-task/SKILL.md"));
    for directory in ["docs", "tasks"] {
        for entry in fs::read_dir(root.join(directory)).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|extension| extension == "md") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn check(root: &Path) -> Result<(), String> {
    let paths = markdown_files(root)?;
    let mut total_lines = 0;
    for path in &paths {
        let body = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let lines = body.lines().count();
        if lines > MAX_FILE_LINES {
            return Err(format!(
                "{} has {lines} lines (limit {MAX_FILE_LINES})",
                path.display()
            ));
        }
        total_lines += lines;
        check_links(path, &body)?;
    }
    if total_lines > MAX_CONTEXT_LINES {
        return Err(format!(
            "agent context has {total_lines} lines (limit {MAX_CONTEXT_LINES})"
        ));
    }
    read_tasks(root)?;
    Ok(())
}

fn check_links(path: &Path, body: &str) -> Result<(), String> {
    for segment in body.split("](").skip(1) {
        let Some(target) = segment.split(')').next() else {
            continue;
        };
        let target = target.trim().trim_matches('<').trim_matches('>');
        if target.starts_with("http:")
            || target.starts_with("https:")
            || target.starts_with("mailto:")
        {
            continue;
        }
        let (file, anchor) = target.split_once('#').unwrap_or((target, ""));
        let destination = if file.is_empty() {
            path.to_path_buf()
        } else {
            path.parent().expect("Markdown has parent").join(file)
        };
        if !destination.exists() {
            return Err(format!("{}: missing link target {target}", path.display()));
        }
        if !anchor.is_empty() {
            let linked = fs::read_to_string(&destination)
                .map_err(|e| format!("{}: {e}", destination.display()))?;
            let headings = linked
                .lines()
                .filter_map(|line| line.strip_prefix('#'))
                .map(|line| line.trim_start_matches('#').trim())
                .map(slug);
            if !headings.into_iter().any(|heading| heading == anchor) {
                return Err(format!("{}: missing anchor {target}", path.display()));
            }
        }
    }
    Ok(())
}

fn slug(heading: &str) -> String {
    heading
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '-')
        .map(|ch| {
            if ch.is_whitespace() {
                '-'
            } else {
                ch.to_ascii_lowercase()
            }
        })
        .collect()
}

pub fn read_tasks(root: &Path) -> Result<Vec<Task>, String> {
    let mut tasks = Vec::new();
    for entry in fs::read_dir(root.join("tasks")).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.file_name().is_some_and(|name| name == "README.md")
            || path.extension().is_none_or(|extension| extension != "md")
        {
            continue;
        }
        let body = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut lines = body.lines();
        if lines.next() != Some("---") {
            return Err(format!("{}: missing task frontmatter", path.display()));
        }
        let status = lines.next().and_then(|line| line.strip_prefix("status: "));
        let priority = lines
            .next()
            .and_then(|line| line.strip_prefix("priority: "));
        if lines.next() != Some("---") {
            return Err(format!(
                "{}: frontmatter must have exactly two fields",
                path.display()
            ));
        }
        let status = status.ok_or_else(|| format!("{}: missing status", path.display()))?;
        let priority = priority.ok_or_else(|| format!("{}: missing priority", path.display()))?;
        if ![
            "todo",
            "in_progress",
            "blocked",
            "deferred",
            "done",
            "canceled",
        ]
        .contains(&status)
        {
            return Err(format!("{}: invalid status {status}", path.display()));
        }
        if !["low", "normal", "high", "urgent"].contains(&priority) {
            return Err(format!("{}: invalid priority {priority}", path.display()));
        }
        let title = lines
            .find_map(|line| line.strip_prefix("# "))
            .ok_or_else(|| format!("{}: missing task title", path.display()))?;
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        tasks.push(Task {
            status: status.into(),
            priority: priority.into(),
            title: title.into(),
            path: relative.display().to_string(),
        });
    }
    tasks.sort_by(|a, b| {
        let rank = |priority: &str| match priority {
            "urgent" => 0,
            "high" => 1,
            "normal" => 2,
            _ => 3,
        };
        rank(&a.priority)
            .cmp(&rank(&b.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(tasks)
}
