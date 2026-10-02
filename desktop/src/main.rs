#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use exo_core::{
    build_tree, can_sign_in, machines, org_access, org_query, parse_request, repo_grants, repos_query, request_body,
    vault_repos, visible_to, Machine, Node, Person, Team, RULES_PATHS,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tauri::Emitter;

// ponytail: vault list is hard-coded, same as the Python app; move to a config file when teams need different lists
const VAULTS: [(&str, &str, &str); 2] = [
    ("ExoPulse docs", "eddLai/ExoPulse_docs", "The team's technical documents, papers and progress notes"),
    ("aIwalk Corp", "eddLai/aIwalk_Corp", "Company records, founders only"),
];
/// Where access requests travel, as issues; the org's members can open issues there but not write.
const REQUESTS: &str = "exo-access-requests";

/// Runs gh; Ok(stdout) on exit 0, Err(stderr or why it could not start) otherwise.
fn gh(args: &[&str]) -> Result<String, String> {
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
}

#[derive(Serialize)]
struct Access {
    org: String,
    tree: Node,
    people: BTreeMap<String, Person>,
    /// Teams an owner can move people in (core excluded); empty for everyone else.
    teams: Vec<Team>,
    /// Every machine and the code repo it carries (aliases only); the page marks which ones the person can reach.
    machines: Vec<Machine>,
    /// For each visible person, the indexes into `machines` they can sign in to.
    reach: BTreeMap<String, Vec<usize>>,
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
    error: Option<String>,
    vaults: Vec<Vault>,
}

#[tauri::command(async)]
fn team_access() -> State {
    let user = match gh(&["api", "user", "--jq", ".login"]) {
        Ok(login) => login.trim().to_string(),
        Err(e) => return State { user: None, error: Some(e), vaults: vec![] },
    };
    let vaults = VAULTS.iter().map(|&(name, repo, about)| {
        let rules = RULES_PATHS.iter().find_map(|p| raw(repo, p).ok());
        let access = rules.as_deref().and_then(vault_repos).map(|v| {
            let query = |q: String| gh(&["api", "graphql", "-f", &format!("query={q}")]).unwrap_or_default();
            let mut org = org_access(&query(org_query(&v.org)));
            let owner = org.people.get(&user).is_some_and(|p| p.grants.is_none());
            repo_grants(&mut org.people, &query(repos_query(&v.org, owner)), &user);
            let teams = if owner { org.teams.into_iter().filter(|t| t.slug != "core").collect() } else { vec![] };
            let machines = machines(rules.as_deref().unwrap_or_default());
            let people = visible_to(org.people, &user);
            let reach = people.iter().map(|(login, p)| (login.clone(),
                machines.iter().enumerate().filter(|(_, m)| can_sign_in(m, p.grants.as_ref())).map(|(i, _)| i).collect()))
                .collect();
            Access {
                tree: build_tree(&v, &org.repo_teams), people, teams, machines, reach,
                requests: open_requests(&v.org, !owner), org: v.org,
            }
        });
        Vault { name, repo, about, access }
    }).collect();
    State { user: Some(user), error: None, vaults }
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

fn open_url(url: &str) {
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
        .args(["auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web"])
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

fn main() {
    // `--dump` prints what the page would get, for checking without a window
    if std::env::args().any(|a| a == "--dump") {
        println!("{}", serde_json::to_string_pretty(&team_access()).unwrap());
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![team_access, sign_in, set_team, request_access, approve_request, decline_request])
        .run(tauri::generate_context!())
        .expect("error while running aIwalk System Setup");
}
