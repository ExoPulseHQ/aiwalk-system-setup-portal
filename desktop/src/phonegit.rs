//! The vault on a phone: download, "Get latest" and "Send changes" without a git program, which Android does not
//! have. Same shape as vault.rs on a computer: the book is cloned shallow, then each submodule on its own, so one
//! the account cannot read only skips itself; the results are exo_core's `Got` rows and `summary`.
//!
//! Sending is `vault ship` cut down to what a phone needs: each folder's changes are one commit on top of the team's
//! newest, pushed, and the book then records the folders that moved. Nothing is ever merged here: when the person
//! and the team changed the same file, nothing is sent and their text stays, to be sorted out on a computer.
//!
//! Library: git2 (libgit2, compiled from source by libgit2-sys with the NDK's clang, as `ring` already is), not gix.
//! libgit2 has every step here as one call: shallow fetch (`depth`), a checkout that refuses to overwrite the
//! person's own changes (`CheckoutBuilder::safe`) and the submodule list; with gix, updating a work tree that is
//! already there would be ours to write. libgit2 is built without HTTPS of its own (no OpenSSL): `https://` goes
//! through the transport registered below on top of ureq, so TLS is the rustls (ring) the app already ships.
//! libgit2 has no partial clone and no sparse checkout; what stands in for them is described next.
//!
//! What comes to the phone: everything except the papers, about 6 GB (3 GB of files, most of it pdf, pptx and docx,
//! and as much again of git data). A phone without that room can take the notes and the files up to 2 MB instead,
//! about 2 GB; the choice is made at Download and kept with the copy (`aiwalk.small` in its git config).
//! libgit2 cannot ask for a clone without large files, but the request it writes goes through the transport below,
//! which adds git's own `filter blob:limit` line to it (`with_filter`) when the server offers that. The files GitHub then leaves out are kept in the index as skip-worktree entries
//! (`place`): not on the phone, not seen as deleted, and still in every commit sent from here. The on-demand
//! folders (the papers) come the same way with a much lower limit, so only their small notes arrive. Any file left
//! on GitHub can be brought to the phone by itself (`fetch`): its content is asked from GitHub by the id git already
//! holds for that path, and written in place; being what git records there, it is not a change.
//!
//! The computer app never compiles this file except for its test (`--features phone-git`); it keeps calling git.

use exo_core::exo_repos::Got;
use git2::build::CheckoutBuilder;
use git2::transport::{self, Service, SmartSubtransport, SmartSubtransportStream, Transport};
use git2::{AutotagOption, ErrorCode, FetchOptions, IndexEntry, IndexTime, ObjectType, PushOptions, RemoteCallbacks, Repository, Signature, StatusOptions, TreeWalkMode, TreeWalkResult};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, Once};

/// Reports progress: fraction of the whole job done (0..1) and what is happening now.
pub type Progress<'a> = &'a dyn Fn(f32, &str);

/// The GitHub token git sends, only ever to github.com. None reads public repos only.
static TOKEN: Mutex<Option<String>> = Mutex::new(None);

pub fn use_token(token: Option<String>) { *TOKEN.lock().unwrap() = token; }

// ---------------------------------------------------------------- https over ureq

/// Makes libgit2 fetch https:// through `Https`, and accept folders in shared storage. Once per process.
fn https() {
    static ONCE: Once = Once::new();
    // SAFETY: both run once, before any repo or remote is opened by this process (Once orders them before every use)
    ONCE.call_once(|| unsafe {
        transport::register("https", |remote| Transport::smart(remote, true, Https)).expect("https transport");
        // the test's own server on this computer (git http-backend) speaks plain http
        #[cfg(test)]
        transport::register("http", |remote| Transport::smart(remote, true, Https)).expect("http transport");
        // Android's shared storage shows every file as owned by another user, so libgit2's ownership check (git's
        // safe.directory) refuses each copy there with "not owned by current user". Only this app's folders are opened.
        let _ = git2::opts::set_verify_owner_validation(false);
        // a tree or an index entry may name a file that was left on GitHub (see the top of this file)
        git2::opts::strict_object_creation(false);
    });
}

struct Https;

impl SmartSubtransport for Https {
    fn action(&self, url: &str, action: Service) -> Result<Box<dyn SmartSubtransportStream>, git2::Error> {
        let (post, service) = match action {
            Service::UploadPackLs => (false, "git-upload-pack"),
            Service::UploadPack => (true, "git-upload-pack"),
            Service::ReceivePackLs => (false, "git-receive-pack"),
            Service::ReceivePack => (true, "git-receive-pack"),
        };
        let path = if post { format!("/{service}") } else { format!("/info/refs?service={service}") };
        let base = url.trim_end_matches('/').to_string();
        Ok(Box::new(Stream { url: format!("{base}{path}"), base, post, service, body: vec![], reply: None }))
    }
    fn close(&self) -> Result<(), git2::Error> { Ok(()) }
}

/// One request: what libgit2 writes is the request body, sent at its first read; reads then come from the reply.
struct Stream { url: String, base: String, post: bool, service: &'static str, body: Vec<u8>, reply: Option<Box<dyn Read + Send>> }

/// For a copy made small: files larger than this stay on GitHub. At 2 MB the vault keeps every note and 3901 of
/// its 4164 files in 0.77 GB of 3.10 (measured 2026-10-06); what stays behind is slide decks, Word files, pdf and
/// a few large figures.
const LIMIT: &str = "blob:limit=2m";

/// Whether the copy being worked on is a small one (see the top of this file). Set from the person's choice at
/// Download, and from the copy's own record before every later fetch.
static SMALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn small(on: bool) { SMALL.store(on, std::sync::atomic::Ordering::Relaxed) }

/// Reads the choice the copy at `tree` was made with. A copy without the record (made before the choice existed)
/// takes everything from now on; what it left behind earlier stays one tap away.
fn recall(tree: &Path) {
    small(Repository::open(tree).and_then(|r| r.config()).and_then(|c| c.get_bool("aiwalk.small")).unwrap_or(false))
}

/// The on-demand folders (the papers): only their notes come; every paper is one tap away (`fetch`).
const LIMIT_ON_DEMAND: &str = "blob:limit=64k";

/// The repos (by address) whose server said it can leave large files out.
static FILTERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The on-demand repos (by address), which get LIMIT_ON_DEMAND.
static ON_DEMAND: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn pkt(line: &[u8]) -> Vec<u8> { [format!("{:04x}", line.len() + 4).as_bytes(), line].concat() }

/// libgit2's upload-pack request with git's partial-clone filter added: `filter` among the first want's
/// capabilities and a `filter <spec>` line before the first flush. `thin-pack` is taken out, so nothing arrives as
/// a difference against a file that was left out. A request this does not understand goes out as it came.
fn with_filter(body: &[u8], spec: &str) -> Vec<u8> {
    let (mut out, mut at, mut asked, mut done) = (Vec::with_capacity(body.len() + 64), 0, false, false);
    while at + 4 <= body.len() {
        let Some(n) = std::str::from_utf8(&body[at..at + 4]).ok().and_then(|h| usize::from_str_radix(h, 16).ok()) else { return body.to_vec() };
        if n == 0 {
            if asked && !done { out.extend(pkt(format!("filter {spec}\n").as_bytes())); done = true }
            out.extend(b"0000");
            at += 4;
            continue
        }
        if n < 4 || at + n > body.len() { return body.to_vec() }
        let line = &body[at + 4..at + n];
        if at == 0 && line.starts_with(b"want ") {
            let text = String::from_utf8_lossy(line).trim_end().replace(" thin-pack", "");
            out.extend(pkt(format!("{text} filter\n").as_bytes()));
            asked = true;
        } else { out.extend(&body[at..at + n]) }
        at += n;
    }
    out
}

impl Write for Stream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.body.extend_from_slice(b); Ok(b.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.reply.is_none() {
            let mut reply = self.send()?;
            self.reply = Some(if self.post || self.service != "git-upload-pack" { Box::new(reply) } else {
                // the list of branches, which also says what the server can do: read whole to see whether it filters
                let mut all = vec![];
                reply.read_to_end(&mut all)?;
                let mut filters = FILTERS.lock().unwrap();
                filters.retain(|b| b != &self.base);
                if all.windows(8).any(|w| w == b" filter " || w == b" filter\n") { filters.push(self.base.clone()) }
                Box::new(std::io::Cursor::new(all))
            });
        }
        self.reply.as_mut().unwrap().read(buf)
    }
}

impl Stream {
    fn send(&mut self) -> std::io::Result<ureq::BodyReader<'static>> {
        use base64::Engine;
        let io = |e: String| std::io::Error::other(e);
        let mut req = ureq::http::Request::builder().uri(&self.url).header("User-Agent", "git/2.0 (aiwalk-setup)");
        // the token goes to GitHub only: a .gitmodules may name any server
        if let (Some(t), true) = (TOKEN.lock().unwrap().clone(), self.url.starts_with("https://github.com/")) {
            req = req.header("Authorization", format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{t}"))));
        }
        let reply = if self.post {
            if self.service == "git-upload-pack" && FILTERS.lock().unwrap().contains(&self.base) {
                let spec = if ON_DEMAND.lock().unwrap().contains(&self.base) { Some(LIMIT_ON_DEMAND) }
                    else if SMALL.load(std::sync::atomic::Ordering::Relaxed) { Some(LIMIT) } else { None };
                if let Some(spec) = spec { self.body = with_filter(&self.body, spec) }
            }
            req.method("POST").header("Content-Type", format!("application/x-{}-request", self.service))
                .header("Accept", format!("application/x-{}-result", self.service))
                .body(std::mem::take(&mut self.body)).map_err(|e| io(e.to_string())).and_then(|r| agent().run(r).map_err(|e| io(e.to_string())))
        } else {
            req.method("GET").body(()).map_err(|e| io(e.to_string())).and_then(|r| agent().run(r).map_err(|e| io(e.to_string())))
        }?;
        match reply.status().as_u16() {
            200 => Ok(reply.into_body().into_reader()),
            // GitHub answers a private repo it will not show this account as if it were not there
            401 | 403 | 404 => Err(io(format!("no access (HTTP {})", reply.status().as_u16()))),
            s => Err(io(format!("GitHub answered HTTP {s}"))),
        }
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    // timeout_recv_body is the time allowed for the WHOLE reply, not for a pause in it: at 120 seconds (0.3.13) a
    // phone on mobile data was cut off a fifth of the way into the book ("timeout: receive body"). A slow network
    // gets an hour, as the installer download does.
    // ponytail: ureq has no "nothing arrived for N seconds"; a connection that goes silent waits out the hour (or
    // the phone's own TCP timeout). Read in a thread with a watchdog if people meet that.
    let s = std::time::Duration::from_secs;
    AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false)
        .timeout_connect(Some(s(30))).timeout_recv_response(Some(s(300))).timeout_recv_body(Some(s(3600)))
        .build().into())
}

// ---------------------------------------------------------------- clone and update

fn no_access(e: &git2::Error) -> bool { e.message().contains("no access") }

fn no_space(e: &str) -> bool { e.contains("No space left") || e.contains("ENOSPC") }

/// Fetch options; `f` hears the fraction of objects received. `first`: a new copy, which takes only the newest
/// commit (shallow). An update must not ask for that again: "only the newest commit" makes the server forget what
/// this copy already has and send every file of the tip once more, kept beside the old ones (0.3.13 did, 364 MB
/// for each update of the book). Asked plainly, it sends what is new since the commit this copy stops at.
/// libgit2 cannot fetch shallow from a folder (file://), which only the test uses for repos it moves.
fn options<'a>(url: &str, first: bool, f: &'a dyn Fn(f32)) -> FetchOptions<'a> {
    let mut cb = RemoteCallbacks::new();
    cb.transfer_progress(move |p| { if p.total_objects() > 0 { f(p.received_objects() as f32 / p.total_objects() as f32) } true });
    let mut o = FetchOptions::new();
    o.remote_callbacks(cb).depth(if first && !url.starts_with("file://") { 1 } else { 0 }).download_tags(AutotagOption::None);
    o
}

/// Clones `url` into `dest` (new or empty): the tip of `branch`, or of the repo's default branch.
fn get(url: &str, dest: &Path, branch: Option<&str>, f: &dyn Fn(f32)) -> Result<Repository, git2::Error> {
    https();
    // by hand, not libgit2's clone: its checkout stops at the first file that was left on GitHub
    let repo = Repository::init(dest)?;
    {
        let mut origin = repo.remote("origin", url)?;
        origin.fetch(&[] as &[&str], Some(&mut options(url, true, f)), None)?;
        let branch = match branch {
            Some(b) => b.to_string(),
            None => origin.default_branch()?.as_str().ok().and_then(|r| r.strip_prefix("refs/heads/")).unwrap_or("main").to_string(),
        };
        let tip = repo.find_commit(repo.refname_to_id(&format!("refs/remotes/origin/{branch}"))?)?;
        repo.reference(&format!("refs/heads/{branch}"), tip.id(), true, "aIwalk: download")?;
        repo.set_head(&format!("refs/heads/{branch}"))?;
        place(&repo, None, &tip.tree()?)?;
    }
    Ok(repo)
}

/// Makes the files on the phone those of `new`, coming from `old` (None: a new, empty copy). Files of `new` that
/// were left on GitHub are not written; the index holds them as skip-worktree, which is git's own way of saying
/// "not here on purpose". Coming from `old`, a file the person changed that `new` changes too stops it (Conflict).
fn place(repo: &Repository, old: Option<&git2::Tree>, new: &git2::Tree) -> Result<(), git2::Error> {
    let odb = repo.odb()?;
    let (mut here, mut later) = (vec![], vec![]);
    new.walk(TreeWalkMode::PreOrder, |dir, e| {
        let Ok(name) = e.name() else { return TreeWalkResult::Ok };
        match e.kind() {
            Some(ObjectType::Tree) => {}
            Some(ObjectType::Blob) if !odb.exists(e.id()) => later.push((format!("{dir}{name}"), e.id(), e.filemode())),
            _ => here.push(format!("{dir}{name}")),
        }
        TreeWalkResult::Ok
    })?;
    // files the team deleted are named too: only named paths are touched
    let mut paths: std::collections::HashSet<String> = here.into_iter().collect();
    if let Some(old) = old {
        old.walk(TreeWalkMode::PreOrder, |dir, e| {
            if let (Ok(name), false) = (e.name(), e.kind() == Some(ObjectType::Tree)) {
                let path = format!("{dir}{name}");
                if !later.iter().any(|(l, _, _)| l == &path) { paths.insert(path); }
            }
            TreeWalkResult::Ok
        })?;
    }
    if !paths.is_empty() {   // no path named would mean every path
        let mut co = CheckoutBuilder::new();
        if old.is_some() { co.safe(); } else { co.force(); }
        co.disable_pathspec_match(true);
        for p in &paths { co.path(p.as_str()); }
        repo.checkout_tree(new.as_object(), Some(&mut co))?;
    }
    let mut index = repo.index()?;
    index.read(true)?;
    for (path, id, mode) in later {
        // a copy of this file's earlier version is on the phone (fetched, or from before it grew past the limit) and
        // the person did not change it: it goes, or it would read as their change and be sent over the team's new one
        if let (Some(was), Some(file)) = (index.get_path(Path::new(&path), 0), repo.workdir().map(|w| w.join(&path))) {
            if was.id != id && file.is_file() && git2::Oid::hash_file(ObjectType::Blob, &file).is_ok_and(|h| h == was.id) { let _ = std::fs::remove_file(&file); }
        }
        let zero = IndexTime::new(0, 0);
        // 0x4000 in flags: this entry has extended flags; 1 << 14 there: skip-worktree
        index.add(&IndexEntry { ctime: zero, mtime: zero, dev: 0, ino: 0, mode: mode as u32, uid: 0, gid: 0, file_size: 0, id,
                                flags: 0x4000, flags_extended: 1 << 14, path: path.into_bytes() })?;
    }
    index.write()
}

/// Why a fast-forward did not happen.
enum NotMoved { Changed, Diverged, Failed(git2::Error) }

impl From<git2::Error> for NotMoved { fn from(e: git2::Error) -> Self { NotMoved::Failed(e) } }

/// Fetches the branch checked out in `dir` and moves to GitHub's tip of it, fast-forward only. The checkout refuses
/// to overwrite a file the person changed that the team's version changes too (Changed); their other changes stay.
fn forward(dir: &Path, f: &dyn Fn(f32)) -> Result<(), NotMoved> {
    https();
    let repo = Repository::open(dir)?;
    let head = repo.head()?;
    let (Some(name), Some(here)) = (head.name().ok().map(String::from), head.target()) else { return Err(git2::Error::from_str("no branch checked out").into()) };
    let branch = name.strip_prefix("refs/heads/").ok_or_else(|| git2::Error::from_str("no branch checked out"))?;
    let theirs = format!("refs/remotes/origin/{branch}");
    let before = repo.refname_to_id(&theirs).ok();
    let mut origin = repo.find_remote("origin")?;
    let url = origin.url().unwrap_or_default().to_string();
    origin.fetch(&[&format!("+refs/heads/{branch}:{theirs}")], Some(&mut options(&url, false, f)), None)?;
    let new = repo.refname_to_id(&theirs)?;
    if new == here { return Ok(()) }
    // shallow history cannot prove a fast-forward. Every tip GitHub ever gave is in the reflog of origin's branch
    // (a Get latest refused for the person's changes leaves HEAD on an older one); a commit made here (by another
    // app) is in none of them
    let given = |id| before == Some(id) || repo.reflog(&theirs).is_ok_and(|log| log.iter().any(|e| e.id_new() == id));
    if !given(here) { return Err(NotMoved::Diverged) }
    match place(&repo, Some(&repo.find_commit(here)?.tree()?), &repo.find_commit(new)?.tree()?) {
        Err(e) if e.code() == ErrorCode::Conflict => return Err(NotMoved::Changed),
        r => r?,
    }
    repo.reference(&name, new, true, "aIwalk: get latest")?;
    Ok(())
}

/// Names of the on-demand submodules (System/vault_rules.json, as vault.rs reads it).
fn on_demand(tree: &Path) -> Vec<String> {
    let rules = std::fs::read_to_string(tree.join("System/vault_rules.json")).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&rules).ok()
        .and_then(|v| v["repos"]["on_demand"].as_array().cloned()).unwrap_or_default()
        .iter().filter_map(|n| n.as_str().map(String::from)).collect()
}

/// A submodule's URL as https: git@github.com: rewritten (no SSH key on a phone), ../x resolved against the book's.
fn sub_url(book: &Repository, url: &str) -> String {
    let url = url.replace("git@github.com:", "https://github.com/");
    if !url.starts_with("./") && !url.starts_with("../") { return url }
    let mut base = book.find_remote("origin").ok().and_then(|r| r.url().ok().map(String::from)).unwrap_or_default();
    let mut rest = url.as_str();
    loop {
        if let Some(r) = rest.strip_prefix("./") { rest = r }
        else if let Some(r) = rest.strip_prefix("../") { base.truncate(base.trim_end_matches('/').rfind('/').unwrap_or(0)); rest = r }
        else { break }
    }
    format!("{}/{rest}", base.trim_end_matches('/'))
}

fn empty(dir: &Path) -> bool { std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none()) }

/// Brings each submodule in (a first time) or forward (`update`), one at a time. Progress runs from `from` to 1.
fn each(tree: &Path, update: bool, from: f32, progress: Progress) -> Vec<(String, Got)> {
    https();
    let Ok(book) = Repository::open(tree) else { return vec![] };
    let lazy = on_demand(tree);
    let subs: Vec<(String, String, String, Option<String>)> = book.submodules().unwrap_or_default().iter().filter_map(|s| Some((
        s.name().ok()?.to_string(), s.path().to_string_lossy().replace('\\', "/"),
        sub_url(&book, s.url().ok()??), s.branch().ok().flatten().map(String::from)))).collect();
    let mut rows = vec![];
    for (i, (name, path, url, branch)) in subs.iter().enumerate() {
        let n = subs.len() as f32;
        let at = |part: f32| from + (1.0 - from) * (i as f32 + part) / n;
        let say = format!("Bringing in {path} ({} of {})", i + 1, subs.len());
        progress(at(0.0), &say);
        let f = |part: f32| progress(at(part), &say);
        let dir = tree.join(path);
        let got = if lazy.contains(name) {
            // the folder's list of files and its small notes; the papers themselves one at a time, when asked for.
            // Only from a server that can leave files out: anything else would send all of it.
            let base = url.trim_end_matches('/').to_string();
            { let mut d = ON_DEMAND.lock().unwrap(); if !d.contains(&base) { d.push(base.clone()) } }
            if dir.join(".git").exists() { if update { let _ = forward(&dir, &f); } }
            else if empty(&dir) && !url.starts_with("file://") {
                let made = !dir.exists();
                if get(&url, &dir, branch.as_deref(), &f).is_err() || !FILTERS.lock().unwrap().contains(&base) {
                    let _ = std::fs::remove_dir_all(&dir);
                    if !made { let _ = std::fs::create_dir_all(&dir); }
                }
            }
            Got::Later
        } else if dir.join(".git").exists() {
            match if update { forward(&dir, &f) } else { Ok(()) } {
                Ok(()) => Got::Ok,
                Err(NotMoved::Changed | NotMoved::Diverged) => Got::Kept,
                Err(NotMoved::Failed(_)) => Got::Skipped,   // no access any more, or no network: the notes stay as they were
            }
        } else if !empty(&dir) {
            Got::Skipped   // files there that are not a copy: never cloned over
        } else {
            let made = !dir.exists();
            match get(&url, &dir, branch.as_deref(), &f) {
                Ok(_) => Got::Ok,
                Err(_) => {
                    // an unreadable folder stays an empty directory, as on a computer
                    let _ = std::fs::remove_dir_all(&dir);
                    if !made { let _ = std::fs::create_dir_all(&dir); }
                    Got::Skipped
                }
            }
        };
        rows.push((path.clone(), got));
    }
    rows
}

/// Clones `url` into `dest`: the book is the first half of the progress, its folders the second.
pub fn clone(url: &str, dest: &Path, progress: Progress) -> Result<Vec<(String, Got)>, String> {
    progress(0.0, "Connecting to GitHub");
    if !empty(dest) { return Err(format!("{} already has files in it; move them away first", dest.display())) }
    let made = !dest.exists();
    std::fs::create_dir_all(dest).map_err(|e| format!("Could not make the folder {}: {e}", dest.display()))?;
    if let Err(e) = get(url, dest, None, &|f| progress(f / 2.0, &format!("Downloading the shared notes ({:.0}%)", f * 100.0))) {
        let _ = std::fs::remove_dir_all(dest);
        if !made { let _ = std::fs::create_dir_all(dest); }
        return Err(if no_access(&e) { "No access to this vault. Sign in first, or ask an owner for access.".into() }
            else if no_space(e.message()) { "The phone is out of space. Free some space and download again.".into() }
            else { format!("Download failed: {}", e.message()) });
    }
    // kept with the copy, so every later Get latest asks GitHub the same way
    if let Ok(mut c) = Repository::open(dest).and_then(|r| r.config()) { let _ = c.set_bool("aiwalk.small", SMALL.load(std::sync::atomic::Ordering::Relaxed)); }
    let rows = each(dest, false, 0.5, progress);
    progress(1.0, "Done");
    Ok(rows)
}

/// Moves the book forward (fast-forward only), then each readable folder, even when the book could not move.
pub fn pull(tree: &Path, progress: Progress) -> (Result<(), String>, Vec<(String, Got)>) {
    recall(tree);
    progress(0.0, "Getting the latest shared notes");
    let book = forward(tree, &|f| progress(f * 0.2, "Getting the latest shared notes")).map_err(|e| match e {
        NotMoved::Changed => "Could not update: you changed files here that the team changed too. Your changes are kept.".to_string(),
        NotMoved::Diverged => "Could not update: your copy has changes that are not on GitHub yet".to_string(),
        NotMoved::Failed(e) if no_access(&e) => "Could not update: no access. Sign in again, or ask an owner for access.".to_string(),
        NotMoved::Failed(e) if no_space(e.message()) => "Could not update: the phone is out of space".to_string(),
        NotMoved::Failed(e) => format!("Could not update: {}", e.message()),
    });
    let rows = each(tree, true, 0.2, progress);
    progress(1.0, "Done");
    (book, rows)
}

// ---------------------------------------------------------------- send changes

/// Files in `repo` that differ from its last commit: changed, new or gone. Folders that are repos of their own are
/// left to their own turn.
fn changed(repo: &Repository) -> Result<Vec<String>, git2::Error> {
    let mut o = StatusOptions::new();
    o.include_untracked(true).recurse_untracked_dirs(true).exclude_submodules(true);
    // libgit2's status does not know skip-worktree: a file left on GitHub would read as deleted here
    let index = repo.index()?;
    let left = |p: &str| index.get_path(Path::new(p), 0).is_some_and(|e| e.flags_extended & (1 << 14) != 0)
        && !repo.workdir().is_some_and(|w| w.join(p).exists());
    // new files inside dot-folders (tool state, the vault's .trash) are never vault content
    Ok(repo.statuses(Some(&mut o))?.iter().filter(|e| !(e.status().contains(git2::Status::WT_NEW) && e.path().is_ok_and(|p| p.starts_with('.'))))
        .filter_map(|e| e.path().ok().map(String::from)).filter(|p| !p.ends_with('/') && !left(p)).collect())
}

/// Every changed file under `tree`, the book's and each folder's, as paths from the vault's top.
pub fn pending(tree: &Path) -> Vec<String> {
    let Ok(book) = Repository::open(tree) else { return vec![] };
    let mut all = changed(&book).unwrap_or_default();
    for path in folders(&book, tree) {
        if let Ok(r) = Repository::open(tree.join(&path)) { all.extend(changed(&r).unwrap_or_default().into_iter().map(|p| format!("{path}/{p}"))) }
    }
    all
}

/// The book's folders that are copies on this phone.
fn folders(book: &Repository, tree: &Path) -> Vec<String> {
    book.submodules().unwrap_or_default().iter().map(|s| s.path().to_string_lossy().replace('\\', "/")).filter(|p| tree.join(p).join(".git").exists()).collect()
}

/// The files left on GitHub: known to git at these paths (from the vault's top), not on the phone.
pub fn left(tree: &Path) -> Vec<String> {
    let Ok(book) = Repository::open(tree) else { return vec![] };
    let of = |repo: &Repository, dir: &Path, prefix: &str| -> Vec<String> {
        repo.index().map(|i| i.iter().filter(|e| e.flags_extended & (1 << 14) != 0)
            .filter_map(|e| String::from_utf8(e.path).ok()).filter(|p| !dir.join(p).exists()).map(|p| format!("{prefix}{p}")).collect()).unwrap_or_default()
    };
    let mut all = of(&book, tree, "");
    for path in folders(&book, tree) {
        if let Ok(r) = Repository::open(tree.join(&path)) { all.extend(of(&r, &tree.join(&path), &format!("{path}/"))) }
    }
    all.sort_by_key(|p| p.to_lowercase());
    all
}

/// The folders of the vault at `tree` that are on demand (the papers), as paths from its top.
pub fn on_demand_folders(tree: &Path) -> Vec<String> {
    let lazy = on_demand(tree);
    Repository::open(tree).map(|b| b.submodules().unwrap_or_default().iter()
        .filter(|s| s.name().is_ok_and(|n| lazy.iter().any(|l| l == n))).map(|s| s.path().to_string_lossy().replace('\\', "/")).collect()).unwrap_or_default()
}

/// Brings one file left on GitHub to its place. `get(repo's address, blob id, where to write)` fetches the content.
pub fn fetch(tree: &Path, path: &str, get: &dyn Fn(&str, &str, &Path) -> Result<(), String>) -> Result<(), String> {
    let say = |e: git2::Error| e.message().to_string();
    let book = Repository::open(tree).map_err(say)?;
    // the folder the path is in (the longest that matches), else the book itself
    let mut subs = folders(&book, tree);
    subs.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let (dir, inner) = match subs.iter().find(|s| path.starts_with(&format!("{s}/"))) {
        Some(s) => (tree.join(s), &path[s.len() + 1..]),
        None => (tree.to_path_buf(), path),
    };
    let repo = Repository::open(&dir).map_err(say)?;
    let entry = repo.index().map_err(say)?.get_path(Path::new(inner), 0).ok_or("git does not know this file")?;
    let url = repo.find_remote("origin").map_err(say)?.url().unwrap_or_default().to_string();
    let file = dir.join(inner);
    if file.exists() { return Ok(()) }
    if let Some(p) = file.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
    // written beside its place and moved in whole: a download that stops half way leaves no half file to be sent
    let part = file.with_file_name(format!(".{}.part", file.file_name().unwrap_or_default().to_string_lossy()));
    let done = get(&url, &entry.id.to_string(), &part)
        .and_then(|()| git2::Oid::hash_file(ObjectType::Blob, &part).map_err(say))
        .and_then(|h| if h == entry.id { Ok(()) } else { Err("what arrived is not the file git recorded".to_string()) })
        .and_then(|()| std::fs::rename(&part, &file).map_err(|e| e.to_string()));
    if done.is_err() { let _ = std::fs::remove_file(&part); }
    done
}

/// A file's content from GitHub by its blob id, as the signed-in account.
fn github_blob(url: &str, id: &str, to: &Path) -> Result<(), String> {
    let (owner, repo) = exo_core::vault_ship::github_repo(url.trim_end_matches('/')).ok_or("this folder is not on GitHub")?;
    let mut req = ureq::http::Request::builder().method("GET").uri(format!("https://api.github.com/repos/{owner}/{repo}/git/blobs/{id}"))
        .header("Accept", "application/vnd.github.raw+json").header("User-Agent", "aiwalk-system-setup");
    if let Some(t) = TOKEN.lock().unwrap().clone() { req = req.header("Authorization", format!("Bearer {t}")); }
    let mut reply = agent().run(req.body(()).map_err(|e| e.to_string())?).map_err(|e| format!("GitHub did not answer: {e}"))?;
    match reply.status().as_u16() {
        200 => {}
        401 | 403 | 404 => return Err("no access. Sign in again, or ask an owner for access".into()),
        s => return Err(format!("GitHub answered HTTP {s}")),
    }
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    std::io::copy(&mut reply.body_mut().as_reader(), &mut file).map(|_| ()).map_err(|e| format!("the download stopped: {e}"))
}

/// One changed file, sorted the way the vault plugin's Sync vault page sorts it on a computer (ship.ts scan).
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct Change {
    pub path: String,
    /// "changed", "new", "deleted", "renamed" or "conflict"
    pub code: &'static str,
    /// "changed", "new" (not in git yet), "empty" (a new file with nothing in it) or "held" (not to be uploaded)
    pub group: &'static str,
    pub why: String,
    /// a login token or key: stays unticked whatever is pressed
    pub locked: bool,
    /// its size in MB when over BIG_MB, else 0: left unticked for the person to tick on purpose
    pub big: f32,
}

/// Over this a file is left unticked: git keeps every version of a file for good.
pub const BIG_MB: f32 = 30.0;

/// ".bak", ".bak2", ".bak.md": a backup copy.
fn backup(path: &str) -> bool {
    path.match_indices(".bak").any(|(i, _)| { let rest = path[i + 4..].trim_start_matches(|c: char| c.is_ascii_digit()); rest.is_empty() || rest.starts_with('.') })
}

/// A new file with nothing in it (an Untitled.md never written in).
fn blank(file: &Path) -> bool {
    let Ok(m) = std::fs::metadata(file) else { return false };
    m.is_file() && (m.len() == 0 || (m.len() < 4096 && std::fs::read_to_string(file).is_ok_and(|t| t.trim().is_empty())))
}

/// Every change under `tree`, the book's and each folder's, grouped. `may_push(folder's address)` says whether the
/// account may change that folder; where it may not, the folder's changes are listed as held.
pub fn scan(tree: &Path, may_push: &dyn Fn(&str) -> bool) -> Vec<Change> {
    let Ok(book) = Repository::open(tree) else { return vec![] };
    let of = |repo: &Repository, dir: &Path, prefix: &str, writable: bool| -> Vec<Change> {
        let mut o = StatusOptions::new();
        o.include_untracked(true).recurse_untracked_dirs(true).exclude_submodules(true);
        let Ok(st) = repo.statuses(Some(&mut o)) else { return vec![] };
        let index = repo.index().ok();
        st.iter().filter_map(|e| {
            let p = e.path().ok()?.to_string();
            let file = dir.join(&p);
            // a file left on GitHub is not a deletion (libgit2's status does not know skip-worktree)
            if p.ends_with('/') || (!file.exists() && index.as_ref().and_then(|i| i.get_path(Path::new(&p), 0)).is_some_and(|x| x.flags_extended & (1 << 14) != 0)) { return None }
            let s = e.status();
            let fresh = s.contains(git2::Status::WT_NEW);
            // new files inside dot-folders (tool state, Obsidian's trash) are never vault content
            if fresh && prefix.is_empty() && p.starts_with('.') { return None }
            let code = if s.contains(git2::Status::CONFLICTED) { "conflict" } else if fresh || s.contains(git2::Status::INDEX_NEW) { "new" }
                else if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) { "deleted" }
                else if s.intersects(git2::Status::WT_RENAMED | git2::Status::INDEX_RENAMED) { "renamed" } else { "changed" };
            let mut c = Change { path: format!("{prefix}{p}"), code, group: if fresh { "new" } else { "changed" }, why: String::new(), locked: false, big: 0.0 };
            if fresh && blank(&file) { c.group = "empty"; c.why = "empty file".into(); }
            if exo_core::vault_ship::looks_secret(&p) && code != "deleted" { c.group = "held"; c.why = "looks like a login token or key".into(); c.locked = true; }
            else if !writable { c.group = "held"; c.why = "a folder you may read but not change".into(); c.locked = true; }
            else if backup(&p) { c.group = "held"; c.why = "backup copy".into(); }
            if code != "deleted" && c.group != "held" {
                let mb = std::fs::metadata(&file).map_or(0.0, |m| m.len() as f32 / 1048576.0);
                if mb > BIG_MB { c.big = mb }
            }
            Some(c)
        }).collect()
    };
    let mut all = of(&book, tree, "", true);
    for path in folders(&book, tree) {
        let Ok(r) = Repository::open(tree.join(&path)) else { continue };
        let mut inside = of(&r, &tree.join(&path), &format!("{path}/"), true);
        if inside.is_empty() { continue }
        // asked only for a folder that has changes: one question to GitHub each
        let url = r.find_remote("origin").ok().and_then(|o| o.url().ok().map(String::from)).unwrap_or_default();
        if !may_push(&url) { inside = of(&r, &tree.join(&path), &format!("{path}/"), false) }
        all.extend(inside);
    }
    all
}

/// One commit as the page lists it.
#[derive(serde::Serialize, Debug, Clone)]
pub struct Row { pub hash: String, pub who: String, pub when: i64, pub what: String }

fn row(c: &git2::Commit) -> Row {
    Row { hash: c.id().to_string()[..7].to_string(), who: c.author().name().unwrap_or_default().to_string(), when: c.time().seconds(),
          what: c.summary().ok().flatten().unwrap_or_default().to_string() }
}

/// What the team pushed that this copy does not have yet (asked from GitHub now, nothing on the phone changed),
/// and what arrived in the last two days. The shared notes only, as on a computer.
pub fn incoming(tree: &Path, f: &dyn Fn(f32)) -> Result<(Vec<Row>, Vec<Row>), String> {
    recall(tree);
    https();
    let say = |e: git2::Error| if no_access(&e) { "no access. Sign in again, or ask an owner for access".to_string() } else { e.message().to_string() };
    let repo = Repository::open(tree).map_err(say)?;
    let head = repo.head().map_err(say)?;
    let (name, here) = (head.name().map_err(say)?.to_string(), head.target().ok_or("no branch checked out")?);
    let branch = name.strip_prefix("refs/heads/").ok_or("no branch checked out")?;
    let theirs = format!("refs/remotes/origin/{branch}");
    {
        let mut origin = repo.find_remote("origin").map_err(say)?;
        let url = origin.url().unwrap_or_default().to_string();
        origin.fetch(&[&format!("+refs/heads/{branch}:{theirs}")], Some(&mut options(&url, false, f)), None).map_err(say)?;
    }
    let walk = |from: git2::Oid, hide: Option<git2::Oid>, keep: &dyn Fn(&git2::Commit) -> bool| -> Vec<Row> {
        let Ok(mut w) = repo.revwalk() else { return vec![] };
        if w.push(from).is_err() { return vec![] }
        if let Some(h) = hide { let _ = w.hide(h); }
        w.flatten().filter_map(|id| repo.find_commit(id).ok()).take(200).filter(|c| keep(c)).map(|c| row(&c)).collect()
    };
    let new = repo.refname_to_id(&theirs).map_err(say)?;
    let rows = if new == here { vec![] } else { walk(new, Some(here), &|_| true) };
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) - 2 * 86400;
    let recent = walk(here, None, &|c| c.parent_count() <= 1 && c.time().seconds() >= since);
    Ok((rows, recent))
}

/// Moves `files` (paths from the vault's top) into the vault's own trash, .trash at its top: where Obsidian puts
/// what it deletes, so a wrong press can be undone there. Ok(how many went).
pub fn trash(tree: &Path, files: &[String]) -> Result<usize, String> {
    let bin = tree.join(".trash");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let mut n = 0;
    for f in files {
        if f.split('/').any(|part| part.is_empty() || part == ".." || part == ".") { return Err(format!("{f}: not a file of this vault")) }
        let from = tree.join(f);
        if !from.is_file() { continue }
        let name = from.file_name().unwrap_or_default().to_string_lossy().into_owned();
        // a second file of the same name gets a number, as Obsidian does
        let to = (0..1000).map(|i| if i == 0 { bin.join(&name) } else { bin.join(format!("{i} {name}")) }).find(|p| !p.exists()).ok_or("the trash is full of files with that name")?;
        std::fs::rename(&from, &to).map_err(|e| format!("{f}: {e}"))?;
        n += 1;
    }
    Ok(n)
}

/// Today's local date, as libgit2 reckons the time zone.
fn today() -> Option<(i64, i64, i64)> {
    let now = Signature::now("x", "x@x").ok()?.when();
    let off = now.offset_minutes();
    exo_core::vault_ship::local_date(&format!("x <x@x> {} {}{:02}{:02}", now.seconds(), if off < 0 { '-' } else { '+' }, off.abs() / 60, off.abs() % 60))
}

/// Commits what changed in `dir` (and, for the book, the folders in `links` that just moved) on top of the team's
/// newest and pushes it. Ok((files sent, files held back because their names look like a key)).
/// `only`, when given, are the files to send (paths inside `dir`); every other change stays on the phone.
fn send_one(vault: &Path, dir: &Path, msg: &str, who: (&str, &str), links: &[String], only: Option<&[String]>, f: &dyn Fn(f32)) -> Result<(usize, Vec<String>), String> {
    https();
    let say = |e: git2::Error| e.message().to_string();
    let repo = Repository::open(dir).map_err(say)?;
    let mut files = changed(&repo).map_err(say)?;
    if let Some(only) = only { files.retain(|p| only.contains(p)) }
    // a deletion may go (that is how a secret leaves the repo); adding or changing one may not
    let lines: Vec<String> = files.iter().map(|p| format!("{}\t{p}", if dir.join(p).exists() { 'A' } else { 'D' })).collect();
    let held = exo_core::vault_ship::leaks(&lines.iter().map(String::as_str).collect::<Vec<_>>(), &[]);
    files.retain(|p| !held.contains(p));
    if files.is_empty() && links.is_empty() { return Ok((0, held)) }
    for attempt in 1..=2 {
        // the team's newest first, so the commit sits on top of it and there is never anything to merge
        forward(dir, f).map_err(|e| match e {
            NotMoved::Changed => "you changed a file here that the team changed too. Nothing was sent and your text is kept; sort it out on a computer".to_string(),
            NotMoved::Diverged => "this copy has a commit from another app that is not on GitHub".to_string(),
            NotMoved::Failed(e) if no_access(&e) => "no access. Sign in again, or ask an owner for access".to_string(),
            NotMoved::Failed(e) => say(e),
        })?;
        let head = repo.head().map_err(say)?;
        let name = head.name().map_err(say)?.to_string();
        let branch = name.strip_prefix("refs/heads/").ok_or("no branch checked out")?.to_string();
        let parent = head.peel_to_commit().map_err(say)?;
        if attempt == 1 {
            // the two checks vault ship makes on a computer (vaultcheck.rs), on the team's newest. A primary log's
            // ownership block is brought up to date; a new link that resolves to nothing stops the upload.
            if dir == vault {
                for p in files.iter().filter(|p| exo_core::vault_ship::is_primary(p) && dir.join(p).exists()) {
                    if let Ok(Some(new)) = crate::vaultcheck::ownership(vault, &dir.join(p), p, &today) { let _ = crate::vaultcheck::write_text(&dir.join(p), &new); }
                }
            }
            let last = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            let before = |p: &str| last.as_ref().and_then(|t| t.get_path(Path::new(p)).ok()).and_then(|e| repo.find_blob(e.id()).ok())
                .map(|b| String::from_utf8_lossy(b.content()).into_owned()).unwrap_or_default();
            // a file left on GitHub exists, though it is not on the phone: a link to it is not a broken one
            let also: std::collections::HashSet<String> = left(vault).iter().flat_map(|p| {
                let n = p.rsplit('/').next().unwrap_or(p).to_string();
                [n.strip_suffix(".md").map(String::from), Some(n)]
            }).flatten().collect();
            let bad = crate::vaultcheck::broken_links(vault, dir, &files, &before, &also)?;
            if !bad.is_empty() { return Err(format!("these links lead to no file in the vault (a file that is not a note needs its extension): {}", bad.join("; "))) }
        }
        let mut index = repo.index().map_err(say)?;
        // read again from disk: forward() moved it through a handle of its own, and the copy this handle read for
        // the list of changes is from before; a tree written from that one would drop what the team just added
        index.read(true).map_err(say)?;
        for p in files.iter().chain(links) {
            if dir.join(p).exists() { index.add_path(Path::new(p)) } else { index.remove_path(Path::new(p)) }.map_err(say)?;
        }
        index.write().map_err(say)?;
        let tree = repo.find_tree(index.write_tree().map_err(say)?).map_err(say)?;
        if tree.id() == parent.tree_id() { return Ok((0, held)) }
        let sig = Signature::now(who.0, who.1).map_err(say)?;
        let made = repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent]).map_err(say)?;
        // a push GitHub refuses still returns Ok: what it said about the branch comes through the callback
        let refused = std::cell::RefCell::new(None::<String>);
        let pushed = {
            let mut cb = RemoteCallbacks::new();
            cb.push_update_reference(|_, status| { *refused.borrow_mut() = status.map(String::from); Ok(()) });
            let mut o = PushOptions::new();
            o.remote_callbacks(cb);
            repo.find_remote("origin").and_then(|mut origin| origin.push(&[format!("refs/heads/{branch}:refs/heads/{branch}")], Some(&mut o)))
        };
        let problem = pushed.err().map(|e| if no_access(&e) { "you may read this folder but not change it. Ask an owner for write access".to_string() } else { say(e) })
            .or(refused.into_inner());
        let Some(problem) = problem else {
            repo.reference(&format!("refs/remotes/origin/{branch}"), made, true, "aIwalk: sent").map_err(say)?;
            return Ok((files.len(), held))
        };
        // not sent: the commit is taken back (the files stay as they are, still counted as changed), so the copy
        // never holds a commit GitHub does not have. Someone pushing in between gets one more try on top of theirs.
        repo.reference(&name, parent.id(), true, "aIwalk: not sent").map_err(say)?;
        if attempt == 2 || problem.contains("write access") { return Err(problem) }
    }
    unreachable!()
}

/// Sends the changes under `tree`: each folder first, then the book with the folders that moved. `only`, when
/// given, are the files to send, as paths from the vault's top; without it, every change goes. Ok(what was sent).
pub fn send(tree: &Path, msg: &str, who: (&str, &str), only: Option<&[String]>, progress: Progress) -> Result<String, String> {
    recall(tree);
    let book = Repository::open(tree).map_err(|e| e.message().to_string())?;
    let subs = folders(&book, tree);
    let (mut sent, mut moved, mut held, mut problems) = (0, vec![], vec![], vec![]);
    for (i, path) in subs.iter().enumerate() {
        let at = |part: f32| 0.8 * (i as f32 + part) / subs.len() as f32;
        progress(at(0.0), &format!("Sending {path}"));
        let inner: Option<Vec<String>> = only.map(|o| o.iter().filter_map(|f| f.strip_prefix(&format!("{path}/")).map(String::from)).collect());
        if inner.as_ref().is_some_and(|i| i.is_empty()) { continue }
        match send_one(tree, &tree.join(path), msg, who, &[], inner.as_deref(), &|p| progress(at(p), &format!("Sending {path}"))) {
            Ok((n, h)) => { if n > 0 { sent += n; moved.push(path.clone()) } held.extend(h.into_iter().map(|p| format!("{path}/{p}"))) }
            Err(e) => problems.push(format!("{path}: {e}")),
        }
    }
    progress(0.8, "Sending the shared notes");
    match send_one(tree, tree, msg, who, &moved, only, &|p| progress(0.8 + 0.2 * p, "Sending the shared notes")) {
        Ok((n, h)) => { sent += n; held.extend(h) }
        Err(e) => problems.push(format!("Shared notes: {e}")),
    }
    progress(1.0, "Done");
    let mut text = match sent { 0 => "Nothing to send".to_string(), 1 => "Sent 1 file".to_string(), n => format!("Sent {n} files") };
    if !held.is_empty() { text += &format!(". Held back, the name looks like a key or password: {}", held.join(", ")) }
    if problems.is_empty() { Ok(text) } else { Err(format!("{}. {text}", problems.join(". "))) }
}

// ---------------------------------------------------------------- the phone's commands

/// A long job on the phone (a download, an upload): tells the page how far it is, and writes the same words to a
/// file the app's foreground service reads (WorkService.kt), so the notification that keeps Android from stopping
/// the job says what it is doing. The file goes when the job ends, however it ends, which is how the service knows
/// to stop: the page may be asleep in the background by then and cannot be relied on to say so.
#[cfg(target_os = "android")]
struct Working { app: tauri::AppHandle, repo: String, file: std::path::PathBuf, last: std::cell::RefCell<String> }

#[cfg(target_os = "android")]
fn work_path() -> std::path::PathBuf { crate::home().join("work.status") }

#[cfg(target_os = "android")]
impl Working {
    fn new(app: &tauri::AppHandle, repo: &str, text: &str) -> Self {
        let w = Working { app: app.clone(), repo: repo.to_string(), file: work_path(), last: Default::default() };
        w.say(0.0, text);
        w
    }
    fn say(&self, f: f32, text: &str) {
        use tauri::Emitter;
        let _ = self.app.emit("vault-progress", (&self.repo, f, text));
        // progress comes many times a second; the file is written when the words change
        if *self.last.borrow() != text { let _ = std::fs::write(&self.file, text); *self.last.borrow_mut() = text.to_string(); }
    }
}

#[cfg(target_os = "android")]
impl Drop for Working { fn drop(&mut self) { let _ = std::fs::remove_file(&self.file); } }

/// The file a running job writes its words to, for the page to hand to the foreground service.
#[cfg(target_os = "android")]
#[tauri::command]
pub fn work_file() -> String { work_path().to_string_lossy().into_owned() }

/// Where the vaults go: Documents/aIwalk in shared storage, the only kind of folder Obsidian for Android can open.
// ponytail: the owner of the phone (user 0); a work profile has its own /storage/emulated/<n>. Ask Android
// (Environment.getExternalStorageDirectory) when someone keeps the vault in a work profile.
#[cfg(target_os = "android")]
const SHARED: &str = "/storage/emulated/0/Documents/aIwalk";

#[cfg(target_os = "android")]
fn dest(repo: &str) -> std::path::PathBuf { Path::new(SHARED).join(repo.rsplit('/').next().unwrap_or(repo)) }

#[cfg(target_os = "android")]
#[derive(serde::Serialize)]
pub struct Local {
    copies: std::collections::BTreeMap<String, String>,
    obsidian: bool,
    /// this app can add a file in Documents/aIwalk, tried with a small one. Android 11 and later allow that much to
    /// any app; changing files Obsidian wrote needs "All files access", which the page asks Android (MainActivity.kt)
    writable: bool,
}

#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_local(repos: Vec<String>) -> Local {
    let root = Path::new(SHARED);
    let probe = root.join(".probe");
    let writable = std::fs::create_dir_all(root).is_ok() && std::fs::write(&probe, b"aIwalk").is_ok() && std::fs::remove_file(&probe).is_ok();
    let copies = repos.into_iter().filter_map(|r| { let d = dest(&r); d.join(".git").exists().then(|| (r, d.to_string_lossy().into())) }).collect();
    Local { copies, obsidian: true, writable }
}

#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_download(app: tauri::AppHandle, repo: String, small: bool) -> Result<String, String> {
    use_token(crate::login::active().map(|(_, t)| t));
    self::small(small);
    let w = Working::new(&app, &repo, "Downloading the vault");
    Ok(exo_core::exo_repos::summary("Downloaded", &clone(&format!("https://github.com/{repo}.git"), &dest(&repo), &|f, text| w.say(f, text))?))
}

#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_update(app: tauri::AppHandle, repo: String, path: String) -> Result<String, String> {
    use_token(crate::login::active().map(|(_, t)| t));
    let w = Working::new(&app, &repo, "Getting the latest");
    let (book, rows) = pull(Path::new(&path), &|f, text| w.say(f, text));
    let folders = if rows.is_empty() { String::new() } else { format!(". {}", exo_core::exo_repos::summary("Folders updated", &rows)) };
    match book {
        Err(e) => Err(format!("{e}{folders}")),
        Ok(()) => Ok(exo_core::exo_repos::summary("Up to date", &rows)),
    }
}

/// The files left on GitHub for the copy at `path`, for the page to choose from.
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_left(path: String) -> (Vec<String>, Vec<String>) { (left(Path::new(&path)), on_demand_folders(Path::new(&path))) }

/// Brings `files` to the phone, one after the other. Ok(what arrived); Err names the first that did not.
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_fetch(app: tauri::AppHandle, repo: String, path: String, files: Vec<String>, everything: bool) -> Result<String, String> {
    use_token(crate::login::active().map(|(_, t)| t));
    let w = Working::new(&app, &repo, "Getting files");
    // asked for all that was left behind: the copy is no longer a small one, and Get latest stops leaving files out
    if everything { if let Ok(mut c) = Repository::open(&path).and_then(|r| r.config()) { let _ = c.set_bool("aiwalk.small", false); } }
    for (i, f) in files.iter().enumerate() {
        w.say(i as f32 / files.len() as f32, &format!("Getting {} ({} of {})", f.rsplit('/').next().unwrap_or(f), i + 1, files.len()));
        fetch(Path::new(&path), f, &github_blob).map_err(|e| format!("{f}: {e}{}", if i > 0 { format!(". {i} arrived before it") } else { String::new() }))?;
    }
    Ok(if files.len() == 1 { format!("{} is on the phone", files[0].rsplit('/').next().unwrap_or(&files[0])) } else { format!("{} files are on the phone", files.len()) })
}

/// The changes in the copy at `path`, grouped, for the page to tick from before anything is sent.
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_changes(path: String) -> Vec<Change> {
    // whether the account may change a folder: GitHub's own answer for that repo
    scan(Path::new(&path), &|url| exo_core::vault_ship::github_repo(url.trim_end_matches('/'))
        .and_then(|(o, r)| crate::github::get(&format!("repos/{o}/{r}")).ok()).is_some_and(|v| v["permissions"]["push"] == serde_json::Value::Bool(true)))
}

/// What the team pushed that is not here yet, and what arrived lately: (not here yet, last two days).
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_incoming(path: String) -> Result<(Vec<Row>, Vec<Row>), String> {
    use_token(crate::login::active().map(|(_, t)| t));
    incoming(Path::new(&path), &|_| {})
}

/// Empty files to the vault's trash.
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_trash(path: String, files: Vec<String>) -> Result<usize, String> { trash(Path::new(&path), &files) }

/// Sends them, as the signed-in account (GitHub's no-reply address, as on a computer).
#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_send(app: tauri::AppHandle, repo: String, path: String, message: String, files: Vec<String>) -> Result<String, String> {
    use_token(crate::login::active().map(|(_, t)| t));
    let user = crate::github::get("user").map_err(|_| "Sign in first".to_string())?;
    let (Some(login), Some(id)) = (user["login"].as_str(), user["id"].as_u64()) else { return Err("Sign in first".into()) };
    if message.trim().is_empty() { return Err("Say what changed in one line first".into()) }
    if files.is_empty() { return Err("Tick at least one file".into()) }
    let w = Working::new(&app, &repo, "Uploading");
    send(Path::new(&path), message.trim(), (login, &format!("{id}+{login}@users.noreply.github.com")), Some(&files), &|f, text| w.say(f, text))
}

// ---------------------------------------------------------------- test

/// Runs on a computer: cargo test -p aiwalk-setup --features phone-git phonegit -- --nocapture
/// Needs the network: the submodules come from public GitHub repos over https, through the transport above.
#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git").current_dir(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "protocol.file.allow=always"])
            .args(args).output().unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    fn commit(work: &Path, file: &str, body: &str) {
        std::fs::write(work.join(file), body).unwrap();
        git(work, &["add", "-A"]);
        git(work, &["commit", "-q", "-m", file]);
        git(work, &["push", "-q", "origin", "HEAD"]);
    }

    #[test]
    fn phonegit_clones_skips_and_moves_forward() {
        let root = std::env::temp_dir().join(format!("aiwalk-phonegit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let url = |p: &Path| format!("file://{}", p.display());

        // a team folder this test can move: a bare repo and a work copy pushing to it
        let notes = root.join("notes.git");
        git(&root, &["init", "-q", "--bare", "-b", "main", &notes.to_string_lossy()]);
        let notes_work = root.join("notes-work");
        git(&root, &["clone", "-q", &url(&notes), &notes_work.to_string_lossy()]);
        commit(&notes_work, "a.md", "one");

        // the book: two public GitHub repos over https, a repo that does not exist (refused), papers on demand
        let book = root.join("book.git");
        git(&root, &["init", "-q", "--bare", "-b", "main", &book.to_string_lossy()]);
        let work = root.join("book-work");
        git(&root, &["clone", "-q", &url(&book), &work.to_string_lossy()]);
        std::fs::create_dir_all(work.join("System")).unwrap();
        std::fs::write(work.join("System/vault_rules.json"), r#"{"repos": {"on_demand": ["papers"]}}"#).unwrap();
        git(&work, &["submodule", "add", "-q", "--name", "hello", "https://github.com/octocat/Hello-World.git", "Hello"]);
        git(&work, &["submodule", "add", "-q", "--name", "spoon", "https://github.com/octocat/Spoon-Knife.git", "Spoon"]);
        git(&work, &["submodule", "add", "-q", "--name", "notes", &url(&notes), "Notes"]);
        git(&work, &["submodule", "add", "-q", "--name", "papers", &url(&notes), "Papers"]);
        // a private or missing repo: GitHub refuses it as it refuses a repo this account may not read
        std::fs::write(work.join(".gitmodules"), std::fs::read_to_string(work.join(".gitmodules")).unwrap()
            + "[submodule \"secret\"]\n\tpath = Secret\n\turl = git@github.com:octocat/this-repo-does-not-exist-aiwalk.git\n").unwrap();
        git(&work, &["update-index", "--add", "--cacheinfo", &format!("160000,{},Secret", "7fd1a60b01f91b314f59955a4e4d4e80d8edf11d")]);
        commit(&work, "README.md", "book");

        // the book itself over https too: a public repo, shallow
        let hello = root.join("hello-book");
        let rows = clone("https://github.com/octocat/Hello-World.git", &hello, &|_, _| {}).unwrap();
        assert!(rows.is_empty() && hello.join("README").exists(), "public repo cloned over https");
        let shallow = Repository::open(&hello).unwrap().is_shallow();
        println!("https book: Hello-World cloned, shallow = {shallow}");
        assert!(shallow);

        let dest = root.join("phone");
        let rows = clone(&url(&book), &dest, &|_, _| {}).unwrap();
        let msg = exo_core::exo_repos::summary("Downloaded", &rows);
        println!("{msg}");
        for (p, g) in &rows { println!("{}", exo_core::exo_repos::row(p, *g)) }
        assert!(rows.contains(&("Hello".into(), Got::Ok)) && rows.contains(&("Spoon".into(), Got::Ok)));
        assert!(rows.contains(&("Notes".into(), Got::Ok)) && rows.contains(&("Secret".into(), Got::Skipped)));
        assert!(rows.contains(&("Papers".into(), Got::Later)));
        assert!(dest.join("Hello/README").exists() && dest.join("Spoon/index.html").exists());
        assert!(Repository::open(dest.join("Spoon")).unwrap().is_shallow(), "folders are shallow too");
        assert!(empty(&dest.join("Secret")) && empty(&dest.join("Papers")));
        assert_eq!(std::fs::read_to_string(dest.join("Notes/a.md")).unwrap(), "one");

        // the team moves the book and a folder: Get latest brings both
        commit(&notes_work, "a.md", "two");
        commit(&work, "README.md", "book 2");
        let (b, rows) = pull(&dest, &|_, _| {});
        println!("get latest: {b:?} {rows:?}");
        assert!(b.is_ok());
        assert_eq!(std::fs::read_to_string(dest.join("README.md")).unwrap(), "book 2");
        assert_eq!(std::fs::read_to_string(dest.join("Notes/a.md")).unwrap(), "two");
        assert!(rows.contains(&("Notes".into(), Got::Ok)) && rows.contains(&("Secret".into(), Got::Skipped)));

        // the person changed files the team changes too: refused, their text kept; an unrelated change survives
        std::fs::write(dest.join("README.md"), "mine").unwrap();
        std::fs::write(dest.join("Notes/a.md"), "mine too").unwrap();
        std::fs::write(dest.join("System/vault_rules.json"), r#"{"repos": {"on_demand": ["papers"]}, "x": 1}"#).unwrap();
        commit(&notes_work, "a.md", "three");
        commit(&work, "README.md", "book 3");
        let (b, rows) = pull(&dest, &|_, _| {});
        println!("with local changes: {b:?}\n{}", exo_core::exo_repos::summary("Folders updated", &rows));
        assert!(b.unwrap_err().contains("Your changes are kept"));
        assert!(rows.contains(&("Notes".into(), Got::Kept)));
        assert_eq!(std::fs::read_to_string(dest.join("README.md")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(dest.join("Notes/a.md")).unwrap(), "mine too");

        // their change put back: the next Get latest goes through, the unrelated change still there
        std::fs::write(dest.join("README.md"), "book 2").unwrap();
        let (b, _) = pull(&dest, &|_, _| {});
        assert!(b.is_ok(), "{b:?}");
        assert_eq!(std::fs::read_to_string(dest.join("README.md")).unwrap(), "book 3");
        assert!(std::fs::read_to_string(dest.join("System/vault_rules.json")).unwrap().contains("\"x\""));

        // a commit made on the phone by another app: never moved past, never overwritten
        git(&dest.join("Notes"), &["checkout", "-q", "--", "a.md"]);
        git(&dest.join("Notes"), &["commit", "-q", "--allow-empty", "-m", "mine"]);
        commit(&notes_work, "a.md", "four");
        let (_, rows) = pull(&dest, &|_, _| {});
        assert!(rows.contains(&("Notes".into(), Got::Kept)), "{rows:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// By hand, against a real vault: how long a phone-style download takes and what it leaves on disk.
    ///   AIWALK_TEST_REPO=Org/book AIWALK_TEST_TOKEN=... AIWALK_TEST_DIR=/some/empty/place \
    ///     cargo test -p aiwalk-setup --features phone-git phonegit_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn phonegit_real_vault() {
        let (repo, dir) = (std::env::var("AIWALK_TEST_REPO").unwrap(), std::path::PathBuf::from(std::env::var("AIWALK_TEST_DIR").unwrap()));
        use_token(std::env::var("AIWALK_TEST_TOKEN").ok());
        let t = std::time::Instant::now();
        if empty(&dir) {
            let rows = clone(&format!("https://github.com/{repo}.git"), &dir, &|_, _| {}).unwrap();
            println!("{} in {:.0} s", exo_core::exo_repos::summary("Downloaded", &rows), t.elapsed().as_secs_f32());
        }
        let left = out(&dir, &["ls-files", "-v"]).lines().filter(|l| l.starts_with("S ")).count();
        println!("book: {} files left on GitHub; changes seen: {:?}", left, pending(&dir).len());
        assert!(pending(&dir).is_empty());
        let t = std::time::Instant::now();
        let (b, _) = pull(&dir, &|_, _| {});
        println!("get latest: {b:?} in {:.0} s", t.elapsed().as_secs_f32());
    }

    fn out(dir: &Path, args: &[&str]) -> String {
        String::from_utf8_lossy(&std::process::Command::new("git").current_dir(dir).args(args).output().unwrap().stdout).trim().to_string()
    }

    /// git's own server (git http-backend) behind a few lines of Python: shallow copies and pushes then go through
    /// the same transport and the same protocol as with GitHub, which a folder (file://) does not exercise.
    const SERVER: &str = r#"
import http.server, os, subprocess, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path, _, q = self.path.partition('?')
        n = int(self.headers.get('Content-Length') or 0)
        env = dict(os.environ, GIT_PROJECT_ROOT=sys.argv[1], GIT_HTTP_EXPORT_ALL='1', REQUEST_METHOD=self.command, PATH_INFO=path,
                   QUERY_STRING=q, CONTENT_TYPE=self.headers.get('Content-Type', ''), CONTENT_LENGTH=str(n), REMOTE_ADDR='127.0.0.1',
                   HTTP_CONTENT_ENCODING=self.headers.get('Content-Encoding', ''), GIT_PROTOCOL=self.headers.get('Git-Protocol', ''))
        out = subprocess.run(['git', 'http-backend'], input=self.rfile.read(n), env=env, capture_output=True).stdout
        head, _, body = out.partition(b'\r\n\r\n')
        lines = [l.split(': ', 1) for l in head.decode().split('\r\n')]
        self.send_response(int(dict(lines).get('Status', '200').split()[0]))
        for k, v in lines:
            if k != 'Status': self.send_header(k, v)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    do_POST = do_GET
    def log_message(self, *a): pass
s = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
"#;

    #[test]
    fn phonegit_sends_changes_from_a_shallow_copy() {
        use std::io::BufRead;
        let root = std::env::temp_dir().join(format!("aiwalk-phonesend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let srv = root.join("srv");
        std::fs::create_dir_all(&srv).unwrap();
        // stopped when the test ends, also by a failed assert: left running it holds the test's output open
        struct Server(std::process::Child);
        impl Drop for Server { fn drop(&mut self) { let _ = self.0.kill(); } }
        let mut server = Server(std::process::Command::new("python3").args(["-c", SERVER, &srv.to_string_lossy()]).stdout(std::process::Stdio::piped()).spawn().unwrap());
        let mut port = String::new();
        std::io::BufReader::new(server.0.stdout.take().unwrap()).read_line(&mut port).unwrap();
        let url = |name: &str| format!("http://127.0.0.1:{}/{name}.git", port.trim());
        let bare = |name: &str| {
            let dir = srv.join(format!("{name}.git"));
            git(&root, &["init", "-q", "--bare", "-b", "main", &dir.to_string_lossy()]);
            git(&dir, &["config", "http.receivepack", "true"]);
            git(&dir, &["config", "uploadpack.allowFilter", "true"]);
            dir
        };
        let (notes, book, papers) = (bare("notes"), bare("book"), bare("papers"));
        let papers_work = root.join("papers-work");
        git(&root, &["clone", "-q", &url("papers"), &papers_work.to_string_lossy()]);
        commit(&papers_work, "README.md", "the papers");
        commit(&papers_work, "smith2020.pdf", &"%PDF ".repeat(40_000));   // 200 KB: over the on-demand limit

        // the team's side, with a real git: two commits each, so a shallow copy really is missing history
        let notes_work = root.join("notes-work");
        git(&root, &["clone", "-q", &url("notes"), &notes_work.to_string_lossy()]);
        commit(&notes_work, "a.md", "one");
        commit(&notes_work, "b.md", "b one");
        let large = "0123456789abcdef".repeat(200_000);   // 3.2 MB: over the limit, stays on the server
        commit(&notes_work, "slides.pptx", &large);
        // 1 MB that does not compress and is under the limit: it comes to the phone, once
        let mut x = 7u64;
        let figure: String = (0..1_000_000).map(|_| { x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (b'!' + (x >> 33) as u8 % 90) as char }).collect();
        commit(&notes_work, "figure.txt", &figure);
        let work = root.join("book-work");
        git(&root, &["clone", "-q", &url("book"), &work.to_string_lossy()]);
        commit(&work, "old.md", "to be deleted");
        git(&work, &["submodule", "add", "-q", "-b", "main", "--name", "notes", &url("notes"), "Notes"]);
        git(&work, &["submodule", "add", "-q", "-b", "main", "--name", "papers", &url("papers"), "Papers"]);
        std::fs::create_dir_all(work.join("System")).unwrap();
        std::fs::write(work.join("System/vault_rules.json"), r#"{"repos": {"on_demand": ["papers"]}}"#).unwrap();
        commit(&work, "paper.pdf", &large);
        commit(&work, "README.md", "book");

        // a copy that takes everything: the large files come, the papers still do not
        small(false);
        let whole = root.join("phone-whole");
        clone(&url("book"), &whole, &|_, _| {}).unwrap();
        assert!(whole.join("paper.pdf").exists() && whole.join("Notes/slides.pptx").exists());
        assert!(whole.join("Papers/README.md").exists() && !whole.join("Papers/smith2020.pdf").exists());
        assert_eq!(left(&whole), ["Papers/smith2020.pdf"]);
        assert_eq!(on_demand_folders(&whole), ["Papers"]);
        assert!(pending(&whole).is_empty(), "{:?}", pending(&whole));

        // the rest of this test: a small copy, as a phone short of room chooses
        small(true);
        let phone = root.join("phone");
        clone(&url("book"), &phone, &|_, _| {}).unwrap();
        // the large files are not on the phone, nor in its git data; git knows they are left out on purpose
        assert!(!phone.join("paper.pdf").exists() && !phone.join("Notes/slides.pptx").exists());
        assert!(phone.join("README.md").exists() && phone.join("Notes/a.md").exists() && phone.join("Notes/b.md").exists());
        assert!(out(&phone, &["ls-files", "-v"]).lines().any(|l| l == "S paper.pdf"), "{}", out(&phone, &["ls-files", "-v"]));
        let size = |d: &Path| out(d, &["count-objects", "-v"]).lines().filter_map(|l| l.strip_prefix("size-pack: ").and_then(|n| n.parse::<u64>().ok())).sum::<u64>();
        println!("git data on the phone: book {} KB, Notes {} KB", size(&phone), size(&phone.join("Notes")));
        assert!(size(&phone) < 500 && size(&phone.join("Notes")) < 1500);
        assert!(Repository::open(&phone).unwrap().is_shallow() && Repository::open(phone.join("Notes")).unwrap().is_shallow());
        assert!(pending(&phone).is_empty(), "{:?}", pending(&phone));

        // the papers: their notes are here, each paper is known and not here; one is brought by itself
        assert!(phone.join("Papers/README.md").exists() && !phone.join("Papers/smith2020.pdf").exists());
        assert_eq!(left(&phone), ["Notes/slides.pptx", "paper.pdf", "Papers/smith2020.pdf"]);
        let from_server = |remote: &str, id: &str, to: &Path| -> Result<(), String> {
            let bare = srv.join(remote.rsplit('/').next().unwrap());
            let o = std::process::Command::new("git").current_dir(bare).args(["cat-file", "blob", id]).output().map_err(|e| e.to_string())?;
            if !o.status.success() { return Err("no such blob".into()) }
            std::fs::write(to, o.stdout).map_err(|e| e.to_string())
        };
        fetch(&phone, "Papers/smith2020.pdf", &from_server).unwrap();
        fetch(&phone, "paper.pdf", &from_server).unwrap();
        assert_eq!(std::fs::read_to_string(phone.join("Papers/smith2020.pdf")).unwrap().len(), 200_000);
        assert_eq!(std::fs::read_to_string(phone.join("paper.pdf")).unwrap().len(), 3_200_000);
        assert_eq!(left(&phone), ["Notes/slides.pptx"]);
        assert!(pending(&phone).is_empty(), "a file brought from GitHub is not a change: {:?}", pending(&phone));
        // a download that is not the recorded file leaves nothing behind
        assert!(fetch(&phone, "Notes/slides.pptx", &|_, _, to| std::fs::write(to, "wrong").map_err(|e| e.to_string())).is_err());
        assert!(!phone.join("Notes/slides.pptx").exists() && pending(&phone).is_empty());
        let who = ("jo", "1+jo@users.noreply.github.com");
        assert_eq!(send(&phone, "m", who, None, &|_, _| {}).unwrap(), "Nothing to send");

        // a changed note in a folder, a new file and a deleted one in the book, and a file that looks like a key
        std::fs::write(phone.join("Notes/a.md"), "from the phone").unwrap();
        std::fs::write(phone.join("new.md"), "new").unwrap();
        std::fs::remove_file(phone.join("old.md")).unwrap();
        std::fs::write(phone.join("my-token.txt"), "x").unwrap();
        // the page's list: every change, grouped the way the plugin groups it on a computer
        std::fs::write(phone.join("Untitled.md"), " \n").unwrap();
        std::fs::write(phone.join("draft.bak.md"), "x").unwrap();
        std::fs::write(phone.join(".hidden-state"), "x").unwrap();
        let ch = scan(&phone, &|_| true);
        let of = |p: &str| ch.iter().find(|c| c.path == p).map(|c| (c.code, c.group, c.locked));
        assert_eq!(of("Notes/a.md"), Some(("changed", "changed", false)));
        assert_eq!(of("new.md"), Some(("new", "new", false)));
        assert_eq!(of("old.md"), Some(("deleted", "changed", false)));
        assert_eq!(of("my-token.txt"), Some(("new", "held", true)));
        assert_eq!(of("Untitled.md"), Some(("new", "empty", false)));
        assert_eq!(of("draft.bak.md"), Some(("new", "held", false)));
        assert_eq!(of(".hidden-state"), None);
        assert!(ch.iter().all(|c| c.big == 0.0 && c.path != "paper.pdf" && c.path != "Notes/slides.pptx"), "{ch:?}");
        // a folder the account may read but not change: its files are held, and say why
        let ro = scan(&phone, &|url| !url.ends_with("notes.git"));
        assert_eq!(ro.iter().find(|c| c.path == "Notes/a.md").map(|c| (c.group, c.locked)), Some(("held", true)));
        // the empty file goes to the vault's trash and is no longer a change
        assert_eq!(trash(&phone, &["Untitled.md".to_string()]).unwrap(), 1);
        assert!(phone.join(".trash/Untitled.md").exists() && trash(&phone, &["../x".to_string()]).is_err());
        assert!(scan(&phone, &|_| true).iter().all(|c| !c.path.contains("Untitled")));
        std::fs::remove_file(phone.join("draft.bak.md")).unwrap();
        std::fs::remove_file(phone.join(".hidden-state")).unwrap();

        let mut p = pending(&phone);
        p.sort();
        assert_eq!(p, ["Notes/a.md", "my-token.txt", "new.md", "old.md"]);
        let said = send(&phone, "docs: from the phone", who, None, &|_, _| {}).unwrap();
        println!("{said}");
        assert!(said.starts_with("Sent 3 files") && said.contains("my-token.txt"), "{said}");
        assert_eq!(out(&notes, &["show", "main:a.md"]), "from the phone");
        assert_eq!(out(&book, &["show", "main:new.md"]), "new");
        assert!(out(&book, &["ls-tree", "--name-only", "main"]).lines().all(|l| l != "old.md" && l != "my-token.txt"));
        assert_eq!(out(&book, &["log", "-1", "--format=%an <%ae> %s", "main"]), "jo <1+jo@users.noreply.github.com> docs: from the phone");
        // the book records the folder's new commit, and that commit is on the server
        assert_eq!(out(&book, &["rev-parse", "main:Notes"]), out(&notes, &["rev-parse", "main"]));
        assert_eq!(out(&notes, &["rev-list", "--count", "main"]), "5", "history on the server is whole, not cut at the shallow point");
        // the files left on GitHub are still in what was sent: not deleted by a phone that never had them
        assert_eq!(out(&notes, &["cat-file", "-s", "main:slides.pptx"]), "3200000");
        assert_eq!(out(&book, &["cat-file", "-s", "main:paper.pdf"]), "3200000");
        assert_eq!(pending(&phone), ["my-token.txt"]);

        // only what is ticked goes: the other change stays on the phone, still listed
        std::fs::write(phone.join("new.md"), "new 2").unwrap();
        std::fs::write(phone.join("keep.md"), "not yet").unwrap();
        assert_eq!(send(&phone, "docs: one of two", who, Some(&["new.md".to_string()]), &|_, _| {}).unwrap(), "Sent 1 file");
        assert_eq!(out(&book, &["show", "main:new.md"]), "new 2");
        assert!(out(&book, &["ls-tree", "--name-only", "main"]).lines().all(|l| l != "keep.md"));
        assert!(pending(&phone).contains(&"keep.md".to_string()));
        std::fs::remove_file(phone.join("keep.md")).unwrap();
        // a new link to nothing stops the upload, as on a computer; a link to a file left on GitHub is not broken
        std::fs::write(phone.join("new.md"), "see [[No_Such_Note]], [[slides.pptx]] and [[a]]").unwrap();
        let e = send(&phone, "docs: links", who, Some(&["new.md".to_string()]), &|_, _| {}).unwrap_err();
        println!("{e}");
        assert!(e.contains("new.md: [[No_Such_Note]]") && !e.contains("slides.pptx") && !e.contains("[[a]]"), "{e}");
        assert_eq!(out(&book, &["show", "main:new.md"]), "new 2");
        std::fs::write(phone.join("new.md"), "new 2").unwrap();

        // what the team pushed is listed before anything on the phone moves; Bring in is Get latest
        git(&work, &["pull", "-q"]);
        commit(&work, "team.md", "from the team");
        let (rows, recent) = incoming(&phone, &|_| {}).unwrap();
        assert_eq!(rows.iter().map(|r| (r.who.as_str(), r.what.as_str())).collect::<Vec<_>>(), [("t", "team.md")]);
        assert!(!phone.join("team.md").exists() && recent.iter().any(|r| r.who == "jo" && r.what == "docs: one of two"), "{recent:?}");
        assert!(pull(&phone, &|_, _| {}).0.is_ok() && phone.join("team.md").exists());
        let (rows, recent) = incoming(&phone, &|_| {}).unwrap();
        assert!(rows.is_empty() && recent.iter().any(|r| r.what == "team.md"));

        // the team moved on in the meantime, in another file: the phone's change goes on top of theirs
        git(&notes_work, &["pull", "-q"]);
        commit(&notes_work, "c.md", "theirs");
        std::fs::write(phone.join("Notes/b.md"), "b from the phone").unwrap();
        assert!(send(&phone, "m2", who, None, &|_, _| {}).unwrap().starts_with("Sent 1 file"));
        assert_eq!(out(&notes, &["show", "main:b.md"]), "b from the phone");
        assert_eq!(out(&notes, &["show", "main:c.md"]), "theirs");
        assert_eq!(std::fs::read_to_string(phone.join("Notes/c.md")).unwrap(), "theirs");
        assert_eq!(out(&notes, &["cat-file", "-s", "main:slides.pptx"]), "3200000");

        // the team changes a large file and deletes a note, in two commits: Get latest takes both, the large one
        // still not here, and what the phone already has (the 1 MB file) is not downloaded a second time
        let had = size(&phone.join("Notes"));
        git(&notes_work, &["pull", "-q"]);
        commit(&notes_work, "slides.pptx", &(large.clone() + "more"));
        git(&notes_work, &["rm", "-q", "c.md"]);
        commit(&notes_work, "e.md", "e");
        let (_, rows) = pull(&phone, &|_, _| {});
        assert!(rows.contains(&("Notes".into(), Got::Ok)), "{rows:?}");
        assert!(!phone.join("Notes/c.md").exists() && phone.join("Notes/e.md").exists() && !phone.join("Notes/slides.pptx").exists());
        println!("Notes git data: {had} KB before two team commits, {} KB after", size(&phone.join("Notes")));
        assert!(size(&phone.join("Notes")) < had + 300);
        assert!(pending(&phone).iter().all(|p| p == "my-token.txt"), "{:?}", pending(&phone));

        // the team replaces a paper that was brought to the phone: the old copy goes, and is not sent over theirs
        commit(&papers_work, "smith2020.pdf", &"%PDF2 ".repeat(40_000));
        pull(&phone, &|_, _| {});
        assert!(!phone.join("Papers/smith2020.pdf").exists() && left(&phone).contains(&"Papers/smith2020.pdf".to_string()));
        assert!(pending(&phone).iter().all(|p| p == "my-token.txt"), "{:?}", pending(&phone));
        assert_eq!(out(&papers, &["rev-list", "--count", "main"]), "3", "nothing was sent to the papers");

        // the same file changed by both: nothing sent, the text kept, no commit left behind that GitHub lacks
        git(&notes_work, &["pull", "-q"]);
        commit(&notes_work, "a.md", "theirs again");
        std::fs::write(phone.join("Notes/a.md"), "mine again").unwrap();
        let before = out(&phone.join("Notes"), &["rev-parse", "HEAD"]);
        let e = send(&phone, "m3", who, None, &|_, _| {}).unwrap_err();
        println!("{e}");
        assert!(e.contains("Notes: you changed a file here that the team changed too"), "{e}");
        assert_eq!(std::fs::read_to_string(phone.join("Notes/a.md")).unwrap(), "mine again");
        assert_eq!(out(&notes, &["show", "main:a.md"]), "theirs again");
        assert_eq!(out(&phone.join("Notes"), &["rev-parse", "HEAD"]), before);

        // a folder the account may read but not change: said so, and the commit is taken back
        git(&notes, &["config", "http.receivepack", "false"]);
        std::fs::write(phone.join("Notes/a.md"), "theirs again").unwrap();
        std::fs::write(phone.join("Notes/d.md"), "not allowed").unwrap();
        let e = send(&phone, "m4", who, None, &|_, _| {}).unwrap_err();
        println!("{e}");
        assert!(e.contains("write access"), "{e}");
        assert_eq!(out(&phone.join("Notes"), &["rev-parse", "HEAD"]), out(&phone.join("Notes"), &["rev-parse", "origin/main"]));
        assert!(pending(&phone).contains(&"Notes/d.md".to_string()));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
