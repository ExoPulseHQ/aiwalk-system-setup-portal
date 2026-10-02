//! Owner tools: who is in the organisation, invitations, owner role, and one person's access to one repo.
//! GitHub refuses all of these for anyone who is not an org owner, so the app adds no check of its own.

use crate::gh;
use exo_core::{org_access, org_query, plan_access};
use serde::Serialize;

#[derive(Serialize)]
pub struct Member { login: String, name: String, owner: bool }

#[derive(Serialize)]
pub struct Invite { id: u64, login: String, owner: bool, created: String }

#[derive(Serialize)]
pub struct People { members: Vec<Member>, invites: Vec<Invite> }

fn json(args: &[&str]) -> Result<serde_json::Value, String> {
    serde_json::from_str(&gh(args)?).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn org_people(org: String) -> Result<People, String> {
    let a = org_access(&gh(&["api", "graphql", "-f", &format!("query={}", org_query(&org))])?);
    let members = a.people.into_iter().map(|(login, p)| Member { name: p.name, owner: p.grants.is_none(), login }).collect();
    let inv = json(&["api", &format!("orgs/{org}/invitations")])?;
    let invites = inv.as_array().cloned().unwrap_or_default().iter().map(|i| Invite {
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
    let id = gh(&["api", &format!("users/{login}"), "--jq", ".id"]).map_err(|_| format!("There is no GitHub account called {login}"))?;
    let mut args = vec!["api".to_string(), "-X".into(), "POST".into(), format!("orgs/{org}/invitations"),
                        "-F".into(), format!("invitee_id={}", id.trim()), "-f".into(), format!("role={}", if owner { "admin" } else { "direct_member" })];
    for t in &teams {
        let tid = gh(&["api", &format!("orgs/{org}/teams/{t}"), "--jq", ".id"])?;
        args.extend(["-F".into(), format!("team_ids[]={}", tid.trim())]);
    }
    gh(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    Ok(format!("Invited {login}. GitHub emails them; they join once they accept."))
}

#[tauri::command(async)]
pub fn cancel_invite(org: String, id: u64) -> Result<String, String> {
    gh(&["api", "-X", "DELETE", &format!("orgs/{org}/invitations/{id}")]).map(|_| "Invitation cancelled".into())
}

/// Makes `login` an owner (admin of every repo, can change everyone's access) or a plain member.
#[tauri::command(async)]
pub fn set_role(org: String, login: String, owner: bool) -> Result<String, String> {
    gh(&["api", "-X", "PUT", &format!("orgs/{org}/memberships/{login}"), "-f", &format!("role={}", if owner { "admin" } else { "member" })])?;
    Ok(format!("{login} is now {}", if owner { "an owner" } else { "a member" }))
}

/// Takes `login` out of the organisation: every team and repo grant goes with it. Their copies stay where they are.
#[tauri::command(async)]
pub fn remove_member(org: String, login: String) -> Result<String, String> {
    gh(&["api", "-X", "DELETE", &format!("orgs/{org}/memberships/{login}")]).map(|_| format!("{login} left the organisation"))
}

/// Gives `login` exactly `level` (0 none, 1 read, 2 write) on `repo`, through the repo's own team where it has one.
/// Err names any shared team that still gives more, since changing it would change other repos too.
#[tauri::command(async)]
pub fn set_access(org: String, repo: String, login: String, level: u8) -> Result<String, String> {
    let teams = org_access(&gh(&["api", "graphql", "-f", &format!("query={}", org_query(&org))])?).teams;
    let plan = plan_access(&teams, &login, &repo, level);
    for t in &plan.leave {
        gh(&["api", "-X", "DELETE", &format!("orgs/{org}/teams/{t}/memberships/{login}")])?;
    }
    if let Some(t) = &plan.join {
        gh(&["api", "-X", "PUT", &format!("orgs/{org}/teams/{t}/memberships/{login}"), "-f", "role=member"])?;
    }
    let direct = format!("repos/{org}/{repo}/collaborators/{login}");
    match plan.direct {
        Some(p) => { gh(&["api", "-X", "PUT", &direct, "-f", &format!("permission={p}")])?; }
        // no direct grant wanted; GitHub answers 404 when there was none, which is fine
        None => { let _ = gh(&["api", "-X", "DELETE", &direct]); }
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
    if add { gh(&["api", "-X", "PUT", &path, "-f", "role=member"])?; } else { gh(&["api", "-X", "DELETE", &path])?; }
    Ok(format!("{login} {} {host}", if add { "may now connect to" } else { "no longer has extra access to" }))
}
