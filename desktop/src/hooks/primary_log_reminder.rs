//! `.claude/hooks/primary_log_reminder.py`: PreToolUse on Edit/Write/MultiEdit. Editing a primary log (a file named
//! `_*.md`) gets a reminder of CLAUDE.md §Primary Log Editing Rule; when the log has an ownership block, also whether
//! its generated half is stale now, from the vault's `scripts/sync_ownership.py --check`.
//!
//! That check is the vault's own Python script, so it needs Python: found the way the app's Python badge finds it.
//! Without one the reminder goes out without the ownership part, as the Python does when the check cannot start.

use super::git_discipline_reminder::{context, fill, py_or, py_strip, program, tool_input};
use super::new_repo_worktree_check::abspath;
use super::{text, texts, Out};
use std::path::Path;

const EMBEDDED: &str = include_str!("texts/primary_log_reminder.json");

/// os.path.basename.
fn basename(p: &str) -> &str {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    p.rsplit(seps).next().unwrap_or(p)
}

/// A primary log: `re.match(r"^_.+\.md$", base)`, where Python's `$` also matches before a final newline.
fn is_primary_log(base: &str) -> bool {
    fancy_regex::Regex::new(r"^_.+\.md\n?$").unwrap().is_match(base).unwrap_or(false)
}

/// os.path.relpath(path, start); None where Python raises (Windows: another drive).
#[cfg(not(windows))]
fn relpath(path: &str, start: &str) -> Option<String> {
    let (p, s) = (abspath(path), abspath(start));
    let (p, s): (Vec<&str>, Vec<&str>) = (p.split('/').filter(|c| !c.is_empty()).collect(), s.split('/').filter(|c| !c.is_empty()).collect());
    let i = p.iter().zip(&s).take_while(|(a, b)| a == b).count();
    let rel: Vec<&str> = std::iter::repeat_n("..", s.len() - i).chain(p[i..].iter().copied()).collect();
    Some(if rel.is_empty() { ".".into() } else { rel.join("/") })
}
#[cfg(windows)]
fn relpath(path: &str, start: &str) -> Option<String> {
    use std::path::Component;
    let parts = |x: &str| -> Vec<String> { Path::new(&abspath(x)).components().filter(|c| !matches!(c, Component::RootDir | Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let (p, s) = (parts(path), parts(start));
    if p.first() != s.first() { return None }   // another drive
    let orig: Vec<String> = Path::new(&abspath(path)).components().filter(|c| !matches!(c, Component::RootDir | Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let i = p.iter().zip(&s).take_while(|(a, b)| a == b).count();
    let rel: Vec<String> = std::iter::repeat_n("..".to_string(), s.len() - i).chain(orig[i..].iter().cloned()).collect();
    Some(if rel.is_empty() { ".".into() } else { rel.join("\\") })
}

/// The program and leading arguments that start Python 3 here (the Python hook uses its own interpreter).
fn python() -> Option<Vec<String>> {
    let cmd = crate::python::python_state()["command"].as_str()?.to_string();
    // ponytail: the badge gives one string; a path with spaces is taken whole when it is a file, else split on spaces
    Some(if Path::new(&cmd).is_file() { vec![cmd] } else { cmd.split_whitespace().map(String::from).collect() })
}

/// The ownership sentence, or None where the Python's try block gives up and says nothing about it.
fn ownership(fp: &str, root: &str, t: &serde_json::Value) -> Option<String> {
    let body = std::fs::read_to_string(fp).ok()?;
    if !body.contains("<!-- ownership:start -->") { return None }
    let script = Path::new(root).join("scripts").join("sync_ownership.py").to_string_lossy().into_owned();
    let py = python()?;
    let args: Vec<&str> = py[1..].iter().map(String::as_str).chain([script.as_str(), "--check", fp]).collect();
    let (code, _, err) = program(&py[0], &args, 20)?;
    let state = match code {
        1 => text(t, "state_stale"),
        0 => text(t, "state_ok"),
        _ => {
            let err: String = py_strip(&err).chars().take(120).collect();
            fill(&text(t, "state_failed"), &[("stderr", if err.is_empty() { "?" } else { &err })])
        }
    };
    Some(fill(&text(t, "ownership"), &[("state", &state), ("path", &relpath(fp, root)?)]))
}

pub fn run(stdin: &[u8]) -> Out {
    let Some(ti) = tool_input(stdin) else { return Out::quiet() };
    let Some(fp) = py_or(&ti, &["file_path", "path"]) else { return Out::quiet() };
    let base = basename(&fp);
    if !is_primary_log(base) { return Out::quiet() }

    let t = texts("primary_log_reminder", EMBEDDED);
    let mut reminder = fill(&text(&t, "reminder"), &[("file", base)]);
    if let Some(root) = super::project_dir().map(|p| p.to_string_lossy().into_owned()) {
        if Path::new(&root).join("scripts").join("sync_ownership.py").exists() && Path::new(&fp).exists() {
            reminder += &ownership(&fp, &root, &t).unwrap_or_default();
        }
    }
    context(&reminder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_logs() {
        for b in ["_Management.md", "_a.md", "__.md", "_a.md\n", "_中文.md"] { assert!(is_primary_log(b), "{b:?}") }
        for b in ["_.md", "Management.md", "_a.md.bak", "_a.MD", "_a\nb.md", "_a.md\n\n", "x_a.md"] { assert!(!is_primary_log(b), "{b:?}") }
        assert_eq!(basename("/v/L1/_Log.md"), "_Log.md");
        assert_eq!(basename("_Log.md"), "_Log.md");
        assert_eq!(basename("/v/"), "");
    }

    #[cfg(not(windows))]
    #[test]
    fn relative_paths() {
        assert_eq!(relpath("/v/L1/_a.md", "/v").as_deref(), Some("L1/_a.md"));
        assert_eq!(relpath("/w/_a.md", "/v/x").as_deref(), Some("../../w/_a.md"));
        assert_eq!(relpath("/v", "/v").as_deref(), Some("."));
    }
}
