//! `.claude/hooks/primary_log_reminder.py`: PreToolUse on Edit/Write/MultiEdit. Editing a primary log (a file named
//! `_*.md`) gets a reminder of CLAUDE.md §Primary Log Editing Rule; when the log has an ownership block, also whether
//! its generated half is stale now: the same check as `aiwalk-setup vault sync-ownership --check`, run in-process.

use super::git_discipline_reminder::{context, fill, py_or, py_strip, tool_input};
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

/// The ownership sentence, or None when the log has no ownership block (or sits on another drive than the vault).
fn ownership(fp: &str, root: &str, t: &serde_json::Value) -> Option<String> {
    let body = std::fs::read_to_string(fp).ok()?;
    if !body.contains("<!-- ownership:start -->") { return None }
    let rel = relpath(fp, root)?;
    let state = match crate::vaultcli::ownership(Path::new(root), Path::new(fp), &rel) {
        Ok(Some(_)) => text(t, "state_stale"),
        Ok(None) => text(t, "state_ok"),
        Err(e) => {
            let e: String = py_strip(&e).chars().take(120).collect();
            fill(&text(t, "state_failed"), &[("stderr", if e.is_empty() { "?" } else { &e })])
        }
    };
    Some(fill(&text(t, "ownership"), &[("state", &state), ("path", &rel)]))
}

pub fn run(stdin: &[u8]) -> Out {
    let Some(ti) = tool_input(stdin) else { return Out::quiet() };
    let Some(fp) = py_or(&ti, &["file_path", "path"]) else { return Out::quiet() };
    let base = basename(&fp);
    if !is_primary_log(base) { return Out::quiet() }

    let t = texts("primary_log_reminder", EMBEDDED);
    let mut reminder = fill(&text(&t, "reminder"), &[("file", base)]);
    if let Some(root) = super::project_dir().map(|p| p.to_string_lossy().into_owned()) {
        if Path::new(&fp).exists() {
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
