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
    /// Whether typing `claude` in a new terminal finds it. The installer puts it in ~/.local/bin, which a Mac's
    /// shell does not search by default, so someone who then types `claude` is told there is no such command.
    in_terminal: bool,
    /// The line that makes a new terminal find it, for this person's shell; empty when it is found already.
    path_fix: String,
}

/// Asks the person's own login shell, not this app's environment: an app started from the Dock has a short PATH
/// that says nothing about what a terminal would find.
#[cfg(unix)]
fn in_terminal() -> bool {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    sh(&shell, &["-lic", "command -v claude"], 8).0 == 0
}
#[cfg(not(unix))]
fn in_terminal() -> bool { true }   // ponytail: Windows' installer edits the user PATH itself; not checked here

fn path_fix(found_at: &str) -> String {
    let dir = std::path::Path::new(found_at).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = dir.replace(&home().to_string_lossy().into_owned(), "$HOME");
    let shell = std::env::var("SHELL").unwrap_or_default();
    let rc = if shell.ends_with("zsh") { "~/.zshrc" } else if shell.ends_with("fish") { return format!("fish_add_path {dir}") } else { "~/.bashrc" };
    format!("echo 'export PATH=\"{dir}:$PATH\"' >> {rc}")
}

#[tauri::command(async)]
pub fn claude_state() -> ClaudeState {
    let Some(path) = find() else { return ClaudeState::default() };
    let version = sh(&path, &["--version"], 15).1.split_whitespace().next().unwrap_or_default().to_string();
    let status: serde_json::Value = serde_json::from_str(&sh(&path, &["auth", "status", "--json"], 20).1).unwrap_or_default();
    let found = in_terminal();
    ClaudeState {
        signed_in: status["loggedIn"].as_bool().unwrap_or(false),
        method: status["authMethod"].as_str().unwrap_or_default().into(),
        plan: status["subscriptionType"].as_str().unwrap_or_default().into(),
        path_fix: if found { String::new() } else { path_fix(&path) },
        in_terminal: found,
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

/// gstack (github.com/garrytan/gstack): the skill suite the vault's CLAUDE.md routes reviews, investigations and QA
/// through. It only works installed in ~/.claude/skills (its skills name that path), so it is installed per computer.
fn gstack_dir() -> std::path::PathBuf { home().join(".claude/skills/gstack") }

/// Installed means its setup ran: the suite is there and one of the skills it links (review) is in place.
#[tauri::command(async)]
pub fn gstack_state() -> bool {
    gstack_dir().join("SKILL.md").is_file() && home().join(".claude/skills/review/SKILL.md").exists()
}

/// Clones (or updates) gstack and runs its setup, which needs bun: installed first when missing. Each line goes to the
/// page as "gstack-step". Windows runs it in Git Bash, which gstack's setup is written for.
#[tauri::command(async)]
pub fn gstack_install(app: tauri::AppHandle) -> Result<String, String> {
    const SCRIPT: &str = r#"set -e
export PATH="$HOME/.bun/bin:$PATH"
if ! command -v bun >/dev/null 2>&1; then echo "Installing bun"; curl -fsSL https://bun.sh/install | bash; export PATH="$HOME/.bun/bin:$PATH"; fi
d="$HOME/.claude/skills/gstack"
if [ -d "$d/.git" ]; then echo "Updating gstack"; git -C "$d" pull -q --ff-only; else echo "Downloading gstack"; git clone -q --single-branch --depth 1 https://github.com/garrytan/gstack.git "$d"; fi
cd "$d" && echo "Setting up gstack (a few minutes)" && ./setup -q 2>&1"#;
    let bash = if cfg!(windows) {
        // bun's own installer for Windows, then Git for Windows' bash
        let _ = crate::cmd("powershell").args(["-NoProfile", "-Command", "if (-not (Get-Command bun -ErrorAction SilentlyContinue)) { irm bun.sh/install.ps1 | iex }"]).status();
        ["C:/Program Files/Git/bin/bash.exe".into(), home().join("AppData/Local/Programs/Git/bin/bash.exe")].into_iter()
            .find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).ok_or("gstack needs Git Bash (Git for Windows) on Windows: install it from git-scm.com, then try again")?
    } else { "bash".into() };
    let mut child = crate::cmd(&bash).args(["-c", SCRIPT]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        let line = line.trim();
        if !line.is_empty() { let _ = app.emit("gstack-step", line); }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if ok && gstack_state() { Ok("gstack installed. Start a new Claude session to use it".into()) } else { Err("gstack setup did not finish; try again, or see github.com/garrytan/gstack".into()) }
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
