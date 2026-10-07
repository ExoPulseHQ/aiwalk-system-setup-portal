//! The GitHub accounts signed in on this computer, kept by the app itself in the system keyring (one secret: the
//! accounts with their tokens, and which one is active). The app answers git when it asks for a GitHub password
//! (`aiwalk-setup git-credential`), so nothing here needs gh. Where gh is installed it is handed the same sign-in,
//! because people and agents still type `gh pr create`; a gh that is missing or refuses changes nothing.

use exo_core::Auth;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What renews an account's sign-in: GitHub's refresh token for it, and when the token it goes with runs out.
#[derive(Serialize, Deserialize, Clone)]
struct Renew { refresh: String, until: i64 }

#[derive(Serialize, Deserialize, Default)]
struct Kept {
    active: String,
    accounts: BTreeMap<String, String>,
    /// absent in what versions before 0.3.20 kept, and for a token that does not run out
    #[serde(default)]
    renew: BTreeMap<String, Renew>,
}

/// A token is renewed when it has less than this many seconds left.
pub const SOON: i64 = 600;

fn kept() -> Kept { crate::secrets::load("github").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }

fn keep(k: &Kept) -> Result<(), String> {
    if k.accounts.is_empty() { crate::secrets::forget("github"); return Ok(()) }
    crate::secrets::save("github", &serde_json::to_string(k).map_err(|e| e.to_string())?)
}

/// (login, token) of the account git and this app use. GitHub's sign-ins for this app last eight hours: one that is
/// about to run out is renewed here first, so git, the vault's tools and the app's window all get a token that works
/// without the person signing in again.
pub fn active() -> Option<(String, String)> {
    let k = kept();
    let token = k.accounts.get(&k.active)?.clone();
    if due(k.renew.get(&k.active).map(|r| r.until), crate::github::now()) {
        if let Some(fresh) = renewed(&k.active) { return Some((k.active, fresh)) }
    }
    Some((k.active, token))
}

/// When the active account's token runs out, if it does.
pub fn runs_out() -> Option<i64> {
    let k = kept();
    k.renew.get(&k.active).map(|r| r.until)
}

/// Whether a token that runs out at `until` should be renewed now.
fn due(until: Option<i64>, now: i64) -> bool { until.is_some_and(|u| u - now < SOON) }

/// Held while one program renews: GitHub takes a refresh token once, and several programs ask for the sign-in at
/// the same moment (the window, git's helper, the vault plugin). The second one waits and finds the work done.
struct Renewing(std::path::PathBuf);

impl Renewing {
    fn take() -> Option<Self> {
        let dir = crate::home().join(".config/aiwalk-setup");
        let _ = std::fs::create_dir_all(&dir);
        let lock = dir.join("renew.lock");
        for _ in 0..150 {
            if std::fs::create_dir(&lock).is_ok() { return Some(Renewing(lock)) }
            // left behind by a program that died while renewing
            if std::fs::metadata(&lock).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_secs() > 60)) { let _ = std::fs::remove_dir(&lock); }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        None
    }
}

impl Drop for Renewing { fn drop(&mut self) { let _ = std::fs::remove_dir(&self.0); } }

/// Renews `login`'s token and keeps the new one. None when it cannot be renewed (no refresh token, GitHub refuses,
/// no network): the old token is then used as it is, and the person signs in again once it has run out.
fn renewed(login: &str) -> Option<String> {
    let _held = Renewing::take()?;
    let mut k = kept();   // read again: another program may have renewed while this one waited
    let r = k.renew.get(login)?.clone();
    if !due(Some(r.until), crate::github::now()) { return k.accounts.get(login).cloned() }
    let (token, refresh, until) = crate::github::renew(&r.refresh).ok()?;
    k.accounts.insert(login.into(), token.clone());
    k.renew.insert(login.into(), Renew { refresh, until });
    keep(&k).ok()?;
    if k.active == login { tell_gh(&token); }
    Some(token)
}

/// `aiwalk-setup github renew`: renews the active sign-in now, whatever time it has left. Says what happened, never
/// the token. For checking by hand that renewing works.
pub fn renew_now() -> Result<String, String> {
    let _held = Renewing::take().ok_or("another program is renewing")?;
    let mut k = kept();
    let login = k.active.clone();
    let r = k.renew.get(&login).cloned().ok_or("this sign-in has nothing to renew it by: sign in once with this version")?;
    let (token, refresh, until) = crate::github::renew(&r.refresh)?;
    k.accounts.insert(login.clone(), token.clone());
    k.renew.insert(login.clone(), Renew { refresh, until });
    keep(&k)?;
    tell_gh(&token);   // the one gh held stopped working the moment this one was made
    Ok(format!("renewed for {login}: good for {} minutes more", (until - crate::github::now()) / 60))
}

pub fn accounts() -> Vec<Auth> {
    let k = kept();
    k.accounts.iter().map(|(login, token)| Auth { login: login.clone(), active: *login == k.active, protocol: "https".into(),
        method: if token.starts_with("github_pat_") { "temporary" } else { "account" }.into() }).collect()
}

/// Keeps `token` for `login` and makes it the active account.
pub fn add(login: &str, token: &str) -> Result<(), String> {
    let mut k = kept();
    k.accounts.insert(login.into(), token.into());
    // what renews this token, when GitHub gave it with one; an older one for the account no longer fits
    match crate::github::take_renewal(token) {
        Some((refresh, until)) => { k.renew.insert(login.into(), Renew { refresh, until }); }
        None => { k.renew.remove(login); }
    }
    k.active = login.into();
    keep(&k)?;
    setup_git();
    Ok(())
}

/// A computer that signed in before the app kept its own: takes over the token gh holds, once.
pub fn adopt(login: &str, token: &str) { if kept().accounts.is_empty() { let _ = add(login, token); } }

/// Signs `login` out here; another kept account, if any, becomes the active one.
pub fn remove(login: &str) -> Result<(), String> {
    let mut k = kept();
    k.accounts.remove(login);
    k.renew.remove(login);
    if k.active == login { k.active = k.accounts.keys().next().cloned().unwrap_or_default() }
    keep(&k)?;
    let _ = crate::gh(&["auth", "logout", "--hostname", "github.com", "--user", login]);
    Ok(())
}

pub fn switch(login: &str) -> Result<(), String> {
    let mut k = kept();
    if !k.accounts.contains_key(login) { return Err(format!("{login} is not signed in on this computer")) }
    k.active = login.into();
    keep(&k)?;
    let _ = crate::gh(&["auth", "switch", "--hostname", "github.com", "--user", login]);
    Ok(())
}

/// gh, where there is one, gets the same sign-in; its answer does not matter.
pub fn tell_gh(token: &str) {
    let _ = crate::sh_stdin("gh", &["auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--with-token"], Some(token), 60);
}

/// Makes this program the one git asks for a github.com password, in the user's own git config. The empty first
/// value drops any helper set before it (gh's, the system's), as `gh auth setup-git` does.
fn setup_git() {
    // nowhere lasting to point git at (a Mac running the app from Downloads or the disk image): nothing is written
    // rather than a path that is gone when the app quits; repair_git writes it at the first start from Applications
    if crate::lasting_exe().is_none() { return }
    let key = "credential.https://github.com.helper";
    let _ = crate::cmd("git").args(["config", "--global", "--replace-all", key, ""]).output();
    let _ = crate::cmd("git").args(["config", "--global", "--add", key, &helper()]).output();
}

/// At every start with someone signed in: when git's settings do not name this app as it would be named now (an
/// earlier run from a temporary folder, the app moved or reinstalled elsewhere), they are written again.
pub fn repair_git() {
    if kept().accounts.is_empty() || crate::lasting_exe().is_none() { return }
    let now = crate::cmd("git").args(["config", "--global", "--get-all", "credential.https://github.com.helper"]).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if now.lines().last().map(str::trim) != Some(helper().as_str()) { setup_git() }
}

/// This program as a git credential helper, the way git config writes one.
pub fn helper() -> String {
    let exe = crate::lasting_exe().or_else(|| std::env::current_exe().ok()).unwrap_or_default();
    format!("!'{}' git-credential", exe.to_string_lossy().replace('\\', "/"))
}

/// `aiwalk-setup git-credential <get|store|erase>`: git's credential helper protocol. Only `get` answers.
pub fn credential(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("get") { return 0 }
    let mut input = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    if let Some((login, token)) = active() { print!("{}", exo_core::credential_reply(&input, &login, &token).unwrap_or_default()) }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_in_kept_by_an_older_version_still_reads_and_one_about_to_run_out_is_due() {
        // what versions before the renewal kept: no "renew" at all
        let old: Kept = serde_json::from_str(r#"{"active":"jo","accounts":{"jo":"t1"}}"#).unwrap();
        assert!(old.renew.is_empty() && old.accounts["jo"] == "t1");
        let new: Kept = serde_json::from_str(r#"{"active":"jo","accounts":{"jo":"t1"},"renew":{"jo":{"refresh":"r1","until":5000}}}"#).unwrap();
        assert_eq!(new.renew["jo"].until, 5000);
        // a token with no end is never renewed; one with ten minutes or less left is
        assert!(!due(None, 1000));
        assert!(!due(Some(5000), 1000) && due(Some(5000), 4401) && due(Some(5000), 9000));
    }
}
