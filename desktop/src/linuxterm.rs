//! Android: what Claude for Obsidian needs in Android's Linux Terminal, put where the Terminal's VM sees it. Shared
//! storage's Documents/aIwalk is /mnt/shared/Documents/aIwalk there, so one line in the Terminal sets the VM up:
//! `sh /mnt/shared/Documents/aIwalk/linux-terminal/setup.sh`, with the aiwalk-claude-vm that came in this app
//! (release.yml builds it before the APK; build.rs takes it in). Nothing to fetch, and always the app's own version.

use std::path::Path;

const SETUP: &str = include_str!("../linux-terminal/setup.sh");
/// aiwalk-claude-vm for the Terminal's Debian (aarch64), or empty in a build without it: setup.sh then downloads the
/// same version from the release.
const VM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/aiwalk-claude-vm"));

/// setup.sh and aiwalk-claude-vm in `dir`, each written only when it differs, so a start does not rewrite 2 MB.
pub fn put(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let same = |p: &Path, b: &[u8]| std::fs::metadata(p).is_ok_and(|m| m.len() == b.len() as u64) && std::fs::read(p).is_ok_and(|x| x == b);
    let write = |name: &str, b: &[u8]| { let p = dir.join(name); if !same(&p, b) { let _ = std::fs::write(&p, b); } };
    write("setup.sh", SETUP.replace("@VERSION@", env!("CARGO_PKG_VERSION")).as_bytes());
    // an older app's copy would be a different version than setup.sh names
    if VM.is_empty() { let _ = std::fs::remove_file(dir.join("aiwalk-claude-vm")); } else { write("aiwalk-claude-vm", VM); }
}

#[cfg(test)]
mod tests {
    #[test]
    fn setup_names_this_version_and_is_written_again_only_when_changed() {
        let dir = std::env::temp_dir().join(format!("linuxterm-test-{}", std::process::id()));
        super::put(&dir);
        let s = std::fs::read_to_string(dir.join("setup.sh")).unwrap();
        assert!(s.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))) && !s.contains("@VERSION@"));
        let at = std::fs::metadata(dir.join("setup.sh")).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        super::put(&dir);
        assert_eq!(std::fs::metadata(dir.join("setup.sh")).unwrap().modified().unwrap(), at);
        let _ = std::fs::remove_dir_all(dir);
    }
}
