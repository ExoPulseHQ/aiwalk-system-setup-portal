//! `.claude/hooks/primary_log_reminder.py`: PreToolUse on Edit/Write/MultiEdit and Bash. Editing a primary log (a file
//! named `_*.md`) gets a reminder of CLAUDE.md §Primary Log Editing Rule; when the log has an ownership block, also
//! whether its generated half is stale now: the same check as `aiwalk-setup vault sync-ownership --check`, run
//! in-process. A Dev Topic with a code the log does not have yet is refused once, with the log's topics listed, so a
//! continuation goes back into its line instead of opening a new one. Bash writing a primary log is refused: the
//! reminder and these checks only see Edit and Write.

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

/// What an edit takes out and puts in: Edit and MultiEdit their strings, Write the file on disk and the new content.
fn pairs(ti: &serde_json::Value, fp: &str) -> Vec<(String, String)> {
    if let Some(c) = ti["content"].as_str() {
        vec![(std::fs::read_to_string(fp).unwrap_or_default(), c.to_string())]
    } else if let Some(es) = ti["edits"].as_array() {
        es.iter().map(|e| (e["old_string"].as_str().unwrap_or_default().to_string(), e["new_string"].as_str().unwrap_or_default().to_string())).collect()
    } else { vec![(ti["old_string"].as_str().unwrap_or_default().to_string(), ti["new_string"].as_str().unwrap_or_default().to_string())] }
}

/// Dev Topic headings: `# 2026/10/23 ...`.
fn topic_headings(text: &str) -> Vec<String> {
    let re = fancy_regex::Regex::new(r"^# 20\d\d/\d\d/\d\d").unwrap();
    text.lines().filter(|l| re.is_match(l).unwrap_or(false)).map(|l| l.trim_end().to_string()).collect()
}

/// The code a heading ends with, its first one: `(BD-EL, SL, AS)` gives `BD-EL`.
fn topic_code(h: &str) -> Option<String> {
    let code = h.trim_end().strip_suffix(')')?.rsplit_once('(')?.1.split(',').next()?.trim().to_string();
    (!code.is_empty()).then_some(code)
}

/// The topic headings this edit brings in whose code the log on disk has under no heading: a new line of work. One
/// that keeps an existing code (a topic carried on to today and moved to the top) is not new.
fn new_topics(ti: &serde_json::Value, fp: &str) -> Vec<String> {
    let known: Vec<String> = topic_headings(&std::fs::read_to_string(fp).unwrap_or_default()).iter().filter_map(|h| topic_code(h)).collect();
    pairs(ti, fp).iter().flat_map(|(old, new)| {
        let had = topic_headings(old);
        topic_headings(new).into_iter().filter(move |h| !had.contains(h))
    }).filter(|h| topic_code(h).is_none_or(|c| !known.contains(&c))).collect()
}

/// The primary logs a shell command writes: it names a `_*.md` file and has a writer in it (in-place sed or perl, a
/// redirection into the log, tee, cp or mv, a Python write). ponytail: a text match, not a parse; a command that reads
/// a log next to an unrelated write is refused too, and is split in two or done with Edit.
fn bash_written_logs(cmd: &str) -> Vec<String> {
    let re = |p: &str| fancy_regex::Regex::new(p).unwrap();
    if re(r"^\s*(?:\w+=\S+\s+)*(?:ssh|git|aiwalk-setup|sh\s+\S*aiwalk)\s").is_match(cmd).unwrap_or(false) { return vec![] }
    let writer = re(r#"(?i:\b(?:Set-Content|Add-Content|Out-File|Copy-Item|Move-Item|New-Item)\b|\[IO\.File\]::(?:Write|Append))|\bsed\s+(?:-[a-zA-Z]*i|--in-place)|\bperl\s+-[a-zA-Z]*i|\btee\b|\b(?:cp|mv)\s|open\([^)]*['"][wa]|\.write\(|write_text\(|>>?\s*['"]?[^\s'"]*_[^\s/'"]+\.md"#);
    if !writer.is_match(cmd).unwrap_or(false) { return vec![] }
    let mut out: Vec<String> = re(r#"(?:^|[\s/'"=])(_[^\s/'"<>|;&()]+\.md)\b"#).captures_iter(cmd).flatten()
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string())).collect();
    out.dedup();
    out
}

pub fn run(stdin: &[u8]) -> Out {
    let Some(ti) = tool_input(stdin) else { return Out::quiet() };
    let data: serde_json::Value = serde_json::from_slice(stdin).unwrap_or_default();
    let deny = |why: String| Out { stdout: exo_core::root_guard::deny_json(&why) + "\n", ..Out::default() };
    if data["tool_name"] == "Bash" {
        let logs = bash_written_logs(ti.get("command").and_then(|c| c.as_str()).unwrap_or_default());
        if logs.is_empty() { return Out::quiet() }
        return deny(fill(&text(&texts("primary_log_reminder", EMBEDDED), "deny_bash_write"), &[("files", &logs.join(", "))]));
    }
    let Some(fp) = py_or(&ti, &["file_path", "path"]) else { return Out::quiet() };
    let base = basename(&fp);
    if !is_primary_log(base) { return Out::quiet() }

    let t = texts("primary_log_reminder", EMBEDDED);
    let fresh = new_topics(&serde_json::Value::Object(ti.clone()), &fp);
    if !fresh.is_empty() {
        let session = data["session_id"].as_str().unwrap_or_default();
        let key = format!("topic:{fp}:{}", fresh.join("|"));
        if super::writing_style_reminder::first_attempt(&key, session).unwrap_or(false) {
            let existing = topic_headings(&std::fs::read_to_string(&fp).unwrap_or_default());
            let list: Vec<&str> = existing.iter().take(15).map(String::as_str).collect();
            return deny(fill(&text(&t, "deny_new_topic"), &[("headings", &fresh.join("\n")), ("existing", &list.join("\n"))]));
        }
    }
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

    #[test]
    fn new_topic_codes_and_bash_writes() {
        assert_eq!(topic_code("# 2026/10/23 a (BD-EL, SL, AS)").as_deref(), Some("BD-EL"));
        assert_eq!(topic_code("# 2026/10/23 a"), None);
        let dir = std::env::temp_dir().join(format!("plr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fp = dir.join("_Log.md");
        std::fs::write(&fp, "# 2026/10/20 v (PSV-EL)\n\n---\n# 2026/10/17 b (BD-EL, SL, AS)\n").unwrap();
        let fp = fp.to_string_lossy().into_owned();
        let edit = |old: &str, new: &str| serde_json::json!({ "file_path": fp, "old_string": old, "new_string": new });
        assert_eq!(new_topics(&edit("---\n# 2026/10/20", "---\n# 2026/10/23 m (IMP-EL, SL)\n\n---\n# 2026/10/20"), &fp), vec!["# 2026/10/23 m (IMP-EL, SL)"], "a new code");
        assert!(new_topics(&edit("# 2026/10/17 b (BD-EL, SL, AS)", "# 2026/10/23 b2 (BD-EL, SL, AS)"), &fp).is_empty(), "carried on to today");
        assert!(new_topics(&edit("x", "y"), &fp).is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(bash_written_logs("python3 - <<'EOF'\np=\"Business/_Business_Plan.md\"; s=open(p).read()\nopen(p,\"w\").write(s)\nEOF"), vec!["_Business_Plan.md"]);
        assert_eq!(bash_written_logs("sed -i 's/a/b/' L1/_Log.md"), vec!["_Log.md"]);
        assert_eq!(bash_written_logs("Set-Content -Path L1/_Log.md -Value $t"), vec!["_Log.md"]);   // Windows' PowerShell tool
        assert_eq!(bash_written_logs("(Get-Content _Log.md) -replace 'a','b' | out-file _Log.md"), vec!["_Log.md"]);
        assert_eq!(bash_written_logs("echo x >> _Log.md"), vec!["_Log.md"]);
        assert!(bash_written_logs("grep -n x _Log.md > /tmp/out.txt 2>&1").is_empty(), "a read whose output goes elsewhere");
        assert!(bash_written_logs("sed -n 1,20p _Log.md; cat _Log.md").is_empty());
        assert!(bash_written_logs("git add _Log.md && git commit -m 'x > y'").is_empty());
        assert!(bash_written_logs("sh scripts/aiwalk vault ship -m m L1/_Log.md").is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn relative_paths() {
        assert_eq!(relpath("/v/L1/_a.md", "/v").as_deref(), Some("L1/_a.md"));
        assert_eq!(relpath("/w/_a.md", "/v/x").as_deref(), Some("../../w/_a.md"));
        assert_eq!(relpath("/v", "/v").as_deref(), Some("."));
    }
}
