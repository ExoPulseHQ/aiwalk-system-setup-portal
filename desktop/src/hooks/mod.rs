//! The vault's Claude Code hooks inside the app: `aiwalk-setup hook <name>`, so a computer without Python runs them.
//! Each hook is one file here, a port of the vault's `.claude/hooks/<name>.py` that answers the same input the same
//! way: Claude Code's hook JSON on stdin, then the same stdout, stderr and exit code as the Python.
//!
//! Wording lives in data, not in code: the reminders are reworded far more often than the app is released. Each hook
//! has `texts/<name>.json` here (the shipped wording, a JSON object of key to text) and reads the vault's own
//! `.claude/hooks/texts/<name>.json` over it when the vault has one, key by key. So the vault changes its wording
//! without waiting for a release, and a vault without the file gets the wording this build shipped with.
//!
//! A hook must never get in the way by failing: input that is not JSON, a missing file, a git that does not answer
//! all end in "say nothing, exit 0" unless the Python does otherwise for that same input.

#![allow(dead_code)]   // the helpers below are for the hooks, which land one file at a time

mod writing_style_reminder;

use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

mod file_location_reminder;
mod pycompat;
mod skills_registry_reminder;
mod slide_layout_reminder;
mod training_run_reminder;

/// What a hook answers: printed as they are, then the process exits with `code`.
#[derive(Debug, Default, PartialEq)]
pub struct Out { pub stdout: String, pub stderr: String, pub code: i32 }

impl Out {
    /// Says nothing and lets the tool run.
    pub fn quiet() -> Out { Out::default() }
}

/// The project the session is rooted in: CLAUDE_PROJECT_DIR, as the Python hooks read it.
pub fn project_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_PROJECT_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// A hook's wording: the shipped `default` (the embedded texts/<name>.json) with the vault's
/// `.claude/hooks/texts/<name>.json` laid over it key by key. A vault file that is missing or not a JSON object
/// changes nothing.
pub fn texts(name: &str, default: &str) -> Value {
    let mut base: Value = serde_json::from_str(default).unwrap_or(Value::Object(Default::default()));
    let over = project_dir().map(|p| p.join(".claude").join("hooks").join("texts").join(format!("{name}.json")))
        .and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let (Some(b), Some(Value::Object(o))) = (base.as_object_mut(), over) { for (k, v) in o { b.insert(k, v); } }
    base
}

/// One text by key; empty when the key is missing, so a hook with a broken texts file says less rather than fails.
pub fn text(texts: &Value, key: &str) -> String { texts[key].as_str().unwrap_or_default().to_string() }

/// Runs the hook called `name` on stdin; the process exit code. An unknown name is a usage error (exit 2), which
/// Claude Code shows instead of running the tool blind.
pub fn main(args: &[String]) -> i32 {
    let Some(name) = args.first().map(String::as_str) else { eprintln!("usage: aiwalk-setup hook <name>"); return 2 };
    let mut stdin = vec![];
    let _ = std::io::stdin().read_to_end(&mut stdin);
    let out = match name {
        // one line per hook, in the order of the vault's .claude/hooks folder
        "file_location_reminder" => file_location_reminder::run(&stdin),
        "skills_registry_reminder" => skills_registry_reminder::run(&stdin),
        "slide_layout_reminder" => slide_layout_reminder::run(&stdin),
        "training_run_reminder" => training_run_reminder::run(&stdin),
        "writing_style_reminder" => writing_style_reminder::run(&stdin),
        _ => { eprintln!("aiwalk-setup hook: no hook called {name}"); return 2 }
    };
    #[allow(unreachable_code)]
    { let out: Out = out; print!("{}", out.stdout); eprint!("{}", out.stderr); out.code }
}

/// The hooks this build runs, for `--can` and for whoever writes the vault's settings.json.
pub const NAMES: &[&str] = &["root-only-guard", "file_location_reminder", "skills_registry_reminder", "slide_layout_reminder", "training_run_reminder", "writing_style_reminder"];
