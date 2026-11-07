//! Repo keys on a machine: each GitHub repo of the team that a machine works with (a submodule's repo too) gets a key
//! of its own, so the shared login account fetches and pushes as that repo, never as a person
//! (vault Project_Management/Team_Access_Identity_Design_Summary.md). The Machines page checks them and issues the
//! missing ones; hosts/exo-keys.py does the machine's part over ssh, GitHub's part (the repo's deploy keys) is here.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const SCRIPT: &str = include_str!("../../hosts/exo-keys.py");

/// (ssh host, "owner/repo") of a git URL: git@host:o/r(.git), ssh://git@host/o/r, https://host/o/r. The host is
/// github.com or an ssh alias (github-exopulse). None for anything else (a relative or local URL).
pub fn parse(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("ssh://")) {
        let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else {
        let rest = url.split_once('@')?.1;
        rest.split_once(':')?
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (o, r) = path.split_once('/')?;
    (!o.is_empty() && !r.is_empty() && !r.contains('/') && !host.contains('/')).then(|| (host.to_string(), format!("{o}/{r}")))
}

/// The key's short name for a repo: ExoPulseHQ/Askesis gives askesis, ExoPulseHQ/CEINMS_ gives ceinms.
pub fn key_name(repo: &str) -> String {
    let r = repo.rsplit('/').next().unwrap_or(repo).to_lowercase();
    r.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-")
}

/// What git turns `url` into: the longest insteadOf prefix that matches, as git picks it.
fn rewritten(url: &str, rewrites: &BTreeMap<String, String>) -> String {
    rewrites.iter().filter(|(from, _)| url.starts_with(from.as_str())).max_by_key(|(from, _)| from.len())
        .map_or(url.to_string(), |(from, to)| format!("{to}{}", &url[from.len()..]))
}

fn sh_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

fn script(user: &str, tunnel: &str, args: &[&str], secs: u64) -> Result<Value, String> {
    let cmd = std::iter::once("python3 -".to_string()).chain(args.iter().map(|a| sh_quote(a))).collect::<Vec<_>>().join(" ");
    let (code, out) = crate::machines::on_machine(user, tunnel, &cmd, Some(SCRIPT.as_bytes()), secs);
    let line = out.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or_default();
    match serde_json::from_str::<Value>(line) {
        Ok(v) if v["ok"] == true => Ok(v),
        Ok(v) => Err(v["error"].as_str().unwrap_or("the machine refused").to_string()),
        Err(_) if code == 255 => Err(out.trim().to_string()),
        Err(_) => Err(out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("The machine did not answer").to_string()),
    }
}

/// The repo a URL names now: a moved or renamed repo answers GitHub under its new name. None when it cannot be read.
fn current(repo: &str) -> Option<String> {
    crate::github::get(&format!("repos/{repo}")).ok().and_then(|v| v["full_name"].as_str().map(String::from))
}

/// Each team repo the machine uses, with what is in place: {repo, name, used: [where], urls, key, alias, routed,
/// registered (true / false / null when this account cannot read the repo's keys), ok}.
#[tauri::command(async)]
pub fn repo_keys(tunnel: String, user: String, org: String) -> Result<Vec<Value>, String> {
    let scan = script(&user, &tunnel, &["scan"], 60)?;
    let aliases: BTreeMap<String, String> = serde_json::from_value(scan["aliases"].clone()).unwrap_or_default();
    let rewrites: BTreeMap<String, String> = serde_json::from_value(scan["rewrites"].clone()).unwrap_or_default();
    let keys: BTreeMap<String, String> = serde_json::from_value(scan["keys"].clone()).unwrap_or_default();
    // every GitHub URL in use: an origin, or a submodule's URL; where it is used
    let mut uses: Vec<(String, String)> = vec![];
    for r in scan["repos"].as_array().into_iter().flatten() {
        let path = r["path"].as_str().unwrap_or_default();
        if let Some(o) = r["origin"].as_str() { uses.push((o.to_string(), path.to_string())) }
        for s in r["submodules"].as_array().into_iter().flatten() {
            uses.push((s["url"].as_str().unwrap_or_default().to_string(), format!("{path} ({})", s["name"].as_str().unwrap_or_default())));
        }
    }
    let named: BTreeSet<String> = uses.iter().filter_map(|(u, _)| parse(u).map(|p| p.1)).collect();
    // the current name of each, asked side by side (a machine has dozens, most of them other people's repos)
    let now: BTreeMap<String, Option<String>> = std::thread::scope(|s| {
        let hs: Vec<_> = named.iter().map(|r| (r.clone(), s.spawn(move || current(r)))).collect();
        hs.into_iter().map(|(r, h)| (r, h.join().ok().flatten())).collect()
    });
    let mut by: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (url, place) in &uses {
        let Some((_, repo)) = parse(url) else { continue };
        let Some(Some(cur)) = now.get(&repo) else { continue };
        if !cur.to_lowercase().starts_with(&format!("{}/", org.to_lowercase())) { continue }
        let e = by.entry(cur.clone()).or_default();
        e.0.insert(place.clone()); e.1.insert(url.clone());
    }
    Ok(by.into_iter().map(|(repo, (used, urls))| {
        let name = key_name(&repo);
        let alias = format!("github-{name}");
        let key = keys.get(&name).cloned();
        let has_alias = aliases.get(&alias).is_some_and(|f| f.ends_with(&format!("deploy_{name}")));
        let routed = urls.iter().all(|u| parse(&rewritten(u, &rewrites)).is_some_and(|(h, _)| h == alias));
        let body = |k: &str| k.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let registered = match (&key, crate::github::all(&format!("repos/{repo}/keys"))) {
            (Some(k), Ok(list)) => json!(list.iter().any(|g| g["key"].as_str().is_some_and(|x| body(x) == body(k)))),
            (None, Ok(_)) => json!(false),
            (_, Err(_)) => Value::Null,
        };
        let ok = key.is_some() && has_alias && routed && registered == true;
        json!({ "repo": repo, "name": name, "used": used, "urls": urls, "key": key.is_some(), "alias": has_alias, "routed": routed, "registered": registered, "ok": ok })
    }).collect())
}

/// Issues the repo's key on the machine and registers it with GitHub: the machine makes ~/.ssh/deploy_<name> if it has
/// none, the alias github-<name> and the URL rewrites; GitHub gets the public key as the repo's deploy key (with
/// write access: work is pushed from the machines); then the alias is tried. Ok(what GitHub answered).
#[tauri::command(async)]
pub fn issue_repo_key(tunnel: String, user: String, host: String, repo: String, urls: Vec<String>) -> Result<String, String> {
    let name = key_name(&repo);
    let mut args = vec!["add", name.as_str(), repo.as_str()];
    args.extend(urls.iter().map(String::as_str));
    let made = script(&user, &tunnel, &args, 60)?;
    let key = made["key"].as_str().ok_or("The machine gave no key")?.to_string();
    let body = |k: &str| k.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    let list = crate::github::all(&format!("repos/{repo}/keys")).map_err(|e| format!("Cannot read {repo}'s deploy keys ({e}); an owner of the repo can issue this key"))?;
    if !list.iter().any(|g| g["key"].as_str().is_some_and(|x| body(x) == body(&key))) {
        crate::github::send("POST", &format!("repos/{repo}/keys"), Some(json!({
            "title": format!("{host} {user} {name} (aIwalk System Setup)"), "key": key, "read_only": false })))
            .map_err(|e| format!("GitHub did not take the key: {e}"))?;
    }
    let answer = script(&user, &tunnel, &["verify", &name], 40)?["answer"].as_str().unwrap_or_default().to_string();
    if answer.contains(&format!("Hi {repo}!")) || answer.to_lowercase().contains(&format!("hi {}!", repo.to_lowercase())) {
        Ok(format!("{repo}: {host} now fetches and pushes with its own key"))
    } else {
        Err(format!("The key is in place, but GitHub answered: {}", if answer.is_empty() { "nothing" } else { &answer }))
    }
}

/// `aiwalk-setup --repo-keys [account@]MACHINE [--org ORG] [--issue OWNER/REPO]`: the check as JSON, or one key issued.
pub fn cli(args: &[String]) -> i32 {
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let Some(dest) = args.first().filter(|a| !a.starts_with("--")) else { eprintln!("usage: aiwalk-setup --repo-keys [account@]MACHINE [--org ORG] [--issue OWNER/REPO]"); return 2 };
    let (user, tunnel) = match crate::machines::ready(dest) { Ok(x) => x, Err(e) => { eprintln!("{e}"); return 255 } };
    let host = dest.rsplit('@').next().unwrap_or(dest).to_string();
    let org = flag("--org").unwrap_or_else(|| "ExoPulseHQ".into());
    match flag("--issue") {
        Some(repo) => {
            let urls = match repo_keys(tunnel.clone(), user.clone(), org) {
                Ok(rows) => rows.iter().find(|r| r["repo"] == repo.as_str()).and_then(|r| serde_json::from_value(r["urls"].clone()).ok()).unwrap_or_default(),
                Err(e) => { eprintln!("{e}"); return 1 }
            };
            match issue_repo_key(tunnel, user, host, repo, urls) { Ok(m) => { println!("{m}"); 0 } Err(e) => { eprintln!("{e}"); 1 } }
        }
        None => match repo_keys(tunnel, user, org) { Ok(r) => { println!("{}", serde_json::to_string_pretty(&r).unwrap()); 0 } Err(e) => { eprintln!("{e}"); 1 } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_name_their_repo() {
        assert_eq!(parse("git@github.com:ExoPulseHQ/Askesis.git"), Some(("github.com".into(), "ExoPulseHQ/Askesis".into())));
        assert_eq!(parse("git@github-exopulse:ExoPulseHQ/ExoPulse.git"), Some(("github-exopulse".into(), "ExoPulseHQ/ExoPulse".into())));
        assert_eq!(parse("https://github.com/eddLai/CEINMS_.git"), Some(("github.com".into(), "eddLai/CEINMS_".into())));
        assert_eq!(parse("ssh://git@github.com/o/r/"), Some(("github.com".into(), "o/r".into())));
        assert_eq!(parse("../ceinmsrt-plugin-udp-cpp"), None);
        assert_eq!(parse("file:///home/sean/x"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn key_names_match_the_ones_already_on_the_machines() {
        assert_eq!(key_name("ExoPulseHQ/ExoPulse"), "exopulse");
        assert_eq!(key_name("ExoPulseHQ/NTKCAP"), "ntkcap");
        assert_eq!(key_name("ExoPulseHQ/CEINMS_"), "ceinms");
        assert_eq!(key_name("ExoPulseHQ/hexis-core"), "hexis-core");
    }

    #[test]
    fn the_longest_rewrite_wins() {
        let rw: BTreeMap<String, String> = [("git@github.com:".to_string(), "x:".to_string()),
            ("git@github.com:ExoPulseHQ/Askesis.git".to_string(), "git@github-askesis:ExoPulseHQ/Askesis.git".to_string())].into();
        assert_eq!(rewritten("git@github.com:ExoPulseHQ/Askesis.git", &rw), "git@github-askesis:ExoPulseHQ/Askesis.git");
        assert_eq!(rewritten("git@github.com:o/r", &rw), "x:o/r");
        assert_eq!(rewritten("https://github.com/o/r", &rw), "https://github.com/o/r");
    }
}
