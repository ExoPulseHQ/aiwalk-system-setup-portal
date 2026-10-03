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
    if crate::own_appimage().is_some() { return "appimage" }
    if crate::here().starts_with("/usr") { "deb" } else { "manual" }
}

/// The release asset for this computer, as a gh --pattern.
fn pattern() -> Option<&'static str> {
    Some(match (kind(), std::env::consts::ARCH) {
        ("windows", _) => "*_x64-setup.exe",
        // the app bundle itself, packed: it replaces the installed one in place, no disk image to drag from
        ("mac", "aarch64") => "*_aarch64.app.tar.gz",
        ("mac", _) => "*_x64.app.tar.gz",
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
    let file = crate::github::download_asset(REPO, &tag, pattern, &dir).map_err(|e| format!("Could not download {tag}: {e}"))?;
    let f = file.to_string_lossy().into_owned();
    match kind() {
        // Tauri's NSIS installer: /P shows progress only, /R starts the app again when done
        "windows" => { cmd(&file).args(["/P", "/R"]).spawn().map_err(|e| e.to_string())?; app.exit(0); Ok("Installing".into()) }
        "mac" => {
            // .../aIwalk System Setup.app/Contents/MacOS/aiwalk-setup: the bundle is two folders above the program's folder
            let bundle = crate::here().ancestors().nth(2).map(std::path::PathBuf::from).filter(|p| p.extension().is_some_and(|e| e == "app"))
                .ok_or("This copy is not inside an .app, so it cannot replace itself; install the new one from the Releases page")?;
            let parent = bundle.parent().ok_or("The app has no folder around it")?;
            // unpacked next to the installed app, so swapping is a rename on one disk; the old one goes last
            let stage = parent.join(format!(".aiwalk-update-{tag}"));
            let _ = std::fs::remove_dir_all(&stage);
            std::fs::create_dir_all(&stage).map_err(|e| format!("Could not write to {}: {e}. Move the app to a folder you own, or install from the Releases page.", parent.display()))?;
            let (code, out) = sh("tar", &["-xzf", &f, "-C", &stage.to_string_lossy()], 300);
            let fresh = std::fs::read_dir(&stage).ok().and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "app")));
            let Some(fresh) = fresh.filter(|_| code == 0) else {
                let _ = std::fs::remove_dir_all(&stage);
                return Err(format!("The download could not be unpacked: {}", out.lines().last().unwrap_or("no app inside")))
            };
            let old = stage.join("old.app");
            std::fs::rename(&bundle, &old).map_err(|e| format!("Could not move the installed app aside: {e}"))?;
            if let Err(e) = std::fs::rename(&fresh, &bundle) {
                let _ = std::fs::rename(&old, &bundle);   // put the working one back
                let _ = std::fs::remove_dir_all(&stage);
                return Err(format!("Could not put the new app in place: {e}"))
            }
            let _ = std::fs::remove_dir_all(&stage);
            cmd("open").arg("-n").arg(&bundle).spawn().map_err(|e| e.to_string())?;
            app.exit(0);
            Ok("Restarting".into())
        }
        "appimage" => {
            let target = crate::own_appimage().ok_or("This copy is not running from an AppImage")?;
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
