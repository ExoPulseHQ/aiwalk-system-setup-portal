//! The SSH client built into the app, so no computer needs OpenSSH (Windows often has none) and a phone, which has
//! none, reaches the machines the same way. It rides the Cloudflare WebSocket `--ssh-proxy` carries (access.rs) and
//! signs in with the certificate Access signs (key ID = the person's email, so the machine's log names them), else
//! with the person's own key in ~/.ssh. A machine's host key is trusted the first time and must not change after
//! (~/.config/aiwalk/known_hosts), as `StrictHostKeyChecking=accept-new` did.

use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{client, ChannelMsg};
use std::sync::Arc;
use std::time::Duration;

/// What a command left: its exit code (255 when the connection ended without one, as ssh says), stdout and stderr.
pub struct Out { pub code: i32, pub out: Vec<u8>, pub err: Vec<u8> }

fn known_file() -> std::path::PathBuf { crate::home().join(".config/aiwalk/known_hosts") }

/// `host fingerprint` lines. A host not listed yet is added; a listed host with another key is refused.
fn trusted(host: &str, fp: &str) -> bool {
    let f = known_file(); let text = std::fs::read_to_string(&f).unwrap_or_default();
    if let Some(have) = text.lines().find_map(|l| l.strip_prefix(host).and_then(|r| r.strip_prefix(' '))) { return have.trim() == fp }
    let _ = std::fs::create_dir_all(f.parent().unwrap());
    let _ = std::fs::write(&f, format!("{text}{host} {fp}\n"));
    true
}

struct Trust { host: String }
impl client::Handler for Trust {
    type Error = russh::Error;
    async fn check_server_key(&mut self, k: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = match k { PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(keys::HashAlg::Sha256).to_string(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key_fingerprint() };
        Ok(trusted(&self.host, &fp))
    }
}

trait CertFp { fn public_key_fingerprint(&self) -> String; }
impl CertFp for keys::Certificate {
    fn public_key_fingerprint(&self) -> String { keys::PublicKey::from(self.public_key().clone()).fingerprint(keys::HashAlg::Sha256).to_string() }
}

/// The Cloudflare WebSocket to `tunnel` as a byte stream for russh: frames never larger than 16 KB, which the
/// machine's cloudflared reads whole (see access::ssh_proxy).
async fn carrier(tunnel: &str) -> Result<tokio::io::DuplexStream, String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let token = crate::access::token(tunnel)?;
    let mut req = format!("wss://{tunnel}/").into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("Cf-Access-Token", token.parse().map_err(|_| "the kept token is not usable")?);
    let (ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| match e {
        tokio_tungstenite::tungstenite::Error::Http(r) => format!("Cloudflare refused the connection to {tunnel} (HTTP {})", r.status().as_u16()),
        e => format!("Could not reach {tunnel}: {e}"),
    })?;
    let (mut tx, mut rx) = ws.split();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let (mut r, mut w) = tokio::io::split(theirs);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = r.read(&mut buf).await { if n == 0 || tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() { break } }
        let _ = tx.close().await;
    });
    tokio::spawn(async move {
        while let Some(Ok(m)) = rx.next().await {
            match m { Message::Binary(b) => if w.write_all(&b).await.is_err() { break }, Message::Close(_) => break, _ => {} }
        }
        let _ = w.shutdown().await;
    });
    Ok(ours)
}

/// The person's own keys, for a machine that does not take Access's certificates yet; only keys without a passphrase.
fn own_keys() -> Vec<PrivateKey> {
    ["id_ed25519", "id_ecdsa", "id_rsa"].iter().filter_map(|n| keys::load_secret_key(crate::home().join(".ssh").join(n), None).ok()).collect()
}

/// Signed in to `user@tunnel`, or Err with what ssh would have said.
async fn session(user: &str, tunnel: &str) -> Result<client::Handle<Trust>, String> {
    let cfg = Arc::new(client::Config { inactivity_timeout: Some(Duration::from_secs(600)), keepalive_interval: Some(Duration::from_secs(30)), ..Default::default() });
    let stream = carrier(tunnel).await?;
    let mut h = client::connect_stream(cfg, stream, Trust { host: tunnel.to_string() }).await.map_err(|e| match e {
        russh::Error::UnknownKey => format!("{tunnel} has another host key than the one trusted before ({}); ask an owner before going on", known_file().display()),
        e => format!("{tunnel}: {e}"),
    })?;
    if crate::access::ssh_cert_if_stale(tunnel).is_ok() {
        let d = crate::home().join(".cloudflared");
        if let (Ok(k), Ok(c)) = (keys::load_secret_key(d.join(format!("{tunnel}-cf_key")), None), keys::load_openssh_certificate(d.join(format!("{tunnel}-cf_key-cert.pub")))) {
            if h.authenticate_openssh_cert(user, Arc::new(k), c).await.map_err(|e| e.to_string())?.success() { return Ok(h) }
        }
    }
    for k in own_keys() {
        let hash = h.best_supported_rsa_hash().await.map_err(|e| e.to_string())?.flatten();
        if h.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(k), hash)).await.map_err(|e| e.to_string())?.success() { return Ok(h) }
    }
    Err(format!("{user}@{tunnel}: Permission denied (no certificate or key of this computer is accepted)"))
}

/// Runs `command` on the machine (the account's shell reads it, as with ssh), with `stdin` fed to it when given.
pub fn exec(user: &str, tunnel: &str, command: &str, stdin: Option<&[u8]>, timeout: Duration) -> Result<Out, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        tokio::time::timeout(timeout, async {
            let h = session(user, tunnel).await?;
            let mut ch = h.channel_open_session().await.map_err(|e| e.to_string())?;
            ch.exec(true, command).await.map_err(|e| e.to_string())?;
            if let Some(b) = stdin { ch.data(b).await.map_err(|e| e.to_string())?; }
            ch.eof().await.map_err(|e| e.to_string())?;
            let mut o = Out { code: 255, out: vec![], err: vec![] };
            while let Some(m) = ch.wait().await {
                match m {
                    ChannelMsg::Data { data } => o.out.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => o.err.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => o.code = exit_status as i32,
                    _ => {}
                }
            }
            let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
            Ok(o)
        }).await.map_err(|_| format!("{tunnel} did not finish within {} s", timeout.as_secs()))?
    })
}

/// A new key pair for Access to certify, in the files cloudflared's ssh-gen writes (no ssh-keygen needed).
pub fn new_key(key: &std::path::Path, public: &std::path::Path) -> Result<(), String> {
    let k = PrivateKey::random(&mut rand::rng(), keys::Algorithm::Ecdsa { curve: keys::EcdsaCurve::NistP256 }).map_err(|e| e.to_string())?;
    let pem = k.to_openssh(keys::ssh_key::LineEnding::LF).map_err(|e| e.to_string())?;
    crate::access::write_private(key, pem.as_bytes())?;
    std::fs::write(public, format!("{}\n", k.public_key().to_openssh().map_err(|e| e.to_string())?)).map_err(|e| e.to_string())
}

/// A forward from 127.0.0.1:<local> on this computer to a desktop on the machine, open while this is kept.
pub struct Forward { stop: Option<tokio::sync::oneshot::Sender<()>>, alive: Arc<std::sync::atomic::AtomicBool> }
impl Forward {
    pub fn alive(&self) -> bool { self.alive.load(std::sync::atomic::Ordering::Relaxed) }
}
impl Drop for Forward { fn drop(&mut self) { if let Some(s) = self.stop.take() { let _ = s.send(()); } } }

/// Where a forward goes on the machine: a Unix socket (a path) or host:port, as `ssh -L` took it.
pub fn forward(user: &str, tunnel: &str, target: &str, local: u16) -> Result<Forward, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (user, tunnel, target) = (user.to_string(), tunnel.to_string(), target.to_string());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let alive = Arc::new(AtomicBool::new(true));
    let flag = alive.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() { Ok(r) => r, Err(e) => { let _ = ready_tx.send(Err(e.to_string())); return } };
        rt.block_on(async {
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", local)).await { Ok(l) => l, Err(e) => { let _ = ready_tx.send(Err(e.to_string())); return } };
            let h = match session(&user, &tunnel).await { Ok(h) => Arc::new(h), Err(e) => { let _ = ready_tx.send(Err(e)); return } };
            let _ = ready_tx.send(Ok(()));
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    got = listener.accept() => {
                        let Ok((mut sock, _)) = got else { break };
                        if h.is_closed() { break }
                        let (h, target) = (h.clone(), target.clone());
                        tokio::spawn(async move {
                            let ch = if target.starts_with('/') { h.channel_open_direct_streamlocal(target).await }
                                else { let (host, port) = target.rsplit_once(':').unwrap_or(("127.0.0.1", "0")); h.channel_open_direct_tcpip(host, port.parse().unwrap_or(0), "127.0.0.1", 0).await };
                            if let Ok(ch) = ch { let mut s = ch.into_stream(); let _ = tokio::io::copy_bidirectional(&mut sock, &mut s).await; }
                        });
                    }
                }
            }
        });
        flag.store(false, Ordering::Relaxed);
    });
    ready_rx.recv_timeout(Duration::from_secs(25)).map_err(|_| "The machine did not answer in time".to_string())??;
    Ok(Forward { stop: Some(stop), alive })
}

/// This terminal in raw mode while kept (keys go to the machine as typed, as with `ssh -t`), restored when dropped.
struct Raw(#[cfg(unix)] Option<libc::termios>, #[cfg(windows)] Option<(u32, u32)>);
#[cfg(unix)]
impl Raw {
    fn on() -> Raw {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 { return Raw(None) }
            let was = t; libc::cfmakeraw(&mut t); libc::tcsetattr(0, libc::TCSANOW, &t); Raw(Some(was))
        }
    }
    fn size() -> (u32, u32) {
        unsafe { let mut w: libc::winsize = std::mem::zeroed(); if libc::ioctl(1, libc::TIOCGWINSZ, &mut w) == 0 && w.ws_col > 0 { (w.ws_col as u32, w.ws_row as u32) } else { (80, 24) } }
    }
}
#[cfg(unix)]
impl Drop for Raw { fn drop(&mut self) { if let Some(t) = self.0 { unsafe { libc::tcsetattr(0, libc::TCSANOW, &t); } } } }
#[cfg(windows)]
impl Raw {
    fn on() -> Raw {
        use windows_sys::Win32::System::Console::*;
        unsafe {
            let (i, o) = (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE));
            let (mut mi, mut mo) = (0u32, 0u32);
            if GetConsoleMode(i, &mut mi) == 0 || GetConsoleMode(o, &mut mo) == 0 { return Raw(None) }
            SetConsoleMode(i, (mi & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT)) | ENABLE_VIRTUAL_TERMINAL_INPUT);
            SetConsoleMode(o, mo | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            Raw(Some((mi, mo)))
        }
    }
    fn size() -> (u32, u32) {
        use windows_sys::Win32::System::Console::*;
        unsafe {
            let mut b: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut b) == 0 { return (80, 24) }
            ((b.srWindow.Right - b.srWindow.Left + 1) as u32, (b.srWindow.Bottom - b.srWindow.Top + 1) as u32)
        }
    }
}
#[cfg(windows)]
impl Drop for Raw {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Console::*;
        if let Some((mi, mo)) = self.0 { unsafe { SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), mi); SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), mo); } }
    }
}

/// A terminal on the machine in this one: its shell, or `command` with a terminal (`ssh -t`). Follows this window's
/// size. Ok(the exit code).
pub fn shell(user: &str, tunnel: &str, command: Option<&str>) -> Result<i32, String> {
    use std::io::{Read, Write};
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let h = session(user, tunnel).await?;
        let ch = h.channel_open_session().await.map_err(|e| e.to_string())?;
        let (mut size, term) = (Raw::size(), std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()));
        ch.request_pty(false, &term, size.0, size.1, 0, 0, &[]).await.map_err(|e| e.to_string())?;
        match command { Some(c) => ch.exec(true, c).await, None => ch.request_shell(true).await }.map_err(|e| e.to_string())?;
        let (mut rd, wr) = ch.split();
        let raw = Raw::on();
        let (tx, mut keys) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        std::thread::spawn(move || { let mut b = [0u8; 4096]; let mut i = std::io::stdin(); while let Ok(n) = i.read(&mut b) { if n == 0 || tx.send(b[..n].to_vec()).is_err() { break } } });
        let mut tick = tokio::time::interval(Duration::from_millis(300));
        let (mut out, mut err, mut code) = (std::io::stdout(), std::io::stderr(), 255);
        loop {
            tokio::select! {
                k = keys.recv() => match k { Some(b) => { let _ = wr.data(&b[..]).await; } None => { let _ = wr.eof().await; } },
                _ = tick.tick() => { let now = Raw::size(); if now != size { size = now; let _ = wr.window_change(size.0, size.1, 0, 0).await; } },
                m = rd.wait() => match m {
                    Some(ChannelMsg::Data { data }) => { let _ = out.write_all(&data); let _ = out.flush(); }
                    Some(ChannelMsg::ExtendedData { data, .. }) => { let _ = err.write_all(&data); let _ = err.flush(); }
                    Some(ChannelMsg::ExitStatus { exit_status }) => code = exit_status as i32,
                    Some(_) => {}
                    None => break,
                },
            }
        }
        drop(raw);
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
        Ok(code)
    })
}
