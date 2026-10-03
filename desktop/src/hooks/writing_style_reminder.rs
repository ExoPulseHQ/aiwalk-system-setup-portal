//! `aiwalk-setup hook writing_style_reminder`: the vault's `.claude/hooks/writing_style_reminder.py` (PreToolUse on
//! Edit/Write/MultiEdit and Bash), ported line for line.
//!
//! Two jobs. (1) Every NEW file in the vault is denied once: the first attempt to create it is refused with a reason
//! telling the agent to confirm the document type and parent log with the user; the same action again passes. The
//! attempts are remembered as empty marker files in `<gettempdir>/exopulse_newdoc_ack/<md5("session:key")>`, the
//! same names the Python uses, so a session denied by one implementation is let through by the other on the retry.
//! (2) Summary, Slide, Report and primary-log files get the writing-style reminder; a .md without a type suffix is
//! classified from the vault's `System/vault_rules.json`, as the Obsidian plugin does.
//!
//! Patterns are the Python's, verbatim, run through `py_re`, which makes Python's `\s` and `$` mean the same in
//! fancy-regex. Paths follow posixpath (ntpath approximated on Windows). Where the Python would raise (input JSON that
//! is not an object, a field of the wrong type, a malformed vault_rules.json), this exits 1 with a one-line message
//! instead of a traceback.

use super::Out;
use fancy_regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

const EMBEDDED: &str = include_str!("texts/writing_style_reminder.json");
const NAME: &str = "writing_style_reminder";
const SKIP_DIRS: [&str; 6] = [".git", ".claude", ".obsidian", "__pycache__", "node_modules", "graphify-out"];
const SKIP_TOP: [&str; 1] = ["scripts"];
const UNTYPED_SKIP_TOP: [&str; 3] = ["Papers", "Templates", "claude_logs"];
const RULES_PATH: &str = "System/vault_rules.json";

type R<T> = Result<T, String>;

/// Python's `\s` on str: Unicode White_Space plus the separators \x1c-\x1f.
fn py_space(c: char) -> bool { c.is_whitespace() || ('\x1c'..='\x1f').contains(&c) }

/// A Python `re` pattern as fancy-regex: `\s`/`\S` gain \x1c-\x1f, a bare `$` also matches before a final newline,
/// `\Z` is `\z`. `\w` stays the regex crate's (it differs from Python only on combining marks and No/Nl numerals).
fn py_re(p: &str) -> R<Regex> {
    let (mut out, mut class, mut it) = (String::new(), false, p.chars().peekable());
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('s') if class => out += r"\s\x1c-\x1f",
                Some('s') => out += r"[\s\x1c-\x1f]",
                Some('S') if !class => out += r"[^\s\x1c-\x1f]",
                Some('Z') if !class => out += r"\z",
                Some(n) => { out.push('\\'); out.push(n) }
                None => out.push('\\'),
            },
            '[' if !class => {
                class = true;
                out.push('[');
                if it.peek() == Some(&'^') { out.push(it.next().unwrap()) }
                if it.peek() == Some(&']') { out += r"\]"; it.next(); }   // a leading ] is literal in Python
            }
            ']' if class => { class = false; out.push(']') }
            '$' if !class => out += r"(?=\n?\z)",
            c => out.push(c),
        }
    }
    Regex::new(&out).map_err(|e| format!("re.error: {e}"))
}

/// Group i of a match, "" when it did not take part (as re.sub's \\2 does).
fn g<'t>(c: &fancy_regex::Captures<'t, str>, i: usize) -> &'t str { c.get(i).map_or("", |m| m.as_str()) }

fn re(p: &str) -> Regex { py_re(p).expect("a constant pattern compiles") }

/// The texts with `{name}` placeholders filled in one pass (a value is never scanned again).
fn fill(t: &str, vars: &[(&str, &str)]) -> String {
    let (mut out, mut rest) = (String::new(), t);
    while let Some(i) = rest.find('{') {
        out += &rest[..i];
        rest = &rest[i..];
        match vars.iter().find(|(k, _)| rest[1..].starts_with(k) && rest[1 + k.len()..].starts_with('}')) {
            Some((k, v)) => { out += v; rest = &rest[k.len() + 2..] }
            None => { out.push('{'); rest = &rest[1..] }
        }
    }
    out + rest
}

/// json.dumps of a str: ", " and ": " are the caller's, non-ASCII as \u escapes.
fn py_json(s: &str) -> String {
    let mut out = String::new();
    for c in serde_json::to_string(s).unwrap().chars() {
        if (c as u32) < 0x7f { out.push(c) } else { for u in c.encode_utf16(&mut [0; 2]) { out += &format!("\\u{u:04x}") } }
    }
    out
}

/// os.path, as far as this hook uses it.
#[cfg(not(windows))]
mod pypath {
    pub const SEP: &str = "/";
    pub fn join(a: &str, b: &str) -> String {
        if b.starts_with('/') { b.into() } else if a.is_empty() || a.ends_with('/') { format!("{a}{b}") } else { format!("{a}/{b}") }
    }
    pub fn basename(p: &str) -> &str { p.rsplit('/').next().unwrap_or(p) }
    pub fn normpath(p: &str) -> String {
        if p.is_empty() { return ".".into() }
        let slashes = if p.starts_with("//") && !p.starts_with("///") { 2 } else if p.starts_with('/') { 1 } else { 0 };
        let mut new: Vec<&str> = vec![];
        for c in p.split('/') {
            if c.is_empty() || c == "." { continue }
            if c != ".." || (slashes == 0 && new.is_empty()) || new.last() == Some(&"..") { new.push(c) } else if !new.is_empty() { new.pop(); }
        }
        let out = "/".repeat(slashes) + &new.join("/");
        if out.is_empty() { ".".into() } else { out }
    }
    pub fn abspath(p: &str) -> String {
        if p.starts_with('/') { normpath(p) } else { normpath(&join(&super::getcwd(), p)) }
    }
}

/// ntpath, approximated with the OS: GetFullPathNameW is what ntpath.abspath calls too. Not verified against Python.
#[cfg(windows)]
mod pypath {
    pub const SEP: &str = "\\";
    pub fn join(a: &str, b: &str) -> String { std::path::Path::new(a).join(b).to_string_lossy().into() }
    pub fn basename(p: &str) -> &str {
        let p = if p.len() >= 2 && p.as_bytes()[1] == b':' { &p[2..] } else { p };
        p.rsplit(['/', '\\']).next().unwrap_or(p)
    }
    pub fn abspath(p: &str) -> String {
        if p.is_empty() { return super::getcwd() }
        std::path::absolute(p).map(|a| a.to_string_lossy().into()).unwrap_or_else(|_| p.into())
    }
}

use pypath::{abspath, basename, join, SEP};

fn getcwd() -> String { std::env::current_dir().map(|p| p.to_string_lossy().into()).unwrap_or_default() }
fn exists(p: &str) -> bool { std::path::Path::new(p).exists() }

/// os.path.expanduser for `~` and `~/...` from HOME (`~user` is left as it is).
fn expanduser(p: &str) -> String {
    let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let Some(rest) = p.strip_prefix('~') else { return p.to_string() };
    if !(rest.is_empty() || rest.starts_with(sep)) { return p.to_string() }
    #[allow(deprecated)] // home_dir is correct on Windows since Rust 1.85
    let Some(home) = std::env::home_dir() else { return p.to_string() };
    let out = format!("{}{rest}", home.to_string_lossy().trim_end_matches(sep));
    if out.is_empty() { "/".into() } else { out }
}

/// shlex.split (POSIX, no comments); None where it raises ValueError. Same as exo_core::root_guard's private one.
fn shlex_split(s: &str) -> Option<Vec<String>> {
    let s: Vec<char> = s.chars().collect();
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
                ('\\', None) => return None,
                ('\\', Some(c)) => { if esc != 'a' && c != '\\' && c != esc { tok.push('\\') } tok.push(c); state = esc }
                (_, None) => return None,
                (q, Some(c)) => { quoted = true; if c == q { state = 'a' } else if c == '\\' && q == '"' { esc = '"'; state = '\\' } else { tok.push(c) } }
            }
        };
        if !tok.is_empty() || quoted { toks.push(tok) }
        if eof { return Some(toks) }
    }
}

/// tempfile.gettempdir(): the first of TMPDIR, TEMP, TMP, the platform's folders and the cwd that takes a file.
fn gettempdir() -> R<String> {
    let mut dirs: Vec<String> = ["TMPDIR", "TEMP", "TMP"].iter().filter_map(|e| std::env::var_os(e)).map(|v| v.to_string_lossy().into_owned()).filter(|v| !v.is_empty()).collect();
    if cfg!(windows) {
        dirs.push(expanduser(r"~\AppData\Local\Temp"));
        dirs.push(std::env::var("SYSTEMROOT").map(|r| format!(r"{r}\Temp")).unwrap_or_else(|_| r"%SYSTEMROOT%\Temp".into()));
        dirs.extend([r"c:\temp", r"c:\tmp", r"\temp", r"\tmp"].map(String::from));
    } else {
        dirs.extend(["/tmp", "/var/tmp", "/usr/tmp"].map(String::from));
    }
    dirs.push(std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| ".".into()));
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for d in dirs {
        let d = if d == "." { d } else { abspath(&d) };
        let probe = join(&d, &format!("tmp{}_{nonce}", std::process::id()));
        let ok = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).and_then(|mut f| std::io::Write::write_all(&mut f, b"blat"));
        if ok.is_ok() { let _ = std::fs::remove_file(&probe); return Ok(d) }
    }
    Err("FileNotFoundError: No usable temporary directory found".into())
}

/// hashlib.md5(...).hexdigest(); ring has no MD5, and this is the only digest the state files need.
fn md5_hex(data: &[u8]) -> String {
    const S: [[u32; 4]; 4] = [[7, 12, 17, 22], [5, 9, 14, 20], [4, 11, 16, 23], [6, 10, 15, 21]];
    let k: Vec<u32> = (1..=64).map(|i| ((i as f64).sin().abs() * 4294967296.0) as u32).collect();
    let mut h = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0) }
    msg.extend((data.len() as u64).wrapping_mul(8).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            (a, d, c) = (d, c, b);
            b = b.wrapping_add(f.rotate_left(S[i / 16][i % 4]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) { *x = x.wrapping_add(y) }
    }
    h.iter().flat_map(|x| x.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

/// The marker file for (session, key) under the temp folder `tmp`.
fn state_file(tmp: &str, session: &str, key: &str) -> String {
    join(&join(tmp, "exopulse_newdoc_ack"), &md5_hex(format!("{session}:{key}").as_bytes()))
}

/// True the first time this session tries this creation; leaves the marker so the retry passes.
fn first_attempt(key: &str, session: &str) -> R<bool> {
    let tmp = gettempdir()?;
    std::fs::create_dir_all(join(&tmp, "exopulse_newdoc_ack")).map_err(|e| format!("OSError: {e}"))?;
    let m = state_file(&tmp, session, key);
    if exists(&m) { return Ok(false) }
    std::fs::File::create(&m).map_err(|e| format!("OSError: {e}"))?;
    Ok(true)
}

/// Same parse as the plugin's engine.ts frontmatter(): flat `key: value` lines, the last of a key winning.
fn frontmatter(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(Some(c)) = re(r"^---\n([\s\S]*?)\n---(?:\n|$)").captures(text) else { return out };
    let line_re = re(r"^([A-Za-z_][\w-]*):\s*(.*)$");
    for line in g(&c, 1).split('\n') {
        if let Ok(Some(k)) = line_re.captures(line) {
            let v = g(&k, 2).trim_matches(py_space);
            let v = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { &v[1..v.len() - 1] } else { v };
            out.insert(g(&k, 1).to_string(), v.to_string());
        }
    }
    out
}

/// `v[key]`, or the KeyError/TypeError the Python would raise.
fn at<'a>(v: &'a Value, key: &str) -> R<&'a Value> { v.get(key).ok_or_else(|| format!("KeyError: '{key}'")) }
fn as_str<'a>(v: &'a Value) -> R<&'a str> { v.as_str().ok_or_else(|| "TypeError: expected str in vault_rules.json".into()) }
fn as_arr(v: &Value) -> R<&Vec<Value>> { v.as_array().ok_or_else(|| "TypeError: expected list in vault_rules.json".into()) }

/// Python truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `a or b or ""` where the result must be a str.
fn or_str(vals: &[Option<&Value>]) -> R<String> {
    match vals.iter().find(|v| truthy(**v)) {
        None => Ok(String::new()),
        Some(Some(Value::String(s))) => Ok(s.clone()),
        Some(_) => Err("TypeError: expected str".into()),
    }
}

/// Port of the plugin's engine.ts detect(): (type id or None, guessed).
fn detect_type(rules: Option<&Value>, path: &str, text: &str) -> R<(Option<String>, bool)> {
    let Some(types) = rules.and_then(|r| r.get("types")) else { return Ok((None, false)) };
    let types = as_arr(types)?;
    let base = basename(path);
    let fm = if path.ends_with(".md") { frontmatter(text) } else { HashMap::new() };
    let mut cands = vec![];
    for t in types {
        if py_re(as_str(at(at(t, "match")?, "filename")?)?)?.is_match(base).map_err(|e| e.to_string())? { cands.push(t) }
    }
    fn wanted(t: &Value) -> Option<&Value> { t["match"].get("frontmatter").filter(|f| truthy(Some(f))) }
    let fm_ok = |f: &Value| f.as_object().is_some_and(|o| o.iter().all(|(k, v)| fm.get(k).is_some_and(|x| v.as_str() == Some(x))));
    let hit = cands.iter().find(|t| wanted(t).is_some_and(fm_ok)).or_else(|| cands.iter().find(|t| wanted(t).is_none())).or(cands.first());
    if let Some(t) = hit { return Ok((Some(as_str(at(t, "id")?)?.to_string()), false)) }
    for t in types {
        let Some(g) = t.get("guess").filter(|g| truthy(Some(g))) else { continue };
        if path.ends_with(as_str(at(g, "extension")?)?) {
            let mut all = true;
            for k in as_arr(at(g, "frontmatter_has")?)? { all &= fm.contains_key(as_str(k)?) }
            if all { return Ok((Some(as_str(at(t, "id")?)?.to_string()), true)) }
        }
    }
    Ok((None, false))
}

/// Port of the plugin's engine.ts isSystemDoc(): registered guides and templates carry no type suffix by design.
fn is_system_doc(rules: Option<&Value>, rel: &str) -> R<bool> {
    let Some(r) = rules else { return Ok(false) };
    let mut reg: Vec<&str> = vec!["CLAUDE.md", "llms.txt", RULES_PATH];
    if let Some(g) = r.get("guides") { for g in as_arr(g)? { reg.push(as_str(at(g, "path")?)?) } }
    for t in as_arr(at(r, "types")?)? {
        let tpl = at(t, "create")?.get("template");
        if truthy(tpl) { reg.push(as_str(tpl.unwrap())?) }
    }
    Ok(reg.iter().any(|p| rel == *p || p.strip_suffix("_README.md").is_some_and(|d| rel.starts_with(&format!("{d}/")))))
}

fn vault_root() -> Option<String> { super::project_dir().map(|p| abspath(&p.to_string_lossy())) }

/// The part of `ap` below the vault root, split on the separator; None outside the vault.
fn vault_parts(ap: &str, root: &str) -> Option<Vec<String>> {
    ap.strip_prefix(&format!("{root}{SEP}")).map(|rel| rel.split(SEP).map(String::from).collect())
}

/// A vault file being created: inside the vault, not a scratch/dot dir, not yet on disk.
fn is_new_doc(fp: &str, cwd: &str, root: Option<&str>) -> bool {
    if fp.is_empty() || fp.starts_with("/tmp/") || fp.contains("/scratchpad/") { return false }
    let Some(root) = root else { return false };
    let ap = abspath(&join(cwd, &expanduser(fp)));
    let Some(parts) = vault_parts(&ap, root) else { return false };
    if SKIP_TOP.contains(&parts[0].as_str()) || parts.iter().any(|p| SKIP_DIRS.contains(&p.as_str())) { return false }
    !exists(&ap)
}

/// Paths a shell command would create. ponytail: regex + shlex per segment, as the Python; misses exotic forms.
fn bash_targets(cmd: &str, cwd: &str) -> Vec<String> {
    // a remote command creates files on the other machine; its heredoc is not ours to gate
    if re(r"^\s*(?:\w+=\S+\s+)*ssh\s").is_match(cmd).unwrap_or(false) { return vec![] }
    let mut out: Vec<String> = re(r#"open\(\s*['"]([^'"]+)['"]\s*,\s*['"][wax]"#).captures_iter(cmd).flatten().map(|c| g(&c, 1).to_string()).collect();
    // heredoc bodies and quoted strings are text, not shell; keep the tail of a heredoc's opener line
    let heredoc = re(r#"(?s)<<-?\s*['"]?(\w+)['"]?([^\n]*)\n.*?\n\1[ \t]*(?=\n|$)"#);
    let shell = heredoc.try_replacen(cmd, 0, |c: &fancy_regex::Captures<str>| g(c, 2).to_string()).map(|s| s.into_owned()).unwrap_or_else(|_| cmd.to_string());
    let shell = re(r#"'[^']*'|"[^"]*""#).replace_all(&shell, "").into_owned();
    out.extend(re(r#"(?<![<>=])>>?\s*([^\s"'&|;)]+)"#).captures_iter(shell.as_str()).flatten().map(|c| g(&c, 1).to_string()));
    // re.split(r"&&|\|\||;|\|"): a lone & is not a separator
    let (mut segs, mut rest) = (vec![], shell.as_str());
    loop {
        let next = rest.char_indices().find_map(|(i, c)| match (c, &rest[i..]) {
            (_, r) if r.starts_with("&&") || r.starts_with("||") => Some((i, 2)),
            (';' | '|', _) => Some((i, 1)),
            _ => None,
        });
        let Some((i, n)) = next else { segs.push(rest); break };
        segs.push(&rest[..i]);
        rest = &rest[i + n..];
    }
    for seg in segs {
        let Some(argv) = shlex_split(seg.trim_matches(py_space)) else { continue };
        let argv: Vec<String> = argv.into_iter().filter(|a| !a.starts_with('-')).collect();
        let Some(first) = argv.first() else { continue };
        match basename(first) {
            "tee" if argv.len() > 1 => out.extend(argv[1..].iter().cloned()),
            "cp" | "mv" | "install" if argv.len() > 2 => {
                let dst = &argv[argv.len() - 1];
                if std::path::Path::new(&join(cwd, &expanduser(dst))).is_dir() {
                    out.extend(argv[1..argv.len() - 1].iter().map(|s| join(dst, basename(s))));
                } else {
                    out.push(dst.clone());
                }
            }
            "touch" => out.extend(argv[1..].iter().cloned()),
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.into_iter().filter(|t| seen.insert(t.clone()))
        .filter(|t| t != "/dev/null" && !t.contains('$') && !t.contains(['(', ')', '{', '}', '[', ']', ',', '='])).collect()
}

/// Reminder for a vault .md the filename patterns do not catch: the text and whether it guessed a Summary.
fn type_reminder(texts: &Value, fp: &str, cwd: &str, tool: &str, ti: &Value, session: &str) -> R<Option<(String, bool)>> {
    let Some(root) = vault_root() else { return Ok(None) };
    if !fp.ends_with(".md") { return Ok(None) }
    let ap = abspath(&join(cwd, &expanduser(fp)));
    let Some(parts) = vault_parts(&ap, &root) else { return Ok(None) };
    if parts.len() < 2 || SKIP_TOP.iter().chain(&UNTYPED_SKIP_TOP).any(|s| *s == parts[0]) || parts.iter().any(|p| SKIP_DIRS.contains(&p.as_str())) {
        return Ok(None);
    }
    let text = if tool == "Write" && !exists(&ap) {
        or_str(&[ti.get("content")])?
    } else {
        // open(..., encoding="utf-8") reads with universal newlines
        match std::fs::read_to_string(&ap) { Ok(t) => t.replace("\r\n", "\n").replace('\r', "\n"), Err(_) => return Ok(None) }
    };
    let rules: Option<Value> = std::fs::read_to_string(join(&root, &RULES_PATH.replace('/', SEP))).ok().and_then(|t| serde_json::from_str(&t).ok());
    if is_system_doc(rules.as_ref(), &parts.join("/"))? { return Ok(None) }
    let (tid, guessed) = detect_type(rules.as_ref(), &ap, &text)?;
    let base = basename(&ap);
    match tid {
        // once per file per session: the first edit is where the decision belongs
        None => Ok(first_attempt(&format!("untyped:{ap}"), session)?.then(|| (fill(&super::text(texts, "untyped_reminder"), &[("file", base)]), false))),
        // the Python tests its own wording for "猜它是 summary"; this is the same test without reading the wording
        Some(t) if guessed => Ok(Some((fill(&super::text(texts, "guessed_reminder"), &[("file", base), ("type", &t)]), t.starts_with("summary")))),
        Some(_) => Ok(None),
    }
}

/// str(session) as Python prints it.
fn py_str(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "None".into(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.into(),
        Some(v) => v.to_string(),
    }
}

fn hook(stdin: &[u8]) -> R<String> {
    let Ok(data) = serde_json::from_slice::<Value>(stdin) else { return Ok(String::new()) };   // never break the tool call
    if !data.is_object() { return Err("AttributeError: input is not a JSON object".into()) }
    let ti = data.get("tool_input").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Object(Default::default()));
    if !ti.is_object() { return Err("AttributeError: tool_input is not a JSON object".into()) }
    let tool = data.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let session = py_str(data.get("session_id"));
    let mut cwd = or_str(&[data.get("cwd")])?;
    if cwd.is_empty() { cwd = getcwd() }
    let texts = super::texts(NAME, EMBEDDED);
    let root = vault_root();

    // Bash creating vault files: same gate as Write, keyed on the exact command.
    if tool == "Bash" {
        let cmd = or_str(&[ti.get("command")])?;
        // relative targets resolve against a single leading cd; with a second cd we fall back to the session cwd
        let cds = re(r"(?:^|&&|;|\|\|)\s*cd\s").find_iter(cmd.as_str()).flatten().count();
        if cds == 1 {
            if let Ok(Some(m)) = re(r#"^\s*cd\s+("[^"]+"|'[^']+'|[^\s;&|]+)\s*(?:&&|;)"#).captures(cmd.as_str()) {
                cwd = abspath(&join(&cwd, &expanduser(g(&m, 1).trim_matches(['"', '\'']))));
            }
        }
        let new: Vec<String> = bash_targets(&cmd, &cwd).into_iter().filter(|t| is_new_doc(t, &cwd, root.as_deref())).collect();
        if !new.is_empty() && first_attempt(&cmd, &session)? {
            let names: Vec<&str> = new.iter().map(|t| basename(t)).collect();
            return Ok(deny(&texts, &names));
        }
        return Ok(String::new());
    }

    let fp = or_str(&[ti.get("file_path"), ti.get("path")])?;
    let base = basename(&fp);
    let mut parts = vec![];

    // Creating a file: Edit/MultiEdit need an existing file, so only Write creates.
    if tool == "Write" && is_new_doc(&fp, &cwd, root.as_deref()) {
        if first_attempt(&abspath(&join(&cwd, &fp)), &session)? { return Ok(deny(&texts, &[base])) }
        parts.push(fill(&super::text(&texts, "new_doc_reminder"), &[("file", base)]));
    }
    let reminder = || fill(&super::text(&texts, "reminder"), &[("file", base)]);
    if re(r"_Report\.(md|html)$").is_match(base).unwrap_or(false) {
        parts.push(fill(&super::text(&texts, "report_reminder"), &[("file", base)]));
    } else if [r"_Summary\.md$", r"_Slide\.(md|html)$", r"^_.+\.md$"].iter().any(|p| re(p).is_match(base).unwrap_or(false)) {
        parts.push(reminder());
    } else if let Some((extra, summary)) = type_reminder(&texts, &fp, &cwd, tool, &ti, &session)? {
        parts.push(extra);
        if summary { parts.push(reminder()) }
    }
    if parts.is_empty() { return Ok(String::new()) }
    Ok(format!("{{\"hookSpecificOutput\": {{\"hookEventName\": \"PreToolUse\", \"additionalContext\": {}}}}}\n", py_json(&parts.join("\n\n"))))
}

fn deny(texts: &Value, names: &[&str]) -> String {
    exo_core::root_guard::deny_json(&fill(&super::text(texts, "new_doc_deny"), &[("files", &names.join(", "))])) + "\n"
}

pub fn run(stdin: &[u8]) -> Out {
    match hook(stdin) {
        Ok(stdout) => Out { stdout, ..Out::default() },
        Err(e) => Out { stderr: format!("{NAME}: {e}\n"), code: 1, ..Out::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_hashlib() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"The quick brown fox jumps over the lazy dog"), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(md5_hex(&[b'a'; 1000]), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    #[cfg(not(windows))]
    fn state_file_name() {
        assert_eq!(state_file("/t", "sess", "/v/a.md"), format!("/t/exopulse_newdoc_ack/{}", md5_hex(b"sess:/v/a.md")));
        assert_eq!(state_file("/t/", "", "x"), format!("/t/exopulse_newdoc_ack/{}", md5_hex(b":x")));
    }

    #[test]
    #[cfg(not(windows))]
    fn paths_as_posixpath() {
        assert_eq!(normpath_t("/a/./b/../c//d/"), "/a/c/d");
        assert_eq!(normpath_t("//a/../b"), "//b");
        assert_eq!(normpath_t("///a"), "/a");
        assert_eq!(normpath_t("../x/.."), "..");
        assert_eq!(join("/a", "/b"), "/b");
        assert_eq!(basename("a/b/"), "");
    }
    #[cfg(not(windows))]
    fn normpath_t(p: &str) -> String { pypath::normpath(p) }

    #[test]
    fn python_regex_dialect() {
        assert!(py_re(r"_Summary\.md$").unwrap().is_match("x_Summary.md\n").unwrap());   // Python's $ before a final \n
        assert!(!py_re(r"_Summary\.md$").unwrap().is_match("x_Summary.md\n\n").unwrap());
        assert!(py_re(r"^a\sb$").unwrap().is_match("a\x1fb").unwrap());
        assert!(py_re(r"[\s]").unwrap().is_match("\x1c").unwrap());
        assert!(py_re(r"[$]").unwrap().is_match("$").unwrap());
    }

    #[test]
    fn frontmatter_like_engine() {
        let fm = frontmatter("---\nparent_log: \"[[_X]]\"\ntier:   primary  \nbad line\n2x: y\n---\nbody");
        assert_eq!(fm.get("parent_log").map(String::as_str), Some("[[_X]]"));
        assert_eq!(fm.get("tier").map(String::as_str), Some("primary"));
        assert_eq!(fm.len(), 2);
        assert!(frontmatter("--- \nx: 1\n---").is_empty());
    }

    #[test]
    fn classification() {
        let rules: Value = serde_json::json!({"types": [
            {"id": "primary", "match": {"filename": "^_.+\\.md$", "frontmatter": {"tier": "primary"}}, "create": {"template": "Templates/P.md"}},
            {"id": "summary", "match": {"filename": "_Summary\\.md$"}, "guess": {"frontmatter_has": ["parent_log"], "extension": ".md"}, "create": {}},
            {"id": "readme", "match": {"filename": "_README\\.md$"}, "create": {}}],
            "guides": [{"path": "System/G.md"}, {"path": "Templates_README.md"}]});
        let r = Some(&rules);
        assert_eq!(detect_type(r, "/v/L/_Log.md", "").unwrap(), (Some("primary".into()), false));
        assert_eq!(detect_type(r, "/v/L/notes.md", "---\nparent_log: x\n---\n").unwrap(), (Some("summary".into()), true));
        assert_eq!(detect_type(r, "/v/L/notes.md", "no frontmatter").unwrap(), (None, false));
        assert_eq!(detect_type(None, "/v/L/a_Summary.md", "").unwrap(), (None, false));
        assert!(is_system_doc(r, "System/G.md").unwrap());
        assert!(is_system_doc(r, "Templates/anything.md").unwrap());
        assert!(is_system_doc(r, "Templates/P.md").unwrap());
        assert!(!is_system_doc(r, "L1/G.md").unwrap());
    }

    #[test]
    fn bash_target_extraction() {
        let t = |c: &str| bash_targets(c, "/nonexistent-cwd");
        assert_eq!(t("echo hi > a.md && cat b >> c.txt"), ["a.md", "c.txt"]);
        assert_eq!(t("touch x.md; tee -a y.md < z | cat"), ["x.md", "y.md", "<", "z"]);   // as the Python: < and z too
        assert_eq!(t("cp a b c.md"), ["c.md"]);
        assert_eq!(t("python3 - <<'EOF'\nopen('n.md','w').write('x > q')\nEOF"), ["n.md"]);
        assert_eq!(t("cat <<EOF > out.md\nbody > not\nEOF"), ["out.md"]);
        assert_eq!(t("echo 'a > b' \"c > d\" x=>y 2>&1"), Vec::<String>::new());
        assert_eq!(t("ssh host 'echo > x.md'"), Vec::<String>::new());
        assert_eq!(t("touch $S/x.py f(s /dev/null 中文.md"), ["中文.md"]);
        assert_eq!(t("sleep 1 & touch q.md"), Vec::<String>::new());   // a lone & does not split, as in the Python
    }

    #[test]
    fn fill_is_single_pass() {
        assert_eq!(fill("`{file}` {1,8} {type}", &[("file", "{type}"), ("type", "T")]), "`{type}` {1,8} T");
    }
}
