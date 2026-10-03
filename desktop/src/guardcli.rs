//! `aiwalk-setup guard --status | --install | --uninstall` and `aiwalk-setup hook root-only-guard`: the vault's
//! .claude/hooks/root_only_guard.py inside the app, so a computer without Python runs the guard. The logic is
//! exo_core::root_guard; this file reads stdin and ~/.claude/settings.json.
//!
//! `--install` registers this binary (its absolute path, quoted for the shell) in the user's settings.json in place
//! of the Python command, and takes the Python entry out, so one guard runs, not two. The last line printed is the
//! status: on, off, stale or missing-copy (see exo_core::root_guard::status).

use exo_core::root_guard as rg;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

/// The command settings.json gets: the AppImage itself when run from one (its mount point vanishes on exit).
fn own() -> String {
    let exe = crate::own_appimage().or_else(|| std::env::current_exe().ok()).unwrap_or_default();
    rg::hook_command(&exe.to_string_lossy())
}

/// settings.json's text, None when there is none; Err when it is there but cannot be read (it is then never rewritten).
fn read(path: &PathBuf) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn guard(args: &[String]) -> i32 {
    let home = crate::home();
    let settings = home.join(".claude").join("settings.json");
    match args.first().map(String::as_str) {
        Some("--status") => {}
        Some(flag @ ("--install" | "--uninstall")) => {
            let new = read(&settings).and_then(|t| rg::rewrite_settings(t.as_deref(), &own(), flag == "--install", &home).map_err(|e| format!("{}: {e}", settings.display())));
            let written = new.and_then(|new| {
                let bak = settings.with_file_name("settings.json.bak");
                if settings.exists() { std::fs::copy(&settings, &bak).map_err(|e| format!("{}: {e}", bak.display()))?; }
                std::fs::create_dir_all(settings.parent().unwrap()).and_then(|_| std::fs::write(&settings, new)).map_err(|e| format!("{}: {e}", settings.display()))
            });
            if let Err(e) = written { eprintln!("{e}"); return 1 }
        }
        _ => { eprintln!("usage: aiwalk-setup guard --status | --install | --uninstall"); return 2 }
    }
    println!("{}", rg::status(read(&settings).ok().flatten().as_deref(), &own(), &home));
    0
}

/// The PreToolUse hook: Claude Code's JSON on stdin, a deny on stdout or nothing. Input that is not a JSON object is
/// let through silently, as the Python does with input that is not JSON.
pub fn hook(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("root-only-guard") { eprintln!("usage: aiwalk-setup hook root-only-guard"); return 2 }
    let mut buf = vec![];
    let _ = std::io::stdin().read_to_end(&mut buf);
    let Ok(data @ Value::Object(_)) = serde_json::from_slice::<Value>(&buf) else { return 0 };
    let nonempty = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let cwd = nonempty(data.get("cwd")).unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().into());
    let project = std::env::var("CLAUDE_PROJECT_DIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| cwd.clone());
    let tool = data.get("tool_name").and_then(Value::as_str).unwrap_or("");
    if let Some(why) = rg::verdict(tool, data.get("tool_input").unwrap_or(&Value::Null), &project, &cwd) {
        println!("{}", rg::deny_json(&why));
    }
    0
}
