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
/// "up" (the machine's SSH answered through the tunnel), "refused" (Access would not let this sign-in through),
/// "down" (it did not answer).
fn state(tunnel: Option<&str>) -> &'static str {
    let Some(t) = tunnel else { return "no-tunnel" };
    if crate::access::token(t).is_err() { return "sign-in" }
    let Some(app) = app() else { return "down" };
    let (_, out) = sh("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", &format!("ProxyCommand=\"{}\" --ssh-proxy %h", app.replace('%', "%%")), &format!("probe@{t}"), "true"], 30);
    // sshd asking who we are means the whole path works; it refuses "probe", which is fine. Access turning the
    // kept sign-in away (the person was never let in, or was taken out) is not the machine being down
    if out.contains("Permission denied") || out.contains("Too many authentication failures") { "up" }
    else if out.contains("Cloudflare refused the connection") { "refused" } else { "down" }
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

/// A guest's machine by the name an owner gave them: its SSH hostname, once Cloudflare confirms the team's Access
/// guards it. Err with a sentence for the page when the name is not one, or there is no such machine.
#[tauri::command(async)]
pub fn find_machine(name: String) -> Result<String, String> {
    let tunnel = exo_core::machine_tunnel(&name, crate::access::ZONE)?;
    if crate::access::is_team_app(&tunnel)? { Ok(tunnel) } else { Err(format!("There is no machine called {}", name.trim())) }
}

// The machines' names, where a guest can read them: a guest reads no vault, so the list an owner's app keeps in the
// requests repo (the one repo every guest is invited to, for the terms) is how their app learns which machines
// exist. Names only; which of them let a person in is Cloudflare's to say.
const LIST_REPO: &str = "ExoPulseHQ/exo-access-requests";
const LIST: &str = "machines.json";

/// A guest's machines without typing their names: every listed machine that lets this sign-in through, asked without
/// the browser, as (name, tunnel). Empty before the Cloudflare sign-in, and when the list cannot be read.
#[tauri::command(async)]
pub fn my_machines() -> Vec<(String, String)> {
    let Ok(text) = crate::github::raw(LIST_REPO, LIST) else { return vec![] };
    let names: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    // the file is only a list of names: each becomes ssh-<name> in the team's zone or is dropped
    names.into_iter().filter_map(|n| exo_core::machine_tunnel(&n, crate::access::ZONE).ok().map(|t| (n, t)))
        .filter(|(_, t)| crate::access::sign_in_quiet(t)).collect()
}

/// Owners: keeps that list equal to `names`. Ok(true) when it was written, Ok(false) when it was right already.
#[tauri::command(async)]
pub fn publish_machines(mut names: Vec<String>) -> Result<bool, String> {
    use base64::Engine;
    names.sort(); names.dedup();
    let want = serde_json::to_string(&names).map_err(|e| e.to_string())?;
    let path = format!("repos/{LIST_REPO}/contents/{LIST}");
    let now = crate::github::get(&path).ok();
    if crate::github::raw(LIST_REPO, LIST).is_ok_and(|t| t.trim() == want) { return Ok(false) }
    let mut body = serde_json::json!({ "message": "machines: the names a guest's app asks about",
                                       "content": base64::engine::general_purpose::STANDARD.encode(&want) });
    if let Some(sha) = now.as_ref().and_then(|v| v["sha"].as_str()) { body["sha"] = sha.into() }
    crate::github::send("PUT", &path, Some(body)).map(|_| true)
}

/// Signs this computer in to Cloudflare Access for `tunnel`: a browser opens for the GitHub sign-in.
#[tauri::command(async)]
pub fn access_login(app: tauri::AppHandle, tunnels: Vec<String>, quiet: Option<bool>) -> Result<String, String> {
    use tauri::Emitter;
    let name = |t: &str| t.split('.').next().unwrap_or(t).trim_start_matches("ssh-").to_string();
    // "lab-progress": (machines done, machines in all, the one being signed in to now)
    let tell = |done: usize, now: &str| { let _ = app.emit("lab-progress", (done, tunnels.len(), now)); };
    // each machine is its own Access app with its own token. The first sign-in may open the browser; after it the
    // team sign-in it leaves here is traded for the others' tokens without one. Only machines the person may reach
    // are passed in: for any other, the browser would open on a refusal and this would wait.
    let mut failed = vec![];
    // first of all, and for someone no machine lets in yet the only step: Cloudflare gets to know the person
    let enrolled = crate::access::sign_in(crate::access::ENROLL);
    if tunnels.is_empty() { return enrolled.map(|_| "Signed in: an owner can now let you in to a machine".into()) }
    // someone who is not a member: which machines let them in is known to Cloudflare alone, so each is asked without
    // the browser, and one that says no is simply not theirs
    if quiet == Some(true) {
        enrolled?;
        let n = tunnels.iter().enumerate().filter(|(i, t)| {
            tell(*i, &name(t));
            let ok = crate::access::sign_in_quiet(t);
            if ok { crate::access::sign_in_quiet(&t.replacen("ssh-", "status-", 1)); }
            ok
        }).count();
        tell(tunnels.len(), "");
        return Ok(if n == 0 { "Signed in. No machine lets you in yet: ask an owner".into() } else { format!("Signed in: {n} of the machines let you in") });
    }
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

pub(crate) fn ssh_config() -> PathBuf { home().join(".ssh/config") }

/// Whether ~/.ssh/config already holds exactly the aliases the app would write.
#[tauri::command(async)]
pub fn ssh_status(machines: Vec<Machine>) -> &'static str {
    let Some(app) = app() else { return "nothing" };
    let block = ssh_block(&machines, &app);
    // no tunnel exists yet. Every entry has a HostName line; a machine that signs certificates is written as a
    // `Match originalhost` block, with no `Host ` line at all, so looking for "Host " said "nothing" once every
    // machine had certificates, and the page showed neither the button nor that connections were set up
    if !block.contains("HostName ") { return "nothing" }
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

/// `aiwalk-setup vault ssh-setup [--check]`, run in a vault's folder: "Set up connections" from the command line, with
/// the machines of that vault's System/vault_rules.json. --check prints what ssh_status says and writes nothing; a
/// block already current is not written again, so config.bak keeps the file from before the first set-up.
pub fn setup_cli(vault: &std::path::Path, check: bool) -> i32 {
    let rules = vault.join("System/vault_rules.json");
    let Ok(text) = std::fs::read_to_string(&rules) else { eprintln!("✗ {} not found: run this in a vault's folder", rules.display()); return 1 };
    let machines = exo_core::machines(&text);
    let status = ssh_status(exo_core::machines(&text));
    if check { println!("{status}"); return 0 }
    let file = ssh_config();
    match status {
        "nothing" => { println!("nothing: no machine in {} has a tunnel yet, so there is nothing to write", rules.display()); return 0 }
        "current" => { println!("current: {} already holds the app's block", file.display()); return 0 }
        _ => {}
    }
    println!("Writing the app's block into {}", file.display());
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > 0) { println!("The old file is kept as {}", file.with_extension("bak").display()) }
    match ssh_setup(machines) { Ok(m) => { println!("{m}"); 0 } Err(e) => { eprintln!("✗ {e}"); 1 } }
}

// ---------------------------------------------------------------- ssh without ~/.ssh/config
// `aiwalk-setup ssh` and the Machines page's SSH button. Both use the app's own proxy and a fresh certificate, the
// way the app's own ssh calls do (through), so neither needs the block "Set up connections" writes.

/// `aiwalk-setup ssh [ssh options] [account@]<machine> [command…]`. With no account: the one this folder's vault rules
/// name for the machine, else ntk. Signs in first when this computer has no usable sign-in for the machine (the
/// team sign-in traded silently, else the browser, which access::login announces on stderr before opening it).
/// Unix: becomes ssh, so signals and the exit code are ssh's. Windows: waits for ssh and returns its exit code.
pub fn ssh_main(args: &[String]) -> i32 {
    let fail = |e: String| { eprintln!("aiwalk-setup ssh: {e}"); 2 };
    let (opts, dest, command) = match exo_core::ssh_args(args) { Ok(x) => x, Err(e) => return fail(e) };
    let (user, tunnel) = match exo_core::ssh_destination(&dest, crate::access::ZONE) { Ok(x) => x, Err(e) => return fail(e) };
    let user = user.unwrap_or_else(|| exo_core::default_account(std::fs::read_to_string("System/vault_rules.json").ok().as_deref(), &tunnel));
    if crate::access::token(&tunnel).is_err() {
        // a name with no machine behind it is said plainly, not as a failed sign-in
        match crate::access::is_team_app(&tunnel) {
            Ok(true) => {}
            Ok(false) => return fail(format!("there is no machine called {}", dest.rsplit('@').next().unwrap_or(&dest))),
            Err(e) => { eprintln!("aiwalk-setup ssh: {e}"); return 1 }
        }
        eprintln!("This computer has no Cloudflare sign-in for {tunnel} yet; signing in.");
        if let Err(e) = crate::access::sign_in(&tunnel) { eprintln!("aiwalk-setup ssh: {e}"); return 1 }
    }
    // the certificate is the only credential the machines take; without one ssh would only say "Permission denied"
    if let Err(e) = crate::access::ssh_cert_if_stale(&tunnel) { eprintln!("aiwalk-setup ssh: no SSH certificate for {tunnel}: {e}"); return 1 }
    let Some(via) = through(&tunnel) else { return fail("this app's own path cannot be used by ssh".into()) };
    // not crate::cmd: on Windows that hides the console ssh needs. `--` ends ssh's options, so a remote command that
    // starts with "-" stays the command
    let mut c = std::process::Command::new("ssh");
    c.args(via).args(["-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=accept-new"]).args(&opts)
        .arg("--").arg(format!("{user}@{tunnel}")).args(&command);
    #[cfg(unix)]
    { use std::os::unix::process::CommandExt; let e = c.exec(); eprintln!("aiwalk-setup ssh: could not start ssh: {e}"); 127 }
    #[cfg(not(unix))]
    match c.status() { Ok(s) => s.code().unwrap_or(1), Err(e) => { eprintln!("aiwalk-setup ssh: could not start ssh: {e}"); 127 } }
}

/// The SSH button: a terminal window on this computer running `<this app> ssh <user>@<tunnel>`, so the certificate is
/// fetched in there and the person can run the same line again later. Err with a sentence when no terminal program
/// is found; the page then shows the line to copy.
#[tauri::command(async)]
pub fn open_ssh(tunnel: String, user: String) -> Result<String, String> {
    let (_, tunnel) = exo_core::ssh_destination(&format!("{user}@{tunnel}"), crate::access::ZONE)?;
    let app = app().ok_or("This app's own path cannot be used by ssh")?;
    launch(&exo_core::ssh_argv(&app, &user, &tunnel))?;
    Ok(format!("A terminal opened on {}", tunnel.trim_start_matches("ssh-").split('.').next().unwrap_or(&tunnel)))
}

/// Linux: the first terminal program found (exo_core::linux_terminals), the command as separate arguments.
#[cfg(target_os = "linux")]
fn launch(argv: &[String]) -> Result<(), String> {
    let env = std::env::var("TERMINAL").ok();
    let all = exo_core::linux_terminals(env.as_deref());
    let find = |p: &str| if p.contains('/') { Some(PathBuf::from(p)).filter(|p| p.is_file()) } else { crate::on_path(p) };
    let Some((prog, pre)) = all.iter().find_map(|(p, pre)| find(p).map(|x| (x, pre))) else {
        let names: Vec<String> = all.iter().map(|(p, _)| if env.as_deref().map(str::trim) == Some(p.as_str()) { format!("$TERMINAL ({p})") } else { p.clone() }).collect();
        return Err(format!("No terminal program was found on this computer (looked for {}). Copy the command below into a terminal.", names.join(", ")));
    };
    let mut child = crate::cmd(&prog).args(pre).args(argv).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("{} did not start: {e}. Copy the command below into a terminal.", prog.display()))?;
    std::thread::spawn(move || child.wait());   // reaped, so no zombie is left behind
    Ok(())
}

/// macOS: Terminal runs .command files; the file removes itself as its first line.
#[cfg(target_os = "macos")]
fn launch(argv: &[String]) -> Result<(), String> {
    let (_, file) = private_file("command", &exo_core::command_file(argv))?;
    let out = crate::cmd("open").arg(&file).output().map_err(|e| e.to_string())?;
    if out.status.success() { return Ok(()) }
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
    Err(format!("Terminal did not open ({}). Copy the command below into a terminal.", String::from_utf8_lossy(&out.stderr).trim()))
}

/// Windows: a .cmd file (no quoting through cmd's command line), started in its own folder by its bare name, in
/// Windows Terminal when there is one, else a console window of its own.
#[cfg(windows)]
fn launch(argv: &[String]) -> Result<(), String> {
    let (dir, file) = private_file("cmd", &exo_core::batch_file(argv)?)?;
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    // wt.exe is an app alias, so trying it is how to know it is there
    if crate::cmd("wt.exe").current_dir(&dir).args(["-d", ".", "cmd.exe", "/c", &name]).spawn().is_ok() { return Ok(()) }
    crate::cmd("cmd.exe").current_dir(&dir).args(["/c", "start", "", &name]).spawn()
        .map(|_| ()).map_err(|e| format!("No console window could be opened ({e}). Copy the command below into a terminal."))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn launch(_: &[String]) -> Result<(), String> { Err("This device cannot open a terminal. Copy the command below.".into()) }

/// A new folder only this user can open in the temp folder, holding one file `ssh.<ext>` only this user can run.
#[cfg(any(target_os = "macos", windows))]
fn private_file(ext: &str, text: &str) -> Result<(PathBuf, PathBuf), String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("aiwalk-ssh-{}-{nanos}", std::process::id()));
    #[allow(unused_mut)]
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    { use std::os::unix::fs::DirBuilderExt; b.mode(0o700); }
    b.create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;   // a folder that exists already is refused
    let file = dir.join(format!("ssh.{ext}"));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; o.mode(0o700); }
    use std::io::Write;
    o.open(&file).and_then(|mut f| f.write_all(text.as_bytes())).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((dir, file))
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
    let jwt = tokens.into_iter().flatten().next().or_else(|| crate::access::token(crate::access::ENROLL).ok())?;
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
