//! Module Windows VM: the dockur/windows VM for Office and Windows-only tools. Linux only.

use crate::{home, sh};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};
use tauri::Emitter;

const APP_ID: &str = "com.aiwalk.SystemSetup";
const CARD: (&str, &str) = ("0x0ca6", "0x0010"); // Castles EZUSB PC/SC (health-insurance / citizen certificate cards)
const SCHEMA: &str = "com.aiwalk.SystemSetup.WindowsVM"; // the Python app's keyring schema, kept so saved passwords still work
pub const WRONG_PASSWORD: &str = "wrong-password";
pub const NEED_PASSWORD: &str = "need-password";

// The aIwalk helper's scripts travel inside the app.
const HELPER: [(&str, &str); 3] = [
    ("agent.ps1", include_str!("../../windows/agent.ps1")),
    ("install-agent.ps1", include_str!("../../windows/install-agent.ps1")),
    ("cleanup.ps1", include_str!("../../windows/cleanup.ps1")),
];
// key -> (name, comment, icon, command-line flag)
const SHORTCUTS: [(&str, &str, &str, &str, &str); 2] = [
    ("open", "Open Windows", "Start the Windows VM and open its desktop", include_str!("../../icons/windows-open.svg"), "--windows-open"),
    ("stop", "Shut down Windows", "Shut down the Windows VM and free its memory", include_str!("../../icons/windows-stop.svg"), "--windows-stop"),
];

/// A dockur/windows VM, described by its compose file.
#[derive(Serialize, Debug, Default)]
pub struct Vm {
    pub name: String,
    pub running: bool,
    pub compose: String,
    #[serde(skip)]
    pub text: String,
    pub ram: String,
    pub cpus: String,
    pub disk: String,
    pub user: String,
    pub card_reader: bool,
    pub rdp_port: String,
    pub storage: String,
    /// Host folder Windows sees as \\host.lan\Data; also shared in the RDP window.
    pub share: Option<String>,
}

/// `KEY: "value"  # comment` from the compose file's environment.
fn env(text: &str, key: &str, default: &str) -> String {
    text.lines().map(str::trim).find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.split('#').next().unwrap().trim().trim_matches('"').to_string())
        .unwrap_or_else(|| default.into())
}

/// `- "host:container..."` volume or port lines whose container side starts with `target`.
fn mapped<'a>(text: &'a str, target: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let item = l.trim().strip_prefix('-')?.trim().trim_matches(|c| c == '"' || c == '\'');
        let (host, rest) = item.split_once(':')?;
        let rest = rest.split('#').next()?.trim().trim_end_matches('"');
        (rest == target || rest.starts_with(&format!("{target}/")) || rest.starts_with(&format!("{target}:"))).then_some(host)
    })
}

pub fn parse(name: &str, state: &str, compose: &str, text: &str) -> Vm {
    let dir = Path::new(compose).parent().unwrap_or(Path::new(""));
    Vm {
        name: name.into(), running: state == "running", compose: compose.into(), text: text.into(),
        ram: env(text, "RAM_SIZE", "?"), cpus: env(text, "CPU_CORES", "?"), disk: env(text, "DISK_SIZE", "?"),
        user: env(text, "USERNAME", "Docker"),
        card_reader: text.lines().any(|l| l.trim().starts_with("ARGUMENTS:") && l.contains(CARD.0)),
        rdp_port: mapped(text, "3389/tcp").or(mapped(text, "3389")).unwrap_or("3389").into(),
        storage: mapped(text, "/storage").map(String::from).unwrap_or_else(|| dir.join("storage").to_string_lossy().into()),
        share: mapped(text, "/data").or(mapped(text, "/shared")).map(String::from),
    }
}

pub fn find() -> Option<Vm> {
    let (code, out) = sh("docker", &["ps", "-a", "--filter", "ancestor=dockurr/windows", "--format",
        "{{.Names}}|{{.State}}|{{.Label \"com.docker.compose.project.config_files\"}}"], 15);
    let line = out.lines().next().filter(|_| code == 0)?;
    let mut f = line.split('|');
    let (name, state, compose) = (f.next()?, f.next().unwrap_or(""), f.next().unwrap_or("").split(',').next().unwrap_or(""));
    Some(parse(name, state, compose, &std::fs::read_to_string(compose).unwrap_or_default()))
}

/// Space the sparse disk image really takes on this computer.
fn disk_used_gb(vm: &Vm) -> Option<f64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(Path::new(&vm.storage).join("data.img")).ok().map(|m| m.blocks() as f64 * 512.0 / 1073741824.0)
}

/// Rewrites `KEY: "value"` lines, keeping their comments.
pub fn with_env(text: &str, values: &BTreeMap<String, String>) -> String {
    text.lines().map(|line| {
        let key = line.trim().split(':').next().unwrap_or("");
        match values.get(key) {
            Some(v) if !line.trim().starts_with('#') => {
                let (head, rest) = line.split_once(':').unwrap();
                let comment = rest.find('#').map(|i| format!("  {}", &rest[i..])).unwrap_or_default();
                format!("{head}: \"{v}\"{comment}")
            }
            _ => line.to_string(),
        }
    }).collect::<Vec<_>>().join("\n") + "\n"
}

/// Turns the card reader passthrough on or off by commenting its ARGUMENTS line.
pub fn with_card_reader(text: &str, on: bool) -> String {
    let mut done = false;
    let mut out: Vec<String> = text.lines().map(|line| {
        let t = line.trim();
        let body = t.trim_start_matches(['#', ' ']);
        if t.contains(CARD.0) && body.starts_with("ARGUMENTS:") {
            done = true;
            let indent = &line[..line.len() - line.trim_start().len()];
            format!("{indent}{}{body}", if on { "" } else { "# " })
        } else { line.to_string() }
    }).collect();
    if on && !done {
        if let Some(i) = out.iter().position(|l| l.trim() == "environment:") {
            out.insert(i + 1, format!("      ARGUMENTS: \"-device usb-host,bus=xhci.0,vendorid={},productid={}\"", CARD.0, CARD.1));
        }
        if !out.iter().any(|l| l.contains("/dev/bus/usb")) {
            if let Some(j) = out.iter().position(|l| l.trim() == "devices:") { out.insert(j + 1, "      - /dev/bus/usb".into()) }
        }
    }
    out.join("\n") + "\n"
}

fn write_and_recreate(vm: &Vm, text: &str) -> Result<(), String> {
    std::fs::write(&vm.compose, text).map_err(|e| e.to_string())?;
    let (code, out) = sh("docker", &["compose", "-f", &vm.compose, "create", "--force-recreate"], 180);
    if code == 0 { Ok(()) } else { Err(tail(&out)) }
}

fn tail(s: &str) -> String { s.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect() }

pub fn stop(vm: &Vm) -> String {
    let (code, out) = sh("docker", &["compose", "-f", &vm.compose, "stop"], 180);
    if code == 0 { "Windows is shut down".into() } else { format!("Could not shut down Windows: {}", tail(&out)) }
}

fn start(vm: &Vm) -> Result<(), String> {
    let (code, out) = sh("docker", &["compose", "-f", &vm.compose, "start"], 120);
    if code == 0 { Ok(()) } else { Err(format!("Could not start Windows: {}", tail(&out))) }
}

// ---------------------------------------------------------------- keyring

fn keyring_attrs(user: &str) -> HashMap<&str, &str> { HashMap::from([("xdg:schema", SCHEMA), ("user", user)]) }

pub fn load_password(user: &str) -> Option<String> {
    use secret_service::{blocking::SecretService, EncryptionType};
    let ss = SecretService::connect(EncryptionType::Dh).ok()?;
    let found = ss.search_items(keyring_attrs(user)).ok()?;
    let item = found.unlocked.into_iter().next().or_else(|| {
        let item = found.locked.into_iter().next()?;
        item.unlock().ok()?;
        Some(item)
    })?;
    String::from_utf8(item.get_secret().ok()?).ok()
}

fn save_password(user: &str, password: &str) -> Result<(), String> {
    use secret_service::{blocking::SecretService, EncryptionType};
    let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| e.to_string())?;
    let c = ss.get_default_collection().map_err(|e| e.to_string())?;
    if c.is_locked().unwrap_or(false) { c.unlock().map_err(|e| e.to_string())? }
    c.create_item(&format!("Windows VM ({user})"), keyring_attrs(user), password.as_bytes(), true, "text/plain")
        .map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- opening Windows

fn drives(vm: &Vm) -> Vec<String> {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    if let Some(s) = &vm.share {
        found.insert(Path::new(s.trim_end_matches('/')).file_name().unwrap_or_default().to_string_lossy().into(), s.clone());
    }
    // extra folders for Windows, name=path per line
    for line in std::fs::read_to_string(home().join(".config/aiwalk-setup/drives")).unwrap_or_default().lines() {
        if let Some((n, p)) = line.trim().split_once('=') {
            if !n.is_empty() && !p.is_empty() && !n.starts_with('#') {
                found.insert(n.trim().into(), p.trim().replacen('~', &home().to_string_lossy(), 1));
            }
        }
    }
    found.into_iter().filter(|(_, p)| Path::new(p).is_dir()).map(|(n, p)| format!("/drive:{n},{p}")).collect()
}

/// Starts the VM if needed and opens its desktop. Ok once Windows is on screen; Err(WRONG_PASSWORD) when
/// Windows refused the password (never retried, repeated wrong passwords lock the account), else Err(why).
pub fn connect(vm: &Vm, password: &str, progress: &dyn Fn(&str)) -> Result<(), String> {
    if !vm.running { progress("Starting Windows"); start(vm)? }
    // Windows drops the connection until it has booted; retry for about 3 minutes
    for attempt in 0..18 {
        progress(&if attempt == 0 { "Opening the Windows desktop".to_string() } else { format!("Waiting for Windows to finish starting (try {} of 18)", attempt + 1) });
        let mut args = vec!["run".to_string(), "--command=sdl-freerdp".into(), "com.freerdp.FreeRDP".into(), "/cert:ignore".into(), "+home-drive".into()];
        args.extend(drives(vm));
        args.extend(["+clipboard".into(), format!("/u:{}", vm.user), format!("/p:{password}"), "/scale:100".into(),
                     "/dynamic-resolution".into(), format!("/v:127.0.0.1:{}", vm.rdp_port)]);
        let Ok(mut rdp) = Command::new("flatpak").args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            else { return Err("FreeRDP is not installed: flatpak install flathub com.freerdp.FreeRDP".into()) };
        let started = SystemTime::now();
        loop {
            if let Ok(Some(_)) = rdp.try_wait() { break }
            if started.elapsed().unwrap_or_default() > Duration::from_secs(7) {
                // still connected: Windows is on screen; keep draining its output so it never blocks
                std::thread::spawn(move || { let _ = rdp.wait_with_output(); });
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        let out = rdp.wait_with_output().map(|o| String::from_utf8_lossy(&[o.stdout, o.stderr].concat()).into_owned()).unwrap_or_default();
        if out.contains("ACCOUNT_LOCKED_OUT") {
            return Err("Windows locked this account after too many wrong passwords. Wait about 10 minutes, then open Windows again.".into());
        }
        if ["LOGON_FAILURE", "WRONG_PASSWORD", "PASSWORD_EXPIRED"].iter().any(|k| out.contains(k)) { return Err(WRONG_PASSWORD.into()) }
        std::thread::sleep(Duration::from_secs(8));
    }
    Err("Windows did not answer in time, try Open Windows again".into())
}

// ---------------------------------------------------------------- shortcuts

fn shortcut_paths(key: &str, icon_file: &str) -> (PathBuf, PathBuf, PathBuf) {
    let (_, d) = sh("xdg-user-dir", &["DESKTOP"], 5);
    let desktop = if d.trim().is_empty() { home().join("Desktop") } else { PathBuf::from(d.trim()) };
    let name = format!("{APP_ID}.{key}.desktop");
    (home().join(".local/share/applications").join(&name), desktop.join(&name),
     home().join(format!(".local/share/icons/hicolor/scalable/apps/{APP_ID}.{icon_file}.svg")))
}

fn has_shortcut(key: &str) -> bool { shortcut_paths(key, "").0.exists() }

fn set_shortcut(key: &str, on: bool) -> Result<(), String> {
    let (_, name, comment, svg, flag) = SHORTCUTS.iter().find(|s| s.0 == key).ok_or("unknown shortcut")?;
    let icon_file = format!("windows-{key}");
    let (menu, desktop, icon) = shortcut_paths(key, &icon_file);
    if !on {
        for f in [&menu, &desktop] { let _ = std::fs::remove_file(f); }
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(icon.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&icon, svg).map_err(|e| e.to_string())?;
    let entry = format!("[Desktop Entry]\nName={name}\nComment={comment}\nExec={} {flag}\nIcon={APP_ID}.{icon_file}\n\
                         Terminal=false\nType=Application\nCategories=Utility;\n", exe.display());
    for f in [&menu, &desktop] {
        std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(f, &entry).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755));
    }
    sh("gio", &["set", &desktop.to_string_lossy(), "metadata::trusted", "true"], 5);
    sh("gtk-update-icon-cache", &["-f", "-t", &home().join(".local/share/icons/hicolor").to_string_lossy()], 15);
    Ok(())
}

fn notify(text: &str) {
    sh("notify-send", &["-i", &format!("{APP_ID}.windows-open"), "Windows VM", text], 5);
}

/// What the desktop shortcuts run: no window, progress as notifications.
pub fn shortcut_main(flag: &str) {
    let Some(vm) = find() else { return notify("No Windows VM on this computer yet. Open aIwalk System Setup to create one.") };
    if flag == "--windows-stop" {
        if !vm.running { return notify("Windows is already off") }
        notify("Shutting down Windows (up to 2 minutes)…");
        return notify(&stop(&vm));
    }
    let open_app = || { let _ = std::env::current_exe().map(|e| Command::new(e).spawn()); };
    let Some(password) = load_password(&vm.user) else {
        notify("Set the Windows password in aIwalk System Setup first");
        return open_app();
    };
    if !vm.running { notify("Starting Windows, this takes 1–2 minutes…") }
    match connect(&vm, &password, &|_| {}) {
        Err(e) if e == WRONG_PASSWORD => { notify("Windows rejected the saved password. Change it in aIwalk System Setup."); open_app() }
        Err(e) => notify(&e),
        Ok(()) => {}
    }
}

// ---------------------------------------------------------------- cleanup through the aIwalk helper
// A SYSTEM task in Windows (windows/agent.ps1) runs fixed jobs by name. We talk to it through
// <share>/.aiwalk: it touches "alive", we write "request", it writes "progress" and "result.txt".

fn helper_dir(vm: &Vm) -> Option<PathBuf> { vm.share.as_ref().map(|s| Path::new(s).join(".aiwalk")) }

fn wait_helper(dir: &Path, seconds: u64) -> bool {
    for _ in (0..seconds).step_by(5) {
        let fresh = std::fs::metadata(dir.join("alive")).and_then(|m| m.modified()).ok()
            .and_then(|t| t.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(30));
        if fresh { return true }
        std::thread::sleep(Duration::from_secs(5));
    }
    false
}

fn run_cleanup(app: &tauri::AppHandle, dir: &Path) -> String {
    let (result, progress) = (dir.join("result.txt"), dir.join("progress"));
    let _ = std::fs::remove_file(&result);
    let _ = std::fs::remove_file(&progress);
    if let Err(e) = std::fs::write(dir.join("request"), "cleanup") { return e.to_string() }
    for _ in 0..60 * 20 {
        std::thread::sleep(Duration::from_secs(3));
        if let Ok(step) = std::fs::read_to_string(&progress) {
            let _ = app.emit("vm-clean", step.trim_start_matches('\u{feff}').trim());
        }
        if let Ok(out) = std::fs::read_to_string(&result) {
            let after = out.lines().find(|l| l.trim_start_matches('\u{feff}').starts_with("AFTER"))
                .map(|l| l.split_whitespace().skip(1).collect::<Vec<_>>().join(" "));
            let used = find().as_ref().and_then(disk_used_gb).map(|g| format!(". The disk now takes {g:.0} GB here.")).unwrap_or_default();
            return after.map(|a| format!("Cleanup finished. Windows C: {a}")).unwrap_or("Cleanup finished".into()) + &used;
        }
    }
    "Windows did not finish the cleanup within an hour".into()
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
pub struct VmState {
    vm: Option<Vm>,
    disk_used_gb: Option<f64>,
    host_gb: u64,
    host_cpus: usize,
    shortcuts: Vec<(String, String, String, bool)>,
}

#[tauri::command(async)]
pub fn vm_state() -> VmState {
    let vm = find();
    let host_gb = std::fs::read_to_string("/proc/meminfo").ok()
        .and_then(|m| m.lines().next()?.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) / 1048576;
    VmState {
        disk_used_gb: vm.as_ref().and_then(disk_used_gb), vm, host_gb,
        host_cpus: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        shortcuts: SHORTCUTS.iter().map(|s| (s.0.into(), s.1.into(), s.2.into(), has_shortcut(s.0))).collect(),
    }
}

/// Every VM action; Ok(message) or Err(message), with Err(NEED_PASSWORD) / Err(WRONG_PASSWORD) for the page to ask.
#[tauri::command(async)]
pub fn vm_action(app: tauri::AppHandle, action: String, on: Option<bool>, key: Option<String>,
                 values: Option<BTreeMap<String, String>>, password: Option<String>) -> Result<String, String> {
    let vm = find().ok_or("No Windows VM on this computer")?;
    match action.as_str() {
        "open" => {
            if let Some(p) = &password { save_password(&vm.user, p)? }
            let p = load_password(&vm.user).ok_or(NEED_PASSWORD)?;
            connect(&vm, &p, &|text| { let _ = app.emit("vm-step", text); }).map(|_| String::new())
        }
        "stop" => { let _ = app.emit("vm-step", "Shutting down Windows, up to 2 minutes"); Ok(stop(&vm)) }
        "password" => save_password(&vm.user, password.as_deref().unwrap_or_default()).map(|_| "Password saved".into()),
        "card" => {
            if vm.running { let _ = app.emit("vm-step", "Shutting down Windows first"); stop(&vm); }
            let _ = app.emit("vm-step", "Applying the card reader setting");
            let on = on.unwrap_or(false);
            write_and_recreate(&vm, &with_card_reader(&vm.text, on)).map_err(|e| format!("Could not apply the card reader setting: {e}"))?;
            Ok(format!("Card reader passthrough is {}", if on { "on" } else { "off" }))
        }
        "resources" => {
            let values = values.unwrap_or_default();
            if vm.running { let _ = app.emit("vm-step", "Shutting down Windows first"); stop(&vm); }
            let _ = app.emit("vm-step", "Applying the new size");
            write_and_recreate(&vm, &with_env(&vm.text, &values)).map_err(|e| format!("Could not apply: {e}"))?;
            Ok(if values.contains_key("DISK_SIZE") {
                "Applied. To use the bigger disk, extend drive C: in Windows' Disk Management after the next start.".into()
            } else { "Applied, used from the next start".into() })
        }
        "shortcut" => {
            let key = key.unwrap_or_default();
            set_shortcut(&key, on.unwrap_or(false))?;
            Ok(format!("{} the shortcut", if on.unwrap_or(false) { "Added" } else { "Removed" }))
        }
        // Err("needs-setup") when the helper is not installed in Windows yet; "clean-after-setup" waits for it
        "clean" | "clean-after-setup" => {
            let dir = helper_dir(&vm).ok_or("This VM has no shared folder, so cleanup cannot run automatically")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            for (name, body) in HELPER { std::fs::write(dir.join(name), body).map_err(|e| e.to_string())? }
            let starting = !vm.running;
            let _ = app.emit("vm-clean", if starting { "Starting Windows" } else { "Contacting Windows" });
            if starting { start(&vm)? }
            let wait = if action == "clean-after-setup" { 900 } else if starting { 240 } else { 20 };
            if action == "clean-after-setup" { let _ = app.emit("vm-clean", "Waiting for the one-time setup in Windows"); }
            if !wait_helper(&dir, wait) {
                return Err(if action == "clean" { "needs-setup".into() } else { "The aIwalk helper was not set up, try Clean up again".into() });
            }
            Ok(run_cleanup(&app, &dir))
        }
        _ => Err(format!("Unknown action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPOSE: &str = r#"services:
  windows:
    environment:
      RAM_SIZE: "6G"         # memory
      CPU_CORES: "4"
      USERNAME: "m365"
      ARGUMENTS: "-device usb-host,bus=xhci.0,vendorid=0x0ca6,productid=0x0010"  # reader
    devices:
      - /dev/kvm
      - /dev/bus/usb
    ports:
      - "8006:8006"
      - "13389:3389/tcp"     # RDP
    volumes:
      - /home/u/winvm/storage:/storage   # disk
      - /media/u/WIN11:/data
"#;

    #[test]
    fn compose_is_read_like_the_python_app() {
        let vm = parse("winvm", "running", "/home/u/winvm/compose.yaml", COMPOSE);
        assert_eq!((vm.ram.as_str(), vm.cpus.as_str(), vm.disk.as_str(), vm.user.as_str()), ("6G", "4", "?", "m365"));
        assert_eq!((vm.rdp_port.as_str(), vm.storage.as_str()), ("13389", "/home/u/winvm/storage"));
        assert_eq!(vm.share.as_deref(), Some("/media/u/WIN11"));
        assert!(vm.card_reader && vm.running);
    }

    #[test]
    fn edits_keep_comments_and_toggle_the_reader() {
        let v = BTreeMap::from([("RAM_SIZE".to_string(), "8G".to_string())]);
        assert!(with_env(COMPOSE, &v).contains(r#"      RAM_SIZE: "8G"  # memory"#));
        let off = with_card_reader(COMPOSE, false);
        assert!(off.contains("      # ARGUMENTS:") && !parse("", "", "", &off).card_reader);
        assert!(parse("", "", "", &with_card_reader(&off, true)).card_reader);
    }
}
