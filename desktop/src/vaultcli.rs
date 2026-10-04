//! `aiwalk-setup vault <cmd>`: the vault's upload tool (scripts/vault_ship.py, with scripts/sync_ownership.py) inside
//! the app, so a computer without Python can upload. Run it with the vault as the current folder. Messages, exit
//! codes and the git commands it runs are the Python's; the parts that need no git, disk or network are in
//! exo_core::vault_ship. GitHub is asked through the app's own client (github.rs), with gh's token.

use exo_core::vault_ship as vs;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The Python's docstring, which it prints for a command it does not know.
const USAGE: &str = r#"Ship the vault: rebase on origin, sync ownership, check wikilinks, commit as the gh account, push.

The commit identity is the GitHub account `gh` is logged in as, never the local git config,
so what the aiwalk portal's Team access module shows and what lands in history are one and the same:

    user.name  = <gh login>
    user.email = <gh id>+<gh login>@users.noreply.github.com

Usage
    vault_ship.py identity                 print the identity that would be used (exit 1 if gh is not logged in)
    vault_ship.py ship -m "type(scope): message (CODE-DEV)" [paths...]
                                           stage paths (default: all tracked changes), rebase, sync, check, commit, push
    vault_ship.py check-author             pre-commit guard: exit 1 unless GIT_AUTHOR_* match the gh identity
    vault_ship.py can-push <submodule>     "yes" if the gh account may push to that submodule's repo
    vault_ship.py index                    refresh System/vault_index.json (also done on every ship)

Paths inside a submodule are committed and pushed in the submodule first (on its own branch), then the vault
commits the submodule's new gitlink with the same message.

The commit is made first and then rebased onto origin/main. A conflict aborts the rebase, keeps the commit
on this computer unpushed, and lists the files; merge them (an agent is good at markdown merges) and push.
Nothing here rewrites published history or forces a push.
"#;
/// The repo-split commands (scripts/exo_repos.py), printed when one is given the wrong arguments. Kept apart from
/// USAGE, which stays vault_ship.py's docstring for an unknown command.
const REPOS_USAGE: &str = r#"The repo split (scripts/exo_repos.py), also run in the vault's folder:
    owner <path>...                        which repo owns each path
    plan [<vault>]                         split the tracked files by owner; exit 1 if any file has none
    assemble <dest>                        build the split vault at <dest>, each restricted folder a submodule; never pushes
    manifest                               write Papers/_manifest.json (path, bytes, blob sha, notes that link each PDF)
    clone <url> <dest>                     the app's Download: the book, then each readable submodule on its own;
                                           on-demand repos arrive without PDFs
    pull [<tree>]                          the app's Get latest: the book, then each readable submodule to its tip;
                                           a folder whose access was taken away is marked .exo-frozen
    push <tree>                            push every submodule, then the book
"#;
const INDEX: &str = "System/vault_index.json";

/// sys.exit("message"): the message on stderr, exit code 1.
fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

struct Out { code: i32, out: String, err: String }

/// subprocess.run(cmd, cwd=dir, text=True, capture_output=True)
fn run(dir: &Path, cmd: &[&str]) -> Out {
    let text = |b: &[u8]| vs::py_text(&String::from_utf8_lossy(b));
    match crate::cmd(cmd[0]).args(&cmd[1..]).current_dir(dir).output() {
        Ok(o) => Out { code: o.status.code().unwrap_or(1), out: text(&o.stdout), err: text(&o.stderr) },
        Err(e) => Out { code: 1, out: String::new(), err: e.to_string() },
    }
}

/// sh(): git always with core.quotePath=false (non-ASCII names as they are); `check` stops the program on failure.
fn sh(dir: &Path, cmd: &[&str], check: bool) -> String {
    let mut cmd = cmd.to_vec();
    if cmd[0] == "git" { cmd.splice(1..1, ["-c", "core.quotePath=false"]); }
    let r = run(dir, &cmd);
    if check && r.code != 0 {
        die(&format!("✗ {}\n{}", cmd.join(" "), vs::py_strip(if r.err.is_empty() { &r.out } else { &r.err })))
    }
    vs::py_strip(&r.out).to_string()
}

fn lines(s: &str) -> Vec<&str> { vs::py_splitlines(s) }

/// Python's text-mode write: "\n" becomes the platform's line ending.
fn write_text(path: &Path, s: &str) -> std::io::Result<()> {
    std::fs::write(path, if cfg!(windows) { s.replace('\n', "\r\n") } else { s.to_string() })
}

/// A path as os.path.relpath prints it on this platform.
fn native(p: &str) -> String { if cfg!(windows) { p.replace('/', "\\") } else { p.to_string() } }

fn identity() -> (String, String) {
    match crate::github::get("user") {
        Ok(u) if u["login"].is_string() && u["id"].is_number() => {
            let login = u["login"].as_str().unwrap().to_string();
            let email = format!("{}+{login}@users.noreply.github.com", u["id"]);
            (login, email)
        }
        _ => die("✗ not signed in to GitHub (sign in in aIwalk System Setup); refusing to commit under a local git identity"),
    }
}

fn submodule_paths(vault: &Path) -> Vec<String> {
    vs::submodule_paths(&run(vault, &["git", "config", "-z", "-f", ".gitmodules", "--get-regexp", r"submodule\..*\.path"]).out)
}

fn checked_out(vault: &Path, sub: &str) -> bool {
    std::fs::read_dir(vault.join(sub)).is_ok_and(|mut d| d.next().is_some())
}

/// name_key: sha1 of the name a wikilink resolves by, first 12 hex digits.
fn name_key(name: &str) -> String {
    let d = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, vs::name_key_text(name, cfg!(windows)).as_bytes());
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>()[..12].to_string()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// System/vault_index.json: per submodule, the hashed names of its files. An absent submodule keeps its entry.
fn update_index(vault: &Path) -> bool {
    let path = vault.join(INDEX);
    let old = read_json(&path).unwrap_or(Value::Object(Default::default()));
    let mut subs = vec![];
    for s in submodule_paths(vault) {
        if checked_out(vault, &s) {
            let dir = vault.join(&s);
            let files = run(vault, &["git", "-C", &dir.to_string_lossy(), "-c", "core.quotePath=false", "ls-files"]).out;
            let mut keys: Vec<String> = files.split('\n').filter(|f| !f.is_empty()).map(name_key).collect::<HashSet<_>>().into_iter().collect();
            keys.sort();
            subs.push((s, Value::from(keys)));
        } else if let Some(v) = old.get("submodules").and_then(|o| o.get(&s)) {
            subs.push((s, v.clone()));
        }
    }
    let Some(text) = vs::index_json(&old, &subs) else { return false };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    write_text(&path, &text).unwrap_or_else(|e| die(&format!("✗ {}: {e}", path.display())));
    true
}

/// Hashed names that live in a submodule this computer does not have: links to them are No access, not broken.
fn restricted_keys(vault: &Path) -> HashSet<String> {
    let Some(Value::Object(subs)) = read_json(&vault.join(INDEX)).and_then(|i| i.get("submodules").cloned()) else { return HashSet::new() };
    subs.iter().filter(|(s, _)| !checked_out(vault, s))
        .flat_map(|(_, keys)| keys.as_array().cloned().unwrap_or_default())
        .filter_map(|k| k.as_str().map(String::from)).collect()
}

/// os.walk over the vault: every file name, and a note's name without .md. Folders under .git and node_modules
/// are left out by their path text, and a link to a folder is not followed, as os.walk does.
fn walk_names(dir: &Path, names: &mut HashSet<String>) {
    let s = dir.to_string_lossy();
    if s.contains("/.git") || s.contains("/node_modules") { return }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let link = e.file_type().is_ok_and(|t| t.is_symlink());
        let is_dir = if link { std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()) } else { e.file_type().is_ok_and(|t| t.is_dir()) };
        if is_dir {
            if !link { walk_names(&e.path(), names) }
            continue;
        }
        let f = e.file_name().to_string_lossy().into_owned();
        if let Some(n) = f.strip_suffix(".md") { names.insert(n.to_string()); }
        names.insert(f);
    }
}

/// Every [[target]] a change adds to a .md must resolve to a vault file (non-.md targets need their extension).
fn check_wikilinks(vault: &Path, root: &Path, paths: &[String]) -> Vec<String> {
    let mut names = HashSet::new();
    walk_names(vault, &mut names);
    let hidden = restricted_keys(vault);
    let mut bad = vec![];
    for p in paths {
        let file = root.join(p);
        if !p.ends_with(".md") || p.rsplit('/').next() == Some("CLAUDE.md") || !file.exists() { continue }
        let before: HashSet<String> = vs::link_targets(&run(root, &["git", "show", &format!("HEAD:{p}")]).out).into_iter().collect();
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| die(&format!("✗ {p}: {e}")));
        for t in vs::link_targets(&vs::py_text(&text)) {
            if !t.is_empty() && !names.contains(&t) && !before.contains(&t) && !hidden.contains(&name_key(&t)) {
                bad.push(format!("{p}: [[{t}]]"));
            }
        }
    }
    bad
}

/// glob("**/*") over the vault: base name -> first path, in directory order, hidden names left out.
fn glob_index(dir: &Path, names: &mut HashMap<String, PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let entries: Vec<PathBuf> = rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| e.path()).collect();
    for p in entries.iter().filter(|p| p.is_file() && !p.to_string_lossy().contains("/.")) {
        names.entry(p.file_name().unwrap().to_string_lossy().into_owned()).or_insert_with(|| p.clone());
    }
    for p in entries.iter().filter(|p| p.is_dir()) { glob_index(p, names) }
}

/// Today's local date, as git reckons it (std has no time zones).
fn today(vault: &Path) -> Option<(i64, i64, i64)> {
    vs::local_date(&sh(vault, &["git", "-c", "user.name=x", "-c", "user.email=x", "var", "GIT_COMMITTER_IDENT"], false))
}

/// The primary log at `path` with the generated half of its ownership block rebuilt: Some(new text) when that
/// differs from the file, None when it is up to date or has no generated half. `rel` is the path as the block prints it.
pub(crate) fn ownership(vault: &Path, path: &Path, rel: &str) -> Result<Option<String>, String> {
    let mut names = HashMap::new();
    glob_index(vault, &mut names);
    let team = std::fs::read_to_string(vault.join(vs::TEAM)).map_err(|e| format!("{}: {e}", vs::TEAM))?;
    let people = vs::team(&vs::py_text(&team));
    let day = today(vault).ok_or("git did not give today's date")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{rel}: {e}"))?;
    let subdoc = |target: &str| -> Result<Option<String>, String> {
        let base = if cfg!(windows) { target.rsplit(['/', '\\']).next() } else { target.rsplit('/').next() }.unwrap_or(target);
        let Some(found) = names.get(base).or_else(|| names.get(&format!("{base}.md"))) else { return Ok(None) };
        if !found.to_string_lossy().ends_with(".md") { return Ok(None) }
        let bytes = std::fs::read(found).map_err(|e| e.to_string())?;
        Ok(Some(vs::py_text(&String::from_utf8_lossy(&bytes))))
    };
    vs::sync_ownership(&vs::py_text(&text), rel, &people, &subdoc, day)
}

/// The sync as ship runs it: nothing printed, any failure leaves the file alone.
fn sync_ownership(vault: &Path, p: &str) {
    let path = vault.join(p);
    if let Ok(Some(new)) = ownership(vault, &path, &native(p)) { let _ = write_text(&path, &new); }
}

/// `aiwalk-setup vault sync-ownership [--check] [--all] [logs…]`, as scripts/sync_ownership.py: the same lines on
/// stdout, exit 1 when --check finds a log that would change, 2 on a usage error.
fn sync_cli(vault: &Path, a: &[String]) -> i32 {
    let check = a.iter().any(|x| x == "--check");
    let mut args: Vec<PathBuf> = a.iter().filter(|x| !x.starts_with("--")).map(PathBuf::from).collect();
    if a.iter().any(|x| x == "--all") {
        // every log in main.md's table
        let main = std::fs::read_to_string(vault.join("main.md")).unwrap_or_default();
        let mut names = HashMap::new();
        glob_index(vault, &mut names);
        let mut seen = HashSet::new();
        args = fancy_regex::Regex::new(r"\|\s*\[\[(_[^\]|]+)\]\]").unwrap().captures_iter(&main).flatten()
            .filter_map(|c| names.get(&format!("{}.md", c.get(1).map_or("", |m| m.as_str()))).cloned()).filter(|p| seen.insert(p.clone())).collect();
    }
    if args.is_empty() { eprintln!("no log given: pass a path or --all"); return 2 }
    // a path may be relative to the cwd or to the vault root, which here are the same folder
    let paths: Vec<PathBuf> = args.iter().map(|p| std::path::absolute(p).unwrap_or_else(|_| vault.join(p))).collect();
    let missing: Vec<String> = paths.iter().filter(|p| !p.exists()).map(|p| p.to_string_lossy().into_owned()).collect();
    if !missing.is_empty() { eprintln!("not found: {}", missing.join(", ")); return 2 }
    let mut changed = false;
    for path in &paths {
        let rel = path.strip_prefix(vault).unwrap_or(path).to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if !["<!-- ownership:auto:start -->", "<!-- ownership:auto:end -->"].iter().all(|m| text.lines().any(|l| l == *m)) {
            println!("skip (no <!-- ownership:auto:start --> block): {rel}");
            continue
        }
        match ownership(vault, path, &rel) {
            Ok(None) => println!("unchanged: {rel}"),
            Ok(Some(new)) => {
                changed = true;
                if !check { if let Err(e) = write_text(path, &new) { eprintln!("{rel}: {e}"); return 2 } }
                println!("{}{rel}", if check { "would change: " } else { "updated: " });
            }
            Err(e) => { eprintln!("{e}"); return 2 }
        }
    }
    (check && changed) as i32
}

fn ship(vault: &Path, msg: &str, mut paths: Vec<String>) {
    let (name, email) = identity();
    println!("identity: {name} <{email}>");
    if !paths.is_empty() {
        let (groups, rest) = vs::group_paths(&paths, &submodule_paths(vault));
        for (s, inner) in &groups {
            println!("submodule {s}:");
            ship_repo(vault, Some(s), msg, inner.clone(), &name, &email, false);
        }
        paths = rest;   // the vault records each submodule's new commit, after the submodule is on GitHub
    }
    if update_index(vault) {
        if !paths.is_empty() { paths.push(INDEX.into()) } else { sh(vault, &["git", "add", "--", INDEX], true); }   // new the first time
    }
    ship_repo(vault, None, msg, paths, &name, &email, true);
}

fn ship_repo(vault: &Path, sub: Option<&str>, msg: &str, paths: Vec<String>, name: &str, email: &str, required: bool) {
    let root = &sub.map_or(vault.to_path_buf(), |s| vault.join(s));
    let p: Vec<&str> = paths.iter().map(String::as_str).collect();
    // a submodule is often on a detached HEAD after `git submodule update`; it then ships to its remote's default branch
    let mut branch = sh(root, &["git", "rev-parse", "--abbrev-ref", "HEAD"], true);
    if branch == "HEAD" {
        let head = sh(root, &["git", "symbolic-ref", "--short", "refs/remotes/origin/HEAD"], false);
        let head = if head.is_empty() { "origin/main".to_string() } else { head };
        branch = head.split_once('/').map(|(_, b)| b.to_string()).unwrap_or_else(|| die(&format!("✗ no branch in {head}")));
    }
    if p.is_empty() { sh(root, &["git", "add", "-u"], true); } else { sh(root, &[&["git", "add", "--"][..], &p].concat(), true); }
    // a deletion may go (that is how a secret leaves the repo); adding or changing one may not
    let leaks = vs::leaks(&lines(&sh(root, &["git", "diff", "--cached", "--name-status"], true)), &lines(&sh(root, &["git", "ls-files", "-s"], true)));
    if !leaks.is_empty() {
        let l: Vec<&str> = leaks.iter().map(String::as_str).collect();
        sh(root, &[&["git", "reset", "-q", "--"][..], &l].concat(), true);
        eprintln!("held back, looks like a token or key: {}", leaks.join(", "));
    }
    let all = sh(root, &["git", "diff", "--cached", "--name-only"], true);
    let staged: Vec<String> = if paths.is_empty() { lines(&all).iter().map(|s| s.to_string()).collect() } else { vs::wanted(&lines(&all), &paths) };
    let ahead: i64 = sh(root, &["git", "rev-list", "--count", "@{u}..HEAD"], false).parse().unwrap_or(0);
    if staged.is_empty() && ahead == 0 {
        if !required { println!("  nothing to commit here"); return }
        // an earlier upload (another window, another session) already took them: done, not failed
        if !paths.is_empty() && vs::py_strip(&sh(root, &[&["git", "status", "--porcelain", "--"][..], &p].concat(), false)).is_empty() {
            println!("  already in git, nothing left to upload for these paths");
            return;
        }
        die("nothing staged")
    }
    for log in staged.iter().filter(|s| sub.is_none() && vs::is_primary(s) && vault.join(s).exists()) {
        sync_ownership(vault, log);
        sh(vault, &["git", "add", "--", log], true);
    }
    let bad = check_wikilinks(vault, root, &staged);
    if !bad.is_empty() {
        die(&format!("✗ unresolved wikilinks (vault files only; non-.md targets need the extension):\n  {}", bad.join("\n  ")))
    }
    if !staged.is_empty() {
        // commit the index as staged, never `commit -- paths`; anything else staged here (another session's) is unstaged first
        let others: Vec<String> = lines(&sh(root, &["git", "diff", "--cached", "--name-only"], true)).into_iter().filter(|o| !staged.iter().any(|s| s == o)).map(String::from).collect();
        if !others.is_empty() {
            let o: Vec<&str> = others.iter().map(String::as_str).collect();
            sh(root, &[&["git", "reset", "-q", "--"][..], &o].concat(), true);
        }
        sh(root, &["git", "-c", &format!("user.name={name}"), "-c", &format!("user.email={email}"), "commit", "-q", "-m", msg], true);
    }
    sh(root, &["git", "fetch", "-q", "origin"], true);
    for attempt in 1..=2 {
        // --autostash parks everyone else's unstaged edits around the rebase and puts them back afterwards
        let r = run(root, &["git", "rebase", "-q", "--autostash", &format!("origin/{branch}")]);
        if r.code != 0 {
            let conflicts = sh(root, &["git", "diff", "--name-only", "--diff-filter=U"], false);
            run(root, &["git", "rebase", "--abort"]);
            let files = if conflicts.is_empty() { vs::py_strip(if r.err.is_empty() { &r.out } else { &r.err }).to_string() } else { conflicts };
            die(&vs::conflict_message(sub.map(native).as_deref(), &files, &branch))
        }
        let push = run(root, &["git", "push", "-q", "origin", &format!("HEAD:{branch}")]);
        if push.code == 0 { break }
        if attempt == 2 { die(&format!("✗ push failed twice:\n{}", vs::py_strip(if push.err.is_empty() { &push.out } else { &push.err }))) }
        sh(root, &["git", "fetch", "-q", "origin"], true);   // someone pushed in between: rebase once more and retry
    }
    println!("{}", sh(root, &["git", "log", "--oneline", "-1"], true));
}

/// Whether the signed-in account may push to this submodule's GitHub repo (a third-party one stays read-only).
fn can_push(vault: &Path, sub: &str) -> bool {
    let url = run(vault, &["git", "-C", &vault.join(sub).to_string_lossy(), "remote", "get-url", "origin"]).out;
    let Some((owner, repo)) = vs::github_repo(vs::py_strip(&url)) else { return false };
    crate::github::get(&format!("repos/{owner}/{repo}")).is_ok_and(|r| r["permissions"]["push"] == Value::Bool(true))
}

fn check_author(vault: &Path) {
    let (name, email) = identity();
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let an = env("GIT_AUTHOR_NAME").unwrap_or_else(|| sh(vault, &["git", "config", "user.name"], false));
    let ae = env("GIT_AUTHOR_EMAIL").unwrap_or_else(|| sh(vault, &["git", "config", "user.email"], false));
    if (an.as_str(), ae.as_str()) != (name.as_str(), email.as_str()) { die(&vs::author_message(&an, &ae, &name, &email)) }
}

// ---------------------------------------------------------------- scripts/exo_repos.py

use exo_core::exo_repos as er;

/// The Python's LOCAL: lets file:// submodules work (tests, local remotes); harmless for GitHub.
const LOCAL: [(&str, &str); 1] = [("protocol.file.allow", "always")];

/// System/vault_rules.json, whole; exits 1 as the Python's load() would raise.
fn rules(vault: &Path) -> Value {
    let path = vault.join("System/vault_rules.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
    if !v["repos"].is_object() { die(&format!("{}: no \"repos\" section", path.display())) }
    v
}

/// The hook's is_system_doc(), the one copy of the plugin's isSystemDoc().
fn system_doc(rules: &Value, rel: &str) -> bool {
    crate::hooks::writing_style_reminder::is_system_doc(Some(rules), rel).unwrap_or_else(|e| die(&e))
}

/// `git ls-files -s` with names as they are; exits 1 if git fails.
fn ls_files(dir: &Path) -> String {
    let r = run(dir, &["git", "-c", "core.quotepath=off", "ls-files", "-s"]);
    if r.code != 0 { die(&format!("✗ git ls-files in {}\n{}", dir.display(), vs::py_strip(&r.err))) }
    r.out
}

/// tracked(): the paths git tracks, gitlinks left out.
fn tracked(dir: &Path) -> Vec<String> {
    lines(&ls_files(dir)).iter().filter(|l| !l.starts_with("160000")).filter_map(|l| l.split_once('\t')).map(|(_, p)| p.to_string()).collect()
}

/// git as the Python's git(): with LOCAL, output captured.
fn git_local(dir: &Path, args: &[&str]) -> Out { run(dir, &[&["git", "-c", "protocol.file.allow=always"][..], args].concat()) }

/// git_local that exits 1 on failure, as check=True would raise.
fn git_check(dir: &Path, args: &[&str]) {
    let r = git_local(dir, args);
    if r.code != 0 { die(&format!("✗ git {}\n{}", args.join(" "), vs::py_strip(if r.err.is_empty() { &r.out } else { &r.err }))) }
}

/// The last n characters of git's stderr, stripped, as `r.stderr.strip()[-n:]`.
fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = vs::py_strip(s).chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

/// `vault manifest`: Papers/_manifest.json. In a split vault the files of each checked-out repo submodule count as
/// the vault's own (the Python sees only the book there and writes an empty list).
fn manifest(vault: &Path) {
    let rules = rules(vault);
    let cfg = &rules["repos"];
    let repos: HashSet<&str> = cfg["routes"].as_array().into_iter().flatten().filter_map(|r| r["repo"].as_str()).filter(|r| Some(*r) != cfg["book"].as_str()).collect();
    let mut ls = ls_files(vault);
    for (name, path) in crate::vault::submodules(vault) {
        if !repos.contains(name.as_str()) || !vault.join(&path).join(".git").exists() { continue }
        let dir = vault.join(&path);
        if path == er::PAPERS && sh(&dir, &["git", "config", "--bool", "core.sparseCheckout"], false) == "true" {
            die(&format!("✗ {path} is a sparse checkout: most PDFs are not on this computer, so the list would lose them. Write it where {path} is fully checked out."))
        }
        for l in lines(&ls_files(&dir)) {
            if let Some((head, p)) = l.split_once('\t') { ls += &format!("{head}\t{path}/{p}\n") }
        }
    }
    let file = |p: &str| std::fs::metadata(vault.join(p)).ok().filter(|m| m.is_file());
    let (json, line) = er::manifest(&ls, &|p| file(p).map(|m| m.len()), &|p| file(p).and_then(|_| std::fs::read(vault.join(p)).ok()), cfg!(windows));
    let out = vault.join(er::PAPERS).join("_manifest.json");
    write_text(&out, &json).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    println!("{line}");
}

fn owner_cli(vault: &Path, paths: &[String]) {
    let rules = rules(vault);
    let cfg = &rules["repos"];
    for p in paths {
        let o = er::owner(cfg, p, system_doc(&rules, p));
        println!("{} {p} -> {}", o.as_deref().unwrap_or("NONE"), er::placed(cfg, p));
    }
}

fn plan(vault: &Path) -> i32 {
    let rules = rules(vault);
    let (out, code) = er::plan(&rules["repos"], &tracked(vault), &|rel| system_doc(&rules, rel)).unwrap_or_else(|e| die(&e));
    print!("{out}");
    code
}

/// `vault assemble <dest>`: the split vault built from this one, each restricted folder its own repo mounted as a
/// submodule of the book. Nothing is pushed.
fn assemble(src: &Path, dest: &Path) {
    let rules = rules(src);
    let cfg = &rules["repos"];
    if dest.exists() { die(&format!("{} exists; pick an empty path", dest.display())) }
    let homes = er::folders(cfg).unwrap_or_else(|e| die(&e));
    let book = cfg["book"].as_str().unwrap_or("");
    let ident: Vec<String> = ["name", "email"].iter().map(|k| sh(src, &["git", "config", &format!("user.{k}")], false)).collect();
    let own = |rel: &str| er::owner(cfg, rel, system_doc(&rules, rel));
    let (mut count, mut orphans) = (HashMap::<String, usize>::new(), vec![]);
    for rel in tracked(src) {
        if rel == ".gitmodules" || !src.join(&rel).is_file() { continue }   // .gitmodules is rebuilt; deleted files stay out
        let Some(o) = own(&rel) else { orphans.push(rel); continue };
        let to = dest.join(er::placed(cfg, &rel));
        std::fs::create_dir_all(to.parent().unwrap()).and_then(|_| std::fs::copy(src.join(&rel), &to))
            .unwrap_or_else(|e| die(&format!("{rel}: {e}")));
        *count.entry(o).or_default() += 1;
    }
    for rel in &orphans { println!("not copied, no owner: {rel}") }
    let remote = |repo: &str| format!("{}/{repo}.git", std::env::var("EXO_REMOTE_BASE").ok().filter(|b| !b.is_empty())
        .unwrap_or_else(|| format!("git@github.com:{}", cfg["org"].as_str().unwrap_or(""))));
    let init = |path: &Path, repo: &str| {
        std::fs::create_dir_all(path).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
        git_check(path, &["init", "-q", "-b", "main"]);
        git_check(path, &["config", "user.name", &ident[0]]);
        git_check(path, &["config", "user.email", &ident[1]]);
        git_check(path, &["remote", "add", "origin", &remote(repo)]);
    };
    let commit_all = |path: &Path| { git_local(path, &["add", "-A", "-f"]); git_check(path, &["commit", "-q", "-m", "Initial import from ExoPulse_docs"]) };
    // each upstream repo is a submodule of whichever repo owns its folder
    let nested: Vec<(Option<String>, &str, &str)> = cfg["nested"].as_array().into_iter().flatten()
        .map(|n| { let p = n["path"].as_str().unwrap_or(""); (own(&format!("{p}/")), p, n["url"].as_str().unwrap_or("")) }).collect();
    let add_nested = |top: &Path, base: &str, repo: &str| {
        for (_, p, url) in nested.iter().filter(|n| n.0.as_deref() == Some(repo)) {
            let r = git_local(top, &["submodule", "add", "-q", url, &p[base.len().min(p.len())..]]);
            if r.code == 0 { println!("nested  {p}") } else { println!("FAILED  {p}  {}", tail(&r.err, 120)) }
        }
    };
    for (repo, folder) in &homes {
        let path = dest.join(folder);
        if !path.is_dir() { continue }   // nothing to put there yet
        init(&path, repo);
        add_nested(&path, &format!("{folder}/"), repo);
        commit_all(&path);
        println!("{:6}  {repo}  ->  {folder}/", count.get(repo).unwrap_or(&0));
    }
    init(dest, book);
    for (repo, folder) in &homes {
        if dest.join(folder).join(".git").exists() {
            git_check(dest, &["submodule", "add", "-q", "--name", repo, &format!("../{repo}.git"), folder]);
            git_check(dest, &["config", "-f", ".gitmodules", &format!("submodule.{repo}.branch"), "main"]);
        }
    }
    add_nested(dest, "", book);
    commit_all(dest);
    println!("{:6}  {book}  ->  (root)", count.get(book).unwrap_or(&0));
}

/// `vault push <tree>`: every submodule whose origin is on the remote base, then the book, so the book's gitlinks
/// point at commits the remotes already have.
fn push(tree: &Path) {
    let base = std::env::var("EXO_REMOTE_BASE").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "git@github.com:".into());
    for (_, s) in crate::vault::submodules(tree) {
        let p = tree.join(&s);
        if p.join(".git").exists() && git_local(&p, &["remote", "get-url", "origin"]).out.starts_with(&base) {
            let r = git_local(&p, &["push", "-q", "-u", "origin", "main"]);
            if r.code == 0 { println!("pushed  {s}") } else { println!("FAILED  {s}  {}", tail(&r.err, 120)) }
        }
    }
    let r = git_local(tree, &["push", "-q", "-u", "origin", "main"]);
    if r.code == 0 { println!("pushed  (book)") } else { println!("FAILED  (book)  {}", tail(&r.err, 120)) }
}

/// `vault clone <url> <dest>` and `vault pull [<tree>]`: the app's own Download and Get latest (vault.rs), with the
/// Python's lines.
fn clone_cli(url: &str, dest: &Path) {
    let rows = crate::vault::clone(url, dest, &LOCAL, &|_, _| {}).unwrap_or_else(|e| die(&e));
    for (p, got) in rows { println!("{}", crate::vault::row(&p, got)) }
}

fn pull_cli(tree: &Path) {
    let (book, rows) = crate::vault::pull(tree, &LOCAL, &|_, _| {});
    match book { Ok(()) => println!("ok      (book)"), Err(e) => println!("FAILED  (book)  {}", tail(&e, 120)) }
    for (p, got) in rows { println!("{}", crate::vault::row(&p, got)) }
}

/// A path argument as os.path.abspath makes it.
fn abs(p: &str) -> PathBuf { std::path::absolute(p).unwrap_or_else(|e| die(&format!("✗ {p}: {e}"))) }

/// `aiwalk-setup vault <args>`, as `python3 scripts/vault_ship.py <args>`.
pub fn main(a: &[String]) {
    let vault = std::env::current_dir().unwrap_or_else(|e| die(&format!("✗ {e}")));
    match a.first().map(String::as_str) {
        Some("identity") => { let (n, e) = identity(); println!("{n} <{e}>") }
        Some("can-push") if a.len() == 2 => println!("{}", if can_push(&vault, &a[1]) { "yes" } else { "no" }),
        Some("index") => println!("{}", if update_index(&vault) { "updated" } else { "unchanged" }),
        Some("check-author") => check_author(&vault),
        Some("sync-ownership") => std::process::exit(sync_cli(&vault, &a[1..])),
        Some("manifest") => manifest(&vault),
        Some("owner") => owner_cli(&vault, &a[1..]),
        Some("plan") => std::process::exit(plan(&a.get(1).map_or(vault.clone(), |p| abs(p)))),
        Some("assemble") if a.len() == 2 => assemble(&vault, &abs(&a[1])),
        Some("push") if a.len() == 2 => push(&abs(&a[1])),
        Some("clone") if a.len() == 3 => clone_cli(&a[1], &abs(&a[2])),
        Some("pull") => pull_cli(&a.get(1).map_or(vault.clone(), |p| abs(p))),
        Some("ship") if a.iter().any(|x| x == "-m") => {
            let i = a.iter().position(|x| x == "-m").unwrap();
            let Some(msg) = a.get(i + 1) else { die("✗ -m needs a message") };
            ship(&vault, msg, a[1..i].iter().chain(&a[i + 2..]).cloned().collect())
        }
        Some("assemble" | "push" | "clone") => die(REPOS_USAGE),
        _ => die(USAGE),   // sys.exit(__doc__): the docstring and a newline
    }
}
