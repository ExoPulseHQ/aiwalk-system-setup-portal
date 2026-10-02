#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use exo_core::{
    build_tree, machines, parse_gh_status, Auth, org_access, org_query, parse_request, repo_grants, repos_query, request_body,
    vault_repos, visible_to, Machine, Node, Person, Team, RULES_PATHS,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Emitter;

// The phone and VM modules drive a Linux desktop (adb, scrcpy, docker, GNOME keyring).
mod admin;
mod claude;
mod machines;
mod vault;
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

/// Puts the bundled gh, cloudflared and git (Windows: MinGit) ahead of PATH for this app and everything it starts, so a new
/// member's computer needs neither. Bundles sit next to the binary (Linux folder, Windows), or in Contents/Resources (Mac).
fn use_bundled_tools() {
    let roots = [here(), here().join("../Resources"), here().join("../lib/aIwalk System Setup")];
    let dirs: Vec<PathBuf> = roots.iter().flat_map(|r| [r.join("tools/gh"), r.join("tools/git/cmd"), r.join("tools/cloudflared")])
        .filter(|d| d.is_dir()).collect();
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Ok(joined) = std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&path))) {
        std::env::set_var("PATH", joined);
    }
}

#[derive(Serialize)]
struct Tools {
    gh: Option<String>,
    git: Option<String>,
    os: &'static str,
}

/// Where gh and git come from on this computer; None when missing, and the page says how to get it.
#[tauri::command]
fn tools() -> Tools {
    let s = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    Tools { gh: s(on_path("gh")), git: s(on_path("git")), os: std::env::consts::OS }
}

/// macOS: asks Apple's installer for the command line tools, which include git.
#[tauri::command]
fn install_git() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return Command::new("xcode-select").arg("--install").spawn().map(|_| ()).map_err(|e| e.to_string());
    #[allow(unreachable_code)]
    Err("Install git with your system's package manager".into())
}

/// Runs a program with optional stdin, killed after `timeout` seconds; (exit code, stdout + stderr).
/// 124 on timeout, 1 when it could not start, as the Python app did.
pub fn sh_stdin(prog: &str, args: &[&str], stdin: Option<&str>, timeout: u64) -> (i32, String) {
    let child = Command::new(prog).args(args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
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

// ponytail: vault list is hard-coded, same as the Python app; move to a config file when teams need different lists
const VAULTS: [(&str, &str, &str); 2] = [
    ("ExoPulse docs", "ExoPulseHQ/exo-book", "The team's technical documents, papers and progress notes"),
    ("aIwalk Corp", "eddLai/aIwalk_Corp", "Company records, founders only"),
];
/// Where access requests travel, as issues; the org's members can open issues there but not write.
const REQUESTS: &str = "exo-access-requests";

/// Runs gh; Ok(stdout) on exit 0, Err(stderr or why it could not start) otherwise.
pub fn gh(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gh").args(args).output().map_err(|e| format!("gh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn raw(repo: &str, path: &str) -> Result<String, String> {
    gh(&["api", &format!("repos/{repo}/contents/{path}"), "-H", "Accept: application/vnd.github.raw"])
}

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

fn open_requests(org: &str, mine: bool) -> Vec<Request> {
    let repo = format!("{org}/{REQUESTS}");
    let mut args = vec!["issue", "list", "-R", &repo, "--state", "open", "--json", "number,author,body", "--limit", "100"];
    if mine { args.extend(["--author", "@me"]) }
    let Ok(out) = gh(&args) else { return vec![] };
    let issues: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap_or_default();
    issues.iter().filter_map(|i| {
        let body = i["body"].as_str()?.to_string();
        let (repo, level) = parse_request(&body)?;
        Some(Request { number: i["number"].as_u64()?, author: i["author"]["login"].as_str()?.to_string(), repo, level, body })
    }).collect()
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
}

/// Reads everything Team access shows; GitHub answers slowly, so each step is announced to the page as
/// "team-stage" (step, steps, what is being read).
#[tauri::command(async)]
fn team_access(app: tauri::AppHandle) -> State {
    read_access(&|i, n, text| { let _ = app.emit("team-stage", (i, n, text)); })
}

fn read_access(stage: &dyn Fn(usize, usize, &str)) -> State {
    let steps = 2 + VAULTS.len() * 3;
    stage(1, steps, "Checking who is signed in");
    let (user, name) = match gh(&["api", "user", "--jq", ".login, .name"]) {
        Ok(out) => { let mut l = out.lines(); (l.next().unwrap_or_default().to_string(), l.next().filter(|n| *n != "null").map(String::from)) }
        Err(e) => return State { user: None, name: None, accounts: vec![], error: Some(e), vaults: vec![] },
    };
    // gh auth status writes to stderr; sh merges both
    let accounts = parse_gh_status(&sh("gh", &["auth", "status", "--hostname", "github.com"], 15).1);
    stage(2, steps, "Checking how this computer is signed in");
    let vaults = VAULTS.iter().enumerate().map(|(k, &(name, repo, about))| {
        let at = 3 + k * 3;
        stage(at, steps, &format!("Reading how {name} is organised"));
        let rules = RULES_PATHS.iter().find_map(|p| raw(repo, p).ok());
        let access = rules.as_deref().and_then(vault_repos).map(|v| {
            let query = |q: String| gh(&["api", "graphql", "-f", &format!("query={q}")]).unwrap_or_default();
            stage(at + 1, steps, &format!("Reading who can open what in {name}"));
            let mut org = org_access(&query(org_query(&v.org)));
            let owner = org.people.get(&user).is_some_and(|p| p.grants.is_none());
            repo_grants(&mut org.people, &query(repos_query(&v.org, owner)), &user);
            stage(at + 2, steps, "Reading access requests");
            // owners see every team; anyone else only the teams they are in, with no one else's name, which is
            // enough to tell them which machines they may reach
            let teams = if owner { org.teams } else {
                org.teams.into_iter().filter(|t| t.members.contains(&user))
                    .map(|mut t| { t.members.retain(|m| m == &user); t }).collect()
            };
            let machines = machines(rules.as_deref().unwrap_or_default());
            let people = visible_to(org.people, &user);
            Access {
                tree: build_tree(&v, &org.repo_teams), people, teams, machines,
                requests: open_requests(&v.org, !owner), org: v.org,
            }
        });
        let p = gh(&["api", &format!("repos/{repo}"), "--jq", ".permissions | [.admin, .maintain, .push, .triage, .pull] | map(tostring) | join(\" \")"]).unwrap_or_default();
        let permission = match p.split_whitespace().collect::<Vec<_>>()[..] {
            ["true", ..] => 4, [_, "true", ..] => 3, [_, _, "true", ..] => 2, [.., "true", _] | [.., "true"] => 1, _ => 0,
        };
        Vault { name, repo, about, access, permission }
    }).collect();
    State { user: Some(user), name, accounts, error: None, vaults }
}

/// Signs `login` out of gh on this computer; any other account stays signed in and gh makes one of them active.
#[tauri::command(async)]
fn sign_out(login: String) -> Result<(), String> {
    gh(&["auth", "logout", "--hostname", "github.com", "--user", &login])?;
    machines::forget_access();   // the machines must not keep letting the signed-out person in
    Ok(())
}

/// Makes another signed-in account the one git and this app use.
#[tauri::command(async)]
fn switch_account(login: String) -> Result<(), String> {
    gh(&["auth", "switch", "--hostname", "github.com", "--user", &login])?;
    machines::forget_access();   // the next lab connection signs in as this account, not the previous one
    Ok(())
}

/// Adds `login` to or removes them from an org team. GitHub itself refuses anyone who is not an owner.
#[tauri::command(async)]
fn set_team(org: String, team: String, login: String, member: bool) -> Result<(), String> {
    let path = format!("orgs/{org}/teams/{team}/memberships/{login}");
    if member { gh(&["api", "-X", "PUT", &path, "-f", "role=member"]) } else { gh(&["api", "-X", "DELETE", &path]) }.map(|_| ())
}

/// Opens an access request as an issue in the org's request repo.
#[tauri::command(async)]
fn request_access(org: String, repo: String, level: String, note: String) -> Result<(), String> {
    if level != "read" && level != "write" { return Err(format!("unknown level {level}")) }
    gh(&["issue", "create", "-R", &format!("{org}/{REQUESTS}"), "--title", &format!("{level} access to {repo}"),
         "--body", &request_body(&repo, &level, &note)]).map(|_| ())
}

/// Grants an open request and closes it. Who gets access is the issue's author, never anything in its body.
/// A team that grants exactly that repo at that level is used when there is one, otherwise a direct grant.
#[tauri::command(async)]
fn approve_request(org: String, number: u64) -> Result<(), String> {
    let issues = format!("{org}/{REQUESTS}");
    let n = number.to_string();
    let issue: serde_json::Value = serde_json::from_str(&gh(&["issue", "view", &n, "-R", &issues, "--json", "author,body,state"])?)
        .map_err(|e| e.to_string())?;
    if issue["state"] != "OPEN" { return Err("this request is already closed".into()) }
    let login = issue["author"]["login"].as_str().ok_or("request has no author")?;
    let (repo, level) = parse_request(issue["body"].as_str().unwrap_or_default()).ok_or("request body is not readable")?;
    let rank = if level == "write" { 2 } else { 1 };
    let org_json = gh(&["api", "graphql", "-f", &format!("query={}", org_query(&org))])?;
    let team = org_access(&org_json).teams.into_iter()
        .find(|t| t.slug != "core" && t.repos.len() == 1 && t.repos[0] == (repo.clone(), rank));
    match team {
        Some(t) => gh(&["api", "-X", "PUT", &format!("orgs/{org}/teams/{}/memberships/{login}", t.slug), "-f", "role=member"])?,
        None => gh(&["api", "-X", "PUT", &format!("repos/{org}/{repo}/collaborators/{login}"),
                     "-f", &format!("permission={}", if level == "write" { "push" } else { "pull" })])?,
    };
    gh(&["issue", "close", &n, "-R", &issues, "--comment", "Approved in aIwalk System Setup."]).map(|_| ())
}

#[tauri::command(async)]
fn decline_request(org: String, number: u64) -> Result<(), String> {
    gh(&["issue", "close", &number.to_string(), "-R", &format!("{org}/{REQUESTS}"), "--reason", "not planned",
         "--comment", "Declined in aIwalk System Setup."]).map(|_| ())
}

pub fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let _ = Command::new("cmd").args(["/c", "start", "", url]).spawn();
}

/// gh's browser sign-in: emits "gh-code" with the one-time code, true once GitHub approved.
#[tauri::command(async)]
fn sign_in(app: tauri::AppHandle) -> bool {
    let Ok(mut child) = Command::new("gh")
        .args(["auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web", "--scopes", "user:email"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() else { return false };
    // answers "Press Enter to open github.com in your browser" ahead of time
    let _ = child.stdin.take().unwrap().write_all(b"\n");
    // gh writes the code to stderr
    for line in BufReader::new(child.stderr.take().unwrap()).lines().map_while(Result::ok) {
        if let Some(code) = line.split("code: ").nth(1) {
            let _ = app.emit("gh-code", code.trim());
        }
        // without a terminal gh only prints the URL, so open it ourselves
        if let Some(url) = line.split("browser: ").nth(1) {
            open_url(url.trim());
        }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if ok {
        let _ = gh(&["auth", "setup-git"]);
    }
    ok
}

#[cfg(target_os = "linux")]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool {
    tauri::generate_handler![platform, tools, install_git, team_access, claude::claude_state, claude::claude_install, claude::claude_login, machines::reachable, machines::machine_status, machines::open_forward, machines::close_forward, machines::forwards, machines::open_viewer, machines::desktop, machines::lab_identity, machines::lab_sign_out, machines::access_login, machines::ssh_status, machines::ssh_setup,
                             admin::org_people, admin::invite, admin::cancel_invite, admin::set_role, admin::remove_member, admin::set_access, admin::machine_extra, sign_in, sign_out, switch_account,
                             vault::vault_local, vault::vault_download, vault::vault_link, vault::pick_folder, vault::default_folder, vault::vault_update, vault::vault_open, vault::obsidian_install, set_team, request_access, approve_request, decline_request,
                             android::phones, android::phone_action, vm::vm_state, vm::vm_action]
}
#[cfg(not(target_os = "linux"))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool {
    tauri::generate_handler![platform, tools, install_git, team_access, claude::claude_state, claude::claude_install, claude::claude_login, machines::reachable, machines::machine_status, machines::open_forward, machines::close_forward, machines::forwards, machines::open_viewer, machines::desktop, machines::lab_identity, machines::lab_sign_out, machines::access_login, machines::ssh_status, machines::ssh_setup,
                             admin::org_people, admin::invite, admin::cancel_invite, admin::set_role, admin::remove_member, admin::set_access, admin::machine_extra, sign_in, sign_out, switch_account,
                             vault::vault_local, vault::vault_download, vault::vault_link, vault::pick_folder, vault::default_folder, vault::vault_update, vault::vault_open, vault::obsidian_install, set_team, request_access, approve_request, decline_request]
}

fn main() {
    use_bundled_tools();
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
    // `--reach a b …` prints the connection check for those aliases or addresses
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
    if std::env::args().any(|a| a == "--dump") {
        println!("{}", serde_json::to_string_pretty(&read_access(&|_, _, _| {})).unwrap());
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(handlers())
        .build(tauri::generate_context!())
        .expect("error while starting aIwalk System Setup")
        // desktop forwards are ssh processes; none outlives the app
        .run(|_, event| if let tauri::RunEvent::Exit = event { machines::close_all() });
}
