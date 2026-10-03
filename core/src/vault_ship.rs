//! The vault's upload tool, ported from scripts/vault_ship.py and scripts/sync_ownership.py: the parts that need no
//! git, disk or network. Each function keeps the Python's behaviour, regex quirks included, because the Python and
//! this port run side by side on the team's computers and must leave the same history and the same files.

use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

// ---------------------------------------------------------------- Python's string rules

/// `str.isspace()`, which is also what `\s` and `strip()` mean in Python.
pub fn py_space(c: char) -> bool { c.is_whitespace() || ('\x1c'..='\x1f').contains(&c) }

pub fn py_strip(s: &str) -> &str { s.trim_matches(py_space) }

/// `str.splitlines()`: every line break Python knows, no empty last line.
pub fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
            if c == '\r' && it.peek().map(|p| p.1) == Some('\n') { it.next(); start += 1; }
        }
    }
    if start < s.len() { out.push(&s[start..]) }
    out
}

/// What Python's text mode reads: "\r\n" and a lone "\r" become "\n".
pub fn py_text(s: &str) -> String { s.replace("\r\n", "\n").replace('\r', "\n") }

/// `\w` in a Python str pattern.
fn word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

/// `\b` at char index i.
fn boundary(c: &[char], i: usize) -> bool {
    let before = i > 0 && word(c[i - 1]);
    let after = i < c.len() && word(c[i]);
    before != after
}

/// `re.search(rf"\b{re.escape(pat)}\b", s)`
fn has_word(s: &[char], pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    !p.is_empty() && s.len() >= p.len() && (0..=s.len() - p.len()).any(|i| s[i..i + p.len()] == p[..] && boundary(s, i) && boundary(s, i + p.len()))
}

fn last_part(path: &str) -> &str { path.rsplit('/').next().unwrap_or(path) }

// ---------------------------------------------------------------- vault_ship.py

/// SECRET: a login token or key never ships. The last path part is .env or names a token, key or password.
pub fn looks_secret(path: &str) -> bool {
    let last = last_part(path).to_lowercase();
    last == ".env" || ["token", "credential", "secret", "password", "oauth.keys", "id_rsa", "id_ed25519"].iter().any(|k| last.contains(k))
}

/// PRIMARY: a primary log, `_Name.md`.
pub fn is_primary(path: &str) -> bool {
    let last = last_part(path);
    last.len() >= 5 && last.starts_with('_') && last.ends_with(".md")
}

/// `re.findall(r"\[\[([^\]|#]+)", text)`
fn wikilinks(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let (mut out, mut i) = (vec![], 0);
    while i + 2 < c.len() {
        if c[i] == '[' && c[i + 1] == '[' && !matches!(c[i + 2], ']' | '|' | '#') {
            let end = (i + 2..c.len()).find(|&j| matches!(c[j], ']' | '|' | '#')).unwrap_or(c.len());
            out.push(c[i + 2..end].iter().collect());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// link_targets: the names a note links to, outside code fences and code spans.
pub fn link_targets(text: &str) -> Vec<String> {
    // re.sub(r"```.*?```", "", text, flags=re.S)
    let mut s = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("```") {
        match rest[i + 3..].find("```") {
            Some(j) => { s.push_str(&rest[..i]); rest = &rest[i + 3 + j + 3..]; }
            None => break,
        }
    }
    s.push_str(rest);
    // re.sub(r"`[^`\n]*`", "", text)
    let c: Vec<char> = s.chars().collect();
    let (mut t, mut i) = (String::new(), 0);
    while i < c.len() {
        if c[i] == '`' {
            if let Some(j) = (i + 1..c.len()).find(|&j| c[j] == '`' || c[j] == '\n').filter(|&j| c[j] == '`') { i = j + 1; continue }
        }
        t.push(c[i]);
        i += 1;
    }
    wikilinks(&t).iter().map(|l| last_part(py_strip(l).trim_end_matches('\\')).to_string()).collect()
}

/// name_key before hashing: a note's name without .md, any other file's full name, lower case.
/// `windows`: os.path.basename there also splits at a backslash.
pub fn name_key_text(name: &str, windows: bool) -> String {
    let base = if windows { name.rsplit(['/', '\\']).next().unwrap_or(name) } else { last_part(name) };
    let base = if base.to_lowercase().ends_with(".md") { &base[..base.len() - 3] } else { base };
    base.to_lowercase()
}

/// `git config -z -f .gitmodules --get-regexp ...path` -> the submodule paths, in file order.
pub fn submodule_paths(out: &str) -> Vec<String> {
    out.split('\0').filter_map(|e| e.split_once('\n').map(|(_, v)| v.to_string())).collect()
}

/// Paths inside a submodule go to that submodule (longest submodule path first); the rest stay in the vault, which
/// then also records each submodule's new commit. (groups in first-seen order, vault paths)
pub fn group_paths(paths: &[String], subs: &[String]) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    let mut subs: Vec<&String> = subs.iter().collect();
    subs.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    let (mut groups, mut rest): (Vec<(String, Vec<String>)>, Vec<String>) = (vec![], vec![]);
    for p in paths {
        match subs.iter().find(|s| p.starts_with(&format!("{s}/"))) {
            Some(s) => {
                let inner = p[s.len() + 1..].to_string();
                match groups.iter_mut().find(|(g, _)| g == *s) { Some(g) => g.1.push(inner), None => groups.push((s.to_string(), vec![inner])) }
            }
            None => rest.push(p.clone()),
        }
    }
    for (s, _) in &groups { if !rest.contains(s) { rest.push(s.clone()) } }
    (groups, rest)
}

/// Staged names that look like a token or key; deletions may go, and a submodule pointer (mode 160000) is a commit hash.
pub fn leaks(name_status: &[&str], ls_files_stage: &[&str]) -> Vec<String> {
    let links: HashSet<&str> = ls_files_stage.iter().filter(|l| l.starts_with("160000")).map(|l| l.rsplit('\t').next().unwrap()).collect();
    name_status.iter().filter(|l| !l.is_empty() && !l.starts_with('D')).map(|l| l.rsplit('\t').next().unwrap())
        .filter(|n| looks_secret(n) && !links.contains(n)).map(String::from).collect()
}

/// Only what was asked for: other sessions may have staged their own work in this tree.
pub fn wanted(staged: &[&str], paths: &[String]) -> Vec<String> {
    staged.iter().filter(|p| paths.iter().any(|w| **p == w || p.starts_with(&format!("{}/", w.trim_end_matches('/'))))).map(|p| p.to_string()).collect()
}

/// `github\.com[:/]([^/]+)/([^/]+?)(\.git)?$` -> (owner, repo)
pub fn github_repo(url: &str) -> Option<(String, String)> {
    let mut from = 0;
    while let Some(i) = url[from..].find("github.com") {
        let at = from + i + "github.com".len();
        if url[at..].starts_with([':', '/']) {
            if let Some((owner, repo)) = url[at + 1..].split_once('/') {
                if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') {
                    let repo = if repo.len() > 4 && repo.ends_with(".git") { &repo[..repo.len() - 4] } else { repo };
                    return Some((owner.into(), repo.into()));
                }
            }
        }
        from += i + 1;
    }
    None
}

pub const INDEX_ABOUT: &str = "Generated by scripts/vault_ship.py. Hashed file names per submodule, so a link into a folder this computer cannot see reads as No access, not as broken.";

/// Python's truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The index as `json.dump(new, ensure_ascii=False, indent=0, separators=(",", ":"))` writes it, plus "\n".
/// None when nothing changed (or there is nothing to write), as update_index returns False.
pub fn index_json(old: &Value, subs: &[(String, Value)]) -> Option<String> {
    let new = serde_json::json!({ "about": INDEX_ABOUT, "submodules": subs.iter().cloned().collect::<serde_json::Map<_, _>>() });
    if new == *old || (subs.is_empty() && !truthy(old)) { return None }
    let mut s = format!("{{\n\"about\":{},\n\"submodules\":", Value::from(INDEX_ABOUT));
    if subs.is_empty() { s.push_str("{}") } else {
        s.push_str("{\n");
        let items: Vec<String> = subs.iter().map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), dump0(v))).collect();
        s.push_str(&items.join(",\n"));
        s.push_str("\n}");
    }
    s.push_str("\n}\n");
    Some(s)
}

fn dump0(v: &Value) -> String {
    match v {
        Value::Array(a) if !a.is_empty() => format!("[\n{}\n]", a.iter().map(dump0).collect::<Vec<_>>().join(",\n")),
        Value::Object(o) if !o.is_empty() => format!("{{\n{}\n}}", o.iter().map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), dump0(v))).collect::<Vec<_>>().join(",\n")),
        _ => v.to_string(),
    }
}

pub fn conflict_message(submodule: Option<&str>, files: &str, branch: &str) -> String {
    let at = submodule.map(|s| format!(" in submodule {s}")).unwrap_or_default();
    format!("✗ your changes conflict with what others pushed{at}. Your commit is saved on this computer, not pushed.\nConflicting files:\n{files}\nMerge them with `git rebase origin/{branch}`, then run ship again with nothing staged to push it.")
}

pub fn author_message(an: &str, ae: &str, name: &str, email: &str) -> String {
    format!("✗ commit identity {an} <{ae}> is not the gh account {name} <{email}>;\n  commit through scripts/vault_ship.py, or: git -c user.name={name} -c user.email={email} commit …")
}

// ---------------------------------------------------------------- dates

/// Days since 1970-01-01 of a calendar date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// The calendar date of a day number.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// The local date from `git var GIT_COMMITTER_IDENT` ("name <email> 1759490000 +0800").
pub fn local_date(ident: &str) -> Option<(i64, i64, i64)> {
    let mut parts = ident.rsplitn(3, ' ');
    let tz = parts.next()?;
    let secs: i64 = parts.next()?.parse().ok()?;
    let sign = if tz.starts_with('-') { -1 } else { 1 };
    let (h, m): (i64, i64) = (tz.get(1..3)?.parse().ok()?, tz.get(3..5)?.parse().ok()?);
    Some(civil_from_days((secs + sign * (h * 3600 + m * 60)).div_euclid(86400)))
}

/// `\d` as the vault writes it: ASCII or full-width digits.
fn digit(c: char) -> Option<i64> {
    match c { '0'..='9' => Some(c as i64 - '0' as i64), '０'..='９' => Some(c as i64 - '０' as i64), _ => None }
}

/// sync_ownership.day(): a heading date's day number, None when it is not a real date (2026/09/31).
fn day(date: &str) -> Option<i64> {
    let n: Vec<i64> = date.split('/').map(|p| p.chars().try_fold(0, |a, c| digit(c).map(|d| a * 10 + d))).collect::<Option<_>>()?;
    let (y, m, d) = (n[0], n[1], n[2]);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let len = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    ((1..=12).contains(&m) && d >= 1 && d <= len[m as usize - 1] && y >= 1).then(|| days_from_civil(y, m, d))
}

// ---------------------------------------------------------------- sync_ownership.py

pub const TEAM: &str = "System/ExoPulse_Task_Assignment.md";
const A0: &str = "<!-- ownership:auto:start -->";
const A1: &str = "<!-- ownership:auto:end -->";
const B0: &str = "<!-- ownership:start -->";
const RECENT_DAYS: i64 = 60;
const QUIET_DAYS: i64 = 120;

/// One row of the team table: (code, english, chinese).
pub type Person = (String, String, String);

/// One `\s*([^|]+?)\s*` cell.
fn cell(c: &str) -> Option<String> {
    if c.is_empty() { return None }
    let t = py_strip(c);
    Some(if t.is_empty() { c.chars().last().unwrap().to_string() } else { t.to_string() })
}

/// team(): code -> (english, chinese) from `| en | zh | CODE |` rows; a later row for the same code wins, in place.
pub fn team(text: &str) -> Vec<Person> {
    let mut out: Vec<Person> = vec![];
    for line in text.split('\n') {
        let Some(rest) = line.strip_prefix('|') else { continue };
        let cells: Vec<&str> = rest.splitn(4, '|').collect();
        if cells.len() < 4 { continue }
        let code = py_strip(cells[2]);
        let (Some(en), Some(zh)) = (cell(cells[0]), cell(cells[1])) else { continue };
        if !(2..=3).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_uppercase()) { continue }
        match out.iter_mut().find(|p| p.0 == code) { Some(p) => { p.1 = en; p.2 = zh; } None => out.push((code.into(), en, zh)) }
    }
    out
}

/// `^# (20\d\d/\d\d/\d\d)(.*)$` -> the date
fn topic_date(line: &str) -> Option<String> {
    let c: Vec<char> = line.chars().take(12).collect();
    let ok = c.len() == 12 && c[0] == '#' && c[1] == ' ' && c[2] == '2' && c[3] == '0'
        && [4, 5, 7, 8, 10, 11].iter().all(|&i| digit(c[i]).is_some()) && c[6] == '/' && c[9] == '/';
    ok.then(|| c[2..].iter().collect())
}

/// (date, heading, body lines) of every Dev Topic above the ownership block.
fn topics<'a>(lines: &'a [&'a str]) -> Vec<(String, String, &'a [&'a str])> {
    let heads: Vec<usize> = (0..lines.len()).filter(|&i| topic_date(lines[i]).is_some()).collect();
    let stop = lines.iter().position(|l| py_strip(l) == B0).unwrap_or(lines.len());
    heads.iter().enumerate().map(|(k, &i)| {
        let next = heads.get(k + 1).copied().unwrap_or(lines.len());
        let end = next.min(if stop > i { stop } else { lines.len() });
        let heading: String = lines[i].chars().skip(2).collect();
        (topic_date(lines[i]).unwrap(), py_strip(&heading).to_string(), &lines[i + 1..end.max(i + 1)])
    }).collect()
}

/// The `( … )` at the end of a heading, full-width brackets too.
fn suffix(h: &str) -> Option<&str> {
    let t = h.trim_end_matches(py_space);
    let close = t.chars().last().filter(|c| matches!(c, ')' | '）'))?;
    let body = &t[..t.len() - close.len_utf8()];
    let open = body.rfind(['(', ')', '（', '）'])?;
    let oc = body[open..].chars().next()?;
    matches!(oc, '(' | '（').then(|| &body[open + oc.len_utf8()..])
}

/// owners(): "(SL-TP)" is topic SL owned by TP; `(?:-|, ?)([A-Z]{2,3})\b`, duplicates kept.
fn owners(heading: &str, people: &[Person]) -> Vec<String> {
    let Some(inner) = suffix(heading) else { return vec![] };
    let c: Vec<char> = inner.chars().collect();
    let code_at = |j: usize| -> Option<usize> {
        let n = (j..c.len().min(j + 3)).take_while(|&k| c[k].is_ascii_uppercase()).count();
        (2..=n).rev().map(|k| j + k).find(|&e| boundary(&c, e))
    };
    let (mut out, mut i) = (vec![], 0);
    while i < c.len() {
        let starts: Vec<usize> = match c[i] {
            '-' => vec![i + 1],
            ',' if c.get(i + 1) == Some(&' ') => vec![i + 2, i + 1],
            ',' => vec![i + 1],
            _ => vec![],
        };
        match starts.iter().find_map(|&j| code_at(j).map(|e| (j, e))) {
            Some((j, e)) => { out.push(c[j..e].iter().collect::<String>()); i = e; }
            None => i += 1,
        }
    }
    out.retain(|code| people.iter().any(|p| p.0 == *code));
    out
}

/// `re.findall(r"開發者 Developer：([^\n。]{1,60})", text)`
fn signatures(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let p: Vec<char> = "開發者 Developer：".chars().collect();
    let (mut out, mut i) = (vec![], 0);
    while i + p.len() <= c.len() {
        if c[i..i + p.len()] == p[..] {
            let j = i + p.len();
            let k = (j..c.len().min(j + 60)).take_while(|&k| c[k] != '\n' && c[k] != '。').count();
            if k >= 1 { out.push(c[j..j + k].iter().collect()); i = j + k; continue }
        }
        i += 1;
    }
    out
}

/// A sub-doc's text by link target: Ok(None) when it is not a vault .md file.
pub type SubDoc<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

/// signers(): developer codes named in the signature of every sub-doc this topic links to.
fn signers(body: &[&str], people: &[Person], subdoc: SubDoc) -> Result<HashSet<String>, String> {
    let mut found = HashSet::new();
    for target in wikilinks(&body.join("\n")) {
        let Some(text) = subdoc(py_strip(&target))? else { continue };
        for sig in signatures(&text) {
            let sig = sig.split('；').next().unwrap();
            let s: Vec<char> = sig.chars().collect();
            for (code, en, zh) in people {
                if (!zh.is_empty() && zh != "—" && sig.contains(zh.as_str())) || has_word(&s, en) || has_word(&s, code) {
                    found.insert(code.clone());
                }
            }
        }
    }
    Ok(found)
}

/// The heading without its leading date, as the Observed table links it.
fn short(h: &str) -> String {
    let rest = h.strip_prefix("20").map(|r| {
        let n = r.bytes().take_while(|b| b.is_ascii_digit() || *b == b'/').count();
        if n >= 1 && r[n..].starts_with(' ') { &r[n + 1..] } else { h }
    }).unwrap_or(h);
    rest.chars().take(34).collect()
}

/// sync(): the primary log with its Observed half rebuilt, or None when there is no auto block or nothing changed
/// beyond the "上次 <date>" line. `text` is the file as Python's text mode reads it; `rel` is its path as printed.
pub fn sync_ownership(text: &str, rel: &str, people: &[Person], subdoc: SubDoc, today: (i64, i64, i64)) -> Result<Option<String>, String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let today_n = days_from_civil(today.0, today.1, today.2);
    let (mut own, mut contrib, mut last, mut recent) =
        (HashMap::<String, usize>::new(), HashMap::<String, usize>::new(), HashMap::<String, String>::new(), HashMap::<String, Vec<String>>::new());
    let mut tops = topics(&lines);
    tops.sort_by(|a, b| b.0.cmp(&a.0));   // newest first, so "recent" really is the newest three
    for (date, heading, body) in &tops {
        let Some(d) = day(date) else { continue };   // Python says "bad date, topic skipped" on stderr, which ship drops
        let o = owners(heading, people);
        for c in &o { *own.entry(c.clone()).or_default() += 1 }
        let set: BTreeSet<&String> = o.iter().collect();
        for c in signers(body, people, subdoc)? { if !set.contains(&c) { *contrib.entry(c).or_default() += 1 } }
        for c in set {
            let l = last.entry(c.clone()).or_default();
            if date.as_str() > l.as_str() { *l = date.clone() }
            if today_n - d <= RECENT_DAYS {
                let r = recent.entry(c.clone()).or_default();
                if r.len() < 3 { r.push(heading.clone()) }
            }
        }
    }
    for c in contrib.keys() { last.entry(c.clone()).or_default(); }
    let mut who: Vec<String> = own.keys().chain(contrib.keys()).cloned().collect::<BTreeSet<_>>().into_iter().collect();
    who.sort_by_key(|c| (std::cmp::Reverse(own.get(c).copied().unwrap_or(0)), c.clone()));
    let label = |c: &str| {
        let p = people.iter().find(|p| p.0 == c).unwrap();
        format!("{} ({c})", if p.2 != "—" { &p.2 } else { &p.1 })
    };
    let (y, m, d) = today;
    let mut out: Vec<String> = vec![
        "## 實際活動 Observed".into(), "".into(),
        "> 自動產生，勿手改。來源是本主幹每個 Dev Topic 標題結尾的開發者代碼，以及該 Topic 連到的 sub-doc 的開發者署名。".into(),
        format!("> 更新：`python3 scripts/sync_ownership.py {rel}`，上次 {y:04}-{m:02}-{d:02}。"), "".into(),
        "| 成員 | 負責 Topic | 協作 | 最近 | 近 60 天的 Topic |".into(), "|---|---:|---:|---|---|".into(),
    ];
    for c in &who {
        let links: Vec<String> = recent.get(c).map(|r| r.iter().map(|h| format!("[[#{h}\\|{}]]", short(h))).collect()).unwrap_or_default();
        let l = last.get(c).filter(|l| !l.is_empty()).map(String::as_str).unwrap_or("署名");
        out.push(format!("| {} | {} | {} | {l} | {} |", label(c), own.get(c).unwrap_or(&0), contrib.get(c).unwrap_or(&0), links.join("<br>")));
    }
    // drift: compare with the people named in the Declared half
    let b0 = lines.iter().position(|l| *l == B0).unwrap_or(lines.len());
    let declared = declared(&lines[b0..].join("\n"));
    let age = |c: &str| last.get(c).filter(|l| !l.is_empty()).map(|l| today_n - day(l).unwrap());
    let active: BTreeSet<&String> = who.iter().filter(|c| age(c).is_some_and(|a| a <= RECENT_DAYS)).collect();
    let quiet: BTreeSet<&String> = declared.iter().filter(|c| age(c).is_none_or(|a| a > QUIET_DAYS)).collect();
    let mut drift: Vec<String> = active.iter().filter(|c| !declared.contains(**c))
        .map(|c| format!("- {} 近 60 天在本主幹有 {}+ 個 Topic，但宣告的分工裡沒有他。", label(c), recent.get(*c).map_or(0, Vec::len))).collect();
    drift.extend(quiet.iter().filter(|c| people.iter().any(|p| p.0 == ***c)).map(|c| {
        let when = match last.get(*c).filter(|l| !l.is_empty()) { None => "沒有他的 Topic".to_string(), Some(l) => format!("最近一筆是 {l}") };
        format!("- {} 列在宣告的分工，但本主幹{when}。工作可能記在別條主幹，或分工已變。", label(c))
    }));
    out.extend(["".into(), "### 對不上的地方 Drift".into(), "".into()]);
    if drift.is_empty() { out.push("- 宣告與實際一致。".into()) } else { out.extend(drift) }

    let (Some(a), Some(b)) = (lines.iter().position(|l| *l == A0), lines.iter().position(|l| *l == A1)) else { return Ok(None) };
    let new: Vec<&str> = lines[..a + 1].iter().copied().chain(out.iter().map(String::as_str)).chain(lines[b..].iter().copied()).collect();
    // the "上次 <date>" line alone is not a change worth reporting
    let strip = |ls: &[&str]| ls.iter().filter(|l| !l.contains("scripts/sync_ownership.py")).map(|l| l.to_string()).collect::<Vec<_>>();
    Ok((strip(&new) != strip(&lines)).then(|| new.join("\n")))
}

/// `re.findall(r"^- \*\*[^*]*?[（(]([A-Z]{2,3})[)）]\*\*", text, re.M)`
fn declared(text: &str) -> BTreeSet<String> {
    let c: Vec<char> = text.chars().collect();
    let tail = |q: usize| -> Option<(String, usize)> {
        if !matches!(c.get(q), Some('(' | '（')) { return None }
        let n = (q + 1..c.len().min(q + 4)).take_while(|&k| c[k].is_ascii_uppercase()).count();
        (2..=n).rev().find_map(|k| {
            let e = q + 1 + k;
            (matches!(c.get(e), Some(')' | '）')) && c.get(e + 1) == Some(&'*') && c.get(e + 2) == Some(&'*'))
                .then(|| (c[q + 1..e].iter().collect(), e + 3))
        })
    };
    let (mut out, mut from) = (BTreeSet::new(), 0);
    for s in 0..c.len() {
        if s < from || (s > 0 && c[s - 1] != '\n') || !c[s..].starts_with(&['-', ' ', '*', '*']) { continue }
        let mut q = s + 4;
        loop {
            if let Some((code, e)) = tail(q) { out.insert(code); from = e; break }
            if q >= c.len() || c[q] == '*' { break }
            q += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_primary_logs() {
        assert!(looks_secret("tokens.json") && looks_secret("a/b/.env") && looks_secret("x/GH_TOKEN.txt") && looks_secret("Secrets/id_ed25519.pub"));
        assert!(looks_secret("secret") && !looks_secret("secret/notes.md") && !looks_secret("a/.envrc") && !looks_secret("Key.md"));
        assert!(is_primary("_Web.md") && is_primary("System/_x.md") && !is_primary("_.md") && !is_primary("a_b.md") && !is_primary("_x.MD"));
    }

    #[test]
    fn a_submodule_pointer_is_not_a_leak() {
        let ls = ["160000 abc 0\tSecrets", "100644 def 0\tsecret.txt"];
        assert_eq!(leaks(&["M\tSecrets", "A\tsecret.txt", "D\ttoken.json", "R100\told\tmy_token"], &ls), ["secret.txt", "my_token"]);
    }

    #[test]
    fn links_outside_code() {
        let t = "[[A]] `[[B]]` ```\n[[C]]\n``` [[ dir/D.png \\|x]] [[E#h]] [[|no]] [[[F]] ```[[G]]";
        assert_eq!(link_targets(t), ["A", "D.png ", "E", "[F", "G"]);   // Python keeps the space before the escaped pipe
        assert_eq!(name_key_text("a/Note.MD", false), "note");
        assert_eq!(name_key_text("a\\Fig.PNG", true), "fig.png");
    }

    #[test]
    fn paths_split_by_submodule() {
        let subs = vec!["L1".to_string(), "L1/deep".to_string()];
        let paths: Vec<String> = ["L1/a.md", "L1/deep/b.md", "c.md", "L1"].map(String::from).to_vec();
        let (g, rest) = group_paths(&paths, &subs);
        assert_eq!(g, vec![("L1".to_string(), vec!["a.md".to_string()]), ("L1/deep".to_string(), vec!["b.md".to_string()])]);
        assert_eq!(rest, ["c.md", "L1", "L1/deep"]);
        assert_eq!(wanted(&["a/b.md", "ab.md", "c.md"], &["a/".into(), "c.md".into()]), ["a/b.md", "c.md"]);
        assert_eq!(submodule_paths("submodule.x.path\nL1\0submodule.y.path\nPlan Graphs/a\0"), ["L1", "Plan Graphs/a"]);
    }

    #[test]
    fn github_urls() {
        assert_eq!(github_repo("git@github.com:ExoPulseHQ/exo-l1.git"), Some(("ExoPulseHQ".into(), "exo-l1".into())));
        assert_eq!(github_repo("https://github.com/o/r"), Some(("o".into(), "r".into())));
        assert_eq!(github_repo("https://github.com/o/.git"), Some(("o".into(), ".git".into())));
        assert_eq!(github_repo("/tmp/remote.git"), None);
    }

    #[test]
    fn index_written_like_json_dump_indent_0() {
        let subs = vec![("L1".to_string(), serde_json::json!(["a", "b"])), ("P".to_string(), serde_json::json!([]))];
        let s = index_json(&Value::Null, &subs).unwrap();
        assert!(s.ends_with("\"submodules\":{\n\"L1\":[\n\"a\",\n\"b\"\n],\n\"P\":[]\n}\n}\n"));
        assert_eq!(index_json(&serde_json::from_str(&s).unwrap(), &subs), None);
        assert_eq!(index_json(&serde_json::json!({}), &[]), None);
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 3)), (2026, 10, 3));
        assert_eq!(day("2026/09/31"), None);
        assert_eq!(day("2024/02/29"), Some(days_from_civil(2024, 2, 29)));
        assert_eq!(local_date("a <b> 1759521600 +0800"), Some((2025, 10, 4)));
        assert_eq!(local_date("a <b> 1759521600 -0100"), Some((2025, 10, 3)));
    }

    #[test]
    fn owners_and_signatures() {
        let people = team("| Eddie | 賴宏達 | EL | x |\n| Sean | 劉智翔 | SL |\n| Tom | 潘語堂 | TP |\n| A | — | AB |\n");
        assert_eq!(people.len(), 4);
        assert_eq!(owners("2026/01/01 x (SL-TP)", &people), ["TP"]);
        assert_eq!(owners("x （PM-EL, SL）  ", &people), ["EL", "SL"]);
        assert_eq!(owners("x (A-ELX)", &people), Vec::<String>::new());
        assert_eq!(signatures("開發者 Developer：賴宏達（整理）；Y。\n開發者 Developer：。"), ["賴宏達（整理）；Y"]);
        assert!(has_word(&"by Sean.".chars().collect::<Vec<_>>(), "Sean") && !has_word(&"Seanx".chars().collect::<Vec<_>>(), "Sean"));
        assert_eq!(declared("- **賴宏達（EL）** x\n- **潘 (TP)**\n- **x** (SL)**\n"), ["EL", "TP"].map(String::from).into());
        assert_eq!(short("2026/09/01 標題 Title (X-EL)"), "標題 Title (X-EL)");
    }
}
