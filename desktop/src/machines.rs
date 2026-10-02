//! Lab machines through Cloudflare: members never reach a machine's address directly
//! (Machine_Login_Identity_Summary §二). Access lets in members of the GitHub org; the tunnel carries SSH.

use crate::{home, on_path, sh};
use exo_core::{ssh_block, with_ssh_block, Machine};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn cloudflared() -> Option<String> { on_path("cloudflared").map(|p| p.to_string_lossy().into_owned()) }

/// "no-tunnel" (none set up yet), "no-cloudflared", "sign-in" (no Cloudflare sign-in on this computer yet),
/// "up" (the machine's SSH answered through the tunnel), "down" (it did not).
fn state(tunnel: Option<&str>) -> &'static str {
    let Some(t) = tunnel else { return "no-tunnel" };
    let Some(cf) = cloudflared() else { return "no-cloudflared" };
    // only probe with a sign-in already here: otherwise cloudflared would open a browser on its own
    if sh(&cf, &["access", "token", &format!("-app=https://{t}")], 10).0 != 0 { return "sign-in" }
    let proxy = format!("ProxyCommand=\"{cf}\" access ssh --hostname %h");
    let (_, out) = sh("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", &proxy, &format!("probe@{t}"), "true"], 30);
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
    let Some(cf) = cloudflared() else { return BTreeMap::new() };
    let jobs: Vec<_> = tunnels.into_iter().map(|(host, tunnel)| {
        let cf = cf.clone();
        std::thread::spawn(move || {
            let url = format!("https://{}/", tunnel.replacen("ssh-", "status-", 1));
            // without a sign-in cloudflared would open a browser; the page asks for the sign-in instead
            if sh(&cf, &["access", "token", &format!("-app={url}")], 10).0 != 0 { return None }
            let (code, out) = sh(&cf, &["access", "curl", &url, "-s", "--max-time", "15"], 20);
            (code == 0).then(|| serde_json::from_str::<serde_json::Value>(&out).ok()).flatten().map(|v| (host, v))
        })
    }).collect();
    jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
}

/// Signs this computer in to Cloudflare Access for `tunnel`: a browser opens for the GitHub sign-in.
#[tauri::command(async)]
pub fn access_login(tunnel: String) -> Result<String, String> {
    let cf = cloudflared().ok_or("cloudflared is missing; reinstall the app")?;
    let (code, out) = sh(&cf, &["access", "login", &format!("https://{tunnel}")], 300);
    if code != 0 { return Err(out.lines().last().unwrap_or("Sign-in was not finished").into()) }
    // the status hostname sits in the same Access app; the browser already holds the session, so this one passes at once
    let _ = sh(&cf, &["access", "login", &format!("https://{}", tunnel.replacen("ssh-", "status-", 1))], 60);
    Ok("Signed in: the lab machines know you are on the team".into())
}

fn ssh_config() -> PathBuf { home().join(".ssh/config") }

/// Whether ~/.ssh/config already holds exactly the aliases the app would write.
#[tauri::command(async)]
pub fn ssh_status(machines: Vec<Machine>) -> &'static str {
    let Some(cf) = cloudflared() else { return "no-cloudflared" };
    let block = ssh_block(&machines, &cf);
    if !block.contains("Host ") { return "nothing" }   // no tunnel exists yet
    if std::fs::read_to_string(ssh_config()).unwrap_or_default().contains(&block) { "current" } else { "missing" }
}

/// Writes (or refreshes) the app's block in ~/.ssh/config, keeping everything else; the old file is kept as config.bak.
#[tauri::command(async)]
pub fn ssh_setup(machines: Vec<Machine>) -> Result<String, String> {
    let cf = cloudflared().ok_or("cloudflared is missing; reinstall the app")?;
    let file = ssh_config();
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    if !old.is_empty() { std::fs::write(file.with_extension("bak"), &old).map_err(|e| e.to_string())?; }
    std::fs::write(&file, with_ssh_block(&old, &ssh_block(&machines, &cf))).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)); }
    Ok("Connections set up: ssh <account>@<machine> now goes through Cloudflare".into())
}

// ---------------------------------------------------------------- desktops through the tunnel
// A VNC desktop on a lab machine listens on that machine's 127.0.0.1 only. The app forwards a local port to it over
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
    let cf = cloudflared().ok_or("cloudflared is missing; reinstall the app")?;
    let local = free_port(15900 + display as u16);
    let mut child = Command::new("ssh")
        .args(["-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30",
               "-o", "StrictHostKeyChecking=accept-new", "-o", &format!("ProxyCommand=\"{cf}\" access ssh --hostname %h"),
               "-L", &format!("127.0.0.1:{local}:{target}"), &format!("{user}@{tunnel}")])
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

/// Hands vnc://127.0.0.1:<port> to whatever VNC viewer this computer has (Screen Sharing on a Mac).
#[tauri::command]
pub fn open_viewer(port: u16) { crate::open_url(&format!("vnc://127.0.0.1:{port}")); }

/// Starts or stops a desktop with hosts/exo-desktop on the machine, over the same SSH through Cloudflare.
/// action is "start" (display 0 = the lowest free one) or "stop"; returns what exo-desktop printed.
#[tauri::command(async)]
pub fn desktop(tunnel: String, user: String, action: String, display: u8) -> Result<String, String> {
    if action != "start" && action != "stop" { return Err(format!("unknown action {action}")) }
    let cf = cloudflared().ok_or("cloudflared is missing; reinstall the app")?;
    let arg = if display == 0 { String::new() } else { display.to_string() };
    let (code, out) = sh("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=accept-new",
        "-o", &format!("ProxyCommand=\"{cf}\" access ssh --hostname %h"), &format!("{user}@{tunnel}"),
        &format!("~/.local/bin/exo-desktop {action} {arg}")], 60);
    let last = out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or_default().to_string();
    if code == 0 { Ok(last) } else if last.contains("Permission denied") {
        Err(format!("{user} on this machine did not accept this computer's key"))
    } else { Err(if last.is_empty() { "The machine did not answer".into() } else { last }) }
}
