//! `.claude/hooks/new_repo_worktree_check.py`: PreToolUse on Edit/Write/MultiEdit. An edit in the main working tree
//! of a git repo that is not the session's vault (nor under it) and not a linked worktree gets a reminder to open a
//! worktree copy first. Silent everywhere else, and outside any git repo.

use super::git_discipline_reminder::{context, fill, py_or, py_strip, program, tool_input};
use super::{text, texts, Out};
use exo_core::root_guard::realpath;
use std::path::{Path, MAIN_SEPARATOR};

const EMBEDDED: &str = include_str!("texts/new_repo_worktree_check.json");

/// `git -C cwd args…`: its stripped stdout when it exits 0 within 5 seconds.
fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let all: Vec<&str> = ["-C", cwd].into_iter().chain(args.iter().copied()).collect();
    program("git", &all, 5).filter(|r| r.0 == 0).map(|r| py_strip(&r.1).to_string())
}

/// posixpath.normpath.
#[cfg(not(windows))]
fn normpath(p: &str) -> String {
    if p.is_empty() { return ".".into() }
    let lead = if p.starts_with("//") && !p.starts_with("///") { 2 } else if p.starts_with('/') { 1 } else { 0 };
    let mut comps: Vec<&str> = vec![];
    for c in p.split('/').filter(|c| !c.is_empty() && *c != ".") {
        if c != ".." || (lead == 0 && comps.is_empty()) || comps.last() == Some(&"..") { comps.push(c) } else { comps.pop(); }
    }
    let s = "/".repeat(lead) + &comps.join("/");
    if s.is_empty() { ".".into() } else { s }
}

/// os.path.abspath: joined to the working directory and normalised without looking at the disk.
#[cfg(not(windows))]
pub(super) fn abspath(p: &str) -> String {
    if p.starts_with('/') { return normpath(p) }
    let cwd = std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned();
    normpath(&if cwd.ends_with('/') { cwd + p } else { cwd + "/" + p })
}
#[cfg(windows)]
pub(super) fn abspath(p: &str) -> String {
    std::path::absolute(p).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
}

/// os.path.dirname.
#[cfg(not(windows))]
fn dirname(p: &str) -> String {
    let head = &p[..p.rfind('/').map_or(0, |i| i + 1)];
    if !head.is_empty() && head.chars().any(|c| c != '/') { head.trim_end_matches('/').into() } else { head.into() }
}
#[cfg(windows)]
fn dirname(p: &str) -> String { Path::new(p).parent().map_or(p.to_string(), |d| d.to_string_lossy().into_owned()) }

pub fn run(stdin: &[u8]) -> Out {
    let Some(ti) = tool_input(stdin) else { return Out::quiet() };
    let Some(fp) = py_or(&ti, &["file_path", "path"]) else { return Out::quiet() };
    if fp.is_empty() { return Out::quiet() }

    // the nearest existing directory: on Write the file, and maybe its folder, are not there yet
    let mut d = dirname(&abspath(&fp));
    if d.is_empty() { d = ".".into() }
    while !d.is_empty() && !Path::new(&d).is_dir() {
        let parent = dirname(&d);
        if parent == d { return Out::quiet() }
        d = parent;
    }

    let Some(top) = git(&d, &["rev-parse", "--show-toplevel"]).filter(|t| !t.is_empty()) else { return Out::quiet() };
    let top_r = realpath(Path::new(&top)).to_string_lossy().into_owned();
    let vault_r = super::project_dir().map(|v| realpath(&v).to_string_lossy().into_owned()).unwrap_or_default();
    if !vault_r.is_empty() && (top_r == vault_r || top_r.starts_with(&format!("{vault_r}{MAIN_SEPARATOR}"))) {
        return Out::quiet()   // the vault itself, or a submodule or folder under it
    }
    // a linked worktree's git dir is <main>/.git/worktrees/<name>
    let gd = git(&d, &["rev-parse", "--absolute-git-dir"]).unwrap_or_default();
    // git prints forward slashes on Windows too, which the Python (os.sep) never matched
    if gd.replace('\\', "/").contains("/worktrees/") { return Out::quiet() }

    let t = texts("new_repo_worktree_check", EMBEDDED);
    context(&fill(&text(&t, "reminder"), &[("repo", &top_r)]))
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn paths_as_python_writes_them() {
        for (p, n) in [("/a/./b//c/../d", "/a/b/d"), ("//a/b", "//a/b"), ("///a", "/a"), ("/..", "/"), ("a/../..", ".."), ("", ".")] {
            assert_eq!(normpath(p), n, "{p}");
        }
        for (p, d) in [("/a/b", "/a"), ("/a", "/"), ("/", "/"), ("//a", "//"), ("a", "")] { assert_eq!(dirname(p), d, "{p}") }
    }
}
