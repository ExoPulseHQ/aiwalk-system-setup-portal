//! Python 3 on this computer. The vault's own tools need it (scripts/vault_ship.py behind Sync vault, the hooks),
//! and Windows ships none: `python` there is usually the Microsoft Store's alias, which only prints how to install.
//! The search order is the vault plugin's (scripts/obsidian-plugin-exopulse/src/node.ts, findPython), so both agree
//! on which Python is the one.

use crate::sh;
use exo_core::python_version;
use std::path::PathBuf;

/// The program and leading arguments that start Python 3 here, with its version; None when there is none.
fn find() -> Option<(Vec<String>, String)> {
    let answers = |cmd: &[String]| -> Option<String> {
        let args: Vec<&str> = cmd[1..].iter().map(String::as_str).chain(["--version"]).collect();
        let (code, out) = sh(&cmd[0], &args, 10);
        if code == 0 { python_version(&out) } else { None }
    };
    let mut tries: Vec<Vec<String>> = vec![];
    if cfg!(windows) {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        // 1. the py launcher, which real installers add and the Store alias does not
        if let Some(root) = var("SystemRoot") { tries.push(vec![root.join("py.exe").to_string_lossy().into(), "-3".into()]) }
        tries.push(vec!["py".into(), "-3".into()]);
        // 2. python.exe on PATH, but not the alias in ...\WindowsApps
        let on_path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join("python.exe")).filter(|f| f.is_file()).collect()).unwrap_or_default();
        let alias = |p: &PathBuf| p.to_string_lossy().to_lowercase().contains("\\windowsapps\\");
        tries.extend(on_path.iter().filter(|p| !alias(p)).map(|p| vec![p.to_string_lossy().into()]));
        // 3. where the python.org and winget installers put it
        let roots = [var("LOCALAPPDATA").map(|d| d.join("Programs").join("Python")), var("ProgramFiles"), Some(PathBuf::from("C:\\"))];
        for root in roots.into_iter().flatten() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Python3")) && p.join("python.exe").is_file()).collect();
            dirs.sort(); dirs.reverse();   // the newest first
            tries.extend(dirs.into_iter().map(|d| vec![d.join("python.exe").to_string_lossy().into()]));
        }
        // 4. last, the WindowsApps one: the Store's real Python lives where the alias does, and only it answers "Python 3"
        tries.extend(on_path.iter().filter(|p| alias(p)).map(|p| vec![p.to_string_lossy().into()]));
    } else {
        tries.push(vec!["python3".into()]);
        tries.push(vec!["python".into()]);
    }
    tries.into_iter().find_map(|cmd| answers(&cmd).map(|v| (cmd, v)))
}

/// {version, command}: what the badge shows; both null when this computer has no Python 3.
#[tauri::command(async)]
pub fn python_state() -> serde_json::Value {
    match find() {
        Some((cmd, version)) => serde_json::json!({ "version": version, "command": cmd.join(" ") }),
        None => serde_json::json!({ "version": null, "command": null }),
    }
}

/// Installs Python 3 for this person. On Windows through winget (no administrator needed, and it brings the py
/// launcher, so nothing on PATH has to change); elsewhere the system's own way is named.
#[tauri::command(async)]
pub fn python_install() -> Result<String, String> {
    if !cfg!(windows) {
        return Err(if cfg!(target_os = "macos") { "Install it from a terminal: xcode-select --install".into() }
                   else { "Install it from a terminal: sudo apt install python3 (Ubuntu, Debian)".into() })
    }
    let (code, out) = sh("winget", &["install", "-e", "--id", "Python.Python.3.12", "--scope", "user",
                                     "--accept-package-agreements", "--accept-source-agreements"], 900);
    if find().is_some() { return Ok("Python is installed".into()) }
    if code != 0 && out.trim().is_empty() {
        crate::open_url("https://www.python.org/downloads/windows/");
        return Err("This computer has no winget. Install Python from the page that opened (tick \"Add python.exe to PATH\"), then check again.".into())
    }
    Err(format!("The installer did not finish: {}", out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("no answer")))
}
