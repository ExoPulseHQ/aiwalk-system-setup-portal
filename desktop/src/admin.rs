//! Owner tools: who is in the organisation, invitations, owner role, and one person's access to one repo.
//! GitHub refuses all of these for anyone who is not an org owner, so the app adds no check of its own.
//! `pr_permissions` is the one read here that everyone gets.

use crate::github;
use exo_core::{merge_rights, org_access, org_query, permissions_rank, plan_access};
use serde::Serialize;

#[derive(Serialize)]
pub struct Member { login: String, name: String, owner: bool }

#[derive(Serialize)]
pub struct Invite { id: u64, login: String, owner: bool, created: String }

#[derive(Serialize)]
pub struct People { members: Vec<Member>, invites: Vec<Invite> }

#[tauri::command(async)]
pub fn org_people(org: String) -> Result<People, String> {
    let a = org_access(&github::graphql(&org_query(&org))?);
    let members = a.people.into_iter().map(|(login, p)| Member { name: p.name, owner: p.grants.is_none(), login }).collect();
    let invites = github::all(&format!("orgs/{org}/invitations"))?.iter().map(|i| Invite {
        id: i["id"].as_u64().unwrap_or(0),
        login: i["login"].as_str().or(i["email"].as_str()).unwrap_or("?").into(),
        owner: i["role"] == "admin",
        created: i["created_at"].as_str().unwrap_or("").chars().take(10).collect(),
    }).collect();
    Ok(People { members, invites })
}

/// Invites a GitHub account, as member or owner, already placed in `teams`.
#[tauri::command(async)]
pub fn invite(org: String, login: String, owner: bool, teams: Vec<String>) -> Result<String, String> {
    let login = login.trim().trim_start_matches('@');
    let id = github::get(&format!("users/{login}")).map_err(|_| format!("There is no GitHub account called {login}"))?["id"].clone();
    let mut team_ids = vec![];
    for t in &teams { team_ids.push(github::get(&format!("orgs/{org}/teams/{t}"))?["id"].clone()); }
    github::send("POST", &format!("orgs/{org}/invitations"), Some(serde_json::json!({
        "invitee_id": id, "role": if owner { "admin" } else { "direct_member" }, "team_ids": team_ids })))?;
    Ok(format!("Invited {login}. GitHub emails them; they join once they accept."))
}

#[tauri::command(async)]
pub fn cancel_invite(org: String, id: u64) -> Result<String, String> {
    github::send("DELETE", &format!("orgs/{org}/invitations/{id}"), None).map(|_| "Invitation cancelled".into())
}

/// Makes `login` an owner (admin of every repo, can change everyone's access) or a plain member.
#[tauri::command(async)]
pub fn set_role(org: String, login: String, owner: bool) -> Result<String, String> {
    github::send("PUT", &format!("orgs/{org}/memberships/{login}"), Some(serde_json::json!({ "role": if owner { "admin" } else { "member" } })))?;
    Ok(format!("{login} is now {}", if owner { "an owner" } else { "a member" }))
}

/// Takes `login` out of the organisation: every team and repo grant goes with it. Their copies stay where they are.
#[tauri::command(async)]
pub fn remove_member(org: String, login: String) -> Result<String, String> {
    github::send("DELETE", &format!("orgs/{org}/memberships/{login}"), None).map(|_| format!("{login} left the organisation"))
}

/// Gives `login` exactly `level` (0 none, 1 read, 2 write) on `repo`, through the repo's own team where it has one.
/// Err names any shared team that still gives more, since changing it would change other repos too.
#[tauri::command(async)]
pub fn set_access(org: String, repo: String, login: String, level: u8) -> Result<String, String> {
    let teams = org_access(&github::graphql(&org_query(&org))?).teams;
    let plan = plan_access(&teams, &login, &repo, level);
    for t in &plan.leave {
        github::send("DELETE", &format!("orgs/{org}/teams/{t}/memberships/{login}"), None)?;
    }
    if let Some(t) = &plan.join {
        github::send("PUT", &format!("orgs/{org}/teams/{t}/memberships/{login}"), Some(serde_json::json!({ "role": "member" })))?;
    }
    let direct = format!("repos/{org}/{repo}/collaborators/{login}");
    match plan.direct {
        Some(p) => { github::send("PUT", &direct, Some(serde_json::json!({ "permission": p })))?; }
        // no direct grant wanted; GitHub answers 404 when there was none, which is fine
        None => { let _ = github::send("DELETE", &direct, None); }
    }
    if plan.blocked_by.is_empty() { Ok(format!("{login}: {} on {repo}", ["no access", "read", "write"][level.min(2) as usize])) }
    else { Err(format!("{login} still gets more through the {} team, which also covers other repos. Change it under Teams.", plan.blocked_by.join(", "))) }
}

/// Lets one person connect to `host` beyond what their code repos give, or takes that back: membership of the
/// GitHub team machine-<host>, which the machine's Cloudflare Access policy includes. Takes effect at their next
/// sign-in to the machines (sessions last up to 24 hours).
#[tauri::command(async)]
pub fn machine_extra(org: String, host: String, login: String, add: bool) -> Result<String, String> {
    let path = format!("orgs/{org}/teams/machine-{host}/memberships/{login}");
    if add { github::send("PUT", &path, Some(serde_json::json!({ "role": "member" })))?; } else { github::send("DELETE", &path, None)?; }
    Ok(format!("{login} {} {host}", if add { "may now connect to" } else { "no longer has extra access to" }))
}

#[derive(Serialize)]
pub struct PrRepo {
    repo: String,
    /// The viewer's own level on the repo (4 admin … 0 none).
    mine: u8,
    /// False when GitHub would not list the repo's people for this viewer (it needs write); then only `mine` is known.
    listed: bool,
    merge: Vec<String>,
    write: Vec<String>,
    /// Members of merge-<repo>: the people an owner gave the right here, and the only ones an owner can take it from here.
    extra: Vec<String>,
}

/// The team whose members may merge on `repo`, mirroring machine-<host>.
fn merge_team(repo: &str) -> String { format!("merge-{}", repo.to_lowercase()) }

/// A team's slug by its name; GitHub makes the slug, so it is looked up rather than guessed.
fn slug_of(teams: &[serde_json::Value], name: &str) -> Option<String> {
    teams.iter().find(|t| t["name"] == name).and_then(|t| t["slug"].as_str().map(String::from))
}

/// Who may merge and who may open pull requests on each of `repos`, as GitHub reports it (owners, teams and direct
/// grants alike). A repo that is not in `org`, or that this account cannot see, is left out.
#[tauri::command(async)]
pub fn pr_permissions(org: String, repos: Vec<String>) -> Vec<PrRepo> {
    let teams = github::all(&format!("orgs/{org}/teams")).unwrap_or_default();
    std::thread::scope(|s| {
        let jobs: Vec<_> = repos.iter().map(|repo| { let (org, teams) = (&org, &teams); s.spawn(move || {
            let mine = permissions_rank(&github::get(&format!("repos/{org}/{repo}")).ok()?["permissions"]);
            let people = github::all(&format!("repos/{org}/{repo}/collaborators?affiliation=all")).ok();
            let (merge, write) = people.as_deref().map(merge_rights).unwrap_or_default();
            let extra = slug_of(teams, &merge_team(repo)).and_then(|t| github::all(&format!("orgs/{org}/teams/{t}/members")).ok())
                .unwrap_or_default().iter().filter_map(|m| m["login"].as_str().map(String::from)).collect();
            Some(PrRepo { repo: repo.clone(), mine, listed: people.is_some(), merge, write, extra })
        })}).collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    })
}

/// Gives one person the right to merge on `repo`, or takes it back: membership of merge-<repo>, a closed team with
/// maintain on the repo, made the first time it is needed. On GitHub's Free plan write already allows merging on the
/// website, so this right is what the vault plugin and the team's pre-push hook go by.
#[tauri::command(async)]
pub fn merge_right(org: String, repo: String, login: String, add: bool) -> Result<String, String> {
    let name = merge_team(&repo);
    let slug = match slug_of(&github::all(&format!("orgs/{org}/teams"))?, &name) {
        Some(s) => s,
        None if !add => return Err(format!("Nobody was given the right to merge on {repo} here")),
        None => {
            let t = github::send("POST", &format!("orgs/{org}/teams"), Some(serde_json::json!({
                "name": name, "privacy": "closed", "description": format!("May merge pull requests on {repo}") })))?;
            let slug = t["slug"].as_str().ok_or("GitHub made the team but did not say its name")?.to_string();
            // GitHub puts whoever makes a team in it; being an owner already lets them merge, so they leave again
            let me = github::get("user")?["login"].as_str().unwrap_or_default().to_string();
            if !me.eq_ignore_ascii_case(&login) { github::send("DELETE", &format!("orgs/{org}/teams/{slug}/memberships/{me}"), None)?; }
            slug
        }
    };
    let path = format!("orgs/{org}/teams/{slug}/memberships/{login}");
    if add {
        // asked every time, so a team left without the repo by an earlier failure is put right
        github::send("PUT", &format!("orgs/{org}/teams/{slug}/repos/{org}/{repo}"), Some(serde_json::json!({ "permission": "maintain" })))?;
        github::send("PUT", &path, Some(serde_json::json!({ "role": "member" })))?;
    } else { github::send("DELETE", &path, None)?; }
    Ok(format!("{login} {} on {repo}", if add { "can now merge" } else { "can no longer merge" }))
}
