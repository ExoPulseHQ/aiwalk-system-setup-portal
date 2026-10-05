//! Claude Code on this computer: installed or not, signed in or not. The team's vault workflow runs through it.
//! Finds it where the vault plugin looks (scripts/obsidian-plugin-exopulse/src/node.ts findClaude), and installs it
//! with Anthropic's official installer, as the plugin does.

use crate::{home, on_path, open_url, sh};
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use tauri::Emitter;

fn find() -> Option<String> {
    let h = home();
    [h.join(".local/bin/claude"), h.join(".local/bin/claude.exe"), h.join(".claude/local/claude"),
     "/usr/local/bin/claude".into(), "/opt/homebrew/bin/claude".into()]
        .into_iter().find(|p| p.is_file()).or_else(|| on_path("claude")).map(|p| p.to_string_lossy().into_owned())
}

#[derive(Serialize, Default)]
pub struct ClaudeState {
    /// Where the program is; None when it is not installed.
    path: Option<String>,
    version: String,
    signed_in: bool,
    /// "claude.ai", "console", … as `claude auth status` reports it
    method: String,
    /// "max", "pro", … for a claude.ai sign-in
    plan: String,
}

#[tauri::command(async)]
pub fn claude_state() -> ClaudeState {
    let Some(path) = find() else { return ClaudeState::default() };
    let version = sh(&path, &["--version"], 15).1.split_whitespace().next().unwrap_or_default().to_string();
    let status: serde_json::Value = serde_json::from_str(&sh(&path, &["auth", "status", "--json"], 20).1).unwrap_or_default();
    ClaudeState {
        signed_in: status["loggedIn"].as_bool().unwrap_or(false),
        method: status["authMethod"].as_str().unwrap_or_default().into(),
        plan: status["subscriptionType"].as_str().unwrap_or_default().into(),
        path: Some(path), version,
    }
}

/// Runs the official installer; each line it prints goes to the page as "claude-step".
#[tauri::command(async)]
pub fn claude_install(app: tauri::AppHandle) -> Result<String, String> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("powershell", vec!["-NoProfile", "-Command", "irm https://claude.ai/install.ps1 | iex"])
    } else {
        ("bash", vec!["-c", "curl -fsSL https://claude.ai/install.sh | bash 2>&1"])
    };
    let mut child = crate::cmd(cmd).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        let line = line.trim();
        if !line.is_empty() { let _ = app.emit("claude-step", line); }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if ok && find().is_some() { Ok("Claude Code installed".into()) } else { Err("The Claude Code installer did not finish; try again or see claude.ai/code".into()) }
}

/// The running sign-in's input: where the code from the browser goes (claude_code).
static LOGIN_IN: std::sync::Mutex<Option<std::process::ChildStdin>> = std::sync::Mutex::new(None);

/// Hands the running `claude auth login` the code its browser page showed ("Paste this into Claude Code"). The
/// sign-in ends that way whenever the browser cannot answer Claude directly, which is the usual case on a Mac.
#[tauri::command]
pub fn claude_code(code: String) -> Result<(), String> {
    use std::io::Write;
    let code = code.trim();
    if code.is_empty() || code.contains(char::is_whitespace) { return Err("That does not look like the code from the browser".into()) }
    let mut g = LOGIN_IN.lock().unwrap();
    let pipe = g.as_mut().ok_or("No Claude sign-in is waiting; press Sign in to Claude first")?;
    writeln!(pipe, "{code}").and_then(|_| pipe.flush()).map_err(|e| format!("Claude did not take the code: {e}"))
}

/// `claude auth login`: opens the browser for the Anthropic sign-in and waits for it. The browser either answers
/// Claude by itself or shows a code to paste, which claude_code passes on.
#[tauri::command(async)]
pub fn claude_login(app: tauri::AppHandle) -> Result<String, String> {
    let path = find().ok_or("Claude Code is not installed")?;
    let mut child = crate::cmd(&path).args(["auth", "login"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| e.to_string())?;
    *LOGIN_IN.lock().unwrap() = child.stdin.take();
    let err = child.stderr.take().unwrap();
    std::thread::spawn(move || { for _ in BufReader::new(err).lines() {} });
    let mut opened = false;
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        let _ = app.emit("claude-step", line.trim());
        if let Some(url) = line.split_whitespace().find(|w| w.starts_with("https://")) {
            if !opened { open_url(url); opened = true; }
        }
    }
    let _ = child.wait();
    *LOGIN_IN.lock().unwrap() = None;
    if claude_state().signed_in { Ok("Signed in to Claude".into()) } else { Err("Sign-in was not finished".into()) }
}
