//! Lab machines through Cloudflare: members never reach a machine's address directly
//! (Machine_Login_Identity_Summary §二). Access lets in members of the GitHub org; the tunnel carries SSH.

use crate::{home, on_path, sh};
use exo_core::{ssh_block, with_ssh_block, Machine};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn cloudflared() -> Option<String> { on_path("cloudflared").map(|p| p.to_string_lossy().into_owned()) }

/// "no-tunnel" (none set up yet), "no-cloudflared", "sign-in" (no Cloudflare sign-in on this computer yet),
/// "up" (the machine's SSH answered through the tunnel), "down" (it did not).
fn state(tunnel: Option<&str>) -> &'static str {
    let Some(t) = tunnel else { return "no-tunnel" };
    let Some(cf) = cloudflared() else { return "no-cloudflared" };
    // only probe with a sign-in already here: otherwise cloudflared would open a browser on its own
    if sh(&cf, &["access", "token", &format!("-app=https://{t}")], 10).0 != 0 { return "sign-in" }
    let proxy = format!("ProxyCommand=\"{cf}\" access ssh --hostname %h");
    let (_, out) = sh("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", &proxy, &format!("probe@{t}"), "true"], 20);
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
/// Cloudflare sign-in. Machines without one, or not answering, are left out. All at once, 10 seconds each.
#[tauri::command(async)]
pub fn machine_status(tunnels: Vec<(String, String)>) -> BTreeMap<String, serde_json::Value> {
    let Some(cf) = cloudflared() else { return BTreeMap::new() };
    let jobs: Vec<_> = tunnels.into_iter().map(|(host, tunnel)| {
        let cf = cf.clone();
        std::thread::spawn(move || {
            let url = format!("https://{}/", tunnel.replacen("ssh-", "status-", 1));
            // without a sign-in cloudflared would open a browser; the page asks for the sign-in instead
            if sh(&cf, &["access", "token", &format!("-app={url}")], 10).0 != 0 { return None }
            let (code, out) = sh(&cf, &["access", "curl", &url, "-s", "--max-time", "8"], 12);
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
    Ok("Signed in to Cloudflare".into())
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
