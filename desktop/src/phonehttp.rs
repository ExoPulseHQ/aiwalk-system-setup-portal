#![cfg_attr(not(target_os = "android"), allow(dead_code))]
//! Sealed notes for Obsidian on a phone, which cannot start a program: it asks over HTTP on 127.0.0.1. Two servers
//! answer the same way: the Android app (sealed.rs, port 47170) and `aiwalk-sealed-vm` in Android's Linux Terminal
//! (vmkey/, port 47171), for when the Terminal's VM holds the key. `GET /open?file=`, `POST /save?file=` (the text
//! as the body), `GET /state?file=`, each with `token=`; the file is relative to the vault. `GET /whoami` names the
//! GitHub account the app is signed in with (never its token), for the plugin, which cannot run gh on a phone. Kept
//! free of tauri so the VM program can share it. The app adds `/sync/...` through `more` (phonegit.rs phone_sync): Sync
//! vault from inside Obsidian.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The unlocked vaults: the file a vault-relative path names (None while locked), and a note's text (Err(None): locked).
/// `who`: the GitHub login signed in here, if any. `more`: routes this server alone answers (route, query, body), None
/// for the rest.
pub type More = fn(&str, &dyn Fn(&str) -> String, &[u8]) -> Option<(u16, Vec<u8>)>;
pub struct Vaults { pub file: fn(&str) -> Option<PathBuf>, pub open: fn(&Path) -> Result<Vec<u8>, Option<String>>, pub who: fn() -> Option<String>, pub more: More }

/// Answers on 127.0.0.1:`port` from a thread; false when the port is taken.
pub fn serve(port: u16, token: &'static str, v: &'static Vaults) -> bool {
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) else { return false };
    std::thread::spawn(move || for c in listener.incoming().flatten() { std::thread::spawn(move || { let _ = request(c, token, v); }); });
    true
}

fn request(mut c: std::net::TcpStream, token: &str, v: &Vaults) -> std::io::Result<()> {
    let _ = c.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    // the head, then a body as long as Content-Length says
    let head_end = loop {
        let n = c.read(&mut b)?;
        if n == 0 { return Ok(()) }
        buf.extend_from_slice(&b[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4 }
        if buf.len() > 1 << 16 { return Ok(()) }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let len: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
    while buf.len() < head_end + len { let n = c.read(&mut b)?; if n == 0 { break } buf.extend_from_slice(&b[..n]); }
    let body = buf[head_end..].to_vec();
    let target = head.split_whitespace().nth(1).unwrap_or("/");
    let (route, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |k: &str| query.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).map(percent_decode).unwrap_or_default();
    // pairing hands out the token, so it is the one route asked without it (the server checks its own one-time code)
    let (code, out): (u16, Vec<u8>) = if route == "/pair" { (v.more)(route, &param, &body).unwrap_or((404, b"no such request".to_vec())) }
    else if param("token") != token { (403, b"bad token".to_vec()) } else if route == "/whoami" {
        match (v.who)() { Some(l) => (200, l.into_bytes()), None => (404, b"not signed in".to_vec()) }
    } else if let Some(r) = (v.more)(route, &param, &body) { r } else {
        match (route, (v.file)(&param("file"))) {
            ("/state", f) => (200, if f.is_some() { b"unlocked".to_vec() } else { b"locked".to_vec() }),
            (_, None) => (423, b"locked".to_vec()),
            ("/open", Some(f)) => match (v.open)(&f) { Ok(t) => (200, t), Err(None) => (423, b"locked".to_vec()), Err(Some(e)) => (400, e.into_bytes()) },
            ("/save", Some(f)) => match seal_for_vault(&f, &body).and_then(|sealed| { let tmp = f.with_extension("age.saving"); std::fs::write(&tmp, sealed).and_then(|_| std::fs::rename(&tmp, &f)).map_err(|e| e.to_string()) }) {
                Ok(()) => (200, b"saved".to_vec()), Err(e) => (400, e.into_bytes()),
            },
            _ => (404, b"no such request".to_vec()),
        }
    };
    let reason = match code { 200 => "OK", 403 => "Forbidden", 404 => "Not Found", 423 => "Locked", _ => "Bad Request" };
    let kind = if out.first().is_some_and(|b| *b == b'{' || *b == b'[') { "application/json" } else { "text/plain; charset=utf-8" };
    c.write_all(format!("HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).as_bytes())?;
    c.write_all(&out)
}

/// `text` sealed for the group key of the vault `file` is in (its `.keyring.pub`, the nearest above it).
pub fn seal_for_vault(file: &Path, text: &[u8]) -> Result<Vec<u8>, String> {
    let pubkey = file.ancestors().skip(1).map(|d| d.join(".keyring.pub")).find(|p| p.is_file()).ok_or("No .keyring.pub above this note: the vault has no group key.")?;
    let lines: Vec<String> = std::fs::read_to_string(pubkey).map_err(|e| e.to_string())?.lines().map(str::trim).filter(|l| l.starts_with("age1")).map(String::from).collect();
    crate::agefido::encrypt(&crate::agefido::recipients(&lines)?, text)
}

/// A new random token, 32 hex digits.
pub fn token() -> String {
    let mut b = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => { out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?')); i += 3 }
            b'+' => { out.push(b' '); i += 1 }
            c => { out.push(c); i += 1 }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_decode_reads_paths() {
        assert_eq!(super::percent_decode("a%2Fb%20c+d.md.age"), "a/b c d.md.age");
    }

    #[test]
    fn whoami_needs_the_token_and_names_the_login() {
        use std::io::{Read, Write};
        static V: super::Vaults = super::Vaults { file: |_| None, open: |_| Err(None), who: || Some("octo".into()), more: |_, _, _| None };
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(super::serve(port, "t0k", &V));
        let get = |q: &str| { let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap(); write!(c, "GET /whoami?{q} HTTP/1.1\r\n\r\n").unwrap(); let mut s = String::new(); c.read_to_string(&mut s).unwrap(); s };
        let ok = get("token=t0k"); assert!(ok.starts_with("HTTP/1.1 200") && ok.ends_with("\r\n\r\nocto"), "{ok}");
        assert!(get("token=nope").starts_with("HTTP/1.1 403"));
    }
}
