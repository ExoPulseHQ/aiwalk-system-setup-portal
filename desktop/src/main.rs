#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use exo_core::{build_tree, org_access, org_query, vault_repos, visible_to, Node, Person, RULES_PATHS};
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
        let access = RULES_PATHS.iter().find_map(|p| raw(repo, p).ok()).and_then(|r| vault_repos(&r)).map(|v| {
            let org = org_access(&gh(&["api", "graphql", "-f", &format!("query={}", org_query(&v.org))]).unwrap_or_default());
            Access { tree: build_tree(&v, &org.repo_teams), people: visible_to(org.people, &user), org: v.org }
        });
        Vault { name, repo, about, access }
    }).collect();
    State { user: Some(user), error: None, vaults }
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
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![team_access, sign_in])
        .run(tauri::generate_context!())
        .expect("error while running aIwalk System Setup");
}
