//! Sealed notes: vault files encrypted with age (`*.age`) to the vault's group key. The group key sits at the vault's
//! root as `.keyring.age`, itself encrypted to each holder's hardware key (age-plugin-fido2-hmac). Unlock asks the
//! key's PIN and a touch once and keeps the group key in this app's memory until Lock or the app quits; the vault
//! plugin then asks for a note's text with `aiwalk-setup sealed open <file>`, which this app answers over a socket
//! in a folder only this user can enter. Only `*.age` files of a vault with a `.keyring.age` are ever read here.
//!
//! The plain text and the group key never touch the disk. Any program of this user can ask while unlocked, as with
//! ssh-agent: Lock (or quitting the app) ends that.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The vault the group key was unlocked for, and the key itself (age identity lines).
// ponytail: a plain String, not wiped on drop; held until Lock or quit, no idle timeout yet
static KEY: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

const KEYRING: &str = ".keyring.age";

/// age, looked for where installers put it: an app started from the desktop does not get the shell's PATH.
fn age() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.extend([crate::home().join(".local/bin"), "/usr/local/bin".into(), "/opt/homebrew/bin".into(), "/usr/bin".into()]);
    let exe = if cfg!(windows) { "age.exe" } else { "age" };
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// PATH with age's own folder first, so age finds age-plugin-fido2-hmac installed beside it.
fn path_with(age: &Path) -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(age.parent().into_iter().map(Path::to_path_buf).chain(std::env::split_paths(&path))).unwrap_or(path)
}

#[derive(Serialize)]
pub struct State { keyring: bool, unlocked: bool }

/// Whether the vault at `path` has sealed notes, and whether they are unlocked now.
#[tauri::command]
pub fn sealed_state(path: String) -> State {
    let root = Path::new(&path);
    let unlocked = KEY.lock().unwrap().as_ref().is_some_and(|(v, _)| same(v, root));
    State { keyring: root.join(KEYRING).is_file(), unlocked }
}

/// Whether the plugged-in key must get a new PIN first (true for a key just handed out with the shared PIN).
#[tauri::command(async)]
pub fn sealed_key() -> Result<bool, String> {
    #[cfg(target_os = "linux")] { crate::fidokey::must_change_pin() }
    #[cfg(not(target_os = "linux"))] { Ok(false) }
}

/// The PIN every key is handed out with; the key then refuses to work until its holder chose their own.
const HANDED_OUT_PIN: &str = "aIwalkcorp";

/// `old` None: the key still has the PIN it was handed out with.
#[tauri::command(async)]
pub fn sealed_change_pin(old: Option<String>, new: String) -> Result<(), String> {
    #[cfg(target_os = "linux")] { crate::fidokey::change_pin(old.as_deref().unwrap_or(HANDED_OUT_PIN), &new) }
    #[cfg(not(target_os = "linux"))] { let _ = (old, new); Err("Changing the key's PIN works on Linux for now.".into()) }
}

/// Our pico-fido build for the team's boards (Waveshare RP2350-Zero): its source and the changes are described in
/// desktop/firmware/README.md (AGPL-3.0).
#[cfg(target_os = "linux")]
const FIRMWARE: &[u8] = include_bytes!("../firmware/pico-fido-waveshare-rp2350-zero.uf2");

/// An RP2350 board waiting to be flashed: a fresh board starts that way, or any board held on BOOT when plugged in.
/// It shows up as a small drive with INFO_UF2.TXT; the folder it is mounted on.
#[cfg(target_os = "linux")]
fn bootsel_drive() -> Option<PathBuf> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    mounts.lines().filter_map(|l| l.split(' ').nth(1)).map(|m| PathBuf::from(m.replace("\\040", " ")))
        .find(|m| std::fs::read_to_string(m.join("INFO_UF2.TXT")).is_ok_and(|t| t.contains("Board-ID: RP2350")))
}

/// Whether a board is waiting to be made into a key (shown to owners).
#[tauri::command]
pub fn sealed_new_board() -> bool {
    #[cfg(target_os = "linux")] { bootsel_drive().is_some() }
    #[cfg(not(target_os = "linux"))] { false }
}

const RECIPIENTS: &str = ".keyring.recipients";

/// Flashes the waiting board, waits for it to come back as a key, and hands it out: the shared PIN, a credential for
/// sealed notes whose public key joins the vault's `.keyring.recipients` under `holder`, and a new PIN required on first
/// use. The holder can open sealed notes once an unlocked owner gives the waiting keys access (sealed_reseal).
#[tauri::command(async)]
pub fn sealed_setup_board(path: Option<String>, holder: String) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let holder = holder.trim().trim_start_matches('@').to_string();
        if holder.is_empty() || holder.contains(char::is_whitespace) { return Err("Name the GitHub account this key is for.".into()) }
        let drive = bootsel_drive().ok_or("No board waiting: hold BOOT while plugging it in.")?;
        // the key set up next is whichever key is plugged in, so it must be the only one
        if crate::fidokey::present() { return Err("Unplug any other security key first, so the new one is the only key here.".into()) }
        std::fs::write(drive.join("pico-fido.uf2"), FIRMWARE).map_err(|e| format!("Could not write the firmware to the board: {e}"))?;
        // the board reboots into the new firmware on its own once the file is in
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while bootsel_drive().is_some() || !crate::fidokey::present() {
            if std::time::Instant::now() > end { return Err("The board did not come back as a key within 30 s. Unplug it and plug it in again, then set it up again.".into()) }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        std::thread::sleep(std::time::Duration::from_millis(800));   // its USB settles before the first request
        if !crate::fidokey::set_pin_if_none(HANDED_OUT_PIN)? {
            return Ok("Flashed. This key already had a PIN, which was kept (flashing keeps a key's data); nothing else was changed.".into())
        }
        let recipient = new_credential(HANDED_OUT_PIN)?;
        let joined = match path.map(PathBuf::from).filter(|v| v.join(KEYRING).is_file()) {
            Some(vault) => { add_recipient(&vault, &holder, &recipient)?; true }
            None => false,
        };
        crate::fidokey::force_pin_change(HANDED_OUT_PIN)?;
        return Ok(if joined { format!("The key for {holder} is ready. Give it access with Give access while unlocked, then ship .keyring.age and .keyring.recipients.") }
            else { format!("The key for {holder} is ready. No vault here has sealed notes; its public key is {recipient}") })
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = (path, holder); Err("Setting up a key works on Linux for now.".into()) }
}

/// A credential for sealed notes on the plugged-in key (age-plugin-fido2-hmac -g: the PIN, a touch, PIN required to
/// decrypt, no separate identity file); its public key (recipient).
fn new_credential(pin: &str) -> Result<String, String> {
    let age = age().ok_or("age is not installed on this computer.")?;
    let plugin = age.with_file_name(if cfg!(windows) { "age-plugin-fido2-hmac.exe" } else { "age-plugin-fido2-hmac" });
    if !plugin.is_file() { return Err("age-plugin-fido2-hmac is not installed beside age.".into()) }
    // the two choices are read one key at a time: a newline would answer the next question
    let (ok, seen, _) = on_pty(&plugin, &["-g".as_ref()], &[("pin", format!("{pin}\r").as_bytes()), ("require a pin", b"1"), ("separate identity", b"2")])?;
    let r = seen.split_whitespace().find(|w| w.starts_with("age1fido2-hmac1")).map(String::from);
    match r { Some(r) if ok => Ok(r), _ => Err("The key did not make a credential (was it touched?).".into()) }
}

/// Adds `recipient` under `# holder` to the vault's `.keyring.recipients`, the list `.keyring.age` is sealed for.
fn add_recipient(vault: &Path, holder: &str, recipient: &str) -> Result<(), String> {
    let p = vault.join(RECIPIENTS);
    let mut text = std::fs::read_to_string(&p).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') { text.push('\n') }
    text.push_str(&format!("# {holder}\n{recipient}\n"));
    std::fs::write(&p, text).map_err(|e| e.to_string())
}

/// The holders' public keys in `.keyring.recipients`, and whether `.keyring.age` is sealed for exactly that list (the
/// last line `# sealed for: <sha256>` says which list it was sealed for).
fn recipients(vault: &Path) -> (Vec<String>, bool) {
    use ring::digest;
    let text = std::fs::read_to_string(vault.join(RECIPIENTS)).unwrap_or_default();
    let keys: Vec<String> = text.lines().map(str::trim).filter(|l| l.starts_with("age1")).map(String::from).collect();
    let hash = hex(digest::digest(&digest::SHA256, keys.join("\n").as_bytes()).as_ref());
    let sealed = text.lines().rev().find_map(|l| l.strip_prefix("# sealed for: ")).is_some_and(|h| h.trim() == hash);
    (keys, sealed)
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

#[derive(Serialize)]
pub struct Holders { keys: usize, waiting: bool }

#[tauri::command]
pub fn sealed_holders(path: String) -> Holders {
    let (keys, sealed) = recipients(Path::new(&path));
    Holders { keys: keys.len(), waiting: !keys.is_empty() && !sealed }
}

/// Seals the unlocked group key again, for every key in `.keyring.recipients`: keys set up since get access, keys
/// taken off the list lose it for the keyring from now on (not for copies they already have; see the summary).
#[tauri::command(async)]
pub fn sealed_reseal(path: String) -> Result<String, String> {
    let vault = PathBuf::from(&path);
    let (keys, _) = recipients(&vault);
    if keys.is_empty() { return Err("No keys in .keyring.recipients.".into()) }
    let guard = KEY.lock().unwrap();
    let Some((root, key)) = guard.as_ref().filter(|(r, _)| same(r, &vault)) else { return Err("Unlock sealed notes first.".into()) };
    let age = age().ok_or("age is not installed on this computer.")?;
    let tmp = root.join(".keyring.age.saving");
    let mut p = std::process::Command::new(&age).arg("-a").args(keys.iter().flat_map(|k| ["-r", k.as_str()])).arg("-o").arg(&tmp).env("PATH", path_with(&age))
        .stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("Could not start age: {e}"))?;
    let _ = p.stdin.take().unwrap().write_all(format!("{key}\n").as_bytes());
    drop(guard);
    let out = p.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() { let _ = std::fs::remove_file(&tmp); return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    std::fs::rename(&tmp, vault.join(KEYRING)).map_err(|e| e.to_string())?;
    // record which list it is sealed for, replacing the previous record
    let p = vault.join(RECIPIENTS);
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("# sealed for: ")).collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) { lines.pop(); }
    let hash = hex(ring::digest::digest(&ring::digest::SHA256, keys.join("\n").as_bytes()).as_ref());
    std::fs::write(&p, format!("{}\n# sealed for: {hash}\n", lines.join("\n"))).map_err(|e| e.to_string())?;
    Ok(format!("The keyring is sealed for {} key{}. Ship .keyring.age and .keyring.recipients so the others get it.", keys.len(), if keys.len() == 1 { "" } else { "s" }))
}

/// Forgets the group key; the key's light goes back to the locked colour. Also run when the app quits.
#[tauri::command(async)]
pub fn sealed_lock() { if KEY.lock().unwrap().take().is_some() { eprintln!("sealed: locked (Lock or quit)"); light(2) } }

/// Opens the vault's `.keyring.age` with the plugged-in key: the PIN goes in on a pseudo-terminal (age reads it from
/// nowhere else), then the person touches the key.
#[tauri::command(async)]
pub fn sealed_unlock(path: String, pin: String) -> Result<(), String> {
    let root = PathBuf::from(&path);
    let keyring = root.join(KEYRING);
    if !keyring.is_file() { return Err("This vault has no sealed notes.".into()) }
    // age's plugin waits quietly for a key that is not there; say so at once instead
    #[cfg(target_os = "linux")]
    if !crate::fidokey::present() { return Err("Plug in your key, then unlock.".into()) }
    let age = age().ok_or("age is not installed on this computer.")?;
    let run = on_pty(&age, &["-d".as_ref(), "-j".as_ref(), "fido2-hmac".as_ref(), keyring.as_os_str()], &[("pin", format!("{pin}\r").as_bytes())]);
    let (ok, mut seen, answered) = match run { Ok(r) => r, Err(why) => { eprintln!("sealed: unlock stopped: {why}"); light(0); return Err(why.into()) } };
    eprintln!("sealed: unlock exit ok={ok} answered={answered} key lines={}", identities(&seen).lines().count());
    let key = identities(&seen);
    seen.clear();
    light((ok && !key.is_empty()) as u8);
    if !ok || key.is_empty() {
        return Err(if answered == 0 { "The key did not ask for a PIN; this vault's keyring may not be for this key." }
            else { "The key refused: a wrong PIN, or this key is not one of the vault's holders." }.into())
    }
    *KEY.lock().unwrap() = Some((root, key));
    eprintln!("sealed: unlocked {path}");
    watch_key();
    Ok(())
}

/// Runs `program` on a pseudo-terminal (age and its fido2 plugin read PINs and choices from nowhere else) and answers
/// its prompts in order: each (words, reply) is sent once the words appear after the previous answer. Gives up when
/// the key is not plugged in or after a minute. (exit success, everything the terminal showed, answers sent).
fn on_pty(program: &Path, args: &[&std::ffi::OsStr], answers: &[(&str, &[u8])]) -> Result<(bool, String, usize), &'static str> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    let pair = native_pty_system().openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }).map_err(|_| "This computer gave no pseudo-terminal.")?;
    let mut c = CommandBuilder::new(program);
    c.args(args);
    c.env("PATH", path_with(program));
    let mut child = pair.slave.spawn_command(c).map_err(|_| "Could not start age.")?;
    drop(pair.slave);
    let (mut from, mut to) = (pair.master.try_clone_reader().map_err(|_| "The terminal could not be read.")?, pair.master.take_writer().map_err(|_| "The terminal could not be written.")?);
    let mut killer = child.clone_killer();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || { let mut b = [0u8; 4096]; while let Ok(n) = from.read(&mut b) { if n == 0 || tx.send(b[..n].to_vec()).is_err() { break } } });
    let (mut seen, mut next, mut from_at) = (String::new(), 0usize, 0usize);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let why = loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(b) => seen.push_str(&String::from_utf8_lossy(&b)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break None,
            Err(_) => {}
        }
        let low = seen[from_at..].to_lowercase();
        if low.contains("insert your token") { break Some("Plug in the key, then try again.") }
        if let Some((words, reply)) = answers.get(next) {
            if low.contains(words) { let _ = to.write_all(reply).and_then(|_| to.flush()); next += 1; from_at = seen.len(); }
        }
        if std::time::Instant::now() > end { break Some("The key was not touched in time.") }
        if let Ok(Some(_)) = child.try_wait() { while let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(300)) { seen.push_str(&String::from_utf8_lossy(&b)) } break None }
    };
    if let Some(why) = why { let _ = killer.kill(); let _ = child.wait(); return Err(why) }
    Ok((child.wait().map(|s| s.success()).unwrap_or(false), seen, next))
}

fn light(state: u8) {
    #[cfg(target_os = "linux")]
    crate::fidokey::light(state);
    #[cfg(not(target_os = "linux"))]
    let _ = state;
}

/// Locks as soon as the key is unplugged: unlocked means the key is here, not only that it was once.
// ponytail: any FIDO key counts as "here", polled 5 times a second; tell keys apart when holders carry two
fn watch_key() {
    #[cfg(target_os = "linux")]
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut k = KEY.lock().unwrap();
        if k.is_none() { return }
        if !crate::fidokey::present() { *k = None; eprintln!("sealed: locked (key unplugged)"); return }
    });
}

/// The age identity lines in what the terminal showed (the prompts around them are dropped).
fn identities(text: &str) -> String {
    text.split(|c: char| c.is_whitespace()).filter(|w| w.starts_with("AGE-SECRET-KEY-1") && w.len() > 60).collect::<Vec<_>>().join("\n")
}

fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) { (Ok(x), Ok(y)) => x == y, _ => a == b }
}

/// The note's text, when `file` is a sealed file of the unlocked vault.
fn open(file: &Path) -> Result<Vec<u8>, Reply> {
    let guard = KEY.lock().unwrap();
    let Some((root, key)) = guard.as_ref() else { eprintln!("sealed: open {} while locked", file.display()); return Err(Reply::Locked) };
    let (Ok(f), Ok(r)) = (file.canonicalize(), root.canonicalize()) else { return Err(Reply::Error("No such file.".into())) };
    if !f.starts_with(&r) || f.extension().is_none_or(|e| e != "age") || f.file_name().is_some_and(|n| n == KEYRING) {
        return Err(Reply::Error("Only sealed notes of the unlocked vault are opened.".into()))
    }
    let age = age().ok_or(Reply::Error("age is not installed on this computer.".into()))?;
    let mut p = std::process::Command::new(&age).args(["-d", "-i", "-"]).arg(&f).env("PATH", path_with(&age))
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .spawn().map_err(|e| Reply::Error(format!("Could not start age: {e}")))?;
    let _ = p.stdin.take().unwrap().write_all(format!("{key}\n").as_bytes());
    drop(guard);
    let out = p.wait_with_output().map_err(|e| Reply::Error(e.to_string()))?;
    if out.status.success() { Ok(out.stdout) } else { Err(Reply::Error(String::from_utf8_lossy(&out.stderr).trim().to_string())) }
}

enum Reply { Locked, Error(String) }

/// Seals stdin into `file` (an existing `*.age` under a vault with a keyring), replacing it in one step.
fn save(file: &Path) -> Result<(), String> {
    let f = file.canonicalize().map_err(|e| format!("{}: {e}", file.display()))?;
    if f.extension().is_none_or(|e| e != "age") || f.file_name().is_some_and(|n| n == KEYRING) { return Err("Only sealed notes (*.age) are saved here.".into()) }
    let pubkey = f.ancestors().map(|d| d.join(".keyring.pub")).find(|p| p.is_file()).ok_or("No .keyring.pub above this note: the vault has no group key.")?;
    let age = age().ok_or("age is not installed on this computer.")?;
    let tmp = f.with_extension("age.saving");
    let mut p = std::process::Command::new(&age).args(["-a", "-R"]).arg(&pubkey).arg("-o").arg(&tmp).env("PATH", path_with(&age))
        .stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("Could not start age: {e}"))?;
    std::io::copy(&mut std::io::stdin(), p.stdin.as_mut().unwrap()).map_err(|e| e.to_string())?;
    drop(p.stdin.take());
    let out = p.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() { let _ = std::fs::remove_file(&tmp); return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

/// Answers the plugin's requests while the app runs: one line with a file's path in, then one byte (O text, L locked,
/// E message) and the rest; the line WAIT is answered L when the notes lock. A second copy of the app leaves the socket to the first.
#[cfg(unix)]
pub fn serve() {
    use std::os::unix::net::{UnixListener, UnixStream};
    let Ok(dir) = crate::sessioncli::os::private_dir("sealed") else { return };
    let sock = dir.join("sealed.sock");
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(_) if UnixStream::connect(&sock).is_ok() => return,
        Err(_) => { let _ = std::fs::remove_file(&sock); match UnixListener::bind(&sock) { Ok(l) => l, Err(_) => return } }
    };
    std::thread::spawn(move || for mut c in listener.incoming().flatten() {
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while c.read(&mut b).unwrap_or(0) == 1 && b[0] != b'\n' && line.len() < 4096 { line.push(b[0]) }
        // WAIT: answered (L) only once the notes are locked, so a page showing a note can close it at once
        if line == b"WAIT" {
            std::thread::spawn(move || {
                while KEY.lock().unwrap().is_some() { std::thread::sleep(std::time::Duration::from_millis(100)) }
                let _ = c.write_all(b"L");
            });
            continue;
        }
        let reply = match open(Path::new(&String::from_utf8_lossy(&line).to_string())) {
            Ok(text) => [b"O".to_vec(), text].concat(),
            Err(Reply::Locked) => b"L".to_vec(),
            Err(Reply::Error(e)) => [b"E".to_vec(), e.into_bytes()].concat(),
        };
        let _ = c.write_all(&reply);
    });
}

/// `aiwalk-setup sealed open <file>`: the note's text on stdout. Exit 3 when the app is not open or not unlocked (the
/// plugin then says where to unlock), 1 on any other failure.
pub fn cli(args: &[String]) -> i32 {
    // `sealed key`: whether the plugged-in key wants a new PIN; `sealed change-pin`: the current and the new PIN as two
    // lines on stdin (never as arguments, which other programs can read)
    match args.first().map(String::as_str) {
        Some("key") => return match sealed_key() { Ok(force) => { println!("{}", if force { "change-pin" } else { "ok" }); 0 } Err(e) => { eprintln!("{e}"); 1 } },
        Some("change-pin") => {
            let mut lines = std::io::stdin().lines().map_while(Result::ok);
            let (old, new) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
            return match sealed_change_pin(Some(old), new) { Ok(()) => { println!("PIN changed"); 0 } Err(e) => { eprintln!("{e}"); 1 } }
        }
        _ => {}
    }
    // `sealed save <file>`: the note's new text on stdin, sealed again for the vault's group key. Sealing needs only the
    // public key (.keyring.pub), so it works locked too; the text never touches the disk unsealed.
    // `sealed seal <file>`: stdin sealed for the vault's group key, on stdout, for a note not written yet (the plugin
    // creates the file with this text, so a new sealed note is never plain on disk)
    if let (Some("seal"), Some(file)) = (args.first().map(String::as_str), args.get(1)) {
        let run = || -> Result<Vec<u8>, String> {
            let abs = std::path::absolute(file).map_err(|e| e.to_string())?;
            let pubkey = abs.ancestors().skip(1).map(|d| d.join(".keyring.pub")).find(|p| p.is_file()).ok_or("No .keyring.pub above this note: the vault has no group key.")?;
            let age = age().ok_or("age is not installed on this computer.")?;
            let mut p = std::process::Command::new(&age).args(["-a", "-R"]).arg(&pubkey).env("PATH", path_with(&age))
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("Could not start age: {e}"))?;
            std::io::copy(&mut std::io::stdin(), p.stdin.as_mut().unwrap()).map_err(|e| e.to_string())?;
            drop(p.stdin.take());
            let out = p.wait_with_output().map_err(|e| e.to_string())?;
            if out.status.success() { Ok(out.stdout) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
        };
        return match run() { Ok(text) => { let _ = std::io::stdout().write_all(&text); 0 } Err(e) => { eprintln!("{e}"); 1 } }
    }
    if let (Some("save"), Some(file)) = (args.first().map(String::as_str), args.get(1)) {
        return match save(Path::new(file)) { Ok(()) => 0, Err(e) => { eprintln!("{e}"); 1 } }
    }
    // `sealed wait`: returns once the notes are locked (at once when they are, or the app is not running)
    #[cfg(unix)]
    if args.first().map(String::as_str) == Some("wait") {
        let sock = crate::sessioncli::os::private_dir("sealed").map(|d| d.join("sealed.sock"));
        if let Ok(mut c) = sock.and_then(std::os::unix::net::UnixStream::connect) { let _ = c.write_all(b"WAIT\n"); let _ = c.read(&mut [0u8; 1]); }
        return 0;
    }
    let (Some("open"), Some(file)) = (args.first().map(String::as_str), args.get(1)) else { eprintln!("usage: aiwalk-setup sealed open <file> | save <file> | seal <file> | wait | key | change-pin"); return 2 };
    #[cfg(unix)]
    {
        let locked = "Sealed notes are locked: unlock them on Team access in aIwalk System Setup.";
        let abs = std::path::absolute(file).unwrap_or_else(|_| PathBuf::from(file));
        let sock = crate::sessioncli::os::private_dir("sealed").map(|d| d.join("sealed.sock"));
        let Ok(mut c) = sock.and_then(std::os::unix::net::UnixStream::connect) else { eprintln!("{locked}"); return 3 };
        let mut req = abs.to_string_lossy().into_owned().into_bytes();
        req.push(b'\n');
        let mut reply = Vec::new();
        if c.write_all(&req).and_then(|_| c.read_to_end(&mut reply)).is_err() { eprintln!("aIwalk System Setup did not answer."); return 1 }
        match reply.split_first() {
            Some((b'O', text)) => { let _ = std::io::stdout().write_all(text); 0 }
            Some((b'L', _)) => { eprintln!("{locked}"); 3 }
            Some((_, e)) => { eprintln!("{}", String::from_utf8_lossy(e)); 1 }
            None => { eprintln!("aIwalk System Setup did not answer."); 1 }
        }
    }
    #[cfg(not(unix))]
    { let _ = file; eprintln!("Sealed notes open on Linux and macOS for now."); 1 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn identities_keeps_only_key_lines() {
        let k = format!("AGE-SECRET-KEY-1{}", "Q".repeat(58));
        let shown = format!("Please enter you PIN: \r\n\x1b[F\x1b[Kage: Please touch your token...\r\n# created: now\r\n{k}\r\n");
        assert_eq!(super::identities(&shown), k);
        assert_eq!(super::identities("Please touch your token..."), "");
    }
}
