//! Cloudflare Access done by the app itself, so the phone (which cannot run cloudflared) reaches the machines too.
//! The sign-in is still cloudflared's: this reads the token files it keeps in ~/.cloudflared, so both work side by
//! side, and writes the SSH certificate where cloudflared's ssh-gen would. Requests follow cloudflared's own
//! (packages token, sshgen, carrier). Tokens are never printed or logged; errors name the host, not the token.
//!
//! Only the team's own hostnames and Access organisation are accepted: a token goes to nothing else, and an answer
//! that claims another organisation is refused rather than trusted.

use crate::home;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ponytail: one team; move both to vault_rules.json when a second Access organisation appears
const ZONE: &str = ".aiwalkcorp.com";
const TEAM: &str = "aiwalkcorp.cloudflareaccess.com";

fn dir() -> PathBuf { home().join(".cloudflared") }
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

/// Redirects are not followed: a redirect from Access means "sign in", and following it would carry the token on.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0)
        .timeout_global(Some(Duration::from_secs(20))).build().into())
}

fn b64url(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

/// A JWT's claims, without checking its signature (the caller decides whether that matters).
fn claims(jwt: &str) -> Option<Value> {
    let p: Vec<&str> = jwt.trim().split('.').collect();
    if p.len() != 3 { return None }
    serde_json::from_slice(&b64url(p[1])?).ok()
}

/// One of the team's hostnames, lower-cased; anything else (another domain, a port, a path, odd characters) is refused.
fn team_host(h: &str) -> Result<String, String> {
    let h = h.trim().to_ascii_lowercase();
    let clean = !h.is_empty() && h.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') && !h.contains("..");
    if clean && h.ends_with(ZONE) && h.len() > ZONE.len() { Ok(h) } else { Err(format!("{h} is not one of the team's machines")) }
}

// ---------------------------------------------------------------- a. the application token

/// Which Access application guards a hostname. status-<name> and ssh-<name> share one, so the token file is named
/// after the application, not the hostname asked for; Access says which, in a metadata token it signs.
struct App { aud: String, app_host: String }

fn app_info(host: &str) -> Result<App, String> {
    let req = ureq::http::Request::builder().method("HEAD").uri(format!("https://{host}/"))
        .header("cf-access-metadata-request", "true").header("User-Agent", "aiwalk-system-setup")
        .body(()).map_err(|e| e.to_string())?;
    let resp = agent().run(req).map_err(|e| format!("{host} did not answer: {e}"))?;
    let jwt = resp.headers().get("cf-access-metadata").and_then(|v| v.to_str().ok())
        .ok_or_else(|| format!("{host} is not behind Cloudflare Access"))?.to_string();
    let c = verified(&jwt)?;
    app_from(&c, host, now())
}

/// The application named by verified metadata claims, after the checks cloudflared makes, plus the team's own.
fn app_from(c: &Value, host: &str, now: i64) -> Result<App, String> {
    let s = |k: &str| c[k].as_str().unwrap_or_default().to_ascii_lowercase();
    if s("hostname") != host || s("type") != "match" { return Err(format!("Access answered for another hostname than {host}")) }
    if s("auth_domain") != TEAM { return Err(format!("{host} is guarded by another Access organisation than {TEAM}")) }
    let iat = c["iat"].as_i64().unwrap_or(0);
    if iat < now - 24 * 3600 || iat > now + 300 { return Err("Access's answer is out of date; check this computer's clock".into()) }
    let app_host = if s("app_hostname").is_empty() { host.to_string() } else { team_host(&s("app_hostname"))? };
    let aud = s("aud");
    if aud.is_empty() || !aud.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Access named no application".into()) }
    Ok(App { aud, app_host })
}

/// The claims of a token signed by the team's Access organisation (RS256, keys from its certs page, kept for the
/// app's life and fetched again once when a key is not among them).
fn verified(jwt: &str) -> Result<Value, String> {
    static KEYS: Mutex<Option<Value>> = Mutex::new(None);
    let p: Vec<&str> = jwt.split('.').collect();
    let bad = || "Access's answer is not a signed token".to_string();
    if p.len() != 3 { return Err(bad()) }
    let header: Value = serde_json::from_slice(&b64url(p[0]).ok_or_else(bad)?).map_err(|_| bad())?;
    if header["alg"] != "RS256" { return Err(bad()) }
    let kid = header["kid"].as_str().ok_or_else(bad)?;
    let sig = b64url(p[2]).ok_or_else(bad)?;
    let find = |keys: &Value| keys["keys"].as_array().and_then(|ks| ks.iter().find(|k| k["kid"] == kid && k["kty"] == "RSA").cloned());
    let mut kept = KEYS.lock().unwrap();
    let key = match kept.as_ref().and_then(find) {
        Some(k) => k,
        None => {
            let mut resp = agent().get(format!("https://{TEAM}/cdn-cgi/access/certs")).call().map_err(|e| format!("Access did not answer: {e}"))?;
            let keys: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|_| "Access's keys are not readable")?;
            let k = find(&keys).ok_or("Access's answer is signed with a key Access does not list")?;
            *kept = Some(keys);
            k
        }
    };
    let (n, e) = (b64url(key["n"].as_str().unwrap_or_default()).ok_or_else(bad)?, b64url(key["e"].as_str().unwrap_or_default()).ok_or_else(bad)?);
    ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, format!("{}.{}", p[0], p[1]).as_bytes(), &sig)
        .map_err(|_| "Access's answer does not carry Access's signature")?;
    serde_json::from_slice(&b64url(p[1]).ok_or_else(bad)?).map_err(|_| bad())
}

/// cloudflared's name for an application's token file: <app hostname>-<aud>-token.
fn token_file(app: &App) -> String { format!("{}-{}-token", app.app_host, app.aud) }

/// A kept token is usable while it is for this application and has more than a minute left; the minute keeps a
/// connection from starting on a token about to run out.
fn usable(jwt: &str, aud: &str, host: &str, now: i64) -> Result<(), String> {
    let c = claims(jwt).ok_or_else(|| format!("The Cloudflare sign-in kept for {host} is not readable; sign in again"))?;
    let auds: Vec<&str> = match &c["aud"] { Value::String(s) => vec![s.as_str()], Value::Array(a) => a.iter().filter_map(Value::as_str).collect(), _ => vec![] };
    if !auds.contains(&aud) { return Err(format!("The Cloudflare sign-in kept for {host} belongs to another application; sign in again")) }
    if c["iss"].as_str() != Some(&format!("https://{TEAM}")) { return Err(format!("The Cloudflare sign-in kept for {host} is from another organisation")) }
    if c["exp"].as_i64().unwrap_or(0) <= now + 60 { return Err(format!("The Cloudflare sign-in for {host} has run out; sign in again")) }
    Ok(())
}

/// The signed-in person's Access token for `host`, from the file cloudflared keeps. Never printed or logged.
pub fn token(host: &str) -> Result<String, String> {
    let host = team_host(host)?;
    let app = app_info(&host)?;
    let jwt = std::fs::read_to_string(dir().join(token_file(&app)))
        .map_err(|_| format!("This computer has no Cloudflare sign-in for {host}; sign in from the app first"))?;
    usable(&jwt, &app.aud, &host, now())?;
    Ok(jwt.trim().to_string())
}

// ---------------------------------------------------------------- b. SSH through a WebSocket

/// `aiwalk-setup --ssh-proxy <host>` as ssh's ProxyCommand: the SSH bytes on stdin/stdout travel as binary WebSocket
/// frames to wss://<host>/, which Cloudflare hands to the machine's sshd. Ends when either side closes.
/// tungstenite's socket cannot be split between threads, so a thread reads stdin into a channel and this loop
/// alternates: send what stdin gave, then read the socket with a short timeout.
pub fn ssh_proxy(host: &str) -> Result<(), String> {
    use std::sync::mpsc::{channel, TryRecvError};
    use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Error, Message};
    let host = team_host(host)?;
    let token = token(&host)?;
    let mut req = format!("wss://{host}/").into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("Cf-Access-Token", token.parse().map_err(|_| "the kept token is not usable")?);
    let (mut ws, _) = tungstenite::connect(req).map_err(|e| match e {
        Error::Http(r) => format!("Cloudflare refused the connection to {host} (HTTP {})", r.status().as_u16()),
        e => format!("Could not reach {host}: {e}"),
    })?;
    // ponytail: 20 ms poll; keystrokes wait at most that long. Split the TLS stream if it ever shows.
    let tick = Some(Duration::from_millis(20));
    match ws.get_mut() {
        MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(tick),
        MaybeTlsStream::Plain(s) => s.set_read_timeout(tick),
        _ => Ok(()),
    }.map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        // never more than 16 KB a frame: the machine's cloudflared reads each frame into a 16 KB buffer and drops the
        // rest (cfio.Copy, websocket.Conn.Read), which breaks SSH mid-stream (seen with 64 KB uploads)
        let (mut input, mut buf) = (std::io::stdin().lock(), vec![0u8; 16 * 1024]);
        while let Ok(n) = input.read(&mut buf) { if n == 0 || tx.send(buf[..n].to_vec()).is_err() { break } }
    });   // dropping tx is how the loop learns stdin ended
    let mut out = std::io::stdout().lock();
    let lost = |e: Error| format!("The connection to {host} broke: {e}");
    loop {
        loop {
            match rx.try_recv() {
                Ok(b) => ws.send(Message::Binary(b.into())).map_err(lost)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => { let _ = ws.close(None); let _ = ws.flush(); return Ok(()) }
            }
        }
        match ws.read() {
            Ok(Message::Binary(b)) => { if out.write_all(&b).and_then(|_| out.flush()).is_err() { return Ok(()) } }   // ssh is gone
            Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => return Ok(()),
            Ok(_) => {}   // pings are answered by tungstenite itself
            Err(Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(lost(e)),
        }
    }
}

// ---------------------------------------------------------------- c. a short-lived SSH certificate

/// Writes `data` readable by the owner only, whatever mode the file had before.
fn write_private(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
    o.open(path).and_then(|mut f| f.write_all(data)).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?; }
    Ok(())
}

/// The certificate Access signs for `host` (minutes long, key ID = the person's email), in the three files
/// cloudflared's ssh-gen writes: ~/.cloudflared/<host>-cf_key, -cf_key.pub, -cf_key-cert.pub. The key pair is kept
/// and reused, as cloudflared does; ssh-keygen makes it, so no key code lives here. Ok((key file, certificate file)).
pub fn ssh_cert(host: &str) -> Result<(PathBuf, PathBuf), String> {
    let host = team_host(host)?;
    let token = token(&host)?;
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let key = d.join(format!("{host}-cf_key"));
    let (pub_file, cert_file) = (d.join(format!("{host}-cf_key.pub")), d.join(format!("{host}-cf_key-cert.pub")));
    if !(key.is_file() && pub_file.is_file()) {
        // half a pair is useless; ssh-keygen would stop to ask about overwriting it
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&pub_file);
        let (code, out) = crate::sh("ssh-keygen", &["-q", "-t", "ecdsa", "-b", "256", "-N", "", "-C", "", "-f", &key.to_string_lossy()], 20);
        if code != 0 { return Err(format!("ssh-keygen could not make a key: {out}")) }
    }
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?; }
    let public_key = std::fs::read_to_string(&pub_file).map_err(|e| e.to_string())?;
    // the token's issuer is the sign endpoint's host, as cloudflared uses it; checked to be the team's in `usable`
    let issuer = format!("https://{TEAM}");
    let body = serde_json::json!({ "public_key": public_key, "jwt": token, "issuer": issuer }).to_string();
    let mut resp = agent().post(format!("{issuer}/cdn-cgi/access/cert_sign")).header("Content-Type", "application/json")
        .send(body).map_err(|e| format!("Access did not answer: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).unwrap_or_default();
    if status != 200 { return Err(format!("Access would not sign a certificate for {host}: {} (HTTP {status})", v["message"].as_str().unwrap_or("no reason given"))) }
    let cert = v["certificate"].as_str().unwrap_or_default().trim();
    // one line, an OpenSSH certificate: anything else would end up in a file ssh trusts
    if !(cert.split(' ').next().unwrap_or_default().ends_with("-cert-v01@openssh.com") && !cert.contains('\n')) {
        return Err("Access's certificate is not an SSH certificate".into())
    }
    write_private(&cert_file, format!("{cert}\n").as_bytes())?;
    Ok((key, cert_file))
}

// ---------------------------------------------------------------- d. a page behind Access

/// GETs an https:// page on one of the team's hostnames with the person's token; Ok(the body) only on HTTP 200.
pub fn get(url: &str) -> Result<String, String> {
    let rest = url.strip_prefix("https://").ok_or("only https:// pages")?;
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = team_host(host)?;
    let token = token(&host)?;
    let mut resp = agent().get(format!("https://{host}{}", if path.is_empty() { "/" } else { path }))
        .header("cf-access-token", &token).call().map_err(|e| format!("{host} did not answer: {e}"))?;
    match resp.status().as_u16() {
        200 => resp.body_mut().with_config().limit(1024 * 1024).read_to_string().map_err(|e| e.to_string()),
        s @ (300..=399 | 401 | 403) => Err(format!("Cloudflare did not accept the sign-in for {host} (HTTP {s})")),
        s => Err(format!("{host} answered HTTP {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A JWT with these claims and no real signature: only the claims are read in `usable`.
    fn jwt(c: Value) -> String {
        use base64::Engine;
        let e = |v: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        format!("{}.{}.sig", e(br#"{"alg":"RS256"}"#), e(c.to_string().as_bytes()))
    }

    #[test]
    fn unsigned_metadata_is_refused_before_any_request() {
        let unsigned = jwt(json!({ "type": "match" })).replace(".sig", ".");
        for alg in [r#"{"alg":"none"}"#, r#"{"alg":"HS256","kid":"k"}"#] {
            use base64::Engine;
            let h = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(alg);
            let forged = format!("{h}.{}", unsigned.split_once('.').unwrap().1);
            assert!(verified(&forged).is_err());
        }
        assert!(verified("only.two").is_err());
    }

    #[test]
    fn only_the_teams_hostnames() {
        assert_eq!(team_host("SSH-tiger.aiwalkcorp.com").unwrap(), "ssh-tiger.aiwalkcorp.com");
        for bad in ["evil.com", "aiwalkcorp.com", ".aiwalkcorp.com", "x.aiwalkcorp.com.evil.com", "x.aiwalkcorp.com:22",
                    "x.aiwalkcorp.com/a", "a@x.aiwalkcorp.com", "../x.aiwalkcorp.com", ""] {
            assert!(team_host(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_token_file_is_the_applications() {
        let meta = json!({ "type": "match", "hostname": "status-tiger.aiwalkcorp.com", "auth_domain": TEAM, "aud": "BB70", "app_hostname": "ssh-tiger.aiwalkcorp.com", "iat": 1000 });
        let app = app_from(&meta, "status-tiger.aiwalkcorp.com", 1000).unwrap();
        assert_eq!(token_file(&app), "ssh-tiger.aiwalkcorp.com-bb70-token");
        // refused: another hostname, another organisation, a stale answer, an application outside the team, no aud
        assert!(app_from(&meta, "ssh-tiger.aiwalkcorp.com", 1000).is_err());
        let with = |k: &str, v: Value| { let mut m = meta.clone(); m[k] = v; app_from(&m, "status-tiger.aiwalkcorp.com", 1000) };
        assert!(with("auth_domain", json!("other.cloudflareaccess.com")).is_err());
        assert!(with("iat", json!(1000 - 25 * 3600)).is_err());
        assert!(with("app_hostname", json!("x.evil.com")).is_err());
        assert!(with("app_hostname", json!("../../.ssh/x.aiwalkcorp.com")).is_err());
        assert!(with("aud", json!("")).is_err());
        assert!(with("aud", json!("ab/../cd")).is_err());
    }

    #[test]
    fn kept_tokens_are_checked_before_use() {
        let iss = format!("https://{TEAM}");
        let ok = jwt(json!({ "aud": ["aa", "bb"], "exp": 2000, "iss": iss }));
        assert!(usable(&ok, "bb", "h", 1000).is_ok());
        assert!(usable(&jwt(json!({ "aud": "bb", "exp": 2000, "iss": iss })), "bb", "h", 1000).is_ok());
        assert!(usable(&ok, "cc", "h", 1000).unwrap_err().contains("another application"));
        assert!(usable(&ok, "bb", "h", 1950).unwrap_err().contains("run out"));   // less than a minute left
        assert!(usable(&ok, "bb", "h", 3000).unwrap_err().contains("run out"));
        assert!(usable(&jwt(json!({ "aud": "bb", "exp": 2000, "iss": "https://x.cloudflareaccess.com" })), "bb", "h", 1000).is_err());
        assert!(usable("not a token", "bb", "h", 1000).is_err());
    }
}
