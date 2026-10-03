//! The GitHub accounts signed in on this computer, kept by the app itself in the system keyring (one secret: the
//! accounts with their tokens, and which one is active). The app answers git when it asks for a GitHub password
//! (`aiwalk-setup git-credential`), so nothing here needs gh. Where gh is installed it is handed the same sign-in,
//! because people and agents still type `gh pr create`; a gh that is missing or refuses changes nothing.

use exo_core::Auth;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Default)]
struct Kept { active: String, accounts: BTreeMap<String, String> }

fn kept() -> Kept { crate::secrets::load("github").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }

fn keep(k: &Kept) -> Result<(), String> {
    if k.accounts.is_empty() { crate::secrets::forget("github"); return Ok(()) }
    crate::secrets::save("github", &serde_json::to_string(k).map_err(|e| e.to_string())?)
}

/// (login, token) of the account git and this app use.
pub fn active() -> Option<(String, String)> {
    let k = kept();
    k.accounts.get(&k.active).map(|t| (k.active.clone(), t.clone()))
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
    let key = "credential.https://github.com.helper";
    let _ = crate::cmd("git").args(["config", "--global", "--replace-all", key, ""]).output();
    let _ = crate::cmd("git").args(["config", "--global", "--add", key, &helper()]).output();
}

/// This program as a git credential helper, the way git config writes one.
pub fn helper() -> String {
    let exe = crate::own_appimage().or_else(|| std::env::current_exe().ok()).unwrap_or_default();
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
