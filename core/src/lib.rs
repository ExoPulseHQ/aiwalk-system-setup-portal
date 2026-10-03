//! Logic shared by every platform. Nothing here runs a process or touches the network:
//! callers fetch the JSON (desktop through `gh api`, Android over HTTPS) and pass it in.

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

#[derive(Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct Machine {
    pub host: String,
    pub repo: String,
    pub account: String,
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
        h["repos"].as_object().into_iter().flatten().map(move |(repo, acct)| Machine {
            host: host.clone(), repo: repo.clone(), account: acct.as_str().unwrap_or_default().to_string(), ready,
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

/// The block the app keeps in ~/.ssh/config: one alias per tunnelled machine, reached through cloudflared.
/// `cloudflared` is the program's path. Machines without a tunnel are left out.
pub fn ssh_block(machines: &[Machine], cloudflared: &str) -> String {
    let mut seen = BTreeSet::new();
    let mut out = String::from("# >>> aIwalk System Setup: machines through Cloudflare (this block is rewritten by the app)\n");
    for m in machines {
        let Some(t) = &m.tunnel else { continue };
        if !seen.insert(m.host.clone()) { continue }
        if m.cert {
            // the certificate lasts minutes, so ssh asks cloudflared for a fresh one before every connection (Match exec);
            // the person's own keys are still tried after it, for as long as the machine keeps them
            out += &format!("Match originalhost {} exec \"'{cloudflared}' access ssh-gen --hostname {t}\"\n  HostName {t}\n  ProxyCommand \"{cloudflared}\" access ssh --hostname %h\n  \
                             IdentityFile ~/.cloudflared/{t}-cf_key\n  CertificateFile ~/.cloudflared/{t}-cf_key-cert.pub\n", m.host);
        } else {
            out += &format!("Host {}\n  HostName {t}\n  ProxyCommand \"{cloudflared}\" access ssh --hostname %h\n", m.host);
        }
    }
    out + "# <<< aIwalk System Setup\n"
}

/// `config` with the app's block replaced by `block` (or added at the end); everything else kept as it was.
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
    (!repo.is_empty() && (level == "read" || level == "write")).then_some((repo, level))
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

#[cfg(test)]
mod tests {
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
        let signed = ssh_block(&machines(&rules.replace(r#""tunnel": "ssh-host-20"#, r#""cert": true, "tunnel": "ssh-host-20"#)), "/opt/cloud flared");
        assert!(signed.contains(r#"Match originalhost host-20 exec "'/opt/cloud flared' access ssh-gen --hostname ssh-host-20.example.com""#)
            && signed.contains("CertificateFile ~/.cloudflared/ssh-host-20.example.com-cf_key-cert.pub") && !signed.contains("Host host-20"), "{signed}");
        let block = ssh_block(&machines(rules), "/opt/cloudflared");
        assert_eq!(block.matches("Host ").count(), 1, "one alias per tunnelled machine: {block}");
        assert!(block.contains("HostName ssh-host-20.example.com") && block.contains("\"/opt/cloudflared\" access ssh --hostname %h"));
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
    fn tree_groups_by_team() {
        let tree = build_tree(&vault_repos(RULES).unwrap(), &org_access(ORG).repo_teams);
        let groups: Vec<_> = tree.children.iter()
            .map(|g| (g.title.as_str(), g.children.iter().map(|c| c.title.as_str()).collect::<Vec<_>>())).collect();
        assert_eq!(groups, [("Layers", vec!["L3 Simulation AI"]), ("Team shared", vec!["Papers"]),
                            ("Founders", vec!["Mgmt", "Tooling"])]);
    }
}
