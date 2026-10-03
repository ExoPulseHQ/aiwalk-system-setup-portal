//! The root-only guard: the vault's `.claude/hooks/root_only_guard.py`, ported so a computer without Python runs it
//! through `aiwalk-setup hook root-only-guard`. A repo holding `.claude/root_only` may only be edited by a Claude
//! session whose project root is that repo; a session rooted elsewhere never loads the repo's CLAUDE.md or hooks.
//!
//! Parsing is the Python's, regex for regex, written as plain scanning (no regex crate: the heredoc pattern needs a
//! backreference). ponytail: Bash is parsed shallowly, exactly as in the Python. Only write targets count: redirect
//! targets, the arguments of a write command (tee, cp, mv, rm, touch, sed -i, ...), the repo of a git write verb, and
//! the literal path of a Python open-for-write; `cd` moves the base for relative paths. A path merely named in text (a
//! commit message, a heredoc string) is not a target. It misses scripts run by path; reads are never blocked.

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the Python registers; `--install` replaces it with the app's own command.
pub const PY_COMMAND: &str = "python3 \"$HOME/.claude/hooks/root_only_guard.py\"";
const HOOK_ARGS: &str = " hook root-only-guard";
const MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";
const WRITE_CMDS: [&str; 12] = ["tee", "cp", "mv", "rm", "rmdir", "mkdir", "touch", "install", "ln", "truncate", "dd", "rsync"];
const GIT_WRITES: [&str; 23] = ["add", "commit", "checkout", "switch", "restore", "reset", "merge", "rebase", "stash", "clean", "rm", "mv",
                                "apply", "am", "pull", "push", "init", "clone", "worktree", "cherry-pick", "revert", "tag", "branch"];
const PREFIXES: [&str; 6] = ["sudo", "time", "nohup", "env", "command", "export"];

/// Python's `\s` and `\w` on str.
fn space(c: char) -> bool { c.is_whitespace() || ('\x1c'..='\x1f').contains(&c) }
fn word(c: char) -> bool { c.is_alphanumeric() || c == '_' }
fn starts(s: &[char], i: usize, lit: &str) -> bool { lit.chars().enumerate().all(|(k, c)| s.get(i + k) == Some(&c)) }
fn skip_space(s: &[char], mut i: usize) -> usize { while i < s.len() && space(s[i]) { i += 1 } i }
fn text(s: &[char]) -> String { s.iter().collect() }
fn is_quote(c: Option<&char>) -> bool { matches!(c, Some('\'' | '"')) }

/// os.path.expanduser, for `~` and `~/...` (`~user` is left as it is).
fn expanduser(p: &str) -> String {
    let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let Some(rest) = p.strip_prefix('~') else { return p.to_string() };
    if !(rest.is_empty() || rest.starts_with(sep)) { return p.to_string() }
    #[allow(deprecated)] // home_dir is correct on Windows since Rust 1.85
    let Some(home) = std::env::home_dir() else { return p.to_string() };
    let home = home.to_string_lossy();
    let out = format!("{}{rest}", home.trim_end_matches(sep));
    if out.is_empty() { "/".into() } else { out }
}

/// `[rbuf]?['"]`: the index after the quote.
fn quote_open(s: &[char], i: usize) -> Option<usize> {
    if is_quote(s.get(i)) { Some(i + 1) } else if matches!(s.get(i), Some('r' | 'b' | 'u' | 'f')) && is_quote(s.get(i + 1)) { Some(i + 2) } else { None }
}

/// `[rbuf]?['"]([^'"]+)['"]`: the string and the index after its closing quote.
fn quoted(s: &[char], i: usize) -> Option<(String, usize)> {
    let a = quote_open(s, i)?;
    let j = a + s[a..].iter().position(|c| matches!(c, '\'' | '"'))?;
    (j > a).then(|| (text(&s[a..j]), j + 1))
}

/// PY_WRITE at i: `open\(\s*Q\s*,\s*[rbuf]?['"][^'"]*[wax]` or `Path\(\s*Q\s*\)\.write_(?:text|bytes)`.
fn py_write_at(s: &[char], i: usize) -> Option<(String, usize)> {
    if starts(s, i, "open(") {
        let (p, j) = quoted(s, skip_space(s, i + 5))?;
        let j = skip_space(s, j);
        if s.get(j) != Some(&',') { return None }
        let j = quote_open(s, skip_space(s, j + 1))?;
        let run = s[j..].iter().position(|c| matches!(c, '\'' | '"')).map_or(s.len(), |n| j + n);
        return (j..run).rev().find(|&k| matches!(s[k], 'w' | 'a' | 'x')).map(|k| (p, k + 1));
    }
    if starts(s, i, "Path(") {
        let (p, j) = quoted(s, skip_space(s, i + 5))?;
        let j = skip_space(s, j);
        if !starts(s, j, ").write_") { return None }
        return ["text", "bytes"].iter().find(|w| starts(s, j + 8, w)).map(|w| (p, j + 8 + w.len()));
    }
    None
}

/// HEREDOC at i: `<<-?\s*['"]?(\w+)['"]?[^\n]*\n(.*?)\n\s*\1\s*(?=\n|$)`, dot matching newlines. The end of the match.
fn heredoc_at(s: &[char], i: usize) -> Option<usize> {
    if !starts(s, i, "<<") { return None }
    let mut j = i + 2;
    if s.get(j) == Some(&'-') { j += 1 }
    j = skip_space(s, j);
    if is_quote(s.get(j)) { j += 1 }
    let w0 = j;
    while j < s.len() && word(s[j]) { j += 1 }
    let body = j + s[j..].iter().position(|&c| c == '\n')? + 1;
    // the regex backtracks into \w+, so a shorter prefix of the word may close it too; longest first, as it tries
    for wl in (1..=j - w0).rev() {
        let w = text(&s[w0..w0 + wl]);
        for k in (body..s.len()).filter(|&k| s[k] == '\n') {
            let a = skip_space(s, k + 1);
            if !starts(s, a, &w) { continue }
            let e = a + wl;
            let r = skip_space(s, e);
            if r == s.len() { return Some(r) }
            if let Some(p) = (e..r).rev().find(|&p| s[p] == '\n') { return Some(p) }
        }
    }
    None
}

/// The segment separator `\s*(?:&&|\|\||;|\||\n)\s*` at i: the end of the match.
fn sep_at(s: &[char], i: usize) -> Option<usize> {
    let len = |k: usize| if starts(s, k, "&&") || starts(s, k, "||") { 2 } else if matches!(s.get(k), Some(';' | '|' | '\n')) { 1 } else { 0 };
    (i..=skip_space(s, i)).rev().find(|&k| len(k) > 0).map(|k| skip_space(s, k + len(k)))
}

/// REDIRECT at i: `(?<![<>=&])\d?>>?\s*([^\s;|&<>()]+)`.
fn redirect_at(s: &[char], i: usize) -> Option<(String, usize)> {
    if i > 0 && matches!(s[i - 1], '<' | '>' | '=' | '&') { return None }
    let mut j = i + s.get(i).map_or(0, |c| c.is_ascii_digit() as usize);
    if s.get(j) != Some(&'>') { return None }
    j += 1 + (s.get(j + 1) == Some(&'>')) as usize;
    let t0 = skip_space(s, j);
    let t1 = t0 + s[t0..].iter().position(|&c| space(c) || ";|&<>()".contains(c)).unwrap_or(s.len() - t0);
    (t1 > t0).then(|| (text(&s[t0..t1]), t1))
}

/// Every match of `at`, scanned left to right without overlap as re.finditer does: the matches and the rest.
fn scan<T>(s: &[char], at: impl Fn(&[char], usize) -> Option<(T, usize)>) -> (Vec<T>, Vec<char>) {
    let (mut found, mut rest, mut i) = (vec![], vec![], 0);
    while i < s.len() {
        match at(s, i) { Some((t, end)) => { found.push(t); i = end } None => { rest.push(s[i]); i += 1 } }
    }
    (found, rest)
}

/// `\$\{?(\w+)\}?` replaced by the variable set earlier in the command, else the environment's, else left as it is.
fn expand(s: &[char], env: &HashMap<String, String>) -> Vec<char> {
    let (mut out, mut i) = (vec![], 0);
    while i < s.len() {
        let w0 = i + 1 + (s.get(i + 1) == Some(&'{')) as usize;
        let w1 = w0 + s[w0..].iter().position(|&c| !word(c)).unwrap_or(s.len() - w0);
        if s[i] != '$' || w1 == w0 { out.push(s[i]); i += 1; continue }
        let end = w1 + (s.get(w1) == Some(&'}')) as usize;
        let name = text(&s[w0..w1]);
        out.extend(env.get(&name).cloned().or_else(|| std::env::var(&name).ok()).unwrap_or_else(|| text(&s[i..end])).chars());
        i = end;
    }
    out
}

/// shlex.split (POSIX mode, no comments); None where it raises ValueError.
fn shlex_split(s: &[char]) -> Option<Vec<String>> {
    let ws = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let (mut toks, mut i) = (vec![], 0);
    loop {
        let (mut tok, mut quoted, mut state, mut esc) = (String::new(), false, ' ', 'a');
        let eof = loop {
            let c = s.get(i).copied();
            i += 1;
            match (state, c) {
                (' ' | 'a', None) => break true,
                (' ', Some(c)) if ws(c) => {}
                ('a', Some(c)) if ws(c) => { state = ' '; if !tok.is_empty() || quoted { break false } }
                (' ' | 'a', Some('\\')) => { esc = 'a'; state = '\\' }
                (' ' | 'a', Some(q @ ('\'' | '"'))) => state = q,
                (' ' | 'a', Some(c)) => { tok.push(c); state = 'a' }
                ('\\', None) => return None,          // No escaped character
                ('\\', Some(c)) => { if esc != 'a' && c != '\\' && c != esc { tok.push('\\') } tok.push(c); state = esc }
                (_, None) => return None,             // No closing quotation
                (q, Some(c)) => { quoted = true; if c == q { state = 'a' } else if c == '\\' && q == '"' { esc = '"'; state = '\\' } else { tok.push(c) } }
            }
        };
        if !tok.is_empty() || quoted { toks.push(tok) }
        if eof { return Some(toks) }
    }
}

/// Paths a shell command writes to, resolved against cwd (and any `cd` before them).
pub fn bash_targets(cmd: &str, cwd: &Path) -> Vec<PathBuf> {
    let s: Vec<char> = cmd.chars().collect();
    let (py, _) = scan(&s, py_write_at);                       // literal open-for-write, even inside heredocs
    let mut out: Vec<(PathBuf, String)> = py.into_iter().map(|p| (cwd.to_path_buf(), p)).collect();
    let (_, s) = scan(&s, |s, i| heredoc_at(s, i).map(|e| ((), e)));   // heredoc text is data, not arguments
    let (mut here, mut env) = (cwd.to_path_buf(), HashMap::new());
    for seg in segments(&s) {
        // N=/path; cp x $N/f  writes into /path: expand variables set earlier in the command, then the environment's
        let seg = expand(&seg, &env);
        let (redirs, rest) = scan(&seg, redirect_at);
        out.extend(redirs.into_iter().map(|t| (here.clone(), t)));   // a capture never starts with &, so the Python's filter is moot
        let mut tok = shlex_split(&rest).unwrap_or_else(|| text(&rest).split(space).filter(|t| !t.is_empty()).map(String::from).collect());
        while let Some(t) = tok.first() {
            let n = t.chars().take_while(|&c| word(c)).count();
            if n > 0 && t.chars().nth(n) == Some('=') {
                let (k, v) = t.split_once('=').unwrap();
                env.insert(k.to_string(), expanduser(v));
            } else if !PREFIXES.contains(&t.as_str()) {
                break
            }
            tok.remove(0);
        }
        let Some((first, args)) = tok.split_first() else { continue };
        let name = first.rsplit(|c| c == '/' || (cfg!(windows) && c == '\\')).next().unwrap_or("");
        let sed_inplace = |a: &String| a.starts_with("--in-place") || (a.starts_with('-') && a[1..].chars().take_while(|&c| word(c)).any(|c| c == 'i'));
        if name == "cd" && !args.is_empty() {
            here = here.join(expanduser(&args[0]));
        } else if WRITE_CMDS.contains(&name) || (name == "sed" && args.iter().any(sed_inplace)) {
            out.extend(args.iter().filter(|a| !a.starts_with('-')).map(|a| (here.clone(), a.clone())));
        } else if name == "git" {
            let (mut repo, mut i) = (here.clone(), 0);
            while i < args.len() && args[i].starts_with('-') {
                if ["-C", "--git-dir", "--work-tree", "-c"].contains(&args[i].as_str()) && i + 1 < args.len() {
                    if args[i] != "-c" { repo = here.join(expanduser(&args[i + 1])) }
                    i += 1;
                } else if let Some(v) = args[i].strip_prefix("--git-dir=").or_else(|| args[i].strip_prefix("--work-tree=")) {
                    repo = here.join(expanduser(v));
                }
                i += 1;
            }
            if i < args.len() && GIT_WRITES.contains(&args[i].as_str()) { out.push((here.clone(), repo.to_string_lossy().into())) }
        }
    }
    out.into_iter().map(|(b, p)| b.join(expanduser(&p))).collect()
}

/// re.split on the separator: the pieces between matches.
fn segments(s: &[char]) -> Vec<Vec<char>> {
    let (mut segs, mut cur, mut i) = (vec![], vec![], 0);
    while i < s.len() {
        match sep_at(s, i) { Some(e) => { segs.push(std::mem::take(&mut cur)); i = e } None => { cur.push(s[i]); i += 1 } }
    }
    segs.push(cur);
    segs
}

/// os.path.realpath (non-strict): symlinks resolved as far as the path exists, the rest kept, `..` taken physically.
#[cfg(not(windows))]
pub fn realpath(p: &Path) -> PathBuf {
    use std::path::Component;
    fn walk(mut path: PathBuf, rest: &Path, seen: &mut HashMap<PathBuf, Option<PathBuf>>) -> (PathBuf, bool) {
        let comps: Vec<Component> = rest.components().collect();
        for (n, c) in comps.iter().enumerate() {
            let tail = || comps[n + 1..].iter().collect::<PathBuf>();
            match c {
                Component::RootDir => path = PathBuf::from("/"),
                Component::CurDir | Component::Prefix(_) => {}
                Component::ParentDir => { path.pop(); }
                Component::Normal(name) => {
                    let new = path.join(name);
                    if !std::fs::symlink_metadata(&new).map(|m| m.file_type().is_symlink()).unwrap_or(false) { path = new; continue }
                    match seen.get(&new) {
                        Some(Some(r)) => { path = r.clone(); continue }
                        Some(None) => return (new.join(tail()), false),   // a symlink loop
                        None => {}
                    }
                    seen.insert(new.clone(), None);
                    let Ok(target) = std::fs::read_link(&new) else { path = new; continue };
                    let (p, ok) = walk(path, &target, seen);
                    if !ok { return (p.join(tail()), false) }
                    path = p;
                    seen.insert(new, Some(path.clone()));
                }
            }
        }
        (path, true)
    }
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    walk(PathBuf::from("/"), &abs, &mut HashMap::new()).0
}

/// ntpath.realpath (non-strict): the longest existing part as the OS spells it, the rest appended.
#[cfg(windows)]
pub fn realpath(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (mut base, mut tail) = (abs.as_path(), vec![]);
    loop {
        if let Ok(c) = std::fs::canonicalize(base) {
            let s = c.to_string_lossy().to_string();
            let mut r = PathBuf::from(match s.strip_prefix(r"\\?\") { Some(u) if u.starts_with(r"UNC\") => format!(r"\\{}", &u[4..]), Some(d) => d.to_string(), None => s });
            r.extend(tail.iter().rev());
            return r;
        }
        match (base.parent(), base.file_name()) { (Some(up), Some(name)) => { tail.push(name); base = up } _ => return abs }
    }
}

/// The nearest ancestor of path that carries the marker, or None.
pub fn guarded_root(path: &Path) -> Option<PathBuf> {
    let mut p = realpath(path);
    loop {
        if p.join(".claude").join("root_only").is_file() { return Some(p) }
        if !p.pop() { return None }
    }
}

/// Reason to deny, or None. project is the session's root; cwd is where a relative path resolves.
pub fn verdict(tool: &str, input: &Value, project: &str, cwd: &str) -> Option<String> {
    let project = if project.is_empty() { None } else { Some(realpath(Path::new(project))) };
    let cwd = Path::new(cwd);
    let targets = if tool == "Bash" {
        bash_targets(input.get("command").and_then(Value::as_str).unwrap_or(""), cwd)
    } else {
        ["file_path", "notebook_path", "path"].iter().find_map(|k| input.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(|fp| vec![cwd.join(fp)]).unwrap_or_default()   // join keeps an absolute fp as it is
    };
    for t in targets {
        let Some(root) = guarded_root(&t) else { continue };
        if Some(&root) != project.as_ref() {
            let (root, project) = (root.display(), project.as_ref().map_or("an unknown directory".to_string(), |p| p.display().to_string()));
            return Some(format!("{root} is marked root-only (.claude/root_only). This session is rooted at {project}, \
                so that repo's CLAUDE.md and hooks are not loaded and its rules would be skipped. Do not work around this. \
                Tell the user to start Claude inside {root} (cd there, then run claude) and make the change from that session."));
        }
    }
    None
}

/// What the hook prints to deny, byte for byte as Python's json.dumps (", " and ": ", non-ASCII as \u escapes).
pub fn deny_json(why: &str) -> String {
    let mut reason = String::new();
    for c in serde_json::to_string(why).unwrap().chars() {
        if (c as u32) < 0x7f { reason.push(c) } else { for u in c.encode_utf16(&mut [0; 2]) { reason += &format!("\\u{u:04x}") } }
    }
    format!("{{\"hookSpecificOutput\": {{\"hookEventName\": \"PreToolUse\", \"permissionDecision\": \"deny\", \"permissionDecisionReason\": {reason}}}}}")
}

/// The command settings.json runs: this binary, quoted for the shell Claude Code runs hooks in. On Windows the path
/// gets forward slashes, which both Git Bash and cmd accept and neither needs to escape.
pub fn hook_command(exe: &str) -> String {
    let quoted = if cfg!(windows) { exe.replace('\\', "/") } else { exe.chars().flat_map(|c| { let e = matches!(c, '"' | '\\' | '$' | '`'); e.then_some('\\').into_iter().chain([c]) }).collect() };
    format!("\"{quoted}\"{HOOK_ARGS}")
}

/// The program a guard command runs, if the command is a guard: the Python copy or a path of this binary.
fn guard_program(cmd: &str, home: &Path) -> Option<PathBuf> {
    if cmd == PY_COMMAND { return Some(home.join(".claude").join("hooks").join("root_only_guard.py")) }
    let p = cmd.strip_suffix(HOOK_ARGS)?;
    let p = match p.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
        Some(q) if !cfg!(windows) => { let mut out = String::new(); let mut it = q.chars(); while let Some(c) = it.next() { out.push(if c == '\\' { it.next().unwrap_or(c) } else { c }) } out }
        Some(q) => q.to_string(),
        None => p.to_string(),
    };
    Some(PathBuf::from(p))
}

/// JSON kept in file order, so rewriting settings.json moves nothing (serde_json's own map sorts keys).
#[derive(Debug, Clone, PartialEq)]
enum J { Null, Bool(bool), Num(serde_json::Number), Str(String), Arr(Vec<J>), Obj(Vec<(String, J)>) }

impl<'de> Deserialize<'de> for J {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<J, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = J;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("JSON") }
            fn visit_unit<E>(self) -> Result<J, E> { Ok(J::Null) }
            fn visit_bool<E>(self, b: bool) -> Result<J, E> { Ok(J::Bool(b)) }
            fn visit_i64<E>(self, n: i64) -> Result<J, E> { Ok(J::Num(n.into())) }
            fn visit_u64<E>(self, n: u64) -> Result<J, E> { Ok(J::Num(n.into())) }
            fn visit_f64<E>(self, n: f64) -> Result<J, E> { Ok(serde_json::Number::from_f64(n).map_or(J::Null, J::Num)) }
            fn visit_str<E>(self, s: &str) -> Result<J, E> { Ok(J::Str(s.into())) }
            fn visit_string<E>(self, s: String) -> Result<J, E> { Ok(J::Str(s)) }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<J, A::Error> {
                let mut v = vec![];
                while let Some(x) = a.next_element()? { v.push(x) }
                Ok(J::Arr(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<J, A::Error> {
                let mut v: Vec<(String, J)> = vec![];
                while let Some((k, x)) = a.next_entry::<String, J>()? {
                    match v.iter_mut().find(|(e, _)| *e == k) { Some(e) => e.1 = x, None => v.push((k, x)) }   // a dict: last value, first place
                }
                Ok(J::Obj(v))
            }
        }
        d.deserialize_any(V)
    }
}

impl Serialize for J {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            J::Null => s.serialize_unit(),
            J::Bool(b) => s.serialize_bool(*b),
            J::Num(n) => n.serialize(s),
            J::Str(t) => s.serialize_str(t),
            J::Arr(v) => v.serialize(s),
            J::Obj(v) => { let mut m = s.serialize_map(Some(v.len()))?; for (k, x) in v { m.serialize_entry(k, x)? } m.end() }
        }
    }
}

impl J {
    fn get(&self, k: &str) -> Option<&J> { if let J::Obj(v) = self { v.iter().find(|(e, _)| e == k).map(|e| &e.1) } else { None } }
    /// dict.setdefault(k, default) on an object.
    fn setdefault<'a>(v: &'a mut Vec<(String, J)>, k: &str, default: J) -> &'a mut J {
        let i = v.iter().position(|(e, _)| e == k).unwrap_or_else(|| { v.push((k.into(), default)); v.len() - 1 });
        &mut v[i].1
    }
    /// The commands of a PreToolUse entry's hooks.
    fn commands(&self) -> impl Iterator<Item = &str> {
        let hooks = match self.get("hooks") { Some(J::Arr(h)) => h.as_slice(), _ => &[] };
        hooks.iter().filter_map(|h| match h.get("command") { Some(J::Str(c)) => Some(c.as_str()), _ => None })
    }
}

/// settings.json (None: there is none) with every guard entry taken out and, when on, this one added; written as
/// Python's json.dump(indent=2, ensure_ascii=False) writes it. Err leaves the file alone.
pub fn rewrite_settings(text: Option<&str>, own: &str, on: bool, home: &Path) -> Result<String, String> {
    let mut d = match text { Some(t) => serde_json::from_str::<J>(t).map_err(|e| format!("not valid JSON: {e}"))?, None => J::Obj(vec![]) };
    let J::Obj(top) = &mut d else { return Err("not a JSON object".into()) };
    let J::Obj(hooks) = J::setdefault(top, "hooks", J::Obj(vec![])) else { return Err("\"hooks\" is not an object".into()) };
    let J::Arr(pre) = J::setdefault(hooks, "PreToolUse", J::Arr(vec![])) else { return Err("\"hooks.PreToolUse\" is not a list".into()) };
    pre.retain(|e| !e.commands().any(|c| guard_program(c, home).is_some()));
    if on {
        let hook = J::Obj(vec![("type".into(), J::Str("command".into())), ("command".into(), J::Str(own.into()))]);
        pre.push(J::Obj(vec![("matcher".into(), J::Str(MATCHER.into())), ("hooks".into(), J::Arr(vec![hook]))]));
    }
    let pre_empty = pre.is_empty();
    if pre_empty { hooks.retain(|(k, _)| k != "PreToolUse") }
    if hooks.is_empty() { top.retain(|(k, _)| k != "hooks") }
    Ok(serde_json::to_string_pretty(&d).unwrap())
}

/// `on`: only this binary's own command is registered. `stale`: a guard is registered, but not (only) this one: the
/// Python command, or this binary at another path. `missing-copy`: a registered guard's program is not there (the
/// Python's copy, or a binary that moved or was removed). `off`: none.
pub fn status(text: Option<&str>, own: &str, home: &Path) -> &'static str {
    let d = text.and_then(|t| serde_json::from_str::<J>(t).ok()).unwrap_or(J::Null);
    let pre = match d.get("hooks").and_then(|h| h.get("PreToolUse")) { Some(J::Arr(p)) => p.as_slice(), _ => &[] };
    let guards: Vec<(&str, PathBuf)> = pre.iter().flat_map(J::commands).filter_map(|c| guard_program(c, home).map(|p| (c, p))).collect();
    if guards.is_empty() { "off" }
    else if guards.iter().any(|(_, p)| !p.is_file()) { "missing-copy" }
    else if guards.iter().all(|(c, _)| *c == own) { "on" }
    else { "stale" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The Python's --selftest, case for case.
    #[test]
    fn selftest() {
        let t = realpath(&std::env::temp_dir().join(format!("root_guard_test_{}", std::process::id())));
        let (vault, other) = (t.join("vault"), t.join("other"));
        std::fs::create_dir_all(vault.join(".claude")).unwrap();
        std::fs::create_dir_all(vault.join("L1")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(vault.join(".claude").join("root_only"), "").unwrap();
        let (v, o) = (vault.display().to_string(), other.display().to_string());
        let f = vault.join("L1").join("a.md").display().to_string();
        let ed = |p: &str| json!({ "file_path": p });
        let sh = |c: String| json!({ "command": c });
        assert!(verdict("Edit", &ed(&f), &v, &v).is_none());                           // rooted in the vault
        assert!(verdict("Edit", &ed(&f), &o, &o).is_some());                           // rooted elsewhere
        assert!(verdict("Write", &ed("../vault/L1/a.md"), &o, &o).is_some());          // relative path out of another root
        assert!(verdict("Edit", &ed(&f), &format!("{v}/L1"), &v).is_some());           // rooted in a subfolder
        assert!(verdict("Edit", &ed(&format!("{o}/x.py")), &v, &v).is_none());          // unmarked repos are nobody's business
        assert!(verdict("Bash", &sh(format!("cat {f}")), &o, &o).is_none());           // reading is fine
        assert!(verdict("Bash", &sh(format!("echo hi >> {f}")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("git -C {v} commit -am x")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("sed -i s/a/b/ {f}")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("echo hi >> {f}")), &v, &v).is_none());
        // write targets only: a path named in text is not a write
        assert!(verdict("Bash", &sh(format!("python3 - <<'EOF'\ns = s.replace('a', 'built in {v}/L1')\nopen(p, 'w').write(s)\nEOF")), &o, &o).is_none());
        assert!(verdict("Bash", &sh(format!("git commit -m 'trial at {f}' && git push")), &o, &o).is_none());
        assert!(verdict("Bash", &sh(format!("echo 'see {f}' > {o}/note.txt")), &o, &o).is_none());
        assert!(verdict("Bash", &sh(format!("python3 - <<'EOF'\nopen('{f}', 'w').write('x')\nEOF")), &o, &o).is_some());   // literal open-for-write
        assert!(verdict("Bash", &sh(format!("ls {v}/.repos && rm -rf {v}")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("cp /tmp/x {f}")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("cd {v} && echo hi > L1/a.md")), &o, &o).is_some());       // relative after cd
        assert!(verdict("Bash", &sh(format!("git --git-dir {v}/.repos/x.git --work-tree {v} add -f L1/a.md")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("python3 x.py 2>/dev/null > {o}/out.txt")), &o, &o).is_none());
        // a path kept in a variable is still that path
        assert!(verdict("Bash", &sh(format!("N={v}; cp /tmp/x $N/L1/a.md")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("export N={v} && cd $N && git pull")), &o, &o).is_some());
        assert!(verdict("Bash", &sh(format!("N={o}; cp /tmp/x ${{N}}/b.md")), &o, &o).is_none());
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn pieces() {
        let c = |s: &str| s.chars().collect::<Vec<char>>();
        assert_eq!(shlex_split(&c(r#"a 'b c' "d\"e\x" f\ g ''"#)), Some(vec!["a".into(), "b c".into(), r#"d"e\x"#.into(), "f g".into(), "".into()]));
        assert_eq!(shlex_split(&c("a 'b")), None);
        assert_eq!(segments(&c("a && b ;c | d\n e || f")).iter().map(|s| text(s)).collect::<Vec<_>>(), ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(scan(&c("x 2>&1 >a 2>> b =>c"), redirect_at).0, ["a", "b"]);
        assert_eq!(text(&scan(&c("a <<'EOF'\nbody\n  EOF\nb"), |s, i| heredoc_at(s, i).map(|e| ((), e))).1), "a \nb");
        assert_eq!(scan(&c("open('x', 'wb') Path(\"y\").write_text"), py_write_at).0, ["x", "y"]);
        let env = HashMap::from([("N".to_string(), "/v".to_string())]);
        assert_eq!(text(&expand(&c("$N/a ${N}b $NOPE_X"), &env)), "/v/a /vb $NOPE_X");
    }

    #[test]
    fn settings() {
        let home = Path::new("/nonexistent-home");
        let own = hook_command("/opt/a b/aiwalk-setup");
        assert_eq!(own, "\"/opt/a b/aiwalk-setup\" hook root-only-guard");
        let before = r#"{"z": 1, "hooks": {"Stop": [], "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "python3 \"$HOME/.claude/hooks/root_only_guard.py\""}]}, {"matcher": "x", "hooks": [{"type": "command", "command": "other"}]}]}, "a": "中"}"#;
        let on = rewrite_settings(Some(before), &own, true, home).unwrap();
        assert!(on.starts_with("{\n  \"z\": 1,\n  \"hooks\": {\n    \"Stop\": [],") && on.ends_with("\"a\": \"中\"\n}"), "{on}");
        assert!(!on.contains("root_only_guard.py") && on.contains("\"other\"") && on.contains("hook root-only-guard"));
        let off = rewrite_settings(Some(&on), &own, false, home).unwrap();
        assert!(!off.contains("root-only-guard") && off.contains("\"other\""));
        assert_eq!(rewrite_settings(None, &own, false, home).unwrap(), "{}");
        assert_eq!(status(None, &own, home), "off");
        assert_eq!(status(Some(&on), &own, home), "missing-copy");   // /opt/a b/aiwalk-setup is not there
    }
}
