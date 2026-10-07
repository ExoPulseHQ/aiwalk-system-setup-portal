//! Module Android Phone: the phones on this desk (desktop mode, mirroring, keyboard, USB / Wi-Fi).
//! Linux only; adb and scrcpy come with the app folder, or from PATH.

use crate::{find_tool, here, sh_stdin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

const PHONEDESK: &str = "com.eddlai.phonedesk";
const SHIZUKU: &str = "moe.shizuku.privileged.api";
const APP_ID: &str = "com.aiwalk.SystemSetup";
const P_FIRST: &str = "persist.wm.debug.force_desktop_first_on_default_display_for_testing";
const P_RESTRICT: &str = "persist.wm.debug.desktop_mode_enforce_device_restrictions";

/// Root phones: the full desktop setup and its undo (same as the Python app).
fn root_script(on: bool) -> String {
    if on {
        format!("resetprop -p {P_RESTRICT} false\nresetprop -p {P_FIRST} true\nmkdir -p /data/adb/service.d
cat > /data/adb/service.d/landscape.sh <<'S'
#!/system/bin/sh
until [ \"$(getprop sys.boot_completed)\" = 1 ]; do sleep 2; done
wm set-ignore-orientation-request true
wm user-rotation lock 1
S
chmod 755 /data/adb/service.d/landscape.sh
wm density 288
settings put system font_scale 0.9
settings put system accelerometer_rotation 0
settings put global overlay_display_devices none
")
    } else {
        format!("resetprop -p --delete {P_FIRST}\nresetprop -p --delete {P_RESTRICT}
rm -f /data/adb/service.d/landscape.sh
wm density reset
settings put system font_scale 1.0
wm set-ignore-orientation-request false
wm user-rotation free
settings put system accelerometer_rotation 1
settings put global overlay_display_devices none
")
    }
}

fn adb_path() -> String { find_tool("tools/platform-tools/adb", "adb") }
/// Where the two apps this page puts on a phone (the Desktop Mode app and Shizuku) are kept on this computer. They
/// are in no installer: only Linux has this page and few people use it, so they are files of the release, fetched
/// the first time Install is pressed (about 3 MB) and kept here.
fn kept_apks() -> std::path::PathBuf { crate::home().join(".local/share/aiwalk-setup/apk") }

/// The phone app `name`: beside the program (a hand-made bundle), else where a fetched one is kept.
fn apk(name: &str) -> std::path::PathBuf {
    let beside = here().join("apk").join(name);
    if beside.exists() { beside } else { kept_apks().join(name) }
}

/// The phone app `name`, fetched from this version's release when it is not on this computer yet (from the newest
/// release when this version has none, a build made by hand).
fn fetch_apk(name: &str) -> Result<std::path::PathBuf, String> {
    let have = apk(name);
    if have.exists() { return Ok(have) }
    let (repo, dir) = (crate::update::REPO, kept_apks());
    crate::github::download_asset(repo, &format!("v{}", env!("CARGO_PKG_VERSION")), name, &dir).or_else(|first| {
        let latest = crate::github::get(&format!("repos/{repo}/releases/latest")).ok().and_then(|r| r["tag_name"].as_str().map(String::from)).ok_or(first)?;
        crate::github::download_asset(repo, &latest, name, &dir)
    })
}

/// adb with an optional serial; (exit code, stdout without \r). 124 when it timed out.
fn adb(serial: Option<&str>, args: &[&str], timeout: u64) -> (i32, String) {
    let mut cmd = vec![];
    if let Some(s) = serial { cmd.extend(["-s", s]) }
    cmd.extend(args);
    let (code, out) = sh_stdin(&adb_path(), &cmd, None, timeout);
    (code, out.replace('\r', "").trim().to_string())
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct Phone {
    pub serial: String,
    pub wireless: bool,
    pub model: String,
    pub ip: String,
    pub root: bool,
    pub desk_on: bool,
    pub desk_app_on: bool,
    pub phonedesk: bool,
    pub phonedesk_outdated: bool,
    pub shizuku: bool,
}

/// One round trip for everything that changes.
const PROBE: &str = "getprop ro.product.model; getprop {P_FIRST}; settings get global overlay_display_devices; \
    settings get secure enabled_accessibility_services; pm path {PD} >/dev/null 2>&1 && echo yes || echo no; \
    pidof shizuku_server >/dev/null && echo yes || echo no; echo \"$(ip -4 addr show wlan0 2>/dev/null | grep -o 'inet [0-9.]*' | cut -d' ' -f2 | head -1)\"; \
    p=$(pm path {PD} 2>/dev/null | head -1 | cut -d: -f2); echo \"$([ -n \"$p\" ] && sha256sum \"$p\" 2>/dev/null | cut -c1-64)\"";

/// Reads the probe's 8 lines; `bundled` is the SHA-256 of the PhoneDesk APK this app ships.
pub fn parse_probe(serial: &str, out: &str, root: bool, bundled: &str) -> Phone {
    let mut l: Vec<&str> = out.split('\n').collect();
    l.resize(8, "");
    let wireless = serial.contains(':');
    let desk_native = l[1] == "true";
    let desk_app_on = !["", "none", "null"].contains(&l[2]) && l[3].contains(PHONEDESK);
    let phonedesk = l[4] == "yes";
    Phone {
        serial: serial.into(), wireless,
        model: if l[0].is_empty() { serial.into() } else { l[0].into() },
        ip: if wireless { serial.split(':').next().unwrap().into() } else { l[6].into() },
        root, desk_on: if root { desk_native } else { desk_app_on }, desk_app_on, phonedesk,
        // only when there is a bundled APK to compare with
        phonedesk_outdated: phonedesk && !bundled.is_empty() && !l[7].is_empty() && l[7] != bundled,
        shizuku: l[5] == "yes",
    }
}

fn bundled_hash() -> String {
    let Ok(out) = crate::cmd("sha256sum").arg(apk("phonedesk.apk")).output() else { return String::new() };
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_string()
}

static ROOT: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

#[derive(Serialize)]
pub struct Phones {
    phones: Vec<Phone>,
    /// A phone is plugged in but has not allowed USB debugging yet.
    waiting: bool,
}

/// Connects every phone that offers wireless debugging on this network (they advertise themselves; adb accepts only
/// the ones paired with this computer), else the last address a phone was seen at. How many connected.
fn reconnect() -> usize {
    let (_, out) = adb(None, &["mdns", "services"], 20);
    let mut targets: Vec<String> = out.lines().filter(|l| l.contains("_adb-tls-connect"))
        .filter_map(|l| l.split_whitespace().last().map(String::from)).collect();
    if targets.is_empty() {
        // only reaches a phone still in `adb tcpip` mode, which a restart of the phone ends
        if let Ok(ip) = std::fs::read_to_string(ipfile()) { targets.push(format!("{}:5555", ip.trim())) }
    }
    targets.iter().filter(|t| adb(None, &["connect", t], 20).1.contains("connected")).count()
}

/// When the list last tried to reconnect by itself: the page asks every few seconds, a look-up takes about one.
static TRIED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

#[tauri::command(async)]
pub fn phones() -> Phones {
    let devices = || adb(None, &["devices"], 20).1;
    let mut out = devices();
    // a phone that restarted comes back on another port (and maybe another address) and nothing reconnects it:
    // with no phone on Wi-Fi in the list, look for one, at most twice a minute
    let wireless = |out: &str| out.lines().skip(1).any(|l| l.contains("\tdevice") && (l.contains(':') || l.contains("_adb-tls-connect")));
    if !wireless(&out) {
        let due = { let mut t = TRIED.lock().unwrap(); let due = t.is_none_or(|at| at.elapsed() > Duration::from_secs(30)); if due { *t = Some(std::time::Instant::now()) } due };
        if due && reconnect() > 0 { out = devices() }
    }
    let rows: Vec<Vec<&str>> = out.lines().skip(1).filter(|l| l.contains('\t')).map(|l| l.split('\t').collect()).collect();
    let waiting = rows.iter().any(|r| r[1] == "unauthorized");
    let bundled = bundled_hash();
    let probe = PROBE.replace("{P_FIRST}", P_FIRST).replace("{PD}", PHONEDESK);
    let phones: Vec<Phone> = rows.iter().filter(|r| r[1] == "device").map(|r| {
        let serial = r[0];
        let (_, out) = adb(Some(serial), &["shell", &probe], 8);
        // root is asked once per serial
        let root = *ROOT.lock().unwrap().get_or_insert_with(HashMap::new).entry(serial.to_string())
            .or_insert_with(|| adb(Some(serial), &["shell", "su -c true"], 5).0 == 0);
        parse_probe(serial, &out, root, &bundled)
    }).collect();
    // where a phone is now, for the day mDNS does not answer
    if let Some(ip) = phones.iter().map(|p| p.ip.trim()).find(|ip| !ip.is_empty()) {
        if std::fs::read_to_string(ipfile()).map(|s| s.trim() != ip).unwrap_or(true) {
            let _ = std::fs::create_dir_all(ipfile().parent().unwrap());
            let _ = std::fs::write(ipfile(), ip);
        }
    }
    Phones { phones, waiting }
}

fn scrcpy(serial: &str, model: &str, keyboard_only: bool) {
    let mut args = vec!["-s".to_string(), serial.to_string()];
    if keyboard_only {
        args.extend(["--no-video", "--no-audio", "--keyboard=uhid", "--mouse=uhid", "--window-title=Phone keyboard"].map(String::from));
    } else {
        args.extend(["--keyboard=uhid".into(), format!("--window-title={model}")]);
    }
    // group scrcpy's windows under this app in the dock and show our icon inside them
    let _ = crate::cmd(find_tool("tools/scrcpy/scrcpy", "scrcpy")).args(args)
        .env("ADB", adb_path()).env("SCRCPY_ICON_PATH", here().join("icon.png"))
        .envs([("SDL_APP_ID", APP_ID), ("SDL_VIDEO_WAYLAND_WMCLASS", APP_ID), ("SDL_VIDEO_X11_WMCLASS", APP_ID)])
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Starts Shizuku, then sets up what lets it come back by itself (Shizuku 13.6's BootCompleteReceiver, Android 13+):
/// after a restart on Wi-Fi it switches wireless debugging on and starts over it, if it may write secure settings and
/// remembers that it was last started by adb. The one thing a computer cannot do for it is the pairing with the
/// phone's wireless debugging, which Android takes only on the phone; the install message asks for it.
fn start_shizuku(serial: &str) {
    let s = Some(serial);
    let (_, path) = adb(s, &["shell", &format!("pm path {SHIZUKU}")], 20);
    let lib = path.replace("package:", "").replace("base.apk", "lib/arm64/libshizuku.so");
    adb(s, &["shell", &lib], 60);
    adb(s, &["shell", &format!("pm grant {SHIZUKU} android.permission.WRITE_SECURE_SETTINGS")], 20);
    // battery saving must not stop the server or the app that restarts it
    adb(s, &["shell", &format!("dumpsys deviceidle whitelist +{SHIZUKU}")], 20);
    // its home screen is where it writes down "last started by adb"; a moment for it to see the running server
    adb(s, &["shell", &format!("am start -n {SHIZUKU}/moe.shizuku.manager.MainActivity")], 20);
    std::thread::sleep(Duration::from_secs(2));
}

fn install_phonedesk(p: &Phone, step: &dyn Fn(&str)) -> String {
    if !apk("phonedesk.apk").exists() || !apk("shizuku.apk").exists() { step("Getting the Desktop Mode app for the phone, about 3 MB") }
    let (pd, sz) = match (fetch_apk("phonedesk.apk"), fetch_apk("shizuku.apk")) {
        (Ok(pd), Ok(sz)) => (pd, sz),
        (Err(e), _) | (_, Err(e)) => return format!("Could not get the Desktop Mode app for the phone: {e}"),
    };
    let s = Some(p.serial.as_str());
    if adb(s, &["shell", &format!("pm path {SHIZUKU}")], 20).0 != 0
        && { step("Installing Shizuku on the phone"); adb(s, &["install", &sz.to_string_lossy()], 180).0 != 0 } {
        return "Could not install Shizuku".into();
    }
    if p.desk_app_on {  // updating kills the running desk; leave it cleanly first
        adb(s, &["shell", &format!("am start -n {PHONEDESK}/.MainActivity")], 20);
        std::thread::sleep(Duration::from_secs(3));
    }
    step("Installing the Desktop Mode app on the phone, about a minute");
    let (code, said) = adb(s, &["install", "-r", &pd.to_string_lossy()], 180);
    // the copy on the phone was signed by another key (a build made by hand before releases carried this app):
    // Android takes an update only from the key the installed app was signed with
    if said.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        return "The Desktop Mode app on the phone is from an older build that this one cannot update. Remove PhoneDesk on the phone once (Settings, Apps), then press Install again.".into()
    }
    if code != 0 { return "Could not install the Desktop Mode app".into() }
    step("Starting Shizuku");
    adb(s, &["shell", &format!("pm enable {PHONEDESK}")], 20);
    start_shizuku(&p.serial);
    format!("Desktop Mode app installed on {}. Allow Shizuku on the phone the first time you use it. One more step on the \
             phone, once: in Shizuku choose Start via Wireless debugging, then Pairing. After that Shizuku restarts by itself \
             when the phone restarts on Wi-Fi, and you can start it without a computer.", p.model)
}

fn ipfile() -> std::path::PathBuf { crate::home().join(".config/phone-panel/ip") }

/// One action on one phone (or, for "reconnect", on whatever is on Wi-Fi); returns the message to show.
/// The page passes back the phone it showed, so the action works on what the person saw.
#[tauri::command(async)]
pub fn phone_action(app: tauri::AppHandle, action: String, phone: Option<Phone>, on: Option<bool>, reboot: Option<bool>) -> String {
    use tauri::Emitter;
    let step = |text: &str| { let _ = app.emit("phone-step", text); };
    let p = phone.unwrap_or_default();
    let s = Some(p.serial.as_str());
    match action.as_str() {
        "keyboard" => { scrcpy(&p.serial, &p.model, true); "Click the Phone keyboard window, then type".into() }
        "mirror" => { scrcpy(&p.serial, &p.model, false); String::new() }
        "install" => install_phonedesk(&p, &step),
        "desk" if p.root => {
            let on = on.unwrap_or(false);
            sh_stdin(&adb_path(), &["-s", &p.serial, "shell", "su"], Some(&root_script(on)), 30);
            // drop only PhoneDesk's accessibility entry, keep any other services
            adb(s, &["shell", "v=$(settings get secure enabled_accessibility_services); \
                n=$(echo \"$v\" | tr \":\" \"\\n\" | grep -v \"^com.eddlai.phonedesk/\" | paste -sd:); \
                [ \"$v\" != \"$n\" ] && settings put secure enabled_accessibility_services \"$n\""], 20);
            if reboot.unwrap_or(false) { adb(s, &["reboot"], 20); format!("{} is restarting", p.model) }
            else { "Applied, takes effect after a restart".into() }
        }
        "desk" => {
            let on = on.unwrap_or(false);
            if on && !p.shizuku { start_shizuku(&p.serial) }
            // launching PhoneDesk enters the desk; launching it again while the desk runs exits
            adb(s, &["shell", &format!("am start -n {PHONEDESK}/.MainActivity")], 20);
            if on { "Turning on the desktop, check the phone".into() } else { "Back to phone mode".into() }
        }
        "wireless" => {
            if p.ip.is_empty() { return "The phone is not on Wi-Fi".into() }
            step("Switching the phone to Wi-Fi");
            adb(s, &["tcpip", "5555"], 20);
            let _ = std::fs::create_dir_all(ipfile().parent().unwrap());
            let _ = std::fs::write(ipfile(), &p.ip);
            std::thread::sleep(Duration::from_secs(2));
            let (_, out) = adb(None, &["connect", &format!("{}:5555", p.ip)], 20);
            if out.contains("connected") { "Now on Wi-Fi, you can unplug the cable".into() } else { format!("Could not connect over Wi-Fi: {out}") }
        }
        "disconnect" => { adb(None, &["disconnect", &p.serial], 20); "Wi-Fi disconnected".into() }
        "reconnect" => {
            step("Looking for phones on this Wi-Fi");
            let n = reconnect();
            if n > 0 { format!("Connected {n} phone(s)") } else { "No phone found on Wi-Fi".into() }
        }
        _ => format!("Unknown action {action}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reads_app_desk_and_outdated_install() {
        let out = "Pixel 10 Pro\n\n1920x1080/320\ncom.eddlai.phonedesk/.DeskService\nyes\nyes\n192.168.0.5\nabc";
        let p = parse_probe("SER1", out, false, "def");
        assert_eq!((p.model.as_str(), p.desk_on, p.phonedesk_outdated, p.shizuku, p.ip.as_str()),
                   ("Pixel 10 Pro", true, true, true, "192.168.0.5"));
        let root = parse_probe("10.0.0.2:5555", "Pixel\ntrue", true, "");
        assert!(root.desk_on && root.wireless && root.ip == "10.0.0.2");
        assert_eq!(parse_probe("S", "", false, "").model, "S");
        assert!(!parse_probe("S", out, false, "").phonedesk_outdated);
    }
}
