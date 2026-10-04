//! Machines through Cloudflare: members never reach a machine's address directly
//! (Machine_Login_Identity_Summary §二). Access lets in members of the GitHub org; the tunnel carries SSH.

use crate::{home, sh};
use exo_core::{ssh_block, with_ssh_block, Machine};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// This program as ssh sees it in a ProxyCommand. None when its path cannot be written there.
fn app() -> Option<String> {
    crate::own_appimage().or_else(|| std::env::current_exe().ok()).map(|e| e.to_string_lossy().into_owned()).filter(|e| !e.contains('"'))
}

/// ssh options that reach `tunnel` through Cloudflare, carried by this app itself (`--ssh-proxy`, access.rs).
/// Where the machine's Access application signs SSH certificates, a fresh one is fetched (they last minutes) and
/// offered first, so the machine's log names the person; where it does not, or the machine does not trust them yet,
/// ssh goes on to the person's own keys.
fn through(tunnel: &str) -> Option<Vec<String>> {
    let mut o = vec!["-o".to_string(), format!("ProxyCommand=\"{}\" --ssh-proxy %h", app()?.replace('%', "%%"))];   // % is ssh's token marker
    let key = home().join(".cloudflared").join(format!("{tunnel}-cf_key"));
    let cert = std::path::PathBuf::from(format!("{}-cert.pub", key.display()));
    if crate::access::ssh_cert_if_stale(tunnel).is_ok() {
        o.extend(["-o".into(), format!("IdentityFile={}", key.display()), "-o".into(), format!("CertificateFile={}", cert.display())]);
    }
    Some(o)
}

/// "no-tunnel" (none set up yet), "sign-in" (no Cloudflare sign-in on this computer yet),
/// "up" (the machine's SSH answered through the tunnel), "down" (it did not).
fn state(tunnel: Option<&str>) -> &'static str {
    let Some(t) = tunnel else { return "no-tunnel" };
    if crate::access::token(t).is_err() { return "sign-in" }
    let Some(app) = app() else { return "down" };
    let (_, out) = sh("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", &format!("ProxyCommand=\"{}\" --ssh-proxy %h", app.replace('%', "%%")), &format!("probe@{t}"), "true"], 30);
    // sshd asking who we are means the whole path works; it refuses "probe", which is fine
    if out.contains("Permission denied") || out.contains("Too many authentication failures") { "up" } else { "down" }
}

/// The state of each machine, checked all at once; a machine reached through another (`via`) shares its state.
#[tauri::command(async)]
pub fn reachable(machines: Vec<(String, Option<String>)>) -> BTreeMap<String, &'static str> {
    let jobs: Vec<_> = machines.into_iter()
        .map(|(host, tunnel)| std::thread::spawn(move || (host, state(tunnel.as_deref())))).collect();
    jobs.into_iter().filter_map(|j| j.join().ok()).collect()
}

/// Each machine's status (what it has, how busy it is) from its status-<name> hostname, read with this computer's
/// Cloudflare sign-in. Machines without one, or not answering, are left out. All at once; a first connection through
/// Cloudflare can take several seconds, so each gets 15.
#[tauri::command(async)]
pub fn machine_status(tunnels: Vec<(String, String)>) -> BTreeMap<String, serde_json::Value> {
    let jobs: Vec<_> = tunnels.into_iter().map(|(host, tunnel)| {
        std::thread::spawn(move || {
            let url = format!("https://{}/", tunnel.replacen("ssh-", "status-", 1));
            let mut v = crate::access::get(&url).ok().and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok()).filter(|v| v.is_object())?;
            v["host_tools"] = exo_core::host_tools_line(&v["tools"], &shipped()).into();   // the one line the opened row shows
            Some((host, v))
        })
    }).collect();
    jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
}

/// Signs this computer in to Cloudflare Access for `tunnel`: a browser opens for the GitHub sign-in.
#[tauri::command(async)]
pub fn access_login(app: tauri::AppHandle, tunnels: Vec<String>) -> Result<String, String> {
    use tauri::Emitter;
    let name = |t: &str| t.split('.').next().unwrap_or(t).trim_start_matches("ssh-").to_string();
    // "lab-progress": (machines done, machines in all, the one being signed in to now)
    let tell = |done: usize, now: &str| { let _ = app.emit("lab-progress", (done, tunnels.len(), now)); };
    // each machine is its own Access app with its own token. The first sign-in may open the browser; after it the
    // team sign-in it leaves here is traded for the others' tokens without one. Only machines the person may reach
    // are passed in: for any other, the browser would open on a refusal and this would wait.
    let mut failed = vec![];
    for (i, tunnel) in tunnels.iter().enumerate() {
        tell(i, &name(tunnel));
        if crate::access::sign_in(tunnel).is_err() { failed.push(name(tunnel)); continue }
        // the status page is usually the same application; where it is its own, it gets its token too
        let _ = crate::access::sign_in(&tunnel.replacen("ssh-", "status-", 1));
    }
    tell(tunnels.len(), "");
    if failed.is_empty() { Ok("Signed in: the machines know you are on the team".into()) }
    else { Err(format!("Sign-in was not finished for {}", failed.join(", "))) }
}

fn ssh_config() -> PathBuf { home().join(".ssh/config") }

/// Whether ~/.ssh/config already holds exactly the aliases the app would write.
#[tauri::command(async)]
pub fn ssh_status(machines: Vec<Machine>) -> &'static str {
    let Some(app) = app() else { return "nothing" };
    let block = ssh_block(&machines, &app);
    if !block.contains("Host ") { return "nothing" }   // no tunnel exists yet
    if std::fs::read_to_string(ssh_config()).unwrap_or_default().contains(&block) { "current" } else { "missing" }
}

/// Writes (or refreshes) the app's block in ~/.ssh/config, keeping everything else; the old file is kept as config.bak.
#[tauri::command(async)]
pub fn ssh_setup(machines: Vec<Machine>) -> Result<String, String> {
    let app = app().ok_or("This app's own path cannot be written into an ssh config")?;
    let file = ssh_config();
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    if !old.is_empty() { std::fs::write(file.with_extension("bak"), &old).map_err(|e| e.to_string())?; }
    std::fs::write(&file, with_ssh_block(&old, &ssh_block(&machines, &app))).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)); }
    Ok("Connections set up: ssh <account>@<machine> now goes through Cloudflare".into())
}

// ---------------------------------------------------------------- desktops through the tunnel
// A VNC desktop on a machine listens on that machine's 127.0.0.1 only. The app forwards a local port to it over
// SSH through Cloudflare, so the person's own VNC viewer connects to 127.0.0.1:<local port> and nothing else is open.

/// "host:display" -> (ssh process, local port). Closed when the person stops it or the app quits.
static FORWARDS: Mutex<BTreeMap<String, (Child, u16)>> = Mutex::new(BTreeMap::new());

fn free_port(preferred: u16) -> u16 {
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() { return preferred }
    TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(preferred)
}

/// Opens (or reuses) a forward from 127.0.0.1:<local> to a desktop on the machine: its Unix socket when it has one
/// (no port, no VNC password), else its TCP port on the machine's 127.0.0.1. Returns the local port.
/// Err when SSH could not get in, with its last line, so the page can say why.
#[tauri::command(async)]
pub fn open_forward(host: String, tunnel: String, user: String, display: u8, socket: Option<String>, port: Option<u16>) -> Result<u16, String> {
    let key = format!("{host}:{display}");
    let target = match (&socket, port) {
        (Some(path), _) => path.clone(),
        (None, Some(p)) => format!("127.0.0.1:{p}"),
        (None, None) => return Err("This desktop has neither a socket nor a port".into()),
    };
    {
        let mut f = FORWARDS.lock().unwrap();
        if let Some((child, port)) = f.get_mut(&key) {
            if child.try_wait().ok().flatten().is_none() { return Ok(*port) }
            f.remove(&key);
        }
    }
    let via = through(&tunnel).ok_or("This app's own path cannot be used by ssh")?;
    let local = free_port(15900 + display as u16);
    let mut child = crate::cmd("ssh")
        .args(["-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30",
               "-o", "StrictHostKeyChecking=accept-new"]).args(via)
        .args(["-L", &format!("127.0.0.1:{local}:{target}"), &format!("{user}@{tunnel}")])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let addr: SocketAddr = ([127, 0, 0, 1], local).into();
    let start = Instant::now();
    // the forward is ready once the local port answers; ssh exiting first means it could not get in
    while start.elapsed() < Duration::from_secs(25) {
        if let Ok(Some(_)) = child.try_wait() {
            let err = child.wait_with_output().map(|o| String::from_utf8_lossy(&o.stderr).into_owned()).unwrap_or_default();
            let last = err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("SSH ended").to_string();
            return Err(if last.contains("Permission denied") {
                format!("{user}@{host} did not accept this computer's key. Until SSH certificates arrive, ask an owner to add your key.")
            } else { last });
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
            FORWARDS.lock().unwrap().insert(key, (child, local));
            return Ok(local);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let _ = child.kill();
    Err("The machine did not answer in time".into())
}

#[tauri::command]
pub fn close_forward(host: String, display: u8) {
    if let Some((mut child, _)) = FORWARDS.lock().unwrap().remove(&format!("{host}:{display}")) { let _ = child.kill(); }
}

/// Open forwards as "host:display" -> local port, for the page to show after a redraw.
#[tauri::command]
pub fn forwards() -> BTreeMap<String, u16> {
    let mut f = FORWARDS.lock().unwrap();
    f.retain(|_, (child, _)| child.try_wait().ok().flatten().is_none());
    f.iter().map(|(k, (_, p))| (k.clone(), *p)).collect()
}

/// Closes every forward; the app calls it when it quits so no ssh is left running.
pub fn close_all() {
    for (_, (mut child, _)) in std::mem::take(&mut *FORWARDS.lock().unwrap()) { let _ = child.kill(); }
}

/// Hands the forwarded desktop to a viewer this computer has. VNC: vnc://127.0.0.1:<port> (Screen Sharing on a
/// Mac). RDP: Windows' own Remote Desktop; elsewhere whatever opens rdp:// (Windows App on a Mac, Remmina on Linux).
#[tauri::command]
pub fn open_viewer(port: u16, kind: Option<String>) {
    if kind.as_deref() != Some("rdp") { return crate::open_url(&format!("vnc://127.0.0.1:{port}")) }
    if cfg!(windows) { let _ = crate::cmd("mstsc").arg(format!("/v:127.0.0.1:{port}")).spawn(); }
    else if cfg!(target_os = "macos") { crate::open_url(&format!("rdp://full%20address=s:127.0.0.1:{port}")) }
    else { crate::open_url(&format!("rdp://127.0.0.1:{port}")) }
}

/// Starts or stops a desktop with hosts/exo-desktop on the machine, over the same SSH through Cloudflare.
/// action is "start" (display 0 = the lowest free one) or "stop"; returns what exo-desktop printed.
#[tauri::command(async)]
pub fn desktop(tunnel: String, user: String, action: String, display: u8) -> Result<String, String> {
    if action != "start" && action != "stop" { return Err(format!("unknown action {action}")) }
    let via = through(&tunnel).ok_or("This app's own path cannot be used by ssh")?;
    let arg = if display == 0 { String::new() } else { display.to_string() };
    let (target, run) = (format!("{user}@{tunnel}"), format!("~/.local/bin/exo-desktop {action} {arg}"));
    let mut args = vec!["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=accept-new"];
    args.extend(via.iter().map(String::as_str));
    args.extend([target.as_str(), run.as_str()]);
    let (code, out) = sh("ssh", &args, 60);
    let last = out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or_default().to_string();
    if code == 0 { Ok(last) } else if last.contains("Permission denied") {
        Err(format!("{user} on this machine did not accept this computer's key"))
    } else { Err(if last.is_empty() { "The machine did not answer".into() } else { last }) }
}

// ---------------------------------------------------------------- host tools
// exo, exo-status.py and exo-desktop live in the login account's ~/.local/bin on each machine. The app carries the
// copies from hosts/ it was built with, so the Machines page can tell a machine still running an older one.

const HOST_TOOLS: [(&str, &str); 3] = [("exo-status.py", include_str!("../../hosts/exo-status.py")),
    ("exo", include_str!("../../hosts/exo")), ("exo-desktop", include_str!("../../hosts/exo-desktop"))];
const END: &str = "EXO_HOST_TOOL_END";   // heredoc delimiter; a test checks no tool contains it

fn shipped() -> Vec<(&'static str, u32)> { HOST_TOOLS.iter().map(|&(n, t)| (n, exo_core::host_tool_version(t).unwrap_or(0))).collect() }

/// Restarts the status page with the command the account's crontab runs at boot (else the one running now), after
/// stopping the process that listens on 9101, found by that listener's pid. Never `pkill -f exo-status`: the pattern
/// would also match this ssh session's own command line on some machines and end it.
const RESTART: &str = r#"
listener() { ss -ltnpH | grep '127.0.0.1:9101 ' || true; }
pid=$(listener | grep -o 'pid=[0-9]*' | head -n1 | cut -d= -f2)
if [ -z "$pid" ] && [ -n "$(listener)" ]; then echo "Port 9101 belongs to another account; the status page was not restarted" >&2; exit 1; fi
run=$(crontab -l 2>/dev/null | sed -n 's/^@reboot[[:space:]]*\(.*exo-status\.py.*\)$/\1/p' | head -n1)
if [ -z "$run" ] && [ -n "$pid" ]; then run=$(ps -o args= -p "$pid"); fi
if [ -z "$run" ]; then run="/usr/bin/python3 $HOME/.local/bin/exo-status.py"; fi
if [ -n "$pid" ]; then kill "$pid"; for i in $(seq 40); do [ -z "$(listener)" ] && break; sleep 0.25; done; fi
setsid nohup sh -c "$run" </dev/null >/dev/null 2>&1 &
for i in $(seq 40); do [ -n "$(listener)" ] && break; sleep 0.25; done
[ -n "$(listener)" ] || { echo "The status page did not start again: $run" >&2; exit 1; }
"#;

/// Copies the host tools this app carries to `user`'s ~/.local/bin on the machine over SSH through Cloudflare, then
/// restarts the status page so it reports the new versions. Each file is written beside the old one and moved over it,
/// so a running exo never reads half a file and ~/bin/exo, a symlink to ~/.local/bin/exo, keeps working. No sudo.
#[tauri::command(async)]
pub fn update_host_tools(tunnel: String, user: String) -> Result<String, String> {
    let mut script = String::from("set -eu\nmkdir -p ~/.local/bin && cd ~/.local/bin\n");
    for (name, text) in HOST_TOOLS {
        script += &format!("cat > {name}.new <<'{END}'\n{text}{}{END}\nchmod 755 {name}.new && mv -f {name}.new {name}\n",
                           if text.ends_with('\n') { "" } else { "\n" });
    }
    script += RESTART;
    let via = through(&tunnel).ok_or("This app's own path cannot be used by ssh")?;
    let target = format!("{user}@{tunnel}");
    let mut args = vec!["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=accept-new"];
    args.extend(via.iter().map(String::as_str));
    args.extend([target.as_str(), "bash -s"]);
    let (code, out) = crate::sh_stdin("ssh", &args, Some(&script), 90);
    let last = out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or_default().to_string();
    if code == 0 { Ok("Host tools updated and the status page restarted".into()) } else if last.contains("Permission denied") {
        Err(format!("{user} on this machine did not accept this computer's key"))
    } else { Err(if last.is_empty() { "The machine did not answer".into() } else { last }) }
}

/// Forgets this computer's Cloudflare Access sign-in for the team (every lab hostname and the team session), and
/// closes desktop forwards. Called when the GitHub account changes, so the next connection signs in as the new one
/// instead of riding the old person's sign-in for up to a day. Other Cloudflare files are left alone.
pub fn forget_access() {
    close_all();
    let dir = home().join(".cloudflared");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let ours = name.contains(".aiwalkcorp.com-") || name.starts_with("aiwalkcorp.cloudflareaccess.com-org-token");
        if ours && name.contains("token") { let _ = std::fs::remove_file(e.path()); }
    }
}

/// Who this computer is signed in to the machines as: the email and expiry inside one of its Cloudflare Access
/// tokens, and how many of `tunnels` still have none. None when there is no sign-in at all. Shown next to the
/// GitHub account so the two can be seen to match.
#[tauri::command(async)]
pub fn lab_identity(tunnels: Vec<String>) -> Option<serde_json::Value> {
    let tokens: Vec<Option<String>> = tunnels.iter().map(|t| crate::access::token(t).ok()).collect();
    let missing = tokens.iter().filter(|t| t.is_none()).count();
    let jwt = tokens.into_iter().flatten().next()?;
    let payload = jwt.trim().split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    let email = claims["email"].as_str().unwrap_or_default().to_lowercase();
    // the GitHub account's own emails; reading them needs gh's user:email scope, so "matches" stays null without it
    let mine: Option<Vec<String>> = crate::github::all("user/emails").ok()
        .map(|es| es.iter().filter_map(|e| e["email"].as_str().map(String::from)).collect())
        .or_else(|| crate::github::get("user").ok().and_then(|u| u["email"].as_str().map(|e| vec![e.to_string()])));
    let matches = mine.map(|m| m.iter().any(|l| l.eq_ignore_ascii_case(&email)));
    Some(serde_json::json!({ "email": email, "expires": claims["exp"], "matches": matches, "missing": missing }))
}

/// Step 2 undone on its own: used when the lab sign-in belongs to someone other than the GitHub account here.
#[tauri::command]
pub fn lab_sign_out() { forget_access() }

/// base64url without padding, as JWTs use it.
fn base64url(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c { b'A'..=b'Z' => Some(c - b'A'), b'a'..=b'z' => Some(c - b'a' + 26), b'0'..=b'9' => Some(c - b'0' + 52),
                                b'-' => Some(62), b'_' => Some(63), _ => None };
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        buf = (buf << 6) | val(c)? as u32; bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn forgetting_access_removes_only_this_teams_sign_ins() {
        let home = std::env::temp_dir().join(format!("aiwalk-forget-{}", std::process::id()));
        let dir = home.join(".cloudflared");
        std::fs::create_dir_all(&dir).unwrap();
        let ours = ["ssh-dragon.aiwalkcorp.com-cf70-token", "status-dragon.aiwalkcorp.com-cf70-token",
                    "aiwalkcorp.cloudflareaccess.com-org-token"];
        let keep = ["aiwalkcorp.cloudflareaccess.com-jwks", "other.example.com-ab12-token", "cert.pem"];
        for f in ours.iter().chain(&keep) { std::fs::write(dir.join(f), "x").unwrap(); }
        std::env::set_var("HOME", &home);
        super::forget_access();
        for f in ours { assert!(!dir.join(f).exists(), "{f} should be gone") }
        for f in keep { assert!(dir.join(f).exists(), "{f} should stay") }
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn shipped_host_tools_have_versions_and_survive_the_heredoc() {
        for (name, v) in super::shipped() { assert!(v > 0, "{name} has no VERSION line") }
        for (name, text) in super::HOST_TOOLS { assert!(!text.lines().any(|l| l == super::END), "{name} contains the heredoc delimiter") }
    }

    #[test]
    fn jwt_payloads_decode() {
        // {"email":"a@b.c","exp":1}
        assert_eq!(super::base64url("eyJlbWFpbCI6ImFAYi5jIiwiZXhwIjoxfQ").unwrap(), br#"{"email":"a@b.c","exp":1}"#);
        assert!(super::base64url("bad*").is_none());
    }
}
