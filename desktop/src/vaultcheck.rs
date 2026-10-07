//! The vault's two checks on what is about to be uploaded: every new [[link]] resolves, and a primary log's ownership
//! block is up to date. Shared by `vault ship` on a computer (vaultcli.rs, which asks the git program for a file's
//! last version and today's date) and by sending from a phone (phonegit.rs, which asks libgit2).

use exo_core::vault_ship as vs;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const INDEX: &str = "System/vault_index.json";

/// Python's text-mode write: "\n" becomes the platform's line ending.
pub fn write_text(path: &Path, s: &str) -> std::io::Result<()> {
    std::fs::write(path, if cfg!(windows) { s.replace('\n', "\r\n") } else { s.to_string() })
}

pub fn checked_out(vault: &Path, sub: &str) -> bool {
    std::fs::read_dir(vault.join(sub)).is_ok_and(|mut d| d.next().is_some())
}

/// name_key: sha1 of the name a wikilink resolves by, first 12 hex digits.
pub fn name_key(name: &str) -> String {
    let d = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, vs::name_key_text(name, cfg!(windows)).as_bytes());
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>()[..12].to_string()
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
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
    // by the folder's own name, so Windows' backslashes do not let the walk into .git
    if dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".git") || n == "node_modules") { return }
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
/// `root` is the repo the `paths` are in (the vault or one of its folders); `before(path)` is the file's text in the
/// last commit ("" for a new file); `also` are names that exist without being on this device (a phone's files left
/// on GitHub). Ok(the links that resolve to nothing, as "path: [[target]]"); Err when a file cannot be read.
pub fn broken_links(vault: &Path, root: &Path, paths: &[String], before: &dyn Fn(&str) -> String, also: &HashSet<String>) -> Result<Vec<String>, String> {
    let mut names = HashSet::new();
    walk_names(vault, &mut names);
    let hidden = restricted_keys(vault);
    let mut bad = vec![];
    for p in paths {
        let file = root.join(p);
        if !p.ends_with(".md") || p.rsplit('/').next() == Some("CLAUDE.md") || !file.exists() { continue }
        let was: HashSet<String> = vs::link_targets(&before(p)).into_iter().collect();
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{p}: {e}"))?;
        for t in vs::link_targets(&vs::py_text(&text)) {
            if !t.is_empty() && !names.contains(&t) && !also.contains(&t) && !was.contains(&t) && !hidden.contains(&name_key(&t)) {
                bad.push(format!("{p}: [[{t}]]"));
            }
        }
    }
    Ok(bad)
}

/// glob("**/*") over the vault: base name -> first path, in directory order, hidden names left out.
pub fn glob_index(dir: &Path, names: &mut HashMap<String, PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let entries: Vec<PathBuf> = rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| e.path()).collect();
    for p in entries.iter().filter(|p| p.is_file() && !p.to_string_lossy().contains("/.")) {
        names.entry(p.file_name().unwrap().to_string_lossy().into_owned()).or_insert_with(|| p.clone());
    }
    for p in entries.iter().filter(|p| p.is_dir()) { glob_index(p, names) }
}

/// The primary log at `path` with the generated half of its ownership block rebuilt: Some(new text) when that
/// differs from the file, None when it is up to date or has no generated half. `rel` is the path as the block
/// prints it; `today()` is today's local date.
pub fn ownership(vault: &Path, path: &Path, rel: &str, today: &dyn Fn() -> Option<(i64, i64, i64)>) -> Result<Option<String>, String> {
    let mut names = HashMap::new();
    glob_index(vault, &mut names);
    let team = std::fs::read_to_string(vault.join(vs::TEAM)).map_err(|e| format!("{}: {e}", vs::TEAM))?;
    let people = vs::team(&vs::py_text(&team));
    let day = today().ok_or("git did not give today's date")?;
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
