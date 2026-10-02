//! Logic shared by every platform. Nothing here runs a process or touches the network:
//! callers fetch the JSON (desktop through `gh api`, Android over HTTPS) and pass it in.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Where a vault keeps its routing file, tried in order.
pub const RULES_PATHS: [&str; 3] = ["System/vault_rules.json", "Project_Management/vault_rules.json", "vault_rules.json"];

/// GitHub repo permission as a rank: 4 admin, 2 write, 1 read, 0 none.
pub fn perm_rank(permission: &str) -> u8 {
    match permission {
        "ADMIN" => 4,
        "MAINTAIN" => 3,
        "WRITE" => 2,
        "TRIAGE" | "READ" => 1,
        _ => 0,
    }
}

/// The routing file's `repos` section, reduced to what the tree needs.
#[derive(Debug, PartialEq)]
pub struct VaultRepos {
    pub org: String,
    pub book: String,
    /// Every repo except the book, with the folders routed to it, in file order.
    pub repos: Vec<(String, Vec<String>)>,
}

/// Parses vault_rules.json. None when the vault is not split into repos.
pub fn vault_repos(rules_json: &str) -> Option<VaultRepos> {
    let v: Value = serde_json::from_str(rules_json).ok()?;
    let rules = v.get("repos")?;
    let org = rules.get("org")?.as_str().filter(|s| !s.is_empty())?.to_string();
    let book = rules.get("book")?.as_str()?.to_string();
    let mut repos: Vec<(String, Vec<String>)> = Vec::new();
    for route in rules.get("routes")?.as_array()? {
        let (Some(prefix), Some(repo)) = (route["prefix"].as_str(), route["repo"].as_str()) else { continue };
        let i = match repos.iter().position(|(r, _)| r == repo) {
            Some(i) => i,
            None => { repos.push((repo.to_string(), Vec::new())); repos.len() - 1 }
        };
        // folders as routed (Papers/), or the file itself when only a file is routed
        let shown = if prefix.ends_with('/') { prefix } else { prefix.rsplit('/').next().unwrap() };
        if !prefix.starts_with('.') && !repos[i].1.iter().any(|f| f == shown) {
            repos[i].1.push(shown.to_string());
        }
    }
    repos.retain(|(r, _)| *r != book);
    Some(VaultRepos { org, book, repos })
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Node {
    pub title: String,
    pub repo: Option<String>,
    pub detail: String,
    pub children: Vec<Node>,
}

fn layer_of(folders: &[String]) -> Option<String> {
    folders.iter().map(|f| f.trim_end_matches('/')).find(|f| {
        let b = f.as_bytes();
        b.len() > 2 && b[0] == b'L' && b[1].is_ascii_digit() && b[2] == b'_'
    }).map(str::to_string)
}

/// Groups the vault's repos by who GitHub grants them to: the whole team, only core, or a domain team.
pub fn build_tree(vault: &VaultRepos, repo_teams: &BTreeMap<String, BTreeSet<String>>) -> Node {
    let groups = [("Layers", "Detail repos, one per domain"), ("Team shared", "Everyone on the team"), ("Founders", "Core only")];
    let mut items: BTreeMap<&str, Vec<Node>> = BTreeMap::new();
    for (repo, folders) in &vault.repos {
        let layer = layer_of(folders);
        let title = match &layer {
            Some(l) => l.replace('_', " "),
            None => {
                let s = repo.strip_prefix("exo-").unwrap_or(repo);
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        };
        let teams: BTreeSet<&str> = repo_teams.get(repo).into_iter().flatten()
            .map(String::as_str).filter(|t| *t != "core").collect();
        let group = if teams.contains("members") { "Team shared" }
            else if !teams.is_empty() || layer.is_some() { "Layers" } else { "Founders" };
        let detail = if folders.is_empty() { repo.clone() } else { folders.join("  ") };
        items.entry(group).or_default().push(Node { title, repo: Some(repo.clone()), detail, children: vec![] });
    }
    let children = groups.iter().filter_map(|(g, about)| {
        let mut kids = items.remove(g)?;
        kids.sort_by(|a, b| a.title.cmp(&b.title));
        Some(Node { title: g.to_string(), repo: None, detail: about.to_string(), children: kids })
    }).collect();
    Node { title: "main".into(), repo: Some(vault.book.clone()), detail: "Primary logs, guides and templates".into(), children }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Person {
    pub name: String,
    /// None for org owners, who are admins of every repo.
    pub grants: Option<BTreeMap<String, u8>>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Team {
    pub slug: String,
    /// (repo, permission rank) this team grants.
    pub repos: Vec<(String, u8)>,
    pub members: BTreeSet<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct OrgAccess {
    pub people: BTreeMap<String, Person>,
    pub repo_teams: BTreeMap<String, BTreeSet<String>>,
    pub teams: Vec<Team>,
}

/// The GraphQL query whose response `org_access` reads.
pub fn org_query(org: &str) -> String {
    format!("{{ organization(login:\"{org}\") {{ membersWithRole(first:100) {{ edges {{ role node {{ login name }} }} }} \
             teams(first:50) {{ nodes {{ slug members(first:100) {{ nodes {{ login }} }} \
             repositories(first:50) {{ edges {{ permission node {{ name }} }} }} }} }} }} }}")
}

/// Who can reach which repo, from the `org_query` response. Default when the response is unreadable.
pub fn org_access(response_json: &str) -> OrgAccess {
    let mut out = OrgAccess::default();
    let Ok(v) = serde_json::from_str::<Value>(response_json) else { return out };
    let org = &v["data"]["organization"];
    let list = |x: &Value| x.as_array().cloned().unwrap_or_default();
    for edge in list(&org["membersWithRole"]["edges"]) {
        let Some(login) = edge["node"]["login"].as_str() else { continue };
        let name = edge["node"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(login).to_string();
        let grants = (edge["role"] != "ADMIN").then(BTreeMap::new);
        out.people.insert(login.to_string(), Person { name, grants });
    }
    for team in list(&org["teams"]["nodes"]) {
        let slug = team["slug"].as_str().unwrap_or_default();
        let repos: Vec<(String, u8)> = list(&team["repositories"]["edges"]).iter()
            .filter_map(|e| Some((e["node"]["name"].as_str()?.to_string(), perm_rank(e["permission"].as_str()?))))
            .collect();
        for (repo, _) in &repos {
            out.repo_teams.entry(repo.clone()).or_default().insert(slug.to_string());
        }
        let members: BTreeSet<String> = list(&team["members"]["nodes"]).iter()
            .filter_map(|m| m["login"].as_str().map(str::to_string)).collect();
        for login in &members {
            let Some(grants) = out.people.get_mut(login).and_then(|p| p.grants.as_mut()) else { continue };
            for (repo, rank) in &repos {
                let g = grants.entry(repo.clone()).or_insert(0);
                *g = (*g).max(*rank);
            }
        }
        out.teams.push(Team { slug: slug.to_string(), repos, members });
    }
    out
}

/// What `user` may see: org owners see everyone (for View as), anyone else only their own grants.
pub fn visible_to(mut people: BTreeMap<String, Person>, user: &str) -> BTreeMap<String, Person> {
    if people.get(user).is_some_and(|p| p.grants.is_none()) {
        return people;
    }
    people.remove(user).map(|p| BTreeMap::from([(user.to_string(), p)])).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"{"repos": {"org": "ExoPulseHQ", "book": "exo-book", "routes": [
        {"prefix": "main.md", "repo": "exo-book"},
        {"prefix": ".claude/", "repo": "exo-tooling"},
        {"prefix": "L3_Simulation_AI/", "repo": "exo-l3"},
        {"prefix": "Papers/", "repo": "exo-papers"},
        {"prefix": "Project_Management/Strategy_IP_Market/", "repo": "exo-mgmt"},
        {"prefix": "Papers/", "repo": "exo-papers"}]}}"#;

    const ORG: &str = r#"{"data": {"organization": {
        "membersWithRole": {"edges": [
            {"role": "ADMIN", "node": {"login": "owner", "name": "Owner"}},
            {"role": "MEMBER", "node": {"login": "alice", "name": null}}]},
        "teams": {"nodes": [
            {"slug": "members", "members": {"nodes": [{"login": "alice"}, {"login": "owner"}]},
             "repositories": {"edges": [{"permission": "READ", "node": {"name": "exo-papers"}}]}},
            {"slug": "l3-write", "members": {"nodes": [{"login": "alice"}]},
             "repositories": {"edges": [{"permission": "WRITE", "node": {"name": "exo-l3"}},
                                        {"permission": "WRITE", "node": {"name": "exo-papers"}}]}},
            {"slug": "core", "members": {"nodes": []},
             "repositories": {"edges": [{"permission": "ADMIN", "node": {"name": "exo-mgmt"}}]}}]}}}}"#;

    #[test]
    fn routes_become_repos_without_book_or_hidden_folders() {
        let v = vault_repos(RULES).unwrap();
        assert_eq!(v.org, "ExoPulseHQ");
        let names: Vec<_> = v.repos.iter().map(|(r, f)| (r.as_str(), f.join(","))).collect();
        assert_eq!(names, [("exo-tooling", "".into()), ("exo-l3", "L3_Simulation_AI/".into()),
                           ("exo-papers", "Papers/".into()), ("exo-mgmt", "Project_Management/Strategy_IP_Market/".into())]);
        assert_eq!(vault_repos(r#"{"repos": {"org": "", "book": "b", "routes": []}}"#), None);
        assert_eq!(vault_repos("not json"), None);
    }

    #[test]
    fn grants_take_the_highest_team_and_owners_get_everything() {
        let a = org_access(ORG);
        assert_eq!(a.people["owner"].grants, None);
        let alice = a.people["alice"].grants.as_ref().unwrap();
        assert_eq!(alice["exo-papers"], 2);
        assert_eq!(alice["exo-l3"], 2);
        assert!(!alice.contains_key("exo-mgmt"));
        let l3 = a.teams.iter().find(|t| t.slug == "l3-write").unwrap();
        assert_eq!(l3.members.iter().collect::<Vec<_>>(), ["alice"]);
        assert_eq!(org_access("garbage"), OrgAccess::default());
    }

    #[test]
    fn only_owners_see_everyone() {
        assert_eq!(visible_to(org_access(ORG).people, "owner").len(), 2);
        let member = visible_to(org_access(ORG).people, "alice");
        assert_eq!(member.keys().collect::<Vec<_>>(), ["alice"]);
        assert!(visible_to(org_access(ORG).people, "stranger").is_empty());
    }

    #[test]
    fn tree_groups_by_team() {
        let tree = build_tree(&vault_repos(RULES).unwrap(), &org_access(ORG).repo_teams);
        let groups: Vec<_> = tree.children.iter()
            .map(|g| (g.title.as_str(), g.children.iter().map(|c| c.title.as_str()).collect::<Vec<_>>())).collect();
        assert_eq!(groups, [("Layers", vec!["L3 Simulation AI"]), ("Team shared", vec!["Papers"]),
                            ("Founders", vec!["Mgmt", "Tooling"])]);
    }
}
