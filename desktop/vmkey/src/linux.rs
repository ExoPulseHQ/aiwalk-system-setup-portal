//! The program itself (main.rs says what it is for); Linux only, since the key is reached through hidraw.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PORT: u16 = 47171;
/// The vault as Android's Linux Terminal sees it (the phone's shared storage is mounted at /mnt/shared).
const VAULT: &str = "/mnt/shared/Documents/aIwalk/exo-book";

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static KEY: OnceLock<age::x25519::Identity> = OnceLock::new();

fn file(rel: &str) -> Option<PathBuf> { ROOT.get().map(|r| r.join(rel)).filter(|f| f.is_file()) }

fn open(f: &Path) -> Result<Vec<u8>, Option<String>> {
    let (Some(root), Some(key)) = (ROOT.get(), KEY.get()) else { return Err(None) };
    let f = f.canonicalize().map_err(|_| Some("No such file.".to_string()))?;
    if !f.starts_with(root) || f.extension().is_none_or(|e| e != "age") || f.file_name().is_some_and(|n| n == ".keyring.age") {
        return Err(Some("Only sealed notes of an unlocked vault are opened.".into()))
    }
    crate::agefido::decrypt(key, &std::fs::read(&f).map_err(|e| Some(e.to_string()))?).map_err(Some)
}

/// The PIN from the terminal, not echoed.
fn read_pin() -> String {
    let stty = |a: &str| { if let Ok(tty) = std::fs::File::open("/dev/tty") { let _ = std::process::Command::new("stty").arg(a).stdin(tty).status(); } };
    eprint!("PIN of the key: ");
    stty("-echo");
    let mut pin = String::new();
    let _ = std::io::stdin().read_line(&mut pin);
    stty("echo");
    eprintln!();
    pin.trim_end_matches(['\r', '\n']).to_string()
}

pub fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or(VAULT.into()));
    let Ok(root) = root.canonicalize() else { eprintln!("No vault at {}.", root.display()); std::process::exit(1) };
    let keyring = std::fs::read(root.join(".keyring.age")).unwrap_or_else(|_| { eprintln!("{} has no sealed notes.", root.display()); std::process::exit(1) });
    let Some(dev) = crate::fidokey::devices().into_iter().next() else {
        eprintln!("No security key in the Linux Terminal. Plug it in while the Terminal is open, then run this again.");
        std::process::exit(1)
    };
    eprintln!("Key: {}", dev.name);
    let pin = read_pin();
    eprintln!("Touch the key.");
    let id = crate::agefido::Identity { dev: Some(&dev.id), pin: &pin, why: Default::default() };
    let opened = crate::agefido::decrypt(&id, &keyring).ok().map(|t| String::from_utf8_lossy(&t).into_owned()).unwrap_or_default();
    // the keyring holds the group key's age identity line among comments
    let Some(key) = opened.split_whitespace().find(|w| w.starts_with("AGE-SECRET-KEY-1")).and_then(|w| w.parse().ok()) else {
        eprintln!("{}", id.why.borrow_mut().take().unwrap_or_else(|| "This key is not one of the vault's holders.".into()));
        std::process::exit(1)
    };
    let _ = (ROOT.set(root.clone()), KEY.set(key));
    let token: &'static str = crate::phonehttp::token().leak();
    static VAULTS: crate::phonehttp::Vaults = crate::phonehttp::Vaults { file, open };
    if !crate::phonehttp::serve(PORT, token, &VAULTS) { eprintln!("Port {PORT} is taken: is this already running?"); std::process::exit(1) }
    let ticket = root.join(".sealed-token");
    let _ = std::fs::write(&ticket, format!("{PORT} {token}"));
    crate::fidokey::light(Some(&dev.id), 1);
    eprintln!("Unlocked. Obsidian on this phone opens sealed notes now. Unplug the key or press Ctrl+C to lock.");
    while crate::fidokey::devices().iter().any(|d| d.id == dev.id) { std::thread::sleep(std::time::Duration::from_millis(200)) }
    let _ = std::fs::remove_file(&ticket);
    eprintln!("Key unplugged: locked.");
}
