//! Cloudflare, for owners: the tunnels, DNS names and Access applications that put the machines behind GitHub
//! sign-in. An owner pastes an API token into the app once; it is kept in the system keyring, never in a file,
//! and every call here reads it from there.
//!
//! Token permissions (Cloudflare dashboard, My Profile, API Tokens, Create Custom Token):
//!   Account: Cloudflare Tunnel Edit; Access: Apps and Policies Edit; Access: Organizations, Identity Providers,
//!   and Groups Edit; Access: SSH Auditing Edit; Access: Audit Logs Read.   Zone aiwalkcorp.com: DNS Edit.

use serde_json::Value;

// ponytail: one team, one Cloudflare account; move to vault_rules.json when a second one appears
const ACCOUNT: &str = "dfa11c7873c45c21495bad024de07619";
const API: &str = "https://api.cloudflare.com/client/v4";

// ---------------------------------------------------------------- the token, in the keyring
// ponytail: Linux's Secret Service only, as the Windows VM password; add the keyring crate when an owner on
// Windows or macOS needs these tools

#[cfg(target_os = "linux")]
mod store {
    use secret_service::{blocking::SecretService, EncryptionType};
    use std::collections::HashMap;
    fn attrs() -> HashMap<&'static str, &'static str> { HashMap::from([("xdg:schema", "com.aiwalk.setup.Cloudflare"), ("account", super::ACCOUNT)]) }

    pub fn load() -> Option<String> {
        let ss = SecretService::connect(EncryptionType::Dh).ok()?;
        let found = ss.search_items(attrs()).ok()?;
        let item = found.unlocked.into_iter().next().or_else(|| { let i = found.locked.into_iter().next()?; i.unlock().ok()?; Some(i) })?;
        String::from_utf8(item.get_secret().ok()?).ok()
    }
    pub fn save(token: &str) -> Result<(), String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| e.to_string())?;
        let c = ss.get_default_collection().map_err(|e| e.to_string())?;
        if c.is_locked().unwrap_or(false) { c.unlock().map_err(|e| e.to_string())? }
        c.create_item("aIwalk System Setup: Cloudflare", attrs(), token.as_bytes(), true, "text/plain").map(|_| ()).map_err(|e| e.to_string())
    }
    pub fn forget() {
        let Ok(ss) = SecretService::connect(EncryptionType::Dh) else { return };
        let found = ss.search_items(attrs());
        if let Ok(found) = found { for i in found.unlocked.into_iter().chain(found.locked) { let _ = i.delete(); } };
    }
}
#[cfg(not(target_os = "linux"))]
mod store {
    pub fn load() -> Option<String> { None }
    pub fn save(_: &str) -> Result<(), String> { Err("Cloudflare is connected from a Linux computer for now".into()) }
    pub fn forget() {}
}

// ---------------------------------------------------------------- the API

/// One request with `token`. `path` starts at the API root ("accounts/…", "zones/…", "user/tokens/verify").
/// Ok(the answer's "result"); Err(what Cloudflare said).
fn call(token: &str, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let req = ureq::http::Request::builder().method(method).uri(format!("{API}/{}", path.trim_start_matches('/')))
        .header("Authorization", format!("Bearer {token}")).header("Content-Type", "application/json")
        .body(body.map(Value::to_string).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut resp = crate::github::agent().run(req).map_err(|e| format!("Cloudflare did not answer: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("Cloudflare's answer was not readable (HTTP {status})"))?;
    if v["success"] == true { return Ok(v["result"].clone()) }
    let said = v["errors"].as_array().and_then(|e| e.first()).and_then(|e| e["message"].as_str()).unwrap_or("Cloudflare refused");
    Err(format!("{said} (HTTP {status})"))
}

/// A request with the owner's kept token; `path` is below the account ("access/apps", "cfd_tunnel") unless it
/// starts with "zones/" or "user/".
pub fn api(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let token = store::load().ok_or("Cloudflare is not connected on this computer")?;
    let path = if path.starts_with("zones/") || path.starts_with("user/") { path.to_string() } else { format!("accounts/{ACCOUNT}/{path}") };
    call(&token, method, &path, body.as_ref())
}

/// Whether `token` is a live token; Ok(when it expires, "" for never). Account tokens and user tokens verify at
/// different addresses.
fn verify(token: &str) -> Result<String, String> {
    let v = call(token, "GET", &format!("accounts/{ACCOUNT}/tokens/verify"), None).or_else(|_| call(token, "GET", "user/tokens/verify", None))?;
    if v["status"] != "active" { return Err(format!("the token is {}", v["status"].as_str().unwrap_or("not active"))) }
    Ok(v["expires_on"].as_str().unwrap_or("").chars().take(10).collect())
}

/// {connected, expires, problem}: what the owner's page shows.
#[tauri::command(async)]
pub fn cf_state() -> Value {
    let Some(token) = store::load() else { return serde_json::json!({ "connected": false }) };
    match verify(&token) {
        Ok(expires) => serde_json::json!({ "connected": true, "expires": expires }),
        Err(e) => serde_json::json!({ "connected": false, "problem": e }),
    }
}

/// Keeps `token` after checking it is live; the page passes what the owner pasted.
#[tauri::command(async)]
pub fn cf_connect(token: String) -> Result<String, String> {
    let token = token.trim();
    let expires = verify(token)?;
    store::save(token)?;
    Ok(if expires.is_empty() { "Cloudflare connected".into() } else { format!("Cloudflare connected; the token runs until {expires}") })
}

#[tauri::command(async)]
pub fn cf_forget() { store::forget() }

/// Ends every Cloudflare Access session of the person whose GitHub account has the number `github_id`: they are
/// signed out of the machines at once instead of when their sign-in runs out (up to 24 hours). Access keeps the
/// GitHub account number with each identity, so the person is found by it, not by guessing an email.
/// Ok(true) when sessions were ended, Ok(false) when this person never signed in to the machines.
pub fn end_sessions(github_id: u64) -> Result<bool, String> {
    let users = api("GET", "access/users?per_page=100", None)?;
    for u in users.as_array().into_iter().flatten() {
        let Some(id) = u["id"].as_str() else { continue };
        let Ok(seen) = api("GET", &format!("access/users/{id}/last_seen_identity"), None) else { continue };
        if seen["id"].as_u64() != Some(github_id) { continue }
        let email = seen["email"].as_str().or(u["email"].as_str()).ok_or("Cloudflare has no email for this person")?;
        api("POST", "access/organizations/revoke_user", Some(serde_json::json!({ "email": email })))?;
        return Ok(true);
    }
    Ok(false)
}

/// What removing someone says about their machine sign-in, for the owner who removed them. Never fails the removal.
pub fn after_removal(github_id: Option<u64>) -> &'static str {
    let Some(id) = github_id else { return "" };
    if store::load().is_none() { return " Their sign-in to the machines runs out by itself within 24 hours; connect Cloudflare on the Machines page to end it at once next time." }
    match end_sessions(id) {
        Ok(true) => " Their sign-in to the machines is ended.",
        Ok(false) => " They had never signed in to the machines.",
        Err(_) => " Their sign-in to the machines could not be ended (Cloudflare refused); it runs out by itself within 24 hours.",
    }
}
