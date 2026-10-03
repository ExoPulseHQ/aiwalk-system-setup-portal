//! GitHub's API, asked directly over one kept-open HTTPS client instead of starting `gh` for every question.
//! Questions can then run side by side, and nothing here needs a program the phone does not have.
//!
//! Sign-in is the app's own (GitHub's device flow, through the team's OAuth App) and the app keeps the token itself
//! (login.rs). A computer that signed in before that still has it in gh only, so gh is asked when the app has none.

use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const API: &str = "https://api.github.com";

pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(40))).build().into())
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
/// What the token may do, as GitHub reports it with every answer ("repo, read:org, user:email").
static SCOPES: Mutex<String> = Mutex::new(String::new());

pub fn scopes() -> Vec<String> {
    SCOPES.lock().unwrap().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The signed-in account's token. Read once and kept until `forget_token`.
fn token() -> Result<String, String> {
    let mut kept = TOKEN.lock().unwrap();
    if let Some(t) = kept.as_ref() { return Ok(t.clone()) }
    let t = match crate::login::active() {
        Some((_, t)) => t,
        None => crate::gh(&["auth", "token", "--hostname", "github.com"]).map(|t| t.trim().to_string()).unwrap_or_default(),
    };
    if t.is_empty() { return Err("not signed in to GitHub: sign in in aIwalk System Setup".into()) }
    *kept = Some(t.clone());
    Ok(t)
}

/// After a sign-in, sign-out or account switch: the next question reads the token again.
pub fn forget_token() { *TOKEN.lock().unwrap() = None; }

/// The token in use right now, if one was read.
pub fn current_token() -> Option<String> { TOKEN.lock().unwrap().clone() }

/// Uses `token` from here on: a sign-in asks GitHub whose it is before anything keeps it.
pub fn use_token(token: &str) { *TOKEN.lock().unwrap() = Some(token.to_string()); }

/// Downloads the file of release `tag` whose name ends as `pattern` does ("*_amd64.deb") into `dir`. Ok(its path).
pub fn download_asset(repo: &str, tag: &str, pattern: &str, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let release = get(&format!("repos/{repo}/releases/tags/{tag}"))?;
    let asset = release["assets"].as_array().and_then(|a| a.iter().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(pattern.trim_start_matches('*')))))
        .ok_or("The release has no file for this computer")?;
    let (Some(name), Some(url)) = (asset["name"].as_str(), asset["url"].as_str()) else { return Err("The release's file has no address".into()) };
    let req = ureq::http::Request::builder().method("GET").uri(url)
        .header("Authorization", format!("Bearer {}", token()?))
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "aiwalk-system-setup")
        .body(()).map_err(|e| e.to_string())?;
    // tens of megabytes: a slow network gets an hour, not the 40 seconds a question gets
    let slow: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(3600))).build().into();
    let mut resp = slow.run(req).map_err(|e| format!("GitHub did not answer: {e}"))?;
    if !resp.status().is_success() { return Err(format!("GitHub refused the download (HTTP {})", resp.status().as_u16())) }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp.body_mut().as_reader(), &mut file).map_err(|e| format!("The download stopped: {e}"))?;
    Ok(path)
}

/// One request. `path` is "user", "repos/o/r/issues?state=open" or a full URL (a next page).
/// Ok((body text, next page's URL)); Err("what GitHub said (HTTP 404)") for anything that is not a success.
fn call(method: &str, path: &str, body: Option<&Value>, accept: &str) -> Result<(String, Option<String>), String> {
    let url = if path.starts_with("https://") { path.to_string() } else { format!("{API}/{}", path.trim_start_matches('/')) };
    let req = ureq::http::Request::builder().method(method).uri(&url)
        .header("Authorization", format!("Bearer {}", token()?))
        .header("Accept", accept)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "aiwalk-system-setup")
        .header("Content-Type", "application/json")
        .body(body.map(Value::to_string).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut resp = agent().run(req).map_err(|e| format!("GitHub did not answer: {e}"))?;
    let status = resp.status().as_u16();
    if let Some(sc) = resp.headers().get("x-oauth-scopes").and_then(|v| v.to_str().ok()) { *SCOPES.lock().unwrap() = sc.to_string(); }
    let next = resp.headers().get("link").and_then(|l| l.to_str().ok()).and_then(next_link);
    let text = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    if (200..300).contains(&status) { return Ok((text, next)) }
    let said = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_default();
    Err(format!("{} (HTTP {status})", if said.is_empty() { "GitHub refused" } else { &said }))
}

/// The rel="next" URL of a Link header.
fn next_link(link: &str) -> Option<String> {
    link.split(',').find(|p| p.contains("rel=\"next\""))
        .and_then(|p| Some(p.split('<').nth(1)?.split('>').next()?.to_string()))
}

const JSON: &str = "application/vnd.github+json";

fn parse(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() { return Ok(Value::Null) }
    serde_json::from_str(text).map_err(|e| format!("GitHub's answer was not readable: {e}"))
}

pub fn get(path: &str) -> Result<Value, String> { parse(&call("GET", path, None, JSON)?.0) }

/// Every page of a list ("orgs/o/members", "repos/o/r/issues?state=all").
pub fn all(path: &str) -> Result<Vec<Value>, String> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let mut next = Some(format!("{path}{sep}per_page=100"));
    let mut out = vec![];
    while let Some(url) = next {
        let (text, n) = call("GET", &url, None, JSON)?;
        out.extend(parse(&text)?.as_array().cloned().unwrap_or_default());
        next = n;
    }
    Ok(out)
}

/// PUT, POST, PATCH or DELETE, with a JSON body or none. Null when GitHub answers with no body.
pub fn send(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    parse(&call(method, path, body.as_ref(), JSON)?.0)
}

/// A file's text as it is in the repo.
pub fn raw(repo: &str, path: &str) -> Result<String, String> {
    Ok(call("GET", &format!("repos/{repo}/contents/{path}"), None, "application/vnd.github.raw")?.0)
}

/// A GraphQL query; the whole answer as text ({"data": ...}), as the readers in exo-core take it.
pub fn graphql(query: &str) -> Result<String, String> {
    Ok(call("POST", "graphql", Some(&serde_json::json!({ "query": query })), JSON)?.0)
}

// ---------------------------------------------------------------- Sign-in
// GitHub's device flow: the app asks for a one-time code, the person enters it at github.com/login/device in any
// browser, and the app waits for the token. No secret is involved, so the client id below is not one.

/// The team's OAuth App "aIwalk System Setup", owned by ExoPulseHQ, with the device flow switched on.
const CLIENT_ID: &str = "Ov23libbDrZ77FJNDo4L";
/// repo: the vaults and code. read:org: teams. user:email: to tell that the machines' sign-in is the same person.
pub const SCOPES_MEMBER: &str = "repo read:org user:email";
/// Owners add admin:org, for invitations, roles and team membership. Asked for only when an owner unlocks the tools.
pub const SCOPES_OWNER: &str = "repo admin:org user:email";

fn oauth(url: &str, body: Value) -> Result<Value, String> {
    let req = ureq::http::Request::builder().method("POST").uri(url)
        .header("Accept", "application/json").header("Content-Type", "application/json").header("User-Agent", "aiwalk-system-setup")
        .body(body.to_string()).map_err(|e| e.to_string())?;
    let mut resp = agent().run(req).map_err(|e| format!("GitHub did not answer: {e}"))?;
    parse(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?)
}

/// Runs the device flow for `scopes`. `show(code, url)` is called once with what the person must enter and where;
/// then this waits, up to the code's life (15 minutes), for them to approve. Ok(token).
pub fn device_sign_in(scopes: &str, show: &dyn Fn(&str, &str)) -> Result<String, String> {
    let d = oauth("https://github.com/login/device/code", serde_json::json!({ "client_id": CLIENT_ID, "scope": scopes }))?;
    let (Some(device), Some(code)) = (d["device_code"].as_str(), d["user_code"].as_str()) else {
        return Err(format!("GitHub gave no sign-in code: {}", d["error_description"].as_str().or(d["error"].as_str()).unwrap_or("no reason given")))
    };
    show(code, d["verification_uri"].as_str().unwrap_or("https://github.com/login/device"));
    let mut wait = d["interval"].as_u64().unwrap_or(5);
    let until = std::time::Instant::now() + Duration::from_secs(d["expires_in"].as_u64().unwrap_or(900));
    while std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_secs(wait));
        let r = oauth("https://github.com/login/oauth/access_token", serde_json::json!({
            "client_id": CLIENT_ID, "device_code": device, "grant_type": "urn:ietf:params:oauth:grant-type:device_code" }))?;
        if let Some(t) = r["access_token"].as_str() { return Ok(t.to_string()) }
        match r["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => wait = r["interval"].as_u64().unwrap_or(wait + 5),
            Some("access_denied") => return Err("Sign-in was cancelled on the GitHub page".into()),
            Some("expired_token") => break,
            other => return Err(format!("GitHub refused the sign-in: {}", r["error_description"].as_str().or(other).unwrap_or("no reason given"))),
        }
    }
    Err("The code ran out before it was approved. Try again.".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn next_page_comes_from_the_link_header() {
        let link = r#"<https://api.github.com/x?page=1>; rel="prev", <https://api.github.com/x?page=3>; rel="next", <https://api.github.com/x?page=9>; rel="last""#;
        assert_eq!(super::next_link(link).as_deref(), Some("https://api.github.com/x?page=3"));
        assert_eq!(super::next_link(r#"<https://api.github.com/x?page=1>; rel="prev""#), None);
    }
}
