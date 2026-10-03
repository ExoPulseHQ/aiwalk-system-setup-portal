//! GitHub's API, asked directly over one kept-open HTTPS client instead of starting `gh` for every question.
//! Questions can then run side by side, and nothing here needs a program the phone does not have.
//!
//! The token still comes from gh (`gh auth token`) while sign-in is gh's job; that is the one place to change when
//! the app signs in by itself.

use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const API: &str = "https://api.github.com";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(40))).build().into())
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// The signed-in account's token. Asked of gh once and kept until `forget_token`.
fn token() -> Result<String, String> {
    let mut kept = TOKEN.lock().unwrap();
    if let Some(t) = kept.as_ref() { return Ok(t.clone()) }
    let t = crate::gh(&["auth", "token", "--hostname", "github.com"]).map_err(|e| format!("not logged in to GitHub: {e}"))?;
    let t = t.trim().to_string();
    if t.is_empty() { return Err("not logged in to GitHub".into()) }
    *kept = Some(t.clone());
    Ok(t)
}

/// After a sign-in, sign-out or account switch: the next question asks gh for the token again.
pub fn forget_token() { *TOKEN.lock().unwrap() = None; }

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

#[cfg(test)]
mod tests {
    #[test]
    fn next_page_comes_from_the_link_header() {
        let link = r#"<https://api.github.com/x?page=1>; rel="prev", <https://api.github.com/x?page=3>; rel="next", <https://api.github.com/x?page=9>; rel="last""#;
        assert_eq!(super::next_link(link).as_deref(), Some("https://api.github.com/x?page=3"));
        assert_eq!(super::next_link(r#"<https://api.github.com/x?page=1>; rel="prev""#), None);
    }
}
