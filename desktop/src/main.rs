#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use exo_core::{
    build_tree, machines, terms_accepted, terms_title, terms_version, Auth, org_access, org_query, parse_request, permissions_rank, repo_grants, repos_query, request_body,
    vault_repos, visible_to, Machine, Node, Person, Team, RULES_PATHS,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Emitter;

// The phone and VM modules drive a Linux desktop (adb, scrcpy, docker, GNOME keyring).
mod access;
mod admin;
mod claude;
mod cloudflare;
mod deckcli;
mod github;
mod guardcli;
mod hooks;
mod login;
mod machines;
mod ptycli;
mod secrets;
mod sessioncli;
mod update;
mod vault;
mod vaultcli;
#[cfg(target_os = "linux")]
mod android;
#[cfg(target_os = "linux")]
mod vm;

/// The person's home folder on every OS: $HOME on Linux and Mac, the user profile folder on Windows (which has no $HOME).
#[allow(deprecated)] // home_dir is correct on Windows since Rust 1.85
pub fn home() -> PathBuf { std::env::home_dir().unwrap_or_default() }

/// The app's folder: bundled tools (tools/), phone apps (apk/) and icon.png sit next to the binary.
pub fn here() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(PathBuf::from)).unwrap_or_default()
}

/// The AppImage this program runs from, None when it is not one. See exo_core::own_appimage: the environment alone
/// cannot be trusted, since children of another AppImage (Obsidian) inherit its APPIMAGE.
pub fn own_appimage() -> Option<PathBuf> {
    let var = |k: &str| std::env::var(k).ok();
    let exe = std::env::current_exe().ok()?;
    exo_core::own_appimage(var("APPIMAGE").as_deref(), var("APPDIR").as_deref(), &exe.to_string_lossy()).map(PathBuf::from)
}

/// A bundled tool next to the app, else the one on PATH.
pub fn find_tool(bundled: &str, name: &str) -> String {
    let p = here().join(bundled);
    if p.exists() { return p.to_string_lossy().into() }
    on_path(name).map(|p| p.to_string_lossy().into()).unwrap_or_else(|| name.into())
}

/// `name` as PATH would find it (with .exe on Windows).
pub fn on_path(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(&file)).find(|p| p.is_file()))
}

/// Puts the bundled git (Windows: MinGit) ahead of PATH for this app and everything it starts, so a new
/// member's computer needs neither. Bundles sit next to the binary (Linux folder, Windows), or in Contents/Resources (Mac).
fn use_bundled_tools() {
    let roots = [here(), here().join("../Resources"), here().join("../lib/aIwalk System Setup")];
    let dirs: Vec<PathBuf> = roots.iter().map(|r| r.join("tools/git/cmd"))
        .filter(|d| d.is_dir()).collect();
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Ok(joined) = std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&path))) {
        std::env::set_var("PATH", joined);
    }
}

#[derive(Serialize)]
struct Tools {
    git: Option<String>,
    os: &'static str,
}

/// Where git comes from on this computer; None when missing, and the page says how to get it.
#[tauri::command]
fn tools() -> Tools {
    let s = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    Tools { git: s(on_path("git")), os: std::env::consts::OS }
}

/// macOS: asks Apple's installer for the command line tools, which include git.
#[tauri::command]
fn install_git() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return crate::cmd("xcode-select").arg("--install").spawn().map(|_| ()).map_err(|e| e.to_string());
    #[allow(unreachable_code)]
    Err("Install git with your system's package manager".into())
}

/// Runs a program with optional stdin, killed after `timeout` seconds; (exit code, stdout + stderr).
/// 124 on timeout, 1 when it could not start, as the Python app did.
/// A child process; on Windows without the console window a GUI app's child would otherwise flash up.
pub fn cmd(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(prog);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }   // CREATE_NO_WINDOW
    c
}

pub fn sh_stdin(prog: &str, args: &[&str], stdin: Option<&str>, timeout: u64) -> (i32, String) {
    let child = crate::cmd(prog).args(args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child { Ok(c) => c, Err(e) => return (1, e.to_string()) };
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) { let _ = pipe.write_all(input.as_bytes()); }
    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) if start.elapsed() > Duration::from_secs(timeout) => { let _ = child.kill(); let _ = child.wait(); return (124, String::new()) }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return (1, e.to_string()),
        }
    };
    // ponytail: output is read after exit; a tool that fills the 64 KB pipe before exiting stalls until the timeout
    match child.wait_with_output() {
        Ok(o) => (code, String::from_utf8_lossy(&[o.stdout, o.stderr].concat()).trim().to_string()),
        Err(e) => (1, e.to_string()),
    }
}

pub fn sh(prog: &str, args: &[&str], timeout: u64) -> (i32, String) { sh_stdin(prog, args, None, timeout) }

/// Which modules this computer gets: the phone and VM pages need Linux.
#[tauri::command]
fn platform() -> &'static str { std::env::consts::OS }

/// The page to open on, for looking at one page of the real window without clicking: AIWALK_START_PAGE=machines.
#[tauri::command]
fn start_page() -> Option<String> { std::env::var("AIWALK_START_PAGE").ok().filter(|p| !p.is_empty()) }

// ponytail: vault list is hard-coded, same as the Python app; move to a config file when teams need different lists
pub const VAULTS: [(&str, &str, &str); 2] = [
    ("ExoPulse docs", "ExoPulseHQ/exo-book", "The team's technical documents, papers and progress notes"),
    ("aIwalk Corp", "eddLai/aIwalk_Corp", "Company records, founders only"),
];
/// Where access requests travel, as issues; the org's members can open issues there but not write.
const REQUESTS: &str = "exo-access-requests";

/// Runs gh; Ok(stdout) on exit 0, Err(stderr or why it could not start) otherwise.
pub fn gh(args: &[&str]) -> Result<String, String> {
    let out = crate::cmd("gh").args(args).output().map_err(|e| format!("gh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn raw(repo: &str, path: &str) -> Result<String, String> { github::raw(repo, path) }

#[derive(Serialize)]
struct Vault {
    name: &'static str,
    repo: &'static str,
    about: &'static str,
    /// None when the vault is not split into repos or this account cannot read it.
    access: Option<Access>,
    /// This account's permission on the vault repo itself (4 admin … 0 none); what counts for a vault that is one repo.
    permission: u8,
}

#[derive(Serialize)]
struct Access {
    org: String,
    tree: Node,
    people: BTreeMap<String, Person>,
    /// Teams an owner can move people in (core excluded); empty for everyone else.
    teams: Vec<Team>,
    /// Every machine and the code repo it carries (aliases only).
    machines: Vec<Machine>,
    /// Open requests: an owner's are everyone's, anyone else's are their own.
    requests: Vec<Request>,
}

#[derive(Serialize)]
struct Request {
    number: u64,
    author: String,
    repo: String,
    level: String,
    body: String,
}

/// Every open request; whose they are is the issue's author.
fn open_requests(org: &str) -> Vec<Request> {
    let issues = github::all(&format!("repos/{org}/{REQUESTS}/issues?state=open")).unwrap_or_default();
    issues.iter().filter(|i| i.get("pull_request").is_none()).filter_map(|i| {
        let body = i["body"].as_str()?.to_string();
        let (repo, level) = parse_request(&body)?;
        Some(Request { number: i["number"].as_u64()?, author: i["user"]["login"].as_str()?.to_string(), repo, level, body })
    }).collect()
}

/// Closes an issue, with a comment first when one is given. `reason` is "completed" or "not_planned".
fn close_issue(repo: &str, number: u64, reason: &str, comment: Option<&str>) -> Result<(), String> {
    if let Some(c) = comment {
        github::send("POST", &format!("repos/{repo}/issues/{number}/comments"), Some(serde_json::json!({ "body": c })))?;
    }
    github::send("PATCH", &format!("repos/{repo}/issues/{number}"), Some(serde_json::json!({ "state": "closed", "state_reason": reason }))).map(|_| ())
}

#[derive(Serialize)]
struct State {
    /// Signed-in GitHub login, None when gh is missing or signed out.
    user: Option<String>,
    /// Display name from the GitHub profile, for the badge.
    name: Option<String>,
    /// Every account gh is signed in to here; the active one is `user`.
    accounts: Vec<Auth>,
    error: Option<String>,
    vaults: Vec<Vault>,
    /// What the sign-in may do on GitHub ("repo", "read:org", "admin:org", ...); owners' tools need admin:org.
    scopes: Vec<String>,
    /// Signed in, but no vault this account can read lists a machine: the guest path on Machines.
    guest: bool,
}

/// Reads everything Team access shows; GitHub answers slowly, so each step is announced to the page as
/// "team-stage" (step, steps, what is being read).
#[tauri::command(async)]
fn team_access(app: tauri::AppHandle) -> State {
    read_access(&|i, n, text| { let _ = app.emit("team-stage", (i, n, text)); })
}

fn read_access(stage: &dyn Fn(usize, usize, &str)) -> State {
    stage(1, 2, "Checking who is signed in");
    let me = match github::get("user") {
        Ok(v) => v,
        Err(e) => return State { user: None, name: None, accounts: vec![], error: Some(e), vaults: vec![], scopes: vec![], guest: false },
    };
    let user = me["login"].as_str().unwrap_or_default().to_string();
    let name = me["name"].as_str().filter(|n| !n.is_empty()).map(String::from);
    stage(2, 2, "Reading your access from GitHub");
    // a sign-in made before the app kept its own is taken over from gh here, once
    if let Some(t) = github::current_token() { login::adopt(&user, &t) }
    // the vaults are read side by side; each vault asks its questions side by side too
    let vaults: Vec<Vault> = std::thread::scope(|s| {
        let vaults: Vec<_> = VAULTS.iter().map(|&(name, repo, about)| { let user = &user; s.spawn(move || read_vault(name, repo, about, user)) }).collect();
        vaults.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let guest = exo_core::is_guest(true, &vaults.iter().map(|v: &Vault| v.access.as_ref().map_or(0, |a| a.machines.len())).collect::<Vec<_>>());
    State { user: Some(user), name, accounts: login::accounts(), error: None, vaults, scopes: github::scopes(), guest }
}

fn read_vault(name: &'static str, repo: &'static str, about: &'static str, user: &str) -> Vault {
    std::thread::scope(|s| {
        let permission = s.spawn(|| permissions_rank(&github::get(&format!("repos/{repo}")).map(|v| v["permissions"].clone()).unwrap_or_default()));
        let rules = RULES_PATHS.iter().find_map(|p| raw(repo, p).ok());
        let access = rules.as_deref().and_then(vault_repos).map(|v| {
            let requests = s.spawn({ let org = v.org.clone(); move || open_requests(&org) });
            let query = |q: String| github::graphql(&q).unwrap_or_default();
            let mut org = org_access(&query(org_query(&v.org)));
            let owner = org.people.get(user).is_some_and(|p| p.grants.is_none());
            repo_grants(&mut org.people, &query(repos_query(&v.org, owner)), user);
            // owners see every team; anyone else only the teams they are in, with no one else's name, which is
            // enough to tell them which machines they may reach
            let teams = if owner { org.teams } else {
                org.teams.into_iter().filter(|t| t.members.contains(user))
                    .map(|mut t| { t.members.retain(|m| m == user); t }).collect()
            };
            let machines = machines(rules.as_deref().unwrap_or_default());
            let people = visible_to(org.people, user);
            // an owner's requests are everyone's, anyone else's are their own
            let requests = requests.join().unwrap_or_default().into_iter().filter(|r| owner || r.author == user).collect();
            Access { tree: build_tree(&v, &org.repo_teams), people, teams, machines, requests, org: v.org }
        });
        Vault { name, repo, about, access, permission: permission.join().unwrap_or(0) }
    })
}

/// Signs `login` out on this computer; any other account stays signed in and one of them becomes active.
#[tauri::command(async)]
fn sign_out(login: String) -> Result<(), String> {
    login::remove(&login)?;
    github::forget_token();
    machines::forget_access();   // the machines must not keep letting the signed-out person in
    Ok(())
}

/// Makes another signed-in account the one git and this app use.
#[tauri::command(async)]
fn switch_account(login: String) -> Result<(), String> {
    login::switch(&login)?;
    github::forget_token();
    machines::forget_access();   // the next lab connection signs in as this account, not the previous one
    Ok(())
}

/// Adds `login` to or removes them from an org team. GitHub itself refuses anyone who is not an owner.
#[tauri::command(async)]
fn set_team(org: String, team: String, login: String, member: bool) -> Result<(), String> {
    let path = format!("orgs/{org}/teams/{team}/memberships/{login}");
    if member { github::send("PUT", &path, Some(serde_json::json!({ "role": "member" }))) } else { github::send("DELETE", &path, None) }.map(|_| ())
}

/// Opens an access request as an issue in the org's request repo.
#[tauri::command(async)]
fn request_access(org: String, repo: String, level: String, note: String) -> Result<(), String> {
    if level != "read" && level != "write" { return Err(format!("unknown level {level}")) }
    github::send("POST", &format!("repos/{org}/{REQUESTS}/issues"), Some(serde_json::json!({
        "title": format!("{level} access to {repo}"), "body": request_body(&repo, &level, &note) }))).map(|_| ())
}

/// Grants an open request and closes it. Who gets access is the issue's author, never anything in its body.
/// A team that grants exactly that repo at that level is used when there is one, otherwise a direct grant.
#[tauri::command(async)]
fn approve_request(org: String, number: u64) -> Result<(), String> {
    let issues = format!("{org}/{REQUESTS}");
    let issue = github::get(&format!("repos/{issues}/issues/{number}"))?;
    if issue["state"] != "open" { return Err("this request is already closed".into()) }
    let login = issue["user"]["login"].as_str().ok_or("request has no author")?;
    let (repo, level) = parse_request(issue["body"].as_str().unwrap_or_default()).ok_or("request body is not readable")?;
    let rank = if level == "write" { 2 } else { 1 };
    let team = org_access(&github::graphql(&org_query(&org))?).teams.into_iter()
        .find(|t| t.slug != "core" && t.repos.len() == 1 && t.repos[0] == (repo.clone(), rank));
    match team {
        Some(t) => github::send("PUT", &format!("orgs/{org}/teams/{}/memberships/{login}", t.slug), Some(serde_json::json!({ "role": "member" })))?,
        None => github::send("PUT", &format!("repos/{org}/{repo}/collaborators/{login}"),
                             Some(serde_json::json!({ "permission": if level == "write" { "push" } else { "pull" } })))?,
    };
    close_issue(&issues, number, "completed", Some("Approved in aIwalk System Setup."))
}

#[tauri::command(async)]
fn decline_request(org: String, number: u64) -> Result<(), String> {
    close_issue(&format!("{org}/{REQUESTS}"), number, "not_planned", Some("Declined in aIwalk System Setup."))
}

// ---------------------------------------------------------------- Terms
// The team's terms live in exo-access-requests/TERMS.md, so owners change the words without a new release; the
// version line at its top decides when people are asked again. Accepting opens and closes an issue there titled
// "Terms vN accepted": its author, checked by GitHub, is who accepted, and its date is when.

const TERMS: &str = "TERMS.md";

/// The terms text, its version, and the newest version this account accepted (with the date). None when the
/// organisation has no terms file or nobody is signed in, so nothing is asked. {org, pending: invitation page} when
/// this account is invited to the requests repo and has not accepted yet.
#[tauri::command(async)]
fn terms_state() -> Option<serde_json::Value> {
    // asked before anything else is read, so the organisation is the first vault's owner, not the vault's rules
    let org = VAULTS[0].1.split('/').next()?.to_string();
    let repo = format!("{org}/{REQUESTS}");
    // the text and this account's acceptances, side by side
    let (text, mine) = std::thread::scope(|s| {
        let text = s.spawn(|| github::raw(&repo, TERMS).ok());
        let login = github::get("user").ok().and_then(|u| u["login"].as_str().map(String::from)).unwrap_or_default();
        let mine = github::all(&format!("repos/{repo}/issues?state=all&creator={login}")).unwrap_or_default();
        (text.join().ok().flatten(), mine)
    });
    // invited to the requests repo but not accepted yet (a guest or intern): the terms cannot be read until then
    let Some(text) = text else {
        let pending = exo_core::pending_invitation(&github::all("user/repository_invitations").unwrap_or_default(), &repo)?;
        return Some(serde_json::json!({ "org": org, "pending": pending }));
    };
    let version = terms_version(&text)?;
    let accepted = terms_accepted(&serde_json::Value::Array(mine).to_string()).map(|(v, date)| serde_json::json!({ "version": v, "date": date }));
    Some(serde_json::json!({ "org": org, "version": version, "text": text, "accepted": accepted }))
}

#[tauri::command(async)]
fn terms_accept(org: String, version: u32) -> Result<(), String> {
    let repo = format!("{org}/{REQUESTS}");
    let issue = github::send("POST", &format!("repos/{repo}/issues"), Some(serde_json::json!({
        "title": terms_title(version), "body": format!("terms: {version}\n\nAccepted in aIwalk System Setup.") })))?;
    // closed at once: it is a record, not something for the owners to act on
    if let Some(n) = issue["number"].as_u64() { let _ = close_issue(&repo, n, "completed", None); }
    Ok(())
}

/// Owners: everyone's newest acceptance, login -> {version, date}.
#[tauri::command(async)]
fn terms_everyone(org: String) -> BTreeMap<String, serde_json::Value> {
    let rows = github::all(&format!("repos/{org}/{REQUESTS}/issues?state=all")).unwrap_or_default();
    let mut by: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for r in rows { if let Some(l) = r["user"]["login"].as_str() { by.entry(l.to_string()).or_default().push(r.clone()) } }
    by.into_iter().filter_map(|(login, rs)| {
        let (v, date) = terms_accepted(&serde_json::to_string(&rs).ok()?)?;
        Some((login, serde_json::json!({ "version": v, "date": date })))
    }).collect()
}

pub fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = crate::cmd("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = crate::cmd("open").arg(url).spawn();
    // not `cmd /c start`: cmd reads every & in the address as the end of the command, so a sign-in address arrived
    // at the browser cut off after its first parameter ("Invalid redirect URL" from Cloudflare Access)
    #[cfg(windows)]
    let _ = crate::cmd("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
}

/// Signs in with GitHub: emits "gh-code" with the one-time code and opens the page to enter it on; true once
/// GitHub approved. `owner` asks for the permission to manage the organisation as well.
#[tauri::command(async)]
fn sign_in(app: tauri::AppHandle, owner: Option<bool>) -> bool {
    let scopes = if owner == Some(true) { github::SCOPES_OWNER } else { github::SCOPES_MEMBER };
    let token = match github::device_sign_in(scopes, &|code, url| { let _ = app.emit("gh-code", code); open_url(url); }) {
        Ok(t) => t,
        Err(e) => { let _ = app.emit("gh-line", e); return false }
    };
    // whose token it is, then the app keeps it: git, the vault's tools and this app all read the account from there
    github::use_token(&token);
    let kept = github::get("user").and_then(|u| u["login"].as_str().map(String::from).ok_or("GitHub did not say who signed in".into()))
        .and_then(|user| login::add(&user, &token));
    github::forget_token();
    if let Err(e) = kept { let _ = app.emit("gh-line", format!("Signed in, but this computer would not keep the sign-in: {e}")); return false }
    login::tell_gh(&token);
    true
}

#[cfg(target_os = "linux")]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool {
    tauri::generate_handler![platform, start_page, tools, cloudflare::cf_state, cloudflare::cf_connect, cloudflare::cf_forget, cloudflare::cf_share, cloudflare::cf_key, cloudflare::cf_join, cloudflare::cf_renew, update::update_state, update::update_install, terms_state, terms_accept, terms_everyone, install_git, team_access, claude::claude_state, claude::claude_install, claude::claude_login, machines::reachable, machines::machine_status, machines::open_forward, machines::close_forward, machines::forwards, machines::open_viewer, machines::desktop, machines::update_host_tools, machines::lab_identity, machines::lab_sign_out, machines::access_login, machines::ssh_status, machines::ssh_setup, machines::find_machine, machines::open_ssh,
                             admin::org_people, admin::invite, admin::cancel_invite, admin::invite_intern, admin::remove_intern, admin::set_role, admin::remove_member, admin::set_access, admin::machine_extra, admin::machine_guests, admin::machine_guest, admin::public_email, admin::pr_permissions, admin::merge_right, sign_in, sign_out, switch_account,
                             vault::vault_local, vault::vault_download, vault::vault_link, vault::pick_folder, vault::default_folder, vault::vault_update, vault::vault_open, vault::obsidian_install, set_team, request_access, approve_request, decline_request,
                             android::phones, android::phone_action, vm::vm_state, vm::vm_action]
}
#[cfg(not(target_os = "linux"))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool {
    tauri::generate_handler![platform, start_page, tools, cloudflare::cf_state, cloudflare::cf_connect, cloudflare::cf_forget, cloudflare::cf_share, cloudflare::cf_key, cloudflare::cf_join, cloudflare::cf_renew, update::update_state, update::update_install, terms_state, terms_accept, terms_everyone, install_git, team_access, claude::claude_state, claude::claude_install, claude::claude_login, machines::reachable, machines::machine_status, machines::open_forward, machines::close_forward, machines::forwards, machines::open_viewer, machines::desktop, machines::update_host_tools, machines::lab_identity, machines::lab_sign_out, machines::access_login, machines::ssh_status, machines::ssh_setup, machines::find_machine, machines::open_ssh,
                             admin::org_people, admin::invite, admin::cancel_invite, admin::invite_intern, admin::remove_intern, admin::set_role, admin::remove_member, admin::set_access, admin::machine_extra, admin::machine_guests, admin::machine_guest, admin::public_email, admin::pr_permissions, admin::merge_right, sign_in, sign_out, switch_account,
                             vault::vault_local, vault::vault_download, vault::vault_link, vault::pick_folder, vault::default_folder, vault::vault_update, vault::vault_open, vault::obsidian_install, set_team, request_access, approve_request, decline_request]
}

/// Windows: the release build is a GUI program, so a console it is started from is not given to it. A command-line
/// use (any argument) with a standard handle missing attaches to the parent's console, when there is one, and takes
/// it for the missing handles, before anything is printed. Handles a caller passed in (the vault plugin's pipes, ssh's
/// ProxyCommand, Claude's hooks) are left as they are.
#[cfg(windows)]
fn attach_console() {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    let missing = |h| { let v = unsafe { GetStdHandle(h) }; v.is_null() || v == INVALID_HANDLE_VALUE };
    let need: Vec<_> = [(STD_INPUT_HANDLE, "CONIN$"), (STD_OUTPUT_HANDLE, "CONOUT$"), (STD_ERROR_HANDLE, "CONOUT$")]
        .into_iter().filter(|&(h, _)| missing(h)).collect();
    if need.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 { return }
    for (h, name) in need {
        if !missing(h) { continue }   // attaching may have filled it already
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let f = unsafe { CreateFileW(wide.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut()) };
        if f != INVALID_HANDLE_VALUE { unsafe { SetStdHandle(h, f) }; }
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().nth(1).is_some() { attach_console() }
    // `hook root-only-guard`, `guard --status|--install|--uninstall`: .claude/hooks/root_only_guard.py without Python.
    // First, before anything else: the hook runs on every tool call of every Claude session.
    match std::env::args().nth(1).as_deref() {
        // the root-only guard is installed per user and has its own file; every other hook is the vault's own
        Some("hook") if std::env::args().nth(2).as_deref() == Some("root-only-guard") => std::process::exit(guardcli::hook(&std::env::args().skip(2).collect::<Vec<_>>())),
        Some("hook") => std::process::exit(hooks::main(&std::env::args().skip(2).collect::<Vec<_>>())),
        Some("guard") => std::process::exit(guardcli::guard(&std::env::args().skip(2).collect::<Vec<_>>())),
        // `deck pdf <source.html>`: scripts/html-deck-to-pdf.py with the installed Chrome or Edge in place of Playwright
        Some("deck") => std::process::exit(deckcli::main(&std::env::args().skip(2).collect::<Vec<_>>())),
        // git asks this for the github.com password (login.rs sets it as the credential helper)
        Some("git-credential") => std::process::exit(login::credential(&std::env::args().skip(2).collect::<Vec<_>>())),
        // `github token`: the signed-in account's token on stdout, for the vault plugin (what `gh auth token` was)
        Some("github") if std::env::args().nth(2).as_deref() == Some("token") => match login::active() {
            Some((_, t)) => { println!("{t}"); return }
            None => { eprintln!("not signed in to GitHub: sign in in aIwalk System Setup"); std::process::exit(1) }
        },
        _ => {}
    }
    use_bundled_tools();
    // `vault <cmd>`: scripts/vault_ship.py for computers without Python, run in the vault's folder
    if std::env::args().nth(1).as_deref() == Some("pty") {
        std::process::exit(ptycli::main(&std::env::args().skip(2).collect::<Vec<_>>()));
    }
    if std::env::args().nth(1).as_deref() == Some("session") {
        std::process::exit(sessioncli::main(&std::env::args().skip(2).collect::<Vec<_>>()));
    }
    // `ssh [options] [account@]<machine> [command]`: ssh through Cloudflare with the app's proxy and certificate
    if std::env::args().nth(1).as_deref() == Some("ssh") {
        std::process::exit(machines::ssh_main(&std::env::args().skip(2).collect::<Vec<_>>()));
    }
    if std::env::args().nth(1).as_deref() == Some("vault") {
        vaultcli::main(&std::env::args().skip(2).collect::<Vec<_>>());
        return;
    }
    // `--dump` prints what the page would get, for checking without a window
    #[cfg(target_os = "linux")]
    if let Some(flag) = std::env::args().find(|a| a == "--windows-open" || a == "--windows-stop") {
        return vm::shortcut_main(&flag);
    }
    #[cfg(target_os = "linux")]
    if std::env::args().any(|a| a == "--dump-local") {
        let state = vm::vm_state();
        let password_saved = vm::find().is_some_and(|v| vm::load_password(&v.user).is_some());
        println!("{}", serde_json::json!({ "vm": state, "password_saved": password_saved, "phones": android::phones() }));
        return;
    }
    // Cloudflare Access done by the app itself (access.rs). `--ssh-proxy <host>` is ssh's ProxyCommand; `--ssh-cert
    // <host>` and `--access-get <url>` are for checking by hand. Errors go to stderr, where ssh shows them.
    // `--access-login <host> [--no-browser]` is the sign-in on its own. `--ssh-cert <host> --quiet` is what ssh runs
    // before a connection (~/.ssh/config, Match exec): silent, and quick when the certificate kept is still fresh.
    if let Some(flag @ ("--ssh-proxy" | "--ssh-cert" | "--access-get" | "--access-login")) = std::env::args().nth(1).as_deref() {
        let Some(arg) = std::env::args().nth(2) else { eprintln!("{flag} needs a host or URL"); std::process::exit(2) };
        let done = match flag {
            "--ssh-proxy" => access::ssh_proxy(&arg),
            "--access-login" if std::env::args().nth(3).is_none() => access::sign_in(&arg).map(|b| println!("signed in{}", if b { " (browser)" } else { "" })),
            "--access-login" => access::login(&arg, std::env::args().nth(3).as_deref() != Some("--no-browser")).map(|_| println!("signed in")),
            "--ssh-cert" if std::env::args().nth(3).as_deref() == Some("--quiet") => access::ssh_cert_if_stale(&arg),
            "--ssh-cert" => access::ssh_cert(&arg).map(|(k, c)| println!("key {}\ncertificate {}", k.display(), c.display())),
            _ => access::get(&arg).map(|body| println!("{body}")),
        };
        if let Err(e) = done { eprintln!("{e}"); std::process::exit(1) }
        return;
    }
    // `--reach a b …` prints the connection check for those aliases or addresses
    // `--open-ssh <tunnel> <user>`: what the SSH button does, for checking by hand
    if std::env::args().nth(1).as_deref() == Some("--open-ssh") {
        let a: Vec<String> = std::env::args().skip(2).collect();
        let (Some(t), Some(u)) = (a.first(), a.get(1)) else { eprintln!("--open-ssh <tunnel> <user>"); std::process::exit(2) };
        match machines::open_ssh(t.clone(), u.clone()) { Ok(m) => println!("{m}"), Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--reach") {
        println!("{:?}", machines::reachable(std::env::args().skip(2).map(|h| { let t = h.contains('.').then(|| h.clone()); (h, t) }).collect()));
        return;
    }
    // `--forward <host> <tunnel> <user> <port>` opens one desktop forward, prints the VNC greeting it gets, closes it
    if std::env::args().nth(1).as_deref() == Some("--forward") {
        let a: Vec<String> = std::env::args().skip(2).collect();
        // a path is a desktop's socket, a number its TCP port
        let (socket, port) = if a[3].starts_with('/') { (Some(a[3].clone()), None) } else { (None, a[3].parse().ok()) };
        match machines::open_forward(a[0].clone(), a[1].clone(), a[2].clone(), 9, socket, port) {
            Ok(p) => {
                use std::io::Read;
                let mut buf = [0u8; 12];
                let got = std::net::TcpStream::connect(("127.0.0.1", p)).and_then(|mut s| s.read_exact(&mut buf).map(|_| buf));
                println!("local port {p}: {:?}", got.map(|b| String::from_utf8_lossy(&b).trim().to_string()));
            }
            Err(e) => println!("error: {e}"),
        }
        machines::close_all();
        return;
    }
    if std::env::args().any(|a| a == "--claude") {
        println!("{}", serde_json::to_string(&claude::claude_state()).unwrap());
        return;
    }
    // --api METHOD PATH [JSON]: one question to GitHub through the app's own client, for checking it by hand
    if let Some(i) = std::env::args().position(|a| a == "--api") {
        let a: Vec<String> = std::env::args().skip(i + 1).collect();
        let body = a.get(2).map(|b| serde_json::from_str(b).expect("the body must be JSON"));
        match github::send(&a[0], &a[1], body) { Ok(v) => println!("{v}"), Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
        return;
    }
    // --email-of LOGIN: the address a machine would know that GitHub account by (Cloudflare's, else the public one)
    if let Some(i) = std::env::args().position(|a| a == "--email-of") {
        match admin::public_email(std::env::args().nth(i + 1).expect("--email-of LOGIN")) { Some(e) => println!("{e}"), None => { eprintln!("not known"); std::process::exit(1) } }
        return;
    }
    // --people ORG: what the People list gets (members, invitations, interns); reads only
    if let Some(i) = std::env::args().position(|a| a == "--people") {
        match admin::org_people(std::env::args().nth(i + 1).expect("--people ORG")) { Ok(p) => println!("{}", serde_json::to_string_pretty(&p).unwrap()), Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
        return;
    }
    // --can: what this build does from the command line, so the vault plugin uses the app where it can and its own
    // Python otherwise. A subcommand turns true here in the release it first works in.
    if std::env::args().any(|a| a == "--can") {
        println!("{}", serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "vault": true, "pty": true, "session": true, "guard": true, "deck": true, "hook": true, "ssh": true, "github": true, "repos": true, "hooks": hooks::NAMES }));
        return;
    }
    // --cf-selfcheck: token on stdin; see cloudflare::selfcheck
    if std::env::args().any(|a| a == "--cf-selfcheck") {
        let mut token = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut token).expect("the token on stdin");
        if let Err(e) = cloudflare::selfcheck(&token) { eprintln!("{e}"); std::process::exit(1) }
        return;
    }
    // --cf METHOD PATH [JSON]: one call to Cloudflare with the owner's kept token, for checking by hand
    if let Some(i) = std::env::args().position(|a| a == "--cf") {
        let a: Vec<String> = std::env::args().skip(i + 1).collect();
        let body = a.get(2).map(|b| serde_json::from_str(b).expect("the body must be JSON"));
        match cloudflare::api(&a[0], &a[1], body) { Ok(v) => println!("{v}"), Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
        return;
    }
    // --sign-in [owner]: the device flow in the terminal; prints who the token belongs to and what it may do, keeps nothing
    if let Some(i) = std::env::args().position(|a| a == "--sign-in") {
        let scopes = if std::env::args().nth(i + 1).as_deref() == Some("owner") { github::SCOPES_OWNER } else { github::SCOPES_MEMBER };
        match github::device_sign_in(scopes, &|code, url| println!("Enter {code} at {url}")) {
            Ok(t) => println!("token received ({} characters, starts {})", t.len(), &t[..4]),
            Err(e) => { eprintln!("{e}"); std::process::exit(1) }
        }
        return;
    }
    // --pr ORG REPO...: what the Pull request permissions section shows, read only
    if std::env::args().nth(1).as_deref() == Some("--pr") {
        let a: Vec<String> = std::env::args().skip(2).collect();
        println!("{}", serde_json::to_string_pretty(&admin::pr_permissions(a[0].clone(), a[1..].to_vec())).unwrap());
        return;
    }
    // --download TAG PATTERN DIR: the updater's download on its own ("*_amd64.deb")
    if let Some(i) = std::env::args().position(|a| a == "--download") {
        let a: Vec<String> = std::env::args().skip(i + 1).collect();
        match github::download_asset("ExoPulseHQ/aiwalk-system-setup-portal", &a[0], &a[1], std::path::Path::new(&a[2])) {
            Ok(p) => println!("{}", p.display()),
            Err(e) => { eprintln!("{e}"); std::process::exit(1) }
        }
        return;
    }
    if std::env::args().any(|a| a == "--dump") {
        println!("{}", serde_json::to_string_pretty(&read_access(&|_, _, _| {})).unwrap());
        return;
    }
    // every command and flag has returned by here; anything still on the command line is a word this program does
    // not know (`aiwalk-setup help`), and opening the window for it left a shell hanging. macOS passes -psn_… to
    // an app started from Finder.
    if let Some(word) = std::env::args().nth(1).filter(|a| !a.starts_with("-psn")) {
        eprintln!("aiwalk-setup: unknown command {word}\ncommands: vault, session, pty, ssh, deck, hook, guard, github token, git-credential (see `aiwalk-setup --can`); no argument opens the app\nssh [ssh options] [account@]<machine> [command]: ssh to a team machine through Cloudflare, no ~/.ssh/config needed");
        std::process::exit(2);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(handlers())
        .build(tauri::generate_context!())
        .expect("error while starting aIwalk System Setup")
        // desktop forwards are ssh processes; none outlives the app
        .run(|_, event| if let tauri::RunEvent::Exit = event { machines::close_all() });
}
