//! The vault on a phone: download and "Get latest" without a git program, which Android does not have.
//! Same shape as vault.rs on a computer: the book is cloned shallow, then each submodule on its own, so one the
//! account cannot read only skips itself; the results are exo_core's `Got` rows and `summary`. No commit, no push.
//!
//! Library: git2 (libgit2, compiled from source by libgit2-sys with the NDK's clang, as `ring` already is), not gix.
//! libgit2 has every step here as one call: shallow fetch (`depth`), a checkout that refuses to overwrite the
//! person's own changes (`CheckoutBuilder::safe`) and the submodule list; with gix, updating a work tree that is
//! already there would be ours to write. libgit2 is built without HTTPS of its own (no OpenSSL): `https://` goes
//! through the transport registered below on top of ureq, so TLS is the rustls (ring) the app already ships.
//! libgit2 has no partial clone and no sparse checkout, so on-demand folders (the papers) are left for later here.
//!
//! The computer app never compiles this file except for its test (`--features phone-git`); it keeps calling git.

use exo_core::exo_repos::Got;
use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::transport::{self, Service, SmartSubtransport, SmartSubtransportStream, Transport};
use git2::{AutotagOption, ErrorCode, FetchOptions, RemoteCallbacks, Repository};
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
        // Android's shared storage shows every file as owned by another user, so libgit2's ownership check (git's
        // safe.directory) refuses each copy there with "not owned by current user". Only this app's folders are opened.
        let _ = git2::opts::set_verify_owner_validation(false);
    });
}

struct Https;

impl SmartSubtransport for Https {
    fn action(&self, url: &str, action: Service) -> Result<Box<dyn SmartSubtransportStream>, git2::Error> {
        let (post, path) = match action {
            Service::UploadPackLs => (false, "/info/refs?service=git-upload-pack"),
            Service::UploadPack => (true, "/git-upload-pack"),
            _ => return Err(git2::Error::from_str("this app only downloads")),
        };
        Ok(Box::new(Stream { url: format!("{}{path}", url.trim_end_matches('/')), post, body: vec![], reply: None }))
    }
    fn close(&self) -> Result<(), git2::Error> { Ok(()) }
}

/// One request: what libgit2 writes is the request body, sent at its first read; reads then come from the reply.
struct Stream { url: String, post: bool, body: Vec<u8>, reply: Option<ureq::BodyReader<'static>> }

impl Write for Stream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.body.extend_from_slice(b); Ok(b.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.reply.is_none() { self.reply = Some(self.send()?) }
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
            req.method("POST").header("Content-Type", "application/x-git-upload-pack-request")
                .header("Accept", "application/x-git-upload-pack-result")
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
    // no overall timeout: a vault on a slow phone network takes minutes; a stalled connection still ends
    AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false)
        .timeout_connect(Some(std::time::Duration::from_secs(30))).timeout_recv_body(Some(std::time::Duration::from_secs(120)))
        .build().into())
}

// ---------------------------------------------------------------- clone and update

fn no_access(e: &git2::Error) -> bool { e.message().contains("no access") }

fn no_space(e: &str) -> bool { e.contains("No space left") || e.contains("ENOSPC") }

/// Shallow fetch options; `f` hears the fraction of objects received. libgit2 cannot fetch shallow from a folder
/// (file://), which only the test uses for repos it moves; those come whole.
fn options<'a>(url: &str, f: &'a dyn Fn(f32)) -> FetchOptions<'a> {
    let mut cb = RemoteCallbacks::new();
    cb.transfer_progress(move |p| { if p.total_objects() > 0 { f(p.received_objects() as f32 / p.total_objects() as f32) } true });
    let mut o = FetchOptions::new();
    o.remote_callbacks(cb).depth(if url.starts_with("file://") { 0 } else { 1 }).download_tags(AutotagOption::None);
    o
}

/// Clones `url` into `dest` (new or empty): the tip of `branch`, or of the repo's default branch.
fn get(url: &str, dest: &Path, branch: Option<&str>, f: &dyn Fn(f32)) -> Result<Repository, git2::Error> {
    https();
    let mut b = RepoBuilder::new();
    b.fetch_options(options(url, f));
    if let Some(name) = branch { b.branch(name); }
    b.clone(url, dest)
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
    origin.fetch(&[&format!("+refs/heads/{branch}:{theirs}")], Some(&mut options(&url, f)), None)?;
    let new = repo.refname_to_id(&theirs)?;
    if new == here { return Ok(()) }
    // shallow history cannot prove a fast-forward. Every tip GitHub ever gave is in the reflog of origin's branch
    // (a Get latest refused for the person's changes leaves HEAD on an older one); a commit made here (by another
    // app) is in none of them
    let given = |id| before == Some(id) || repo.reflog(&theirs).is_ok_and(|log| log.iter().any(|e| e.id_new() == id));
    if !given(here) { return Err(NotMoved::Diverged) }
    match repo.checkout_tree(&repo.find_object(new, None)?, Some(CheckoutBuilder::new().safe())) {
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
    let rows = each(dest, false, 0.5, progress);
    progress(1.0, "Done");
    Ok(rows)
}

/// Moves the book forward (fast-forward only), then each readable folder, even when the book could not move.
pub fn pull(tree: &Path, progress: Progress) -> (Result<(), String>, Vec<(String, Got)>) {
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

// ---------------------------------------------------------------- the phone's commands

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
pub fn vault_download(app: tauri::AppHandle, repo: String) -> Result<String, String> {
    use tauri::Emitter;
    use_token(crate::login::active().map(|(_, t)| t));
    let report = |f: f32, text: &str| { let _ = app.emit("vault-progress", (&repo, f, text)); };
    Ok(exo_core::exo_repos::summary("Downloaded", &clone(&format!("https://github.com/{repo}.git"), &dest(&repo), &report)?))
}

#[cfg(target_os = "android")]
#[tauri::command(async)]
pub fn vault_update(app: tauri::AppHandle, repo: String, path: String) -> Result<String, String> {
    use tauri::Emitter;
    use_token(crate::login::active().map(|(_, t)| t));
    let (book, rows) = pull(Path::new(&path), &|f, text| { let _ = app.emit("vault-progress", (&repo, f, text)); });
    let folders = if rows.is_empty() { String::new() } else { format!(". {}", exo_core::exo_repos::summary("Folders updated", &rows)) };
    match book {
        Err(e) => Err(format!("{e}{folders}")),
        Ok(()) => Ok(exo_core::exo_repos::summary("Up to date", &rows)),
    }
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
}
