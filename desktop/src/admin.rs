//! Owner tools: who is in the organisation, invitations, owner role, and one person's access to one repo.
//! GitHub refuses all of these for anyone who is not an org owner, so the app adds no check of its own.
//! `pr_permissions` is the one read here that everyone gets.

use crate::github;
use exo_core::{allow_policy, can_edit_people, guest_email, interns, policy_emails, policy_update, with_guest, merge_rights, org_access, org_query, permissions_rank, plan_access, Intern};
use serde::Serialize;

#[derive(Serialize)]
pub struct Member { login: String, name: String, owner: bool }

#[derive(Serialize)]
pub struct Invite { id: u64, login: String, owner: bool, created: String }

#[derive(Serialize)]
pub struct People {
    members: Vec<Member>, invites: Vec<Invite>, interns: Vec<Intern>,
    /// The access-requests repo: an intern whose only repo it is, is a machine guest.
    requests: &'static str,
    /// True only for an owner whose token can change the organisation; the page then shows its edit controls.
    can_edit: bool,
}

/// What an intern gets on invitation, all read: the app's repo (to download it), the requests repo (to read the
/// terms, record acceptance and ask for more), the shared notes and the papers. Owners raise single repos later.
const INTERN_REPOS: [&str; 4] = ["aiwalk-system-setup-portal", crate::REQUESTS, "exo-book", "exo-papers"];

/// Everyone in the organisation, for any member. Invitations and interns need an owner, so they are read only when
/// the caller may edit (a refusal there leaves them empty, it does not fail the page).
#[tauri::command(async)]
pub fn org_people(org: String) -> Result<People, String> {
    let a = org_access(&github::graphql(&org_query(&org))?);
    let me = github::get("user")?["login"].as_str().unwrap_or_default().to_string();
    let can_edit = can_edit_people(a.people.get(&me).is_some_and(|p| p.grants.is_none()), &github::scopes());
    let (interns, invites) = if can_edit {
        let invites = github::all(&format!("orgs/{org}/invitations")).unwrap_or_default().iter().map(|i| Invite {
            id: i["id"].as_u64().unwrap_or(0),
            login: i["login"].as_str().or(i["email"].as_str()).unwrap_or("?").into(),
            owner: i["role"] == "admin",
            created: i["created_at"].as_str().unwrap_or("").chars().take(10).collect(),
        }).collect();
        (org_interns(&org, &a.people.keys().cloned().collect()).unwrap_or_default(), invites)
    } else { (vec![], vec![]) };
    let members = a.people.into_iter().map(|(login, p)| Member { name: p.name, owner: p.grants.is_none(), login }).collect();
    Ok(People { members, invites, interns, can_edit, requests: crate::REQUESTS })
}

/// Interns are not in the organisation, so only their repos know them: the org's outside collaborators, and every
/// repo's outside collaborators and pending invitations, asked side by side.
fn org_interns(org: &str, members: &std::collections::BTreeSet<String>) -> Result<Vec<Intern>, String> {
    let outside = github::all(&format!("orgs/{org}/outside_collaborators"))?;
    let repos: Vec<String> = github::all(&format!("orgs/{org}/repos"))?.iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();
    let ask = |what: &str| std::thread::scope(|s| {
        let jobs: Vec<_> = repos.iter().map(|r| s.spawn(move || github::all(&format!("repos/{org}/{r}/{what}")).map(|v| (r.clone(), v)))).collect();
        jobs.into_iter().map(|j| j.join().unwrap()).collect::<Result<Vec<_>, String>>()
    });
    // with no outside collaborators there are no accepted repos to look for
    let collaborators = if outside.is_empty() { vec![] } else { ask("collaborators?affiliation=outside")? };
    Ok(interns(&outside, &collaborators, &ask("invitations")?, members))
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

/// Invites someone as an intern: repo invitations to `INTERN_REPOS`, no organisation invitation. A machine guest
/// (`guest`) gets read on the requests repo only: the terms and their acceptance, no documents.
#[tauri::command(async)]
pub fn invite_intern(org: String, login: String, guest: Option<bool>) -> Result<String, String> {
    let login = login.trim().trim_start_matches('@');
    github::get(&format!("users/{login}")).map_err(|_| format!("There is no GitHub account called {login}"))?;
    let repos: &[&str] = if guest == Some(true) { &[crate::REQUESTS] } else { &INTERN_REPOS };
    for r in repos { github::send("PUT", &format!("repos/{org}/{r}/collaborators/{login}"), Some(serde_json::json!({ "permission": "pull" })))?; }
    if guest == Some(true) { return Ok(format!("Invited {login} as a guest, read on {} for the terms. GitHub emails them the invitation.", crate::REQUESTS)) }
    Ok(format!("Invited {login} as an intern, read on {}. GitHub emails one invitation per repo. You can then raise single repos for them in the access tree.", repos.join(", ")))
}

/// Takes an intern off every repo of the organisation and cancels the repo invitations (repo, id) they have not accepted.
#[tauri::command(async)]
pub fn remove_intern(org: String, login: String, invites: Vec<(String, u64)>) -> Result<String, String> {
    for (repo, id) in &invites { github::send("DELETE", &format!("repos/{org}/{repo}/invitations/{id}"), None)?; }
    // someone who accepted nothing yet is no outside collaborator, and GitHub answers 404
    if let Err(e) = github::send("DELETE", &format!("orgs/{org}/outside_collaborators/{login}"), None) { if !e.ends_with("(HTTP 404)") { return Err(e) } }
    let id = github::get(&format!("users/{login}")).ok().and_then(|u| u["id"].as_u64());
    Ok(format!("{login} no longer has any repo of {org}.{}", crate::cloudflare::after_removal(id)))
}

#[tauri::command(async)]
pub fn cancel_invite(org: String, id: u64) -> Result<String, String> {
    github::send("DELETE", &format!("orgs/{org}/invitations/{id}"), None).map(|_| "Invitation cancelled".into())
}

/// Makes `login` an owner (admin of every repo, can change everyone's access) or a plain member.
#[tauri::command(async)]
pub fn set_role(org: String, login: String, owner: bool) -> Result<String, String> {
    github::send("PUT", &format!("orgs/{org}/memberships/{login}"), Some(serde_json::json!({ "role": if owner { "admin" } else { "member" } })))?;
    Ok(format!("{login} is now {}.{}", if owner { "an owner" } else { "a member" }, crate::cloudflare::owner_left_note(&login)))
}

/// Takes `login` out of the organisation: every team and repo grant goes with it, and their sign-in to the machines
/// is ended when Cloudflare is connected. Their copies stay where they are.
#[tauri::command(async)]
pub fn remove_member(org: String, login: String) -> Result<String, String> {
    // their GitHub number, read before they are gone: Cloudflare knows people by it
    let id = github::get(&format!("users/{login}")).ok().and_then(|u| u["id"].as_u64());
    github::send("DELETE", &format!("orgs/{org}/memberships/{login}"), None)?;
    Ok(format!("{login} left the organisation.{}{}", crate::cloudflare::after_removal(id), crate::cloudflare::owner_left_note(&login)))
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

/// Each host's guests: the emails its Cloudflare Access application (named after the host) lets in one by one.
/// Hosts without an application, and everything when Cloudflare is not connected here or refuses, are left out.
#[tauri::command(async)]
pub fn machine_guests(hosts: Vec<String>) -> std::collections::BTreeMap<String, Vec<String>> {
    // ponytail: one page of access/apps (Cloudflare's default 25); page through when the account has more
    let Ok(apps) = crate::cloudflare::api("GET", "access/apps", None) else { return Default::default() };
    apps.as_array().into_iter().flatten()
        .filter_map(|a| { let h = a["name"].as_str().filter(|n| hosts.iter().any(|h| h == n))?; Some((h.to_string(), policy_emails(allow_policy(a)?))) })
        .collect()
}

/// Lets one person connect to `host` by the email they sign in to GitHub with, or takes that back: an email rule in
/// the allow policy of the host's Access application, beside the GitHub team rules, which are never touched. For
/// people GitHub teams cannot hold (interns). Removing also ends their Access sessions at once.
#[tauri::command(async)]
pub fn machine_guest(host: String, email: String, add: bool) -> Result<String, String> {
    let email = guest_email(&email)?;
    let apps = crate::cloudflare::api("GET", "access/apps", None)?;
    let app = apps.as_array().into_iter().flatten().find(|a| a["name"] == host.as_str()).ok_or(format!("{host} has no Cloudflare Access application"))?;
    let policy = allow_policy(app).ok_or(format!("{host}'s Access application has no allow policy"))?;
    let include = policy["include"].as_array().cloned().unwrap_or_default();
    let new = with_guest(&include, &email, add);
    if new != include {
        let (app_id, id) = (app["id"].as_str().unwrap_or_default(), policy["id"].as_str().unwrap_or_default());
        crate::cloudflare::api("PUT", &format!("access/apps/{app_id}/policies/{id}"), Some(policy_update(policy, new)))?;
    }
    if add { return Ok(format!("{email} may now connect to {host}")) }
    let ended = crate::cloudflare::api("POST", "access/organizations/revoke_user", Some(serde_json::json!({ "email": email })));
    Ok(format!("{email} can no longer connect to {host}.{}", if ended.is_ok() { " Their sign-in to the machines is ended." }
        else { " Their sign-in to the machines could not be ended; it runs out by itself within 24 hours." }))
}

/// The public email on `login`'s GitHub profile, to start from when asking for the one they sign in with.
#[tauri::command(async)]
pub fn public_email(login: String) -> Option<String> {
    let user = github::get(&format!("users/{login}")).ok()?;
    enrolled_email(user["id"].as_u64()?).or_else(|| user["email"].as_str().map(String::from))
}

/// The email Cloudflare Access knows the GitHub account numbered `github_id` by, once that person has signed in to
/// any of the team's Access applications (access::ENROLL is the one everyone may). It is the very address a
/// machine's email rule is matched against, so it cannot be the wrong one of their addresses.
fn enrolled_email(github_id: u64) -> Option<String> {
    // ponytail: one page of people (100) and one request each for their identity; keep a login -> email note on the
    // application when the team outgrows that
    let users = crate::cloudflare::api("GET", "access/users?per_page=100", None).ok()?;
    users.as_array()?.iter().filter_map(|u| u["id"].as_str()).find_map(|id| {
        let who = crate::cloudflare::api("GET", &format!("access/users/{id}/last_seen_identity"), None).ok()?;
        (who["idp"]["type"] == "github" && who["id"].as_u64() == Some(github_id)).then(|| who["email"].as_str().map(str::to_lowercase))?
    })
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
