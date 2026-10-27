//! Sealed notes: vault files encrypted with age (`*.age`) to the vault's group key. The group key sits at the vault's
//! root as `.keyring.age`, itself encrypted to each holder's hardware key (agefido.rs, the format of
//! age-plugin-fido2-hmac). Unlock asks the key's PIN and a touch once and keeps that vault's group key in this app's
//! memory until Lock, the key is unplugged, or the app quits; the vault plugin then asks for a note's text with
//! `aiwalk-setup sealed open <file>`, which this app answers over a channel only this user can reach. Only `*.age`
//! files of an unlocked vault are ever read here.
//!
//! Each vault has its own group key, so vaults can differ in who may hold it: a vault whose `.keyring.policy` says
//! `keys: yubikey` is sealed only for YubiKeys and opened only with one (the company tier).
//!
//! The plain text and the group keys never touch the disk. Any program of this user can ask while unlocked, as with
//! ssh-agent: Lock (or quitting the app) ends that.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// An unlocked vault: its folder, its group key (age identity lines) and the key that unlocked it (unplugging that
/// one locks it).
// ponytail: the group key is a plain String, not wiped on drop; held until Lock or quit, no idle timeout yet
struct Unlocked { root: PathBuf, key: String, dev: Option<String> }
static OPEN: Mutex<Vec<Unlocked>> = Mutex::new(Vec::new());

const KEYRING: &str = ".keyring.age";
const PUBLIC: &str = ".keyring.pub";
const RECIPIENTS: &str = ".keyring.recipients";
const POLICY: &str = ".keyring.policy";
const WELCOME: &str = ".keyring.welcome.html.age";
/// The note on a key set up but not yet given to anyone; Give access replaces it with the holder's account.
const UNASSIGNED: &str = "unassigned";
/// The setup's own commit message: a key added this way came through the app, not by hand.
const ADD_MSG: &str = "chore(keyring): add a key to the holders";
/// The PIN every board is handed out with; the key then refuses to work until its holder chose their own.
const HANDED_OUT_PIN: &str = "aIwalkcorp";

fn unlocked(root: &Path) -> Option<(String, Option<String>)> {
    OPEN.lock().unwrap().iter().find(|u| same(&u.root, root)).map(|u| (u.key.clone(), u.dev.clone()))
}

/// Whether the vault takes YubiKeys only (its `.keyring.policy`).
fn yubikey_only(vault: &Path) -> bool {
    std::fs::read_to_string(vault.join(POLICY)).is_ok_and(|t| t.lines().any(|l| l.trim() == "keys: yubikey"))
}

#[derive(Serialize)]
pub struct State { keyring: bool, unlocked: bool, yubikey_only: bool }

/// Whether the vault at `path` has sealed notes, whether they are unlocked now, and whether it takes YubiKeys only.
#[tauri::command]
pub fn sealed_state(path: String) -> State {
    let root = Path::new(&path);
    State { keyring: root.join(KEYRING).is_file(), unlocked: unlocked(root).is_some(), yubikey_only: yubikey_only(root) }
}

/// Whether the plugged-in key must get a new PIN first (true for a board just handed out with the shared PIN).
#[tauri::command(async)]
pub fn sealed_key(dev: Option<String>) -> Result<bool, String> { crate::fidokey::must_change_pin(dev.as_deref()) }

/// The keys plugged in, and which of them unlocked a vault: the page asks which key to use when there is more than one.
#[derive(Serialize)]
pub struct Plugged { keys: Vec<crate::fidokey::Dev>, unlocked_by: Option<String> }

#[tauri::command]
pub fn sealed_devices() -> Plugged {
    let unlocked_by = OPEN.lock().unwrap().iter().find_map(|u| u.dev.clone());
    Plugged { keys: crate::fidokey::devices(), unlocked_by }
}

/// `old` None: the key still has the PIN it was handed out with.
#[tauri::command(async)]
pub fn sealed_change_pin(old: Option<String>, new: String, dev: Option<String>) -> Result<(), String> {
    crate::fidokey::change_pin(dev.as_deref(), old.as_deref().unwrap_or(HANDED_OUT_PIN), &new)
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

/// Only an owner of the organisation sets up keys or gives them access: GitHub says whether the signed-in account is
/// one. A holder could still re-seal by hand with age; the keyring files' push rule on GitHub is what stops that copy
/// from reaching anyone.
fn require_owner(what: &str) -> Result<(), String> {
    let org = crate::VAULTS[0].1.split('/').next().unwrap_or_default();
    match crate::github::get(&format!("user/memberships/orgs/{org}")) {
        Ok(m) if m["role"] == "admin" && m["state"] == "active" => Ok(()),
        Ok(_) => Err(format!("Only an owner of {org} can {what}.")),
        Err(e) => Err(format!("Could not ask GitHub whether you are an owner of {org}: {e}")),
    }
}

/// Commits and pushes the keyring files through the vault's own ship (`aiwalk-setup vault ship`, run in the vault),
/// so holders get them with Get latest. The message names no person or team.
fn ship_keyring(vault: &Path, msg: &str, files: &[&str]) -> Result<(), String> {
    let exe = crate::own_appimage().or_else(|| std::env::current_exe().ok()).ok_or("this app's own program was not found")?;
    let out = std::process::Command::new(exe).args(["vault", "ship", "-m", msg]).args(files).current_dir(vault)
        .output().map_err(|e| e.to_string())?;
    if out.status.success() { return Ok(()) }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("vault ship failed").trim().to_string())
}

/// The key `dev` names, or the only one plugged in; its kind too.
fn pick(dev: Option<String>) -> Result<crate::fidokey::Dev, String> {
    let all = crate::fidokey::devices();
    match (dev, all.as_slice()) {
        (_, []) => Err("Plug in your key, then try again.".into()),
        (Some(d), all) => all.iter().find(|x| x.id == d).cloned().ok_or("That key is not plugged in any more.".into()),
        (None, [one]) => Ok(one.clone()),
        (None, _) => Err("More than one key is plugged in: choose which one.".into()),
    }
}

/// Flashes the waiting board, waits for it to come back as a key, and hands it out: the shared PIN, a credential for
/// sealed notes whose public key joins the vault's `.keyring.recipients` unassigned, and a new PIN required on first
/// use. The holder can open sealed notes once an owner with a key gives it access (sealed_reseal).
#[tauri::command(async)]
pub fn sealed_setup_board(path: Option<String>) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        require_owner("set up keys")?;
        let vault = path.map(PathBuf::from).filter(|v| v.join(KEYRING).is_file());
        if vault.as_deref().is_some_and(yubikey_only) { return Err("This vault takes YubiKeys only; a board cannot join it.".into()) }
        let drive = bootsel_drive().ok_or("No board waiting: hold BOOT while plugging it in.")?;
        // the new key is the one that appears after flashing; keys already plugged in stay out of it
        let before: Vec<String> = crate::fidokey::devices().into_iter().map(|d| d.id).collect();
        std::fs::write(drive.join("pico-fido.uf2"), FIRMWARE).map_err(|e| format!("Could not write the firmware to the board: {e}"))?;
        // the board reboots into the new firmware on its own once the file is in
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let new = loop {
            let fresh = crate::fidokey::devices().into_iter().find(|d| !before.contains(&d.id));
            if let (Some(d), None) = (fresh, bootsel_drive()) { break d }
            if std::time::Instant::now() > end { return Err("The board did not come back as a key within 30 s. Unplug it and plug it in again, then set it up again.".into()) }
            std::thread::sleep(std::time::Duration::from_millis(300));
        };
        let dev = Some(new.id.as_str());
        std::thread::sleep(std::time::Duration::from_millis(800));   // its USB settles before the first request
        if !crate::fidokey::set_pin_if_none(dev, HANDED_OUT_PIN)? {
            return Ok("Flashed. This key already had a PIN, which was kept (flashing keeps a key's data); nothing else was changed.".into())
        }
        let recipient = crate::agefido::new_recipient(dev, HANDED_OUT_PIN)?;
        if let Some(v) = &vault { add_recipient(v, &recipient, &new.kind)?; }
        crate::fidokey::force_pin_change(dev, HANDED_OUT_PIN)?;
        return Ok(match &vault {
            Some(v) => format!("The key is ready{}. An owner with a key chooses who holds it in Give access.",
                if ship_keyring(v, ADD_MSG, &[RECIPIENTS]).is_ok() { " and on the holders' list" } else { "; ship .keyring.recipients" }),
            None => format!("The key is ready. No vault here has sealed notes; its public key is {recipient}"),
        })
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = path; Err("Setting up a board works on Linux for now.".into()) }
}

/// Whether the plugged-in key has a PIN yet: when not, adding it sets one, typed twice.
#[tauri::command(async)]
pub fn sealed_key_has_pin(dev: Option<String>) -> Result<bool, String> { crate::fidokey::has_pin(dev.as_deref()) }

/// Adds a key that needs no flashing (a YubiKey, or a board set up before): with its own PIN (set here when it has
/// none), a credential for sealed notes is made on it and its public key joins the list, unassigned, for Give access.
#[tauri::command(async)]
pub fn sealed_add_key(path: String, pin: String, dev: Option<String>) -> Result<String, String> {
    require_owner("add keys")?;
    let vault = PathBuf::from(&path);
    if !vault.join(KEYRING).is_file() { return Err("This vault has no sealed notes yet.".into()) }
    let d = pick(dev)?;
    if OPEN.lock().unwrap().iter().any(|u| u.dev.as_deref() == Some(&d.id)) { return Err("That is the key that unlocked; choose the one to add.".into()) }
    if yubikey_only(&vault) && d.kind != "yubikey" { return Err("This vault takes YubiKeys only.".into()) }
    if pin.chars().count() < 4 { return Err("The PIN needs at least 4 characters.".into()) }
    let set_now = crate::fidokey::set_pin_if_none(Some(&d.id), &pin)?;
    let recipient = crate::agefido::new_recipient(Some(&d.id), &pin)?;
    if recipients(&vault).0.iter().any(|h| h.key == recipient) { return Err("This key is on the list already.".into()) }
    add_recipient(&vault, &recipient, &d.kind)?;
    let shipped = ship_keyring(&vault, ADD_MSG, &[RECIPIENTS]).is_ok();
    Ok(format!("The key is on the list{}{}. An owner with a key chooses its holder in Give access.",
        if set_now { ", with the PIN you typed" } else { "" }, if shipped { "" } else { "; ship .keyring.recipients" }))
}

/// A holder's line pair in `.keyring.recipients`: `# <account|unassigned> <board|yubikey>` over the public key.
#[derive(Clone)]
struct Holder { key: String, account: String, kind: String }

fn add_recipient(vault: &Path, recipient: &str, kind: &str) -> Result<(), String> {
    let p = vault.join(RECIPIENTS);
    let mut text = std::fs::read_to_string(&p).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') { text.push('\n') }
    text.push_str(&format!("# {UNASSIGNED} {kind}\n{recipient}\n"));
    std::fs::write(&p, text).map_err(|e| e.to_string())
}

/// The holders in `.keyring.recipients`, and the keys `.keyring.age` was last sealed for (its last line,
/// `# sealed for: <fingerprint> ...`).
fn recipients(vault: &Path) -> (Vec<Holder>, Vec<String>) {
    let text = std::fs::read_to_string(vault.join(RECIPIENTS)).unwrap_or_default();
    let (mut keys, mut note) = (Vec::new(), (String::new(), String::new()));
    for l in text.lines().map(str::trim) {
        if let Some(c) = l.strip_prefix('#').filter(|_| !l.starts_with("# sealed for:")) {
            let mut w = c.split_whitespace();
            let account = w.next().filter(|a| *a != UNASSIGNED).unwrap_or_default().to_string();
            note = (account, w.next().unwrap_or("unknown").to_string());
        } else if l.starts_with("age1") {
            let (account, kind) = std::mem::take(&mut note);
            keys.push(Holder { key: l.to_string(), account, kind: if kind.is_empty() { "unknown".into() } else { kind } });
        }
    }
    let sealed = text.lines().rev().find_map(|l| l.strip_prefix("# sealed for: ")).map(|f| f.split_whitespace().map(String::from).collect()).unwrap_or_default();
    (keys, sealed)
}

fn write_recipients(vault: &Path, holders: &[Holder], sealed_for: &[String]) -> Result<(), String> {
    let mut text: String = holders.iter().map(|h| format!("# {} {}\n{}\n", if h.account.is_empty() { UNASSIGNED } else { &h.account }, h.kind, h.key)).collect();
    if !sealed_for.is_empty() { text.push_str(&format!("# sealed for: {}\n", sealed_for.join(" "))) }
    std::fs::write(vault.join(RECIPIENTS), text).map_err(|e| e.to_string())
}

/// A short, stable name for a public key, for the sealed-for record.
fn fingerprint(key: &str) -> String { hex(&ring::digest::digest(&ring::digest::SHA256, key.as_bytes()).as_ref()[..8]) }

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

#[derive(Serialize)]
pub struct KeyRow { fp: String, account: String, kind: String, working: bool, added_by: Option<String>, by_app: bool }

#[derive(Serialize)]
pub struct Holders { keys: Vec<KeyRow>, removed: usize, yubikey_only: bool, sealed_by: Option<String>, sealed_in_app: bool }

/// The app's own commit messages for the keyring: a keyring last sealed with any other message was sealed by hand, which
/// the organisation's plan cannot forbid (no push rules on private repositories), so the app shows it.
const SEAL_MSGS: [&str; 2] = ["chore(keyring): seal the vault's group key for its holders", "chore(keyring): start sealed notes"];

/// Every key on the list: whose it is, its kind, whether the keyring is sealed for it (working) or it waits for Give
/// access, and who added it (from git: the commit that brought its line in) and whether through the app's own setup;
/// and how many keys lose access on the next seal.
#[tauri::command]
pub fn sealed_holders(path: String) -> Holders {
    let vault = Path::new(&path);
    let (keys, sealed) = recipients(vault);
    let fps: Vec<String> = keys.iter().map(|h| fingerprint(&h.key)).collect();
    let rows = keys.iter().zip(&fps).map(|(h, fp)| {
        let out = std::process::Command::new("git").arg("-C").arg(vault).args(["log", "--reverse", "--format=%an%x09%s", "-S"]).arg(&h.key).args(["--", RECIPIENTS]).output().ok();
        let first = out.map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or_default().to_string()).unwrap_or_default();
        let (by, subject) = first.split_once('\t').map(|(a, b)| (Some(a.to_string()), b.to_string())).unwrap_or((None, String::new()));
        KeyRow { fp: fp.clone(), account: h.account.clone(), kind: h.kind.clone(), working: sealed.contains(fp), added_by: by, by_app: subject == ADD_MSG }
    }).collect();
    let last = std::process::Command::new("git").arg("-C").arg(vault).args(["log", "-1", "--format=%an%x09%s", "--", KEYRING]).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let (sealed_by, subject) = last.split_once('\t').map(|(a, b)| (Some(a.to_string()), b.to_string())).unwrap_or((None, String::new()));
    Holders { keys: rows, removed: sealed.iter().filter(|f| !fps.contains(f)).count(), yubikey_only: yubikey_only(vault), sealed_in_app: SEAL_MSGS.contains(&subject.as_str()), sealed_by }
}

/// Seals the unlocked group key again, for every key in `.keyring.recipients`, after binding each chosen key to its
/// holder: keys set up since get access, keys taken off the list lose it for the keyring from now on (not for copies
/// they already have; see the summary). A YubiKey-only vault is sealed only for YubiKeys, by an owner who unlocked it
/// with one.
#[tauri::command(async)]
pub fn sealed_reseal(path: String, assign: Vec<(String, String)>) -> Result<String, String> {
    require_owner("give keys access")?;
    let vault = PathBuf::from(&path);
    let Some((key, dev)) = unlocked(&vault) else { return Err("Unlock sealed notes first.".into()) };
    let (mut holders, _) = recipients(&vault);
    if yubikey_only(&vault) {
        if !dev.as_deref().is_some_and(|d| crate::fidokey::devices().iter().any(|x| x.id == d && x.kind == "yubikey")) {
            return Err("Unlock this vault with a YubiKey to give access to it.".into())
        }
        if let Some(h) = holders.iter().find(|h| h.kind != "yubikey") { return Err(format!("This vault takes YubiKeys only, and key {} is not one.", &fingerprint(&h.key)[..8])) }
    }
    // bind each chosen key to its holder: an account of the organisation, checked with GitHub
    let org = crate::VAULTS[0].1.split('/').next().unwrap_or_default();
    for (fp, login) in &assign {
        let m = crate::github::get(&format!("orgs/{org}/memberships/{login}")).map_err(|_| format!("{login} is not in {org}."))?;
        if m["state"] != "active" { return Err(format!("{login} has not joined {org} yet.")) }
        if let Some(h) = holders.iter_mut().find(|h| fingerprint(&h.key) == *fp) { h.account = login.clone() }
    }
    if holders.is_empty() { return Err("No keys in .keyring.recipients.".into()) }
    if holders.iter().any(|h| h.account.is_empty()) { return Err("Choose who holds each key first.".into()) }
    let keys: Vec<String> = holders.iter().map(|h| h.key.clone()).collect();
    seal_keyring(&vault, &key, &keys)?;
    write_recipients(&vault, &holders, &keys.iter().map(|k| fingerprint(k)).collect::<Vec<_>>())?;
    let n = format!("{} key{}", keys.len(), if keys.len() == 1 { "" } else { "s" });
    Ok(match ship_keyring(&vault, "chore(keyring): seal the vault's group key for its holders", &[KEYRING, RECIPIENTS]) {
        Ok(()) => format!("The keyring is sealed for {n} and shipped: the new holders open sealed notes after Get latest."),
        Err(e) => format!("The keyring is sealed for {n}, but shipping it failed ({e}). Ship .keyring.age and .keyring.recipients."),
    })
}

fn seal_keyring(vault: &Path, group_key: &str, keys: &[String]) -> Result<(), String> {
    let sealed = crate::agefido::encrypt(&crate::agefido::recipients(keys)?, format!("{group_key}\n").as_bytes())?;
    let tmp = vault.join(".keyring.age.saving");
    std::fs::write(&tmp, sealed).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, vault.join(KEYRING)).map_err(|e| e.to_string())
}

/// Starts sealed notes in a vault that has none: a new group key, sealed for the plugged-in key of the owner creating
/// it (who becomes its first holder), its public key, the holders' list, the policy when YubiKeys only, and a sealed
/// welcome page; all shipped. The key must be a YubiKey for a YubiKey-only vault.
#[tauri::command(async)]
pub fn sealed_create(path: String, pin: String, dev: Option<String>, yubikeys_only: bool) -> Result<String, String> {
    require_owner("start sealed notes")?;
    let vault = PathBuf::from(&path);
    if vault.join(KEYRING).exists() { return Err("This vault has sealed notes already.".into()) }
    let d = pick(dev)?;
    if yubikeys_only && d.kind != "yubikey" { return Err("A YubiKey-only vault is started with a YubiKey.".into()) }
    let login = crate::github::get("user").ok().and_then(|u| u["login"].as_str().map(String::from)).ok_or("GitHub did not say who is signed in.")?;
    let recipient = crate::agefido::new_recipient(Some(&d.id), &pin)?;
    let group = age::x25519::Identity::generate();
    let secret = secrecy::ExposeSecret::expose_secret(&group.to_string()).to_string();
    seal_keyring(&vault, &secret, std::slice::from_ref(&recipient))?;
    std::fs::write(vault.join(PUBLIC), format!("{}\n", group.to_public())).map_err(|e| e.to_string())?;
    write_recipients(&vault, &[Holder { key: recipient.clone(), account: login, kind: d.kind.clone() }], &[fingerprint(&recipient)])?;
    let mut files = vec![KEYRING, PUBLIC, RECIPIENTS, WELCOME];
    if yubikeys_only { std::fs::write(vault.join(POLICY), "keys: yubikey\n").map_err(|e| e.to_string())?; files.push(POLICY) }
    let welcome = crate::agefido::encrypt(&crate::agefido::recipients(&[group.to_public().to_string()])?, WELCOME_PAGE.as_bytes())?;
    std::fs::write(vault.join(WELCOME), welcome).map_err(|e| e.to_string())?;
    light(Some(&d.id), 1);
    Ok(match ship_keyring(&vault, "chore(keyring): start sealed notes", &files) {
        Ok(()) => "Sealed notes are started here, with your key as the first holder.".into(),
        Err(e) => format!("Sealed notes are started, but shipping failed ({e}). Ship the .keyring files."),
    })
}

/// The welcome page a new vault's keyring comes with (Welcome on Team access opens it).
const WELCOME_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Sealed notes</title>
<style>
  body { margin: 0; font: 15px/1.5 "IBM Plex Sans", system-ui, sans-serif; color: #1e1a29; background: #f7f6fa; }
  main { padding: 28px 32px; } h1 { font-size: 22px; margin: 0 0 8px; } p { margin: 0 0 10px; } small { color: #6b6578; }
  .mark { display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: #8c54d9; margin-right: 8px; }
</style></head>
<body><main>
  <h1><span class="mark"></span>Welcome to sealed notes</h1>
  <p>This page is stored in the vault only as ciphertext. You are reading it because your key just unlocked the vault's group key on this computer.</p>
  <p>Every sealed note in Obsidian opens the same way until you lock, unplug the key that unlocked, or quit aIwalk System Setup.</p>
  <small>.keyring.welcome.html.age, sealed for the group key in .keyring.pub</small>
</main></body></html>
"#;

/// The vault's welcome page, opened with the unlocked group key: proof, on screen, that this key and computer read
/// sealed notes. None when the vault has no welcome page.
#[tauri::command(async)]
pub fn sealed_welcome(path: String) -> Result<Option<String>, String> {
    let f = Path::new(&path).join(WELCOME);
    if !f.is_file() { return Ok(None) }
    match open(&f) {
        Ok(html) => Ok(Some(String::from_utf8_lossy(&html).into_owned())),
        Err(Reply::Locked) => Err("Unlock sealed notes first.".into()),
        Err(Reply::Error(e)) => Err(e),
    }
}

/// Forgets a vault's group key (`path`), or every one (None, as when the app quits); the keys' lights go back to the
/// locked colour.
#[tauri::command(async)]
pub fn sealed_lock(path: Option<String>) {
    let gone: Vec<Unlocked> = {
        let mut open = OPEN.lock().unwrap();
        let (gone, keep) = std::mem::take(&mut *open).into_iter().partition(|u| path.as_deref().is_none_or(|p| same(&u.root, Path::new(p))));
        *open = keep;
        gone
    };
    for u in gone { eprintln!("sealed: locked {} (Lock or quit)", u.root.display()); light(u.dev.as_deref(), 2) }
    #[cfg(target_os = "android")]
    keepalive_update();
}

/// Opens the vault's `.keyring.age` with the plugged-in key: its PIN, then a touch.
#[tauri::command(async)]
pub fn sealed_unlock(path: String, pin: String, dev: Option<String>) -> Result<(), String> {
    let root = PathBuf::from(&path);
    let keyring = root.join(KEYRING);
    if !keyring.is_file() { return Err("This vault has no sealed notes.".into()) }
    let d = pick(dev)?;
    if yubikey_only(&root) && d.kind != "yubikey" && d.kind != "any" { return Err("This vault opens with a YubiKey only.".into()) }
    // the keyring opened in the app: the key is asked for the secret of each holder's stanza until one is its own
    let id = crate::agefido::Identity { dev: Some(&d.id), pin: &pin, why: Default::default() };
    // a keyring that cannot be read is not a wrong key: say so (on Android, the app without All files access)
    let file = std::fs::read(&keyring).map_err(|e| format!("Could not read {KEYRING}: {e}. On Android, give aIwalk System Setup access to all files."))?;
    let opened = crate::agefido::decrypt(&id, &file);
    let key = opened.as_deref().map(|t| identities(&String::from_utf8_lossy(t))).unwrap_or_default();
    light(Some(&d.id), (!key.is_empty()) as u8);
    if key.is_empty() {
        let why = id.why.borrow_mut().take().unwrap_or_else(|| "This key is not one of the vault's holders.".into());
        eprintln!("sealed: unlock failed: {why}");
        return Err(why)
    }
    let mut open = OPEN.lock().unwrap();
    open.retain(|u| !same(&u.root, &root));
    open.push(Unlocked { root: root.clone(), key, dev: Some(d.id.clone()) });
    drop(open);
    #[cfg(target_os = "android")]
    { let _ = std::fs::write(root.join(".sealed-token"), phone_token()); keepalive_update(); }
    eprintln!("sealed: unlocked {path}");
    watch_key();
    Ok(())
}

fn light(dev: Option<&str>, state: u8) { crate::fidokey::light(dev, state) }

/// Locks a vault as soon as the key that unlocked it is unplugged, polled five times a second; other keys may come
/// and go. One watcher for all vaults; it ends when none is unlocked.
fn watch_key() {
    static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WATCHING.swap(true, std::sync::atomic::Ordering::SeqCst) { return }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let here: Vec<String> = crate::fidokey::devices().into_iter().map(|d| d.id).collect();
        let mut open = OPEN.lock().unwrap();
        open.retain(|u| {
            let stays = u.dev.as_deref().is_none_or(|d| here.iter().any(|h| h == d));
            if !stays { eprintln!("sealed: locked {} (key unplugged)", u.root.display()) }
            stays
        });
        let empty = open.is_empty();
        drop(open);
        #[cfg(target_os = "android")]
        keepalive_update();
        if empty { WATCHING.store(false, std::sync::atomic::Ordering::SeqCst); return }
    });
}

/// The age identity lines in the keyring (its comment lines are dropped).
fn identities(text: &str) -> String {
    text.split(|c: char| c.is_whitespace()).filter(|w| w.starts_with("AGE-SECRET-KEY-1") && w.len() > 60).collect::<Vec<_>>().join("\n")
}

fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) { (Ok(x), Ok(y)) => x == y, _ => a == b }
}

/// The unlocked vault `file` is in, if any.
fn vault_of(file: &Path) -> Option<(PathBuf, String)> {
    let f = file.canonicalize().ok()?;
    OPEN.lock().unwrap().iter().find_map(|u| u.root.canonicalize().ok().filter(|r| f.starts_with(r)).map(|r| (r, u.key.clone())))
}

/// The note's text, when `file` is a sealed file of an unlocked vault.
fn open(file: &Path) -> Result<Vec<u8>, Reply> {
    let Some((_, key)) = vault_of(file) else { eprintln!("sealed: open {} while locked", file.display()); return Err(Reply::Locked) };
    let f = file.canonicalize().map_err(|_| Reply::Error("No such file.".into()))?;
    if f.extension().is_none_or(|e| e != "age") || f.file_name().is_some_and(|n| n == KEYRING) {
        return Err(Reply::Error("Only sealed notes of an unlocked vault are opened.".into()))
    }
    let id: age::x25519::Identity = key.lines().next().unwrap_or_default().parse().map_err(|_| Reply::Error("The unlocked key is unreadable.".into()))?;
    let file = std::fs::read(&f).map_err(|e| Reply::Error(e.to_string()))?;
    crate::agefido::decrypt(&id, &file).map_err(Reply::Error)
}

enum Reply { Locked, Error(String) }

/// Seals stdin into `file` (an existing `*.age` under a vault with a keyring), replacing it in one step.
fn save(file: &Path) -> Result<(), String> {
    let f = file.canonicalize().map_err(|e| format!("{}: {e}", file.display()))?;
    if f.extension().is_none_or(|e| e != "age") || f.file_name().is_some_and(|n| n == KEYRING) { return Err("Only sealed notes (*.age) are saved here.".into()) }
    let mut text = Vec::new();
    std::io::stdin().read_to_end(&mut text).map_err(|e| e.to_string())?;
    let sealed = crate::phonehttp::seal_for_vault(&f, &text)?;
    let tmp = f.with_extension("age.saving");
    std::fs::write(&tmp, sealed).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

/// One request from the plugin: a line, then the answer. `<path>` answers one byte (O text, L locked, E message) and
/// the rest; `WAIT <path>` answers L once that file's vault is locked, so a page showing a note can close it at once.
fn answer(mut c: impl Read + Write + Send + 'static) {
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while c.read(&mut b).unwrap_or(0) == 1 && b[0] != b'\n' && line.len() < 4096 { line.push(b[0]) }
    let line = String::from_utf8_lossy(&line).into_owned();
    if let Some(file) = line.strip_prefix("WAIT") {
        let file = PathBuf::from(file.trim());
        std::thread::spawn(move || {
            while if file.as_os_str().is_empty() { !OPEN.lock().unwrap().is_empty() } else { vault_of(&file).is_some() } {
                std::thread::sleep(std::time::Duration::from_millis(100))
            }
            let _ = c.write_all(b"L");
        });
        return
    }
    let reply = match open(Path::new(&line)) {
        Ok(text) => [b"O".to_vec(), text].concat(),
        Err(Reply::Locked) => b"L".to_vec(),
        Err(Reply::Error(e)) => [b"E".to_vec(), e.into_bytes()].concat(),
    };
    let _ = c.write_all(&reply);
}

/// Answers the plugin while the app runs. Unix: a socket in a folder only this user can enter. Windows: a port on
/// 127.0.0.1 and a token in this user's own app-data folder; a request without the token is dropped. A second copy of
/// the app leaves the channel to the first.
pub fn serve() {
    #[cfg(all(unix, not(target_os = "android")))]
    {
        use std::os::unix::net::{UnixListener, UnixStream};
        let Ok(dir) = crate::sessioncli::os::private_dir("sealed") else { return };
        let sock = dir.join("sealed.sock");
        let listener = match UnixListener::bind(&sock) {
            Ok(l) => l,
            Err(_) if UnixStream::connect(&sock).is_ok() => return,
            Err(_) => { let _ = std::fs::remove_file(&sock); match UnixListener::bind(&sock) { Ok(l) => l, Err(_) => return } }
        };
        std::thread::spawn(move || for c in listener.incoming().flatten() { answer(c) });
    }
    #[cfg(windows)]
    {
        let ticket = windows_ticket();
        if std::fs::read_to_string(&ticket).ok().and_then(|t| t.split_whitespace().next().map(String::from))
            .and_then(|port| std::net::TcpStream::connect(format!("127.0.0.1:{port}")).ok()).is_some() { return }
        let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else { return };
        let Ok(port) = listener.local_addr().map(|a| a.port()) else { return };
        let token = crate::phonehttp::token();
        if std::fs::create_dir_all(ticket.parent().unwrap()).and_then(|_| std::fs::write(&ticket, format!("{port} {token}"))).is_err() { return }
        std::thread::spawn(move || for mut c in listener.incoming().flatten() {
            let mut got = [0u8; 33];
            if c.read_exact(&mut got).is_err() || &got[..32] != token.as_bytes() || got[32] != b'\n' { continue }
            answer(c)
        });
    }
}

/// Android: Obsidian cannot start this app's command line, so the app answers it over HTTP (phonehttp.rs) on port
/// 47170. The token is new each time the app starts and is written, at unlock, to `.sealed-token` at the vault's root
/// (kept out of git), where the plugin reads it.
// ponytail: an app with "All files access" could read the token too and ask while unlocked; a pairing step closes that
#[cfg(target_os = "android")]
pub const PHONE_PORT: u16 = 47170;
#[cfg(target_os = "android")]
static PHONE_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn phone_token() -> &'static str { PHONE_TOKEN.get_or_init(crate::phonehttp::token) }

#[cfg(target_os = "android")]
fn phone_file(rel: &str) -> Option<PathBuf> { OPEN.lock().unwrap().iter().map(|u| u.root.join(rel)).find(|f| f.is_file()) }

#[cfg(target_os = "android")]
fn phone_open(f: &Path) -> Result<Vec<u8>, Option<String>> { open(f).map_err(|r| match r { Reply::Locked => None, Reply::Error(e) => Some(e) }) }

#[cfg(target_os = "android")]
pub fn serve_phone() {
    static VAULTS: crate::phonehttp::Vaults = crate::phonehttp::Vaults { file: phone_file, open: phone_open, who: crate::login::active_login, more: crate::phonegit::phone_sync };
    crate::phonehttp::serve(PHONE_PORT, phone_token(), &VAULTS);
    crate::phonegit::copies().iter().for_each(|d| ticket_to(d));
}

/// Android: "47170 <token>" at a vault's root (kept out of git), so the plugin can ask who is signed in without the
/// notes being unlocked. A new token each start, so every copy gets it again then, and a new copy when downloaded.
/// `.sealed-token` stays for unlocking: the Linux Terminal's VM may hold that one.
#[cfg(target_os = "android")]
pub fn ticket_to(root: &Path) { let _ = std::fs::write(root.join(".app-token"), format!("{PHONE_PORT} {}", phone_token())); }

/// Android: the file the app's foreground service watches (WorkService.kt): it is there while a vault is unlocked, so
/// Android does not stop the app and lose the unlocked key; the page starts the service after an unlock.
#[cfg(target_os = "android")]
fn keepalive_file() -> PathBuf { crate::home().join("sealed-unlocked.txt") }

#[cfg(target_os = "android")]
#[tauri::command]
pub fn sealed_keepalive() -> String { keepalive_file().to_string_lossy().into_owned() }

#[cfg(target_os = "android")]
fn keepalive_update() {
    let n = OPEN.lock().unwrap().len();
    if n == 0 { let _ = std::fs::remove_file(keepalive_file()); }
    else { let _ = std::fs::write(keepalive_file(), if n == 1 { "Sealed notes unlocked".to_string() } else { format!("Sealed notes unlocked in {n} vaults") }); }
}

/// Windows: where the running app writes its port and token for the plugin (this user's local app data).
#[cfg(windows)]
fn windows_ticket() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(crate::home).join("aIwalk System Setup").join("sealed.ticket")
}

/// A connection to the running app, or None when it is not running.
fn connect() -> Option<Box<dyn ReadWrite>> {
    #[cfg(target_os = "android")]
    { None }
    #[cfg(all(unix, not(target_os = "android")))]
    { crate::sessioncli::os::private_dir("sealed").ok().and_then(|d| std::os::unix::net::UnixStream::connect(d.join("sealed.sock")).ok()).map(|c| Box::new(c) as Box<dyn ReadWrite>) }
    #[cfg(windows)]
    {
        let t = std::fs::read_to_string(windows_ticket()).ok()?;
        let (port, token) = t.trim().split_once(' ')?;
        let mut c = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).ok()?;
        c.write_all(format!("{token}\n").as_bytes()).ok()?;
        Some(Box::new(c))
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// `aiwalk-setup sealed open <file>`: the note's text on stdout. Exit 3 when the app is not open or not unlocked (the
/// plugin then says where to unlock), 1 on any other failure.
pub fn cli(args: &[String]) -> i32 {
    // `sealed key`: whether the plugged-in key wants a new PIN; `sealed change-pin`: the current and the new PIN as two
    // lines on stdin (never as arguments, which other programs can read)
    match args.first().map(String::as_str) {
        Some("key") => return match sealed_key(None) { Ok(force) => { println!("{}", if force { "change-pin" } else { "ok" }); 0 } Err(e) => { eprintln!("{e}"); 1 } },
        Some("change-pin") => {
            let mut lines = std::io::stdin().lines().map_while(Result::ok);
            let (old, new) = (lines.next().unwrap_or_default(), lines.next().unwrap_or_default());
            return match sealed_change_pin(Some(old), new, None) { Ok(()) => { println!("PIN changed"); 0 } Err(e) => { eprintln!("{e}"); 1 } }
        }
        _ => {}
    }
    // `sealed seal <file>`: stdin sealed for the vault's group key, on stdout, for a note not written yet (the plugin
    // creates the file with this text, so a new sealed note is never plain on disk)
    if let (Some("seal"), Some(file)) = (args.first().map(String::as_str), args.get(1)) {
        let run = || -> Result<Vec<u8>, String> {
            let abs = std::path::absolute(file).map_err(|e| e.to_string())?;
            let mut text = Vec::new();
            std::io::stdin().read_to_end(&mut text).map_err(|e| e.to_string())?;
            crate::phonehttp::seal_for_vault(&abs, &text)
        };
        return match run() { Ok(text) => { let _ = std::io::stdout().write_all(&text); 0 } Err(e) => { eprintln!("{e}"); 1 } }
    }
    // `sealed save <file>`: the note's new text on stdin, sealed again for the vault's group key. Sealing needs only the
    // public key (.keyring.pub), so it works locked too; the text never touches the disk unsealed.
    if let (Some("save"), Some(file)) = (args.first().map(String::as_str), args.get(1)) {
        return match save(Path::new(file)) { Ok(()) => 0, Err(e) => { eprintln!("{e}"); 1 } }
    }
    // `sealed wait [file]`: returns once that file's vault (or every vault) is locked; at once when it is, or the app
    // is not running
    if args.first().map(String::as_str) == Some("wait") {
        let file = args.get(1).map(|f| std::path::absolute(f).unwrap_or_else(|_| PathBuf::from(f)).to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(mut c) = connect() { let _ = c.write_all(format!("WAIT {file}\n").as_bytes()); let _ = c.read(&mut [0u8; 1]); }
        return 0;
    }
    let (Some("open"), Some(file)) = (args.first().map(String::as_str), args.get(1)) else { eprintln!("usage: aiwalk-setup sealed open <file> | save <file> | seal <file> | wait [file] | key | change-pin"); return 2 };
    let locked = "Sealed notes are locked: unlock them on Team access in aIwalk System Setup.";
    let abs = std::path::absolute(file).unwrap_or_else(|_| PathBuf::from(file));
    let Some(mut c) = connect() else { eprintln!("{locked}"); return 3 };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_keeps_only_key_lines() {
        let k = format!("AGE-SECRET-KEY-1{}", "Q".repeat(58));
        let shown = format!("# created: now\r\n# public key: age1xyz\r\n{k}\r\n");
        assert_eq!(identities(&shown), k);
        assert_eq!(identities("no key here"), "");
    }

    #[test]
    fn holders_list_reads_old_and_new_notes() {
        let dir = std::env::temp_dir().join(format!("sealed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(RECIPIENTS), "# eddLai\nage1fido2-hmac1aaa\n# unassigned yubikey\nage1fido2-hmac1bbb\n# sealed for: f1\n").unwrap();
        let (h, sealed) = recipients(&dir);
        assert_eq!((h[0].account.as_str(), h[0].kind.as_str()), ("eddLai", "unknown"));
        assert_eq!((h[1].account.as_str(), h[1].kind.as_str()), ("", "yubikey"));
        assert_eq!(sealed, vec!["f1".to_string()]);
        write_recipients(&dir, &h, &sealed).unwrap();
        let (again, _) = recipients(&dir);
        assert_eq!(again.len(), 2);
        assert_eq!(again[1].kind, "yubikey");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
