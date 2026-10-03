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
    /// `what` is "token" (the Cloudflare API token) or "key" (the team key that opens the shared copy of it).
    fn attrs(what: &'static str) -> HashMap<&'static str, &'static str> {
        // the token keeps the attributes it was first stored with, so one kept before the team key existed is still found
        let mut a = HashMap::from([("xdg:schema", "com.aiwalk.setup.Cloudflare"), ("account", super::ACCOUNT)]);
        if what != "token" { a.insert("what", what); }
        a
    }
    pub fn load(what: &'static str) -> Option<String> {
        let ss = SecretService::connect(EncryptionType::Dh).ok()?;
        let found = ss.search_items(attrs(what)).ok()?;
        // a search for the token's attributes also matches the key's item (it has them all, plus "what"): skip those
        let mine = |i: &secret_service::blocking::Item| what != "token" || !i.get_attributes().is_ok_and(|a| a.contains_key("what"));
        let item = found.unlocked.into_iter().find(|i| mine(i)).or_else(|| { let i = found.locked.into_iter().find(|i| mine(i))?; i.unlock().ok()?; Some(i) })?;
        String::from_utf8(item.get_secret().ok()?).ok()
    }
    pub fn save(what: &'static str, secret: &str) -> Result<(), String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| e.to_string())?;
        let c = ss.get_default_collection().map_err(|e| e.to_string())?;
        if c.is_locked().unwrap_or(false) { c.unlock().map_err(|e| e.to_string())? }
        let label = if what == "token" { "aIwalk System Setup: Cloudflare" } else { "aIwalk System Setup: Cloudflare team key" };
        c.create_item(label, attrs(what), secret.as_bytes(), true, "text/plain").map(|_| ()).map_err(|e| e.to_string())
    }
    pub fn forget(what: &'static str) {
        let Ok(ss) = SecretService::connect(EncryptionType::Dh) else { return };
        let found = ss.search_items(attrs(what));
        if let Ok(found) = found {
            for i in found.unlocked.into_iter().chain(found.locked) {
                if what != "token" || !i.get_attributes().is_ok_and(|a| a.contains_key("what")) { let _ = i.delete(); }
            }
        };
    }
}
#[cfg(not(target_os = "linux"))]
mod store {
    pub fn load(_: &'static str) -> Option<String> { None }
    pub fn save(_: &'static str, _: &str) -> Result<(), String> { Err("Cloudflare is connected from a Linux computer for now".into()) }
    pub fn forget(_: &'static str) {}
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
    let token = store::load("token").ok_or("Cloudflare is not connected on this computer")?;
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

// ---------------------------------------------------------------- the token, shared between owners
// The owners act for one team, so they share one token. It is kept in exo-secrets (owners only) as an age file
// locked with one team key, which the first owner tells the others once, in person. Neither the repo alone nor the
// key alone gives the token. When an owner leaves, the token and the key are both changed.

const SHARED_REPO: &str = "exo-secrets";
const SHARED_FILE: &str = "cloudflare-token.age";
/// Who the owners were when the token was last shared, one login per line. Someone on this list who is no longer an
/// owner still knows the team key and has seen the token, so both must be changed; the page says so until they are.
const SHARED_WITH: &str = "cloudflare-token.owners";

fn owners_now() -> Result<Vec<String>, String> {
    Ok(crate::github::all(&format!("orgs/{}/members?role=admin", org()))?.iter().filter_map(|m| m["login"].as_str().map(String::from)).collect())
}

/// Writes one file in the owners' repo, replacing what was there.
fn put(file: &str, bytes: &[u8], message: &str) -> Result<(), String> {
    use base64::Engine;
    let path = format!("repos/{}/{SHARED_REPO}/contents/{file}", org());
    let sha = crate::github::get(&path).ok().and_then(|f| f["sha"].as_str().map(String::from));
    let mut body = serde_json::json!({ "message": message, "content": base64::engine::general_purpose::STANDARD.encode(bytes) });
    if let Some(sha) = sha { body["sha"] = sha.into() }
    crate::github::send("PUT", &path, Some(body)).map(|_| ())
}

/// Owners who knew the key at the last share and are owners no longer; empty when nothing was shared.
pub fn left_since_shared() -> Vec<String> {
    let Ok(then) = crate::github::raw(&format!("{}/{SHARED_REPO}", org()), SHARED_WITH) else { return vec![] };
    let Ok(now) = owners_now() else { return vec![] };
    exo_core::owners_gone(&then, &now)
}

fn org() -> String { crate::VAULTS[0].1.split('/').next().unwrap_or_default().to_string() }

/// A fresh team key: 100 bits as five groups of four, in letters and digits that are not mistaken for one another.
fn new_key() -> Result<String, String> {
    use ring::rand::SecureRandom;
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut raw = [0u8; 20];
    ring::rand::SystemRandom::new().fill(&mut raw).map_err(|_| "this computer gave no random numbers")?;
    let chars: Vec<char> = raw.iter().map(|b| ALPHABET[(b & 31) as usize] as char).collect();
    Ok(chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-"))
}

/// What a person typed for the key, as the key is written: upper case, groups joined by "-", spaces dropped.
fn tidy_key(k: &str) -> String {
    let flat: String = k.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    flat.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

fn lock(token: &str, key: &str) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let enc = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(key.to_owned()));
    let mut out = vec![];
    let armor = age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor).map_err(|e| e.to_string())?;
    let mut w = enc.wrap_output(armor).map_err(|e| e.to_string())?;
    w.write_all(token.as_bytes()).map_err(|e| e.to_string())?;
    w.finish().and_then(|a| a.finish()).map_err(|e| e.to_string())?;
    Ok(out)
}

fn unlock(file: &[u8], key: &str) -> Result<String, String> {
    use std::io::Read;
    let dec = age::Decryptor::new(age::armor::ArmoredReader::new(file)).map_err(|_| "the team's token file is not readable")?;
    let id = age::scrypt::Identity::new(age::secrecy::SecretString::from(key.to_owned()));
    let mut r = dec.decrypt(std::iter::once(&id as &dyn age::Identity)).map_err(|_| "That is not the team key")?;
    let mut token = String::new();
    r.read_to_string(&mut token).map_err(|e| e.to_string())?;
    Ok(token.trim().to_string())
}

/// Puts the kept token into the team's repo, locked with `key`. `fresh` says the token or the key is new, so
/// today's owners are the ones who know them; otherwise the list of who knew is left as it was.
fn publish(token: &str, key: &str, fresh: bool) -> Result<(), String> {
    put(SHARED_FILE, &lock(token, key)?, "chore: the owners' Cloudflare token, locked with the team key")?;
    let known = crate::github::raw(&format!("{}/{SHARED_REPO}", org()), SHARED_WITH).is_ok();
    if fresh || !known { put(SHARED_WITH, (owners_now()?.join("\n") + "\n").as_bytes(), "chore: who the owners were when the Cloudflare token was shared")?; }
    Ok(())
}

/// The token from the team's repo, opened with `key`; None when the owners have not shared one.
fn fetch(key: &str) -> Result<Option<String>, String> {
    match crate::github::raw(&format!("{}/{SHARED_REPO}", org()), SHARED_FILE) {
        Ok(file) => unlock(file.as_bytes(), key).map(Some),
        Err(e) if e.ends_with("(HTTP 404)") => Ok(None),
        Err(e) => Err(e),
    }
}

/// {connected, expires, problem, shared, has_key}: what the owner's page shows. A kept token that stopped working
/// is replaced from the team's repo when this computer holds the team key, so one owner renewing it renews it for all.
#[tauri::command(async)]
pub fn cf_state() -> Value {
    let shared = crate::github::get(&format!("repos/{}/{SHARED_REPO}/contents/{SHARED_FILE}", org())).is_ok();
    let key = store::load("key");
    let mut state = match store::load("token").map(|t| verify(&t)) {
        Some(Ok(expires)) => serde_json::json!({ "connected": true, "expires": expires }),
        other => {
            let renewed = key.as_deref().and_then(|k| fetch(k).ok().flatten()).and_then(|t| verify(&t).ok().map(|e| (t, e)));
            match renewed {
                Some((token, expires)) => { let _ = store::save("token", &token); serde_json::json!({ "connected": true, "expires": expires }) }
                None => match other { Some(Err(e)) => serde_json::json!({ "connected": false, "problem": e }), _ => serde_json::json!({ "connected": false }) },
            }
        }
    };
    state["shared"] = shared.into();
    state["has_key"] = key.is_some().into();
    // someone who knew the key and the token is an owner no longer: both must change
    if shared { state["left"] = left_since_shared().into(); }
    state
}

/// Keeps `token` after checking it is live; the page passes what the owner pasted. When this computer holds the
/// team key, the shared copy is renewed too, so the other owners get the new token by themselves.
#[tauri::command(async)]
pub fn cf_connect(token: String) -> Result<String, String> {
    let token = token.trim();
    let expires = verify(token)?;
    store::save("token", token)?;
    let also = match store::load("key") { Some(k) => match publish(token, &k, false) { Ok(()) => " The other owners get it too.", Err(_) => " The shared copy could not be renewed; share it again." }, None => "" };
    Ok(format!("Cloudflare connected{}.{also}", if expires.is_empty() { String::new() } else { format!("; the token runs until {expires}") }))
}

/// Shares the kept token with the other owners. Ok(the team key) to tell them; `fresh` makes a new key (after an
/// owner left), otherwise the key this computer already holds is used again.
#[tauri::command(async)]
pub fn cf_share(fresh: bool) -> Result<String, String> {
    let token = store::load("token").ok_or("Connect Cloudflare on this computer first")?;
    let kept = store::load("key").filter(|_| !fresh);
    let made = kept.is_none();
    let key = match kept { Some(k) => k, None => new_key()? };
    publish(&token, &key, made)?;
    store::save("key", &key)?;
    Ok(key)
}

/// The team key this computer holds, to tell a new owner.
#[tauri::command(async)]
pub fn cf_key() -> Option<String> { store::load("key") }

/// Connects with the team key another owner told this one: opens the shared token and keeps both.
#[tauri::command(async)]
pub fn cf_join(key: String) -> Result<String, String> {
    let key = tidy_key(&key);
    let token = fetch(&key)?.ok_or("The owners have not shared a token yet")?;
    let expires = verify(&token).map_err(|e| format!("The team's token no longer works ({e}); ask the owner who shared it to renew it"))?;
    store::save("token", &token)?;
    store::save("key", &key)?;
    Ok(if expires.is_empty() { "Cloudflare connected with the team's token".into() } else { format!("Cloudflare connected with the team's token; it runs until {expires}") })
}

/// One click to a new token: Cloudflare gives this token a new value and the old one stops working at once. Needs
/// the token to carry "API Tokens: Edit", which also lets it make tokens with any of the account's rights, so an
/// owner chooses whether to grant that. The shared copy is renewed when this computer holds the team key.
#[tauri::command(async)]
pub fn cf_renew() -> Result<String, String> {
    let token = store::load("token").ok_or("Connect Cloudflare on this computer first")?;
    // a token made under My Profile is renewed at user/tokens, one made under Manage Account at accounts/<id>/tokens
    let attempt = |verify_path: &str, base: &str| -> Result<String, String> {
        let id = call(&token, "GET", verify_path, None)?["id"].as_str().ok_or("Cloudflare did not name the token")?.to_string();
        call(&token, "PUT", &format!("{base}/{id}/value"), Some(&serde_json::json!({})))?.as_str().map(String::from).ok_or_else(|| "Cloudflare gave no new token".to_string())
    };
    let new = attempt("user/tokens/verify", "user/tokens").or_else(|_| attempt(&format!("accounts/{ACCOUNT}/tokens/verify"), &format!("accounts/{ACCOUNT}/tokens")))
        .map_err(|e| format!("This token may not renew itself ({e}). Give it \"API Tokens: Edit\" in Cloudflare, or use Roll there and paste the new one here."))?;
    store::save("token", &new)?;
    let also = match store::load("key") { Some(k) => match publish(&new, &k, false) { Ok(()) => " The other owners get it too.", Err(_) => " The shared copy could not be renewed; share it again." }, None => "" };
    Ok(format!("The token is renewed; the old one no longer works.{also}"))
}

/// Forgets the token on this computer; the team key too, so nothing here can fetch it again.
#[tauri::command(async)]
pub fn cf_forget() { store::forget("token"); store::forget("key") }

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
    if store::load("token").is_none() { return " Their sign-in to the machines runs out by itself within 24 hours; connect Cloudflare on the Machines page to end it at once next time." }
    match end_sessions(id) {
        Ok(true) => " Their sign-in to the machines is ended.",
        Ok(false) => " They had never signed in to the machines.",
        Err(_) => " Their sign-in to the machines could not be ended (Cloudflare refused); it runs out by itself within 24 hours.",
    }
}

/// --cf-selfcheck: the whole path on this computer with the token given on stdin. Connects, shares, forgets the
/// token and gets it back through the team key alone. Prints what worked, never the token or the key.
pub fn selfcheck(token: &str) -> Result<(), String> {
    println!("connect: {}", cf_connect(token.to_string())?.split(';').next().unwrap_or_default());
    let key = cf_share(false)?;
    println!("share: pushed to {}/{SHARED_REPO}/{SHARED_FILE}, key has {} characters", org(), key.len());
    println!("fetch with the key: {}", fetch(&key)?.is_some_and(|t| t == token.trim()));
    println!("fetch with another key: {}", fetch(&new_key()?).err().unwrap_or_else(|| "OPENED, which is wrong".into()));
    store::forget("token");
    println!("token forgotten here: {}", store::load("token").is_none());
    let st = cf_state();
    println!("state after forgetting: connected {}, shared {}, has_key {}", st["connected"], st["shared"], st["has_key"]);
    println!("token is back in the keyring: {}", store::load("token").is_some_and(|t| t == token.trim()));
    println!("join by typing the key in lower case with spaces: {}", cf_join(key.to_lowercase().replace('-', " ")).is_ok());
    Ok(())
}

/// What to tell an owner who just changed `login`'s role or removed them, when `login` knew the shared token.
pub fn owner_left_note(login: &str) -> &'static str {
    if left_since_shared().iter().any(|l| l.eq_ignore_ascii_case(login)) {
        " They knew the team's Cloudflare token and key: renew the token and make a new key on the Machines page."
    } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_key_opens_the_shared_token_and_nothing_else_does() {
        let key = new_key().unwrap();
        assert_eq!(key.len(), 24, "five groups of four: {key}");
        assert!(key.split('-').all(|g| g.len() == 4 && g.bytes().all(|b| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&b))), "{key}");
        let file = lock("not-a-real-token", &key).unwrap();
        assert!(file.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
        assert!(!String::from_utf8_lossy(&file).contains("not-a-real-token"));
        assert_eq!(unlock(&file, &key).unwrap(), "not-a-real-token");
        assert!(unlock(&file, &new_key().unwrap()).is_err());
        // typed with spaces and lower case, as people do
        assert_eq!(tidy_key(&format!(" {} ", key.to_lowercase().replace('-', " "))), key);
    }
}
