//! `.claude/hooks/git_discipline_reminder.py`: PreToolUse on Bash. A command that commits, merges or pushes (also
//! `git -C <path> …` and ssh-wrapped) gets a reminder of CLAUDE.md §Git Rules; a raw `git commit` or `git push` sent
//! over ssh to a lab machine is denied and pointed at `~/.local/bin/exo code commit|push`, unless the command
//! carries EXO_RAW=1. The commit identity is the GitHub account gh is signed in to, asked of GitHub's API.
//!
//! Also the helpers the other ports in this group share (new_repo_worktree_check, primary_log_reminder): reading
//! the hook input the way the Python's `or` chains do, Python's json.dumps for the answer, filling `{placeholders}`
//! and running a program with a timeout.

use super::{text, texts, Out};
use fancy_regex::Regex;
use serde_json::{Map, Value};
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

const EMBEDDED: &str = include_str!("texts/git_discipline_reminder.json");

/// Python's `\w` on str: letters, numbers and `_` (str.isalnum() or '_').
const W: &str = r"[\p{L}\p{N}_]";
/// Python's `\s` on str, which also takes \x1c-\x1f (str.isspace()); Rust's `\s` does not.
const S: &str = r"[\s\x1c-\x1f]";
const NS: &str = r"[^\s\x1c-\x1f]";

/// Python's `\b` with Python's word characters.
fn b() -> String { format!(r"(?:(?<={W})(?!{W})|(?<!{W})(?={W}))") }

fn re(p: &str) -> Regex { Regex::new(p).expect("hook pattern") }

/// Strips the quoted argument of -m / --message / -F so words in a commit message do not count as commands.
fn strip_msg(cmd: &str) -> String {
    let cmd = re(&format!(r#"(-m|--message|-F){S}+"[^"]*""#)).replace_all(cmd, "${1}").into_owned();
    let cmd = re(&format!(r"(-m|--message|-F){S}+'[^']*'")).replace_all(&cmd, "${1}").into_owned();
    re(&format!(r#"--message=("[^"]*"|'[^']*'|{NS}+)"#)).replace_all(&cmd, "--message").into_owned()
}

/// A `git` followed, in the same shell segment, by the subcommand word.
fn has(cmd: &str, sub: &str) -> bool {
    let b = b();
    re(&format!(r"{b}git{b}[^\n;|&]*{b}{sub}{b}")).is_match(cmd).unwrap_or(false)
}

/// The lab machines by name: the vault's System/vault_rules.json "machines", or the three the Python falls back to.
fn machines() -> Vec<String> {
    let read = || -> Option<Vec<String>> {
        let f = super::project_dir()?.join("System").join("vault_rules.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        v["machines"]["hosts"].as_array()?.iter().map(|h| h["host"].as_str().map(String::from)).collect()
    };
    read().unwrap_or_else(|| ["dragon", "horse", "tiger"].map(String::from).to_vec())
}

/// The first lab machine an `ssh` in the command goes to.
fn lab_host(cmd: &str, hosts: &[String]) -> Option<String> {
    let b = b();
    let alts = hosts.iter().map(|h| fancy_regex::escape(h).into_owned()).collect::<Vec<_>>().join("|");
    let m = re(&format!(r"{b}ssh{b}[^\n;|&]*?{b}(?:[\p{{L}}\p{{N}}_.-]+@)?({alts}){b}")).captures(cmd).ok()??;
    m.get(1).map(|g| g.as_str().to_string()).filter(|h| !h.is_empty())
}

/// (login, noreply address) of the account gh is signed in to; None when there is none or GitHub does not answer.
fn gh_identity() -> Option<(String, String)> {
    let u = crate::github::get("user").ok()?;
    let login = u["login"].as_str()?.to_string();
    let id = u["id"].as_u64()?;
    Some((login.clone(), format!("{id}+{login}@users.noreply.github.com")))
}

pub fn run(stdin: &[u8]) -> Out {
    let t = texts("git_discipline_reminder", EMBEDDED);
    let Some(ti) = tool_input(stdin) else { return Out::quiet() };
    let Some(cmd) = py_or(&ti, &["command"]) else { return Out::quiet() };
    if cmd.is_empty() { return Out::quiet() }
    let scan = strip_msg(&cmd);
    let (push, merge, commit) = (has(&scan, "push"), has(&scan, "merge"), has(&scan, "commit"));
    if !(push || merge || commit) { return Out::quiet() }

    let host = lab_host(&scan, &machines());
    if let Some(host) = host.filter(|_| (commit || push) && !cmd.contains("EXO_RAW=1")) {
        let ident = match gh_identity() {
            Some((login, email)) => fill(&text(&t, "ident"), &[("login", &login), ("email", &email)]),
            None => text(&t, "ident_unknown"),
        };
        let mut what = vec![];
        if commit { what.push(fill(&text(&t, "deny_commit"), &[("host", &host), ("ident", &ident)])) }
        if push { what.push(fill(&text(&t, "deny_push"), &[("host", &host)])) }
        let why = fill(&text(&t, "deny"), &[("host", &host), ("commands", &what.join("\n"))]);
        return Out { stdout: exo_core::root_guard::deny_json(&why) + "\n", ..Out::default() };
    }

    let mut blocks = vec![text(&t, "header")];
    if push || commit {
        blocks.push(match gh_identity() {
            Some((login, email)) => fill(&text(&t, "identity"), &[("login", &login), ("email", &email)]),
            None => text(&t, "identity_unknown"),
        });
    }
    if push { blocks.push(text(&t, "push")) }
    if merge { blocks.push(text(&t, "merge")); blocks.push(text(&t, "merge_worktree")) }
    if commit { blocks.push(text(&t, "commit")) }
    context(&blocks.join("\n"))
}

// ---- shared by this group's hooks ----

/// The hook input's tool_input as the Python reads it: `data.get("tool_input") or {}`. None where the Python would
/// raise (stdin is JSON but not an object, or tool_input is a non-empty non-object) and where it returns early
/// (stdin not JSON); both end the hook quietly with exit 0 here.
pub(super) fn tool_input(stdin: &[u8]) -> Option<Map<String, Value>> {
    let Ok(Value::Object(data)) = serde_json::from_slice::<Value>(stdin) else { return None };
    match data.get("tool_input") {
        Some(Value::Object(o)) => Some(o.clone()),
        Some(v) if !falsy(v) => None,
        _ => Some(Map::new()),
    }
}

/// Python's truth value of a JSON value.
fn falsy(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// `ti.get(k1) or ti.get(k2) or ""`: the first truthy value, which must be a string (None where the Python would
/// raise on a number or a list).
pub(super) fn py_or(ti: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    match keys.iter().find_map(|k| ti.get(*k).filter(|v| !falsy(v))) {
        None => Some(String::new()),
        Some(v) => v.as_str().map(String::from),
    }
}

/// Python's json.dumps of a str: ensure_ascii, so everything outside space..tilde is a \u escape.
pub(super) fn py_str(s: &str) -> String {
    let mut out = String::new();
    for c in serde_json::to_string(s).unwrap().chars() {
        if (c as u32) < 0x7f { out.push(c) } else { for u in c.encode_utf16(&mut [0; 2]) { out += &format!("\\u{u:04x}") } }
    }
    out
}

/// The Python's `print(json.dumps({"hookSpecificOutput": {..., "additionalContext": text}}))`, byte for byte.
pub(super) fn context(text: &str) -> Out {
    let stdout = format!("{{\"hookSpecificOutput\": {{\"hookEventName\": \"PreToolUse\", \"additionalContext\": {}}}}}\n", py_str(text));
    Out { stdout, ..Out::default() }
}

/// Fills `{name}` in one pass; a value is never searched for placeholders itself, and an unknown `{x}` stays.
pub(super) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out += &rest[..i];
        rest = &rest[i..];
        match rest[1..].find('}').map(|j| &rest[1..1 + j]).and_then(|k| values.iter().find(|(n, _)| *n == k)) {
            Some((k, v)) => { out += v; rest = &rest[k.len() + 2..] }
            None => { out.push('{'); rest = &rest[1..] }
        }
    }
    out + rest
}

/// Runs a program with stdin closed: Some((exit code, stdout, stderr)), None when it could not start, timed out
/// or wrote something that is not UTF-8 (where the Python's subprocess.run(text=True) raises). Line ends come back
/// as "\n", as text=True gives them; a process killed by a signal is -1.
pub(super) fn program(prog: &str, args: &[&str], timeout: u64) -> Option<(i32, String, String)> {
    let mut child = crate::cmd(prog).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let reader = |mut p: Box<dyn Read + Send>| std::thread::spawn(move || { let mut b = vec![]; let _ = p.read_to_end(&mut b); b });
    let (o, e) = (reader(Box::new(child.stdout.take()?)), reader(Box::new(child.stderr.take()?)));
    let start = Instant::now();
    let status = loop {
        match child.try_wait().ok()? {
            Some(s) => break s,
            None if start.elapsed() > Duration::from_secs(timeout) => { let _ = child.kill(); let _ = child.wait(); return None }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let text = |b: Vec<u8>| String::from_utf8(b).ok().map(|s| s.replace("\r\n", "\n").replace('\r', "\n"));
    Some((status.code().unwrap_or(-1), text(o.join().ok()?)?, text(e.join().ok()?)?))
}

/// Python's str.strip(): Unicode whitespace and \x1c-\x1f.
pub(super) fn py_strip(s: &str) -> &str { s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcommands() {
        assert!(has("git commit -m x", "commit"));
        assert!(has("git add . && git commit -am x", "commit"));
        assert!(has("git -C /r push origin main", "push"));
        assert!(has("ssh ntk@dragon 'cd ~/x && git commit -m y'", "commit"));
        assert!(!has("git status; echo push", "push"));
        assert!(!has("git log | grep merge", "merge"));
        assert!(!has("git commit-tree x", "commit-tre"));
        assert!(!has("legit pushy", "push"));
        assert!(!has("git pushé", "push"));   // é is a word character in Python, so no \b
        assert!(!has("git push²", "push"));   // ² is a number, so a word character in Python too
    }

    #[test]
    fn messages_are_stripped() {
        assert_eq!(strip_msg(r#"git commit -m "fix push race""#), "git commit -m");
        assert_eq!(strip_msg("git commit --message 'merge it'"), "git commit --message");
        assert_eq!(strip_msg("git commit --message=push"), "git commit --message");
        assert_eq!(strip_msg("git commit -F\x1c\"a push\""), "git commit -F");
        assert!(!has(&strip_msg(r#"git commit -m "push""#), "push"));
    }

    #[test]
    fn lab_hosts() {
        let h = ["dragon", "horse", "tiger"].map(String::from).to_vec();
        assert_eq!(lab_host("ssh ntk@dragon 'git commit'", &h).as_deref(), Some("dragon"));
        assert_eq!(lab_host("ssh -o X=1 horse git push", &h).as_deref(), Some("horse"));
        assert_eq!(lab_host("ssh ntk@dragonfly git push", &h), None);
        assert_eq!(lab_host("echo dragon; ssh x git push", &h), None);
        assert_eq!(lab_host("git push", &h), None);
        assert_eq!(lab_host("ssh a.b-c@tiger x", &["ti.ger".into(), "tiger".into()]).as_deref(), Some("tiger"));
    }

    #[test]
    fn filling_and_json() {
        assert_eq!(fill("a {x} {y} {z}", &[("x", "{y}"), ("y", "2")]), "a {y} 2 {z}");
        assert_eq!(fill("{", &[]), "{");
        assert_eq!(fill("}{x", &[("x", "1")]), "}{x");
        let bs = '\\';   // the expected escapes spelled with a variable, so no tool turns them back into characters
        assert_eq!(py_str("\u{26a0} a\"\n\u{7f}\u{1f600}"), format!("\"{bs}u26a0 a{bs}\"{bs}n{bs}u007f{bs}ud83d{bs}ude00\""));
        assert_eq!(context("x").stdout, "{\"hookSpecificOutput\": {\"hookEventName\": \"PreToolUse\", \"additionalContext\": \"x\"}}\n");
    }

    #[test]
    fn input_shapes() {
        assert!(tool_input(b"").is_none());
        assert!(tool_input(b"[1]").is_none());
        assert!(tool_input(br#"{"tool_input": [1]}"#).is_none());
        assert_eq!(tool_input(br#"{"tool_input": 0}"#), Some(Map::new()));
        let ti = tool_input(br#"{"tool_input": {"file_path": "", "path": "p", "n": 5}}"#).unwrap();
        assert_eq!(py_or(&ti, &["file_path", "path"]).as_deref(), Some("p"));
        assert_eq!(py_or(&ti, &["n"]), None);
        assert_eq!(py_or(&ti, &["missing"]).as_deref(), Some(""));
    }
}
