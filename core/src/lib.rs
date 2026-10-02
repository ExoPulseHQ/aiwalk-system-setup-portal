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

/// Owners, members and teams from the `org_query` response; grants stay empty until `repo_grants`.
/// Default when the response is unreadable.
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
        out.teams.push(Team { slug: slug.to_string(), repos, members });
    }
    out
}

/// Per-repo permissions. Owners may list every repo's collaborators (team and direct grants alike);
/// anyone else can only ask for their own permission on the repos they can see.
pub fn repos_query(org: &str, owner: bool) -> String {
    let field = if owner { "collaborators(first:100, affiliation:ALL) { edges { permission node { login } } }" } else { "viewerPermission" };
    format!("{{ organization(login:\"{org}\") {{ repositories(first:100) {{ nodes {{ name {field} }} }} }} }}")
}

/// Fills `people`'s grants from a `repos_query` response; `viewer` is who ran it.
pub fn repo_grants(people: &mut BTreeMap<String, Person>, response_json: &str, viewer: &str) {
    let Ok(v) = serde_json::from_str::<Value>(response_json) else { return };
    let Some(repos) = v["data"]["organization"]["repositories"]["nodes"].as_array() else { return };
    let mut set = |login: &str, repo: &str, perm: &str| {
        if let Some(g) = people.get_mut(login).and_then(|p| p.grants.as_mut()) {
            g.insert(repo.to_string(), perm_rank(perm));
        }
    };
    for r in repos {
        let Some(name) = r["name"].as_str() else { continue };
        if let Some(edges) = r["collaborators"]["edges"].as_array() {
            for e in edges {
                if let (Some(l), Some(p)) = (e["node"]["login"].as_str(), e["permission"].as_str()) { set(l, name, p) }
            }
        } else if let Some(p) = r["viewerPermission"].as_str() {
            set(viewer, name, p);
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Machine {
    pub host: String,
    pub repo: String,
    pub account: String,
    pub ready: bool,
}

/// Every (host, code repo, account) from vault_rules.json's `machines` section.
pub fn machines(rules_json: &str) -> Vec<Machine> {
    let Ok(v) = serde_json::from_str::<Value>(rules_json) else { return vec![] };
    let Some(hosts) = v["machines"]["hosts"].as_array() else { return vec![] };
    hosts.iter().flat_map(|h| {
        let host = h["host"].as_str().unwrap_or_default().to_string();
        let ready = h["ready"].as_bool().unwrap_or(false);
        h["repos"].as_object().into_iter().flatten().map(move |(repo, acct)| Machine {
            host: host.clone(), repo: repo.clone(), account: acct.as_str().unwrap_or_default().to_string(), ready,
        })
    }).collect()
}

/// Machine logins come with write access to the code repo; read access does not include one.
pub fn can_sign_in(m: &Machine, grants: Option<&BTreeMap<String, u8>>) -> bool {
    grants.map_or(true, |g| g.get(&m.repo).copied().unwrap_or(0) >= 2)
}

/// An access request travels as an issue; the body carries what is asked for, the author is who asks.
pub fn request_body(repo: &str, level: &str, note: &str) -> String {
    format!("repo: {repo}\nlevel: {level}\n\n{note}\n\n<!-- sent by aIwalk System Setup -->")
}

/// (repo, level) from a request body; None unless both are there and level is read or write.
pub fn parse_request(body: &str) -> Option<(String, String)> {
    let field = |k: &str| body.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string());
    let (repo, level) = (field("repo:")?, field("level:")?);
    (!repo.is_empty() && (level == "read" || level == "write")).then_some((repo, level))
}

/// How this computer is signed in to GitHub, from `gh auth status` (the active account).
#[derive(Debug, Serialize, PartialEq, Default)]
pub struct Auth {
    pub login: String,
    /// "account" for a browser sign-in (gho_ token), "temporary" for a fine-grained token (github_pat_, always expires).
    pub method: String,
    /// "ssh" or "https", how git talks to GitHub.
    pub protocol: String,
}

pub fn parse_gh_status(text: &str) -> Option<Auth> {
    let mut accounts: Vec<(Auth, bool)> = vec![];
    for line in text.lines() {
        if let Some(rest) = line.split("Logged in to github.com account ").nth(1) {
            let login = rest.split_whitespace().next().unwrap_or_default().to_string();
            accounts.push((Auth { login, ..Default::default() }, false));
            continue;
        }
        let Some((a, active)) = accounts.last_mut() else { continue };
        let Some((key, value)) = line.trim().trim_start_matches("- ").split_once(": ") else { continue };
        match key {
            "Active account" => *active = value == "true",
            "Git operations protocol" => a.protocol = value.into(),
            "Token" if value.starts_with("github_pat_") => a.method = "temporary".into(),
            "Token" => a.method = "account".into(),
            _ => {}
        }
    }
    accounts.into_iter().find(|(_, active)| *active).map(|(a, _)| a)
}

/// An age public key: "age1" then bech32 text, or a plugin recipient such as "age1yubikey1…".
pub fn is_age_public_key(k: &str) -> bool {
    k.starts_with("age1") && k.len() >= 58 && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// An age-key request carries only the public key; the issue's author is who it belongs to.
pub fn age_request_body(public_key: &str) -> String {
    format!("age-key: {public_key}\n\n<!-- sent by aIwalk System Setup; the private key never leaves the computer -->")
}

pub fn parse_age_request(body: &str) -> Option<String> {
    body.lines().find_map(|l| l.strip_prefix("age-key:")).map(str::trim).filter(|k| is_age_public_key(k)).map(String::from)
}

/// exo-secrets' recipients.txt: `login age1…` per line, the list of who can decrypt. Comments start with #.
pub fn parse_recipients(text: &str) -> Vec<(String, String)> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(char::is_whitespace))
        .map(|(who, key)| (who.to_string(), key.trim().to_string()))
        .filter(|(_, k)| is_age_public_key(k)).collect()
}

/// Adds or replaces `login`'s key (one key per person), or drops `login` when `key` is None.
pub fn with_recipient(text: &str, login: &str, key: Option<&str>) -> String {
    let mut list: Vec<(String, String)> = parse_recipients(text).into_iter().filter(|(w, _)| w != login).collect();
    if let Some(k) = key { list.push((login.into(), k.into())) }
    list.sort();
    let body: String = list.iter().map(|(w, k)| format!("{w} {k}\n")).collect();
    format!("# Who can decrypt this repo: GitHub login and age public key. Changed by aIwalk System Setup;\n\
             # .sops.yaml is generated from this file.\n{body}")
}

/// .sops.yaml encrypting every file to every recipient.
pub fn sops_config(recipients: &[(String, String)]) -> String {
    let keys: Vec<&str> = recipients.iter().map(|(_, k)| k.as_str()).collect();
    format!("# Generated from recipients.txt by aIwalk System Setup; edit that file instead.\ncreation_rules:\n  - age: >-\n      {}\n", keys.join(",\n      "))
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

    const REPOS_OWNER: &str = r#"{"data": {"organization": {"repositories": {"nodes": [
        {"name": "exo-l3", "collaborators": {"edges": [{"permission": "WRITE", "node": {"login": "alice"}},
                                                       {"permission": "ADMIN", "node": {"login": "owner"}}]}},
        {"name": "NTKCAP", "collaborators": {"edges": [{"permission": "READ", "node": {"login": "alice"}}]}}]}}}}"#;

    #[test]
    fn grants_come_from_repo_permissions() {
        let a = org_access(ORG);
        assert_eq!(a.people["owner"].grants, None);
        let l3 = a.teams.iter().find(|t| t.slug == "l3-write").unwrap();
        assert_eq!(l3.members.iter().collect::<Vec<_>>(), ["alice"]);
        assert_eq!(org_access("garbage"), OrgAccess::default());

        let mut people = org_access(ORG).people;
        repo_grants(&mut people, REPOS_OWNER, "owner");
        let alice = people["alice"].grants.as_ref().unwrap();
        assert_eq!((alice["exo-l3"], alice["NTKCAP"]), (2, 1));
        assert_eq!(people["owner"].grants, None);

        let mut people = org_access(ORG).people;
        repo_grants(&mut people, r#"{"data":{"organization":{"repositories":{"nodes":[{"name":"exo-l3","viewerPermission":"READ"}]}}}}"#, "alice");
        assert_eq!(people["alice"].grants.as_ref().unwrap()["exo-l3"], 1);
    }

    #[test]
    fn machines_need_write_on_the_code_repo() {
        let rules = r#"{"machines": {"hosts": [{"host": "host-20", "repos": {"NTKCAP": "ntkcap", "ExoPulse": "exopulse"}, "ready": true}]}}"#;
        let ms = machines(rules);
        assert_eq!(ms.len(), 2);
        let g = BTreeMap::from([("NTKCAP".to_string(), 2u8), ("ExoPulse".to_string(), 1)]);
        let ok: Vec<_> = ms.iter().filter(|m| can_sign_in(m, Some(&g))).map(|m| m.account.as_str()).collect();
        assert_eq!(ok, ["ntkcap"]);
        assert!(ms.iter().all(|m| can_sign_in(m, None)));
        assert!(machines("{}").is_empty());
    }

    #[test]
    fn request_round_trip() {
        assert_eq!(parse_request(&request_body("exo-l3", "write", "for the gait study")), Some(("exo-l3".into(), "write".into())));
        assert_eq!(parse_request("repo: exo-l3\nlevel: admin"), None);
        assert_eq!(parse_request("hello"), None);
    }

    #[test]
    fn only_owners_see_everyone() {
        assert_eq!(visible_to(org_access(ORG).people, "owner").len(), 2);
        let member = visible_to(org_access(ORG).people, "alice");
        assert_eq!(member.keys().collect::<Vec<_>>(), ["alice"]);
        assert!(visible_to(org_access(ORG).people, "stranger").is_empty());
    }

    const K1: &str = "age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p";
    const K2: &str = "age1yubikey1qwt50d05nh5vutpdzmlg5wn80xq5negm4uj9ghv0snvdd3yysf5yw3rhl3t";

    #[test]
    fn age_requests_carry_only_a_valid_public_key() {
        assert_eq!(parse_age_request(&age_request_body(K1)).as_deref(), Some(K1));
        assert_eq!(parse_age_request("age-key: AGE-SECRET-KEY-1QQQ"), None);
        assert_eq!(parse_age_request("age-key: age1short"), None);
    }

    #[test]
    fn recipients_add_replace_and_remove() {
        let t = with_recipient("", "bob", Some(K1));
        let t = with_recipient(&t, "alice", Some(K2));
        assert_eq!(parse_recipients(&t), [("alice".into(), K2.into()), ("bob".into(), K1.into())]);
        let t = with_recipient(&t, "alice", Some(K1));
        assert_eq!(parse_recipients(&t).len(), 2);
        let t = with_recipient(&t, "bob", None);
        assert_eq!(parse_recipients(&t), [("alice".to_string(), K1.to_string())]);
        assert!(sops_config(&parse_recipients(&with_recipient(&t, "bob", Some(K2)))).contains(&format!("{K1},\n      {K2}")));
    }

    #[test]
    fn gh_status_picks_the_active_account() {
        let out = "github.com\n  ✓ Logged in to github.com account old (keyring)\n  - Active account: false\n  - Token: gho_****\n\
                   \n  ✓ Logged in to github.com account eddLai (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: github_pat_11****\n";
        assert_eq!(parse_gh_status(out), Some(Auth { login: "eddLai".into(), method: "temporary".into(), protocol: "https".into() }));
        assert_eq!(parse_gh_status("You are not logged into any GitHub hosts."), None);
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
