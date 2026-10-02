//! Downloading the team's vaults and keeping them current, on every OS.
//! A split vault is the book plus one submodule per restricted folder: each submodule is fetched on its own,
//! so one you cannot read only skips itself (`git clone --recurse-submodules` would abort; see scripts/exo_repos.py).

use crate::home;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::Emitter;

/// What an on-demand repo (Papers) always checks out; the PDFs stay on GitHub until opened.
const ALWAYS: [&str; 8] = ["/_manifest.json", "*.md", "*.jpg", "*.jpeg", "*.png", "*.svg", "*.enl", "*.canvas"];

/// git that signs in with gh, never prompts, and reads git@github.com remotes over HTTPS (no SSH key needed to read).
/// `extra` adds config, e.g. the tests' protocol.file.allow.
fn git(dir: &Path, args: &[&str], extra: &[(&str, &str)]) -> Command {
    let mut c = Command::new("git");
    let mut cfg = vec![("credential.helper", ""), ("credential.helper", "!gh auth git-credential"),
                       ("url.https://github.com/.insteadOf", "git@github.com:")];
    cfg.extend_from_slice(extra);
    c.current_dir(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_COUNT", cfg.len().to_string());
    for (i, (k, v)) in cfg.iter().enumerate() {
        c.env(format!("GIT_CONFIG_KEY_{i}"), k).env(format!("GIT_CONFIG_VALUE_{i}"), v);
    }
    c
}

fn run(dir: &Path, args: &[&str], extra: &[(&str, &str)]) -> Result<String, String> {
    let o = git(dir, args, extra).output().map_err(|e| format!("git: {e}"))?;
    let text = String::from_utf8_lossy(&[o.stdout, o.stderr].concat()).trim().to_string();
    if o.status.success() { Ok(text) } else { Err(text) }
}

/// Submodule (name, path) pairs from the book's .gitmodules.
fn submodules(tree: &Path) -> Vec<(String, String)> {
    run(tree, &["config", "-f", ".gitmodules", "--get-regexp", r"submodule\..*\.path"], &[]).unwrap_or_default()
        .lines().filter_map(|l| {
            let (key, path) = l.split_once(' ')?;
            Some((key.strip_prefix("submodule.")?.strip_suffix(".path")?.to_string(), path.to_string()))
        }).collect()
}

fn on_demand(tree: &Path) -> Vec<String> {
    let rules = std::fs::read_to_string(tree.join("System/vault_rules.json")).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&rules).ok()
        .and_then(|v| v["repos"]["on_demand"].as_array().cloned()).unwrap_or_default()
        .iter().filter_map(|n| n.as_str().map(String::from)).collect()
}

/// First fetch of an on-demand submodule: names only, sparse set, then the tip of main.
fn fetch_on_demand(tree: &Path, name: &str, path: &str, extra: &[(&str, &str)]) -> Result<(), String> {
    run(tree, &["submodule", "init", "--", path], extra)?;
    let url = run(tree, &["config", &format!("submodule.{name}.url")], extra)?;
    // --no-local: a plain path would copy every object and ignore the filter
    run(tree, &["clone", "-q", "--no-local", "--filter=blob:none", "--no-checkout", &url, path], extra)?;
    let p = tree.join(path);
    let mut args = vec!["sparse-checkout", "set", "--no-cone"];
    args.extend(ALWAYS);
    run(&p, &args, extra)?;
    run(&p, &["checkout", "-q", "main"], extra)?;
    run(tree, &["submodule", "absorbgitdirs", "--", path], extra).map(|_| ())
}

/// Brings every readable submodule in; (fetched, skipped) folder paths. `remote` moves each to the tip of its main.
fn update_each(tree: &Path, remote: bool, extra: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
    let lazy = on_demand(tree);
    let (mut ok, mut skipped) = (vec![], vec![]);
    for (name, path) in submodules(tree) {
        let first = !tree.join(&path).join(".git").exists();
        let done = if lazy.contains(&name) && first {
            fetch_on_demand(tree, &name, &path, extra).is_ok()
        } else {
            let mut args = vec!["submodule", "update", "--init", "--recursive"];
            if remote { args.extend(["--remote", "--merge"]) }
            args.extend(["--", &path]);
            run(tree, &args, extra).is_ok()
        };
        if done { ok.push(path) } else {
            // an unreadable folder stays an empty directory
            skipped.push(path)
        }
    }
    (ok, skipped)
}

fn summary(what: &str, ok: &[String], skipped: &[String]) -> String {
    match (ok.len(), skipped.len()) {
        (0, 0) => what.to_string(),
        (n, 0) => format!("{what} with {n} folders"),
        (n, m) => format!("{what} with {n} folders; {m} you cannot open stay empty ({})", skipped.join(", ")),
    }
}

/// Clones `url` into `dest`, reporting the book's download percentage through `progress`.
pub fn clone(url: &str, dest: &Path, extra: &[(&str, &str)], progress: &dyn Fn(u32)) -> Result<String, String> {
    let parent = dest.parent().ok_or("bad folder")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // shallow: members need the current notes, not years of history
    let mut child = git(parent, &["clone", "--progress", "--depth", "1", url, &dest.to_string_lossy()], extra)
        .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("git: {e}"))?;
    let mut err = child.stderr.take().unwrap();
    let (mut tail, mut buf) = (String::new(), [0u8; 256]);
    while let Ok(n) = err.read(&mut buf) {
        if n == 0 { break }
        tail.push_str(&String::from_utf8_lossy(&buf[..n]));
        if tail.len() > 400 { tail = tail[tail.len() - 400..].to_string() }
        if let Some(pct) = tail.rsplit("Receiving objects:").next().and_then(|t| t.trim().split('%').next()?.trim().parse().ok()) {
            progress(pct);
        }
    }
    if !child.wait().is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(dest);
        return Err(if ["not found", "Authentication", "could not read", "403"].iter().any(|k| tail.contains(k)) {
            "No access to this vault. Sign in first, or ask an owner for access.".into()
        } else {
            format!("Download failed: {}", tail.trim().lines().last().unwrap_or("unknown error"))
        });
    }
    let (ok, skipped) = update_each(dest, false, extra);
    Ok(summary("Downloaded", &ok, &skipped))
}

pub fn pull(tree: &Path, extra: &[(&str, &str)]) -> Result<String, String> {
    match run(tree, &["pull", "--ff-only"], extra) {
        Err(e) if e.contains("local changes") || e.contains("diverg") || e.contains("would be overwritten") =>
            return Err("Could not update: your copy has changes that are not on GitHub yet".into()),
        Err(e) => return Err(format!("Could not update: {}", e.lines().last().unwrap_or_default())),
        Ok(_) => {}
    }
    let (ok, skipped) = update_each(tree, true, extra);
    Ok(summary("Up to date", &ok, &skipped))
}

// ---------------------------------------------------------------- Obsidian

fn obsidian_config() -> PathBuf {
    #[cfg(target_os = "macos")]
    return home().join("Library/Application Support/obsidian/obsidian.json");
    #[cfg(windows)]
    return PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("obsidian/obsidian.json");
    #[cfg(target_os = "linux")]
    {
        let flatpak = home().join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json");
        if flatpak.exists() { flatpak } else { home().join(".config/obsidian/obsidian.json") }
    }
}

fn obsidian_installed() -> bool {
    #[cfg(target_os = "linux")]
    return ["/usr/share/applications/obsidian.desktop", "/var/lib/flatpak/exports/share/applications/md.obsidian.Obsidian.desktop"]
        .iter().map(PathBuf::from)
        .chain([home().join(".local/share/applications/obsidian.desktop"),
                home().join(".local/share/flatpak/exports/share/applications/md.obsidian.Obsidian.desktop")])
        .any(|p| p.exists()) || Command::new("which").arg("obsidian").output().is_ok_and(|o| o.status.success());
    #[cfg(target_os = "macos")]
    return Path::new("/Applications/Obsidian.app").exists();
    #[cfg(windows)]
    return PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs/obsidian/Obsidian.exe").exists();
}

fn obsidian_vaults() -> Vec<String> {
    let text = std::fs::read_to_string(obsidian_config()).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["vaults"].as_object().cloned()).unwrap_or_default()
        .values().filter_map(|v| v["path"].as_str().map(String::from)).collect()
}

/// Adds `path` to Obsidian's vault list so it can be opened.
fn register(path: &Path) {
    let file = obsidian_config();
    let mut config: serde_json::Value = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let p = path.to_string_lossy().to_string();
    let vaults = config["vaults"].as_object().cloned().unwrap_or_default();
    if vaults.values().any(|v| v["path"] == p.as_str()) { return }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let id = format!("{:016x}", now.as_nanos() as u64);
    config["vaults"][id] = serde_json::json!({ "path": p, "ts": now.as_millis() as u64 });
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let _ = std::fs::write(file, config.to_string());
}

fn default_dest(repo: &str) -> PathBuf {
    home().join("Documents/aIwalk").join(repo.rsplit('/').next().unwrap_or(repo))
}

/// Which folder the person chose for each repo, so it wins over any other copy Obsidian knows.
fn chosen_file() -> PathBuf { home().join(".config/aiwalk-setup/vaults.json") }

fn chosen() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(chosen_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn remember(repo: &str, path: &Path) {
    let mut c = chosen();
    c.insert(repo.into(), path.to_string_lossy().into());
    let _ = std::fs::create_dir_all(chosen_file().parent().unwrap());
    let _ = std::fs::write(chosen_file(), serde_json::Value::Object(c).to_string());
}

fn is_copy_of(path: &str, repo: &str) -> bool {
    run(Path::new(path), &["remote", "get-url", "origin"], &[]).is_ok_and(|u| {
        let u = u.trim().trim_end_matches('/').trim_end_matches(".git").to_lowercase();
        u.ends_with(&format!("/{}", repo.to_lowercase())) || u.ends_with(&format!(":{}", repo.to_lowercase()))
    })
}

/// This computer's copy of `repo`: the folder chosen in this app, else a vault Obsidian knows whose origin
/// is that repo, else the default folder.
fn find(repo: &str) -> Option<String> {
    let mut paths: Vec<String> = chosen().get(repo).and_then(|v| v.as_str()).map(String::from).into_iter().collect();
    paths.extend(obsidian_vaults());
    paths.push(default_dest(repo).to_string_lossy().into());
    paths.into_iter().find(|p| is_copy_of(p, repo))
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
pub struct Local {
    /// repo -> local folder, for the vaults this computer already has
    copies: std::collections::BTreeMap<String, String>,
    obsidian: bool,
}

#[tauri::command(async)]
pub fn vault_local(repos: Vec<String>) -> Local {
    Local { copies: repos.into_iter().filter_map(|r| Some((r.clone(), find(&r)?))).collect(), obsidian: obsidian_installed() }
}

/// The OS's own folder picker; None when cancelled.
#[tauri::command(async)]
pub fn pick_folder(app: tauri::AppHandle, title: String) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog().file().set_title(title).set_directory(home()).blocking_pick_folder()
        .and_then(|f| f.into_path().ok()).map(|p| p.to_string_lossy().into())
}

/// Where a download goes unless the person picks another folder: Documents/aIwalk/<repo> in their home folder.
#[tauri::command]
pub fn default_folder(repo: String) -> String { default_dest(&repo).to_string_lossy().into() }

/// Uses a copy the person already has: it must be a clone of `repo`.
#[tauri::command(async)]
pub fn vault_link(repo: String, path: String) -> Result<String, String> {
    let url = run(Path::new(&path), &["remote", "get-url", "origin"], &[]).map_err(|_| "That folder is not a git copy of a vault".to_string())?;
    if !is_copy_of(&path, &repo) { return Err(format!("That folder is a copy of {}, not {repo}", url.trim())) }
    register(Path::new(&path));
    remember(&repo, Path::new(&path));
    Ok("Using the copy in that folder".into())
}

/// Downloads into `dest`, or into the default folder when the person did not pick one.
#[tauri::command(async)]
pub fn vault_download(app: tauri::AppHandle, repo: String, dest: Option<String>) -> Result<String, String> {
    let dest = dest.map(PathBuf::from).unwrap_or_else(|| default_dest(&repo));
    if dest.exists() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(format!("{} already has files in it; move them away first", dest.display()));
    }
    let report = |pct: u32| { let _ = app.emit("vault-progress", (&repo, pct)); };
    let msg = clone(&format!("https://github.com/{repo}.git"), &dest, &[], &report)?;
    register(&dest);
    remember(&repo, &dest);
    Ok(msg)
}

#[tauri::command(async)]
pub fn vault_update(path: String) -> Result<String, String> { pull(Path::new(&path), &[]) }

#[tauri::command(async)]
pub fn vault_open(path: String) {
    register(Path::new(&path));
    let encoded: String = path.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    crate::open_url(&format!("obsidian://open?path={encoded}"));
}

/// Linux installs Obsidian from Flathub for this user; elsewhere the download page opens.
#[tauri::command(async)]
pub fn obsidian_install() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let (code, out) = crate::sh("flatpak", &["install", "-y", "--user", "flathub", "md.obsidian.Obsidian"], 900);
        return if code == 0 { Ok("Obsidian installed".into()) } else { Err(format!("Could not install Obsidian: {}", out.lines().last().unwrap_or_default())) };
    }
    #[allow(unreachable_code)]
    { crate::open_url("https://obsidian.md/download"); Ok("Download Obsidian from the page that opened".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: [(&str, &str); 1] = [("protocol.file.allow", "always")];

    fn sh(dir: &Path, args: &[&str]) { assert!(run(dir, args, &FILE).is_ok(), "git {args:?} in {dir:?}") }

    /// A bare repo with one commit holding `files`; returns its URL.
    fn bare(root: &Path, name: &str, files: &[(&str, &str)]) -> String {
        let work = root.join(format!("{name}-work"));
        std::fs::create_dir_all(&work).unwrap();
        for (f, body) in files {
            let p = work.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        sh(&work, &["init", "-q", "-b", "main"]);
        sh(&work, &["add", "-A"]);
        sh(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
        let url = root.join(format!("{name}.git"));
        sh(root, &["clone", "-q", "--bare", &work.to_string_lossy(), &url.to_string_lossy()]);
        url.to_string_lossy().into()
    }

    #[test]
    fn clone_skips_only_the_folder_you_cannot_read_and_keeps_papers_lazy() {
        let root = std::env::temp_dir().join(format!("aiwalk-vault-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let l1 = bare(&root, "exo-l1", &[("note.md", "l1")]);
        let papers = bare(&root, "exo-papers", &[("a.md", "note"), ("a.pdf", "PDF")]);
        let rules = r#"{"repos": {"on_demand": ["exo-papers"]}}"#;
        let work = root.join("book-work");
        std::fs::create_dir_all(work.join("System")).unwrap();
        std::fs::write(work.join("System/vault_rules.json"), rules).unwrap();
        sh(&work, &["init", "-q", "-b", "main"]);
        sh(&work, &["submodule", "add", "-q", "--name", "exo-l1", &l1, "L1_Sensing"]);
        sh(&work, &["submodule", "add", "-q", "--name", "exo-papers", &papers, "Papers"]);
        sh(&work, &["submodule", "add", "-q", "--name", "exo-l2", &bare(&root, "exo-l2", &[("x.md", "x")]), "L2_Platform"]);
        sh(&work, &["add", "-A"]);
        sh(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "book"]);
        let book = root.join("exo-book.git");
        sh(&root, &["clone", "-q", "--bare", &work.to_string_lossy(), &book.to_string_lossy()]);
        // L2 becomes unreadable, as a layer this person has no access to
        std::fs::remove_dir_all(root.join("exo-l2.git")).unwrap();

        let dest = root.join("clone");
        let msg = clone(&format!("file://{}", book.display()), &dest, &FILE, &|_| {}).unwrap();
        assert!(msg.contains("2 folders") && msg.contains("L2_Platform"), "{msg}");
        assert_eq!(std::fs::read_to_string(dest.join("L1_Sensing/note.md")).unwrap(), "l1");
        assert!(dest.join("Papers/a.md").exists() && !dest.join("Papers/a.pdf").exists(), "papers arrive without PDFs");
        assert!(std::fs::read_dir(dest.join("L2_Platform")).map(|mut d| d.next().is_none()).unwrap_or(true));
        assert!(pull(&dest, &FILE).unwrap().starts_with("Up to date"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
