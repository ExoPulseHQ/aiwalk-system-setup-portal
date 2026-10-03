//! One button to the newest release. The installers sit in this app's own (private) repository's Releases; the
//! bundled gh, signed in as the person, downloads the one for this computer, and the OS's installer does the rest.
//!
//! Trust: a release is whatever sits in that repository's Releases, so everyone who can write there can ship to
//! everyone's computer. The release workflow builds only for an owner's tag; keep write access to the repository
//! to people trusted with that.

use crate::{cmd, sh};
use exo_core::newer;

const REPO: &str = "ExoPulseHQ/aiwalk-system-setup-portal";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How this copy was installed, which decides the file to fetch and how it goes in.
fn kind() -> &'static str {
    if cfg!(windows) { return "windows" }
    if cfg!(target_os = "macos") { return "mac" }
    if std::env::var_os("APPIMAGE").is_some() { return "appimage" }
    if crate::here().starts_with("/usr") { "deb" } else { "manual" }
}

/// The release asset for this computer, as a gh --pattern.
fn pattern() -> Option<&'static str> {
    Some(match (kind(), std::env::consts::ARCH) {
        ("windows", _) => "*_x64-setup.exe",
        ("mac", "aarch64") => "*_aarch64.dmg",
        ("mac", _) => "*_x64.dmg",
        ("appimage", _) => "*_amd64.AppImage",
        ("deb", _) => "*_amd64.deb",
        _ => return None,
    })
}

/// This version, the newest release, and whether one click can install it here.
#[tauri::command(async)]
pub fn update_state() -> serde_json::Value {
    let latest = crate::github::get(&format!("repos/{REPO}/releases/latest")).ok()
        .and_then(|r| r["tag_name"].as_str().map(String::from)).unwrap_or_default();
    serde_json::json!({ "current": VERSION, "latest": latest, "newer": newer(&latest, VERSION), "can": pattern().is_some() })
}

/// Downloads release `tag` for this computer and starts its installer. Ok(text) says what happens next.
#[tauri::command(async)]
pub fn update_install(app: tauri::AppHandle, tag: String) -> Result<String, String> {
    let pattern = pattern().ok_or("This copy was not installed from a release; install the new one from the Releases page")?;
    let dir = std::env::temp_dir().join(format!("aiwalk-setup-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_string_lossy().into_owned();
    // tens of megabytes; slow networks get an hour
    let (code, out) = sh("gh", &["release", "download", &tag, "-R", REPO, "-p", pattern, "-D", &d, "--clobber"], 3600);
    if code != 0 { return Err(format!("Could not download {tag}: {}", out.lines().last().unwrap_or("no answer from GitHub"))) }
    let file = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().next().ok_or("The release has no file for this computer")?.path();
    let f = file.to_string_lossy().into_owned();
    match kind() {
        // Tauri's NSIS installer: /P shows progress only, /R starts the app again when done
        "windows" => { cmd(&file).args(["/P", "/R"]).spawn().map_err(|e| e.to_string())?; app.exit(0); Ok("Installing".into()) }
        // ponytail: the disk image opens and the person drags the app over the old one; swap the .app in place
        // (from the .app.tar.gz asset) when that one manual step is worth removing
        "mac" => { crate::open_url(&f); Ok(format!("{tag} is open: drag the app onto Applications, then start it again")) }
        "appimage" => {
            let target = std::path::PathBuf::from(std::env::var_os("APPIMAGE").unwrap_or_default());
            let fresh = target.with_extension("new");
            std::fs::copy(&file, &fresh).map_err(|e| format!("Could not write next to {}: {e}", target.display()))?;
            #[cfg(unix)]
            { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)); }
            std::fs::rename(&fresh, &target).map_err(|e| e.to_string())?;
            cmd(&target).spawn().map_err(|e| e.to_string())?;
            app.exit(0);
            Ok("Restarting".into())
        }
        // the system asks for the password in its own window
        "deb" => {
            let (code, out) = sh("pkexec", &["apt-get", "install", "-y", &f], 900);
            if code == 0 { Ok(format!("{tag} is installed: close the app and start it again")) }
            else { Err(format!("The installer stopped: {}", out.lines().last().unwrap_or("cancelled"))) }
        }
        _ => Err("This copy cannot update itself".into()),
    }
}
