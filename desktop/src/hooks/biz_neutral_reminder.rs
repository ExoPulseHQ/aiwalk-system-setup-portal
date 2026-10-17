//! Before Claude touches a private folder (a route marked `"private": true` in System/vault_rules.json, today
//! `Business/`): the naming discipline that keeps its secrets out of the book, which every member and intern reads.
//! A new primary log (`_*.md`) there is refused, since its title would reach llms.txt; anything else in the folder
//! gets the reminder that file names stay neutral. Commit messages and the vault index are handled by `vault ship`.

use super::pycompat::{basename, context_json, fill};
use super::{text, texts, Out};
use exo_core::vault_ship as vs;
use serde_json::Value;
use std::path::Path;

const EMBEDDED: &str = include_str!("texts/biz_neutral_reminder.json");

/// `p` relative to `root` with forward slashes, when it lies inside it; a relative `p` is taken as already relative.
fn rel(root: &Path, p: &str) -> Option<String> {
    let path = Path::new(p);
    let r = if path.is_absolute() { path.strip_prefix(root).ok()? } else { path };
    Some(r.to_string_lossy().replace('\\', "/"))
}

/// The private folder holding `rel`, if any.
fn holder<'a>(private: &'a [String], rel: &str) -> Option<&'a str> {
    private.iter().map(String::as_str).find(|p| rel.starts_with(&format!("{p}/")))
}

/// The shell words of `cmd` that name a path inside a private folder, quotes stripped.
fn private_words<'a>(root: &Path, private: &[String], cmd: &'a str) -> Vec<String> {
    cmd.split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '&')
        .map(|w| w.trim_matches(|c| c == '"' || c == '\'').trim_start_matches('>'))
        .filter_map(|w| rel(root, w)).filter(|w| holder(private, w).is_some()).collect()
}

/// The answer for one tool call, given the vault root and its private folders.
pub fn decide(root: &Path, private: &[String], data: &Value, t: &Value) -> Out {
    if private.is_empty() { return Out::quiet() }
    let tool = data["tool_name"].as_str().unwrap_or("");
    let ti = &data["tool_input"];
    let deny = |file: &str| Out { stdout: exo_core::root_guard::deny_json(&fill(&text(t, "deny_new_primary"), &[("file", file)])) + "\n", ..Out::quiet() };
    let remind = |key: &str, folder: &str| Out { stdout: context_json("PreToolUse", &fill(&text(t, key), &[("folder", folder)]), ""), ..Out::quiet() };
    match tool {
        "Edit" | "Write" | "MultiEdit" => {
            let fp = ti["file_path"].as_str().or_else(|| ti["path"].as_str()).unwrap_or("");
            let Some(r) = rel(root, fp) else { return Out::quiet() };
            let Some(folder) = holder(private, &r) else { return Out::quiet() };
            if tool == "Write" && vs::is_primary(basename(&r)) && !root.join(&r).exists() { return deny(&r) }
            remind("reminder", folder)
        }
        "Bash" => {
            let cmd = ti["command"].as_str().unwrap_or("");
            let words = private_words(root, private, cmd);
            if words.is_empty() { return Out::quiet() }
            let creates = ["mv ", "cp ", "touch ", "tee ", ">"].iter().any(|v| cmd.contains(v));
            if let Some(w) = words.iter().find(|w| creates && vs::is_primary(basename(w)) && !root.join(w).exists()) { return deny(w) }
            remind("reminder_bash", holder(private, &words[0]).unwrap_or(""))
        }
        _ => Out::quiet(),
    }
}

pub fn run(stdin: &[u8]) -> Out {
    let Ok(data) = serde_json::from_slice::<Value>(stdin) else { return Out::quiet() };
    let Some(root) = super::project_dir() else { return Out::quiet() };
    let rules = std::fs::read_to_string(root.join("System/vault_rules.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let private = rules.map_or(vec![], |r| vs::private_subs(&r["repos"]));
    decide(&root, &private, &data, &texts("biz_neutral_reminder", EMBEDDED))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t() -> Value { serde_json::from_str(EMBEDDED).unwrap() }
    fn denied(o: &Out) -> bool { o.stdout.contains("\"deny\"") }

    #[test]
    fn decisions() {
        let root = std::env::temp_dir().join(format!("biz_hook_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Business")).unwrap();
        std::fs::write(root.join("Business/_Plan.md"), "x").unwrap();
        let p = vec!["Business".to_string()];
        let call = |tool: &str, ti: Value| decide(&root, &p, &json!({"tool_name": tool, "tool_input": ti}), &t());
        let abs = |f: &str| root.join(f).to_string_lossy().into_owned();

        assert!(denied(&call("Write", json!({"file_path": abs("Business/_New.md")}))));
        let edit = call("Write", json!({"file_path": abs("Business/_Plan.md")}));   // exists: an edit, not a new log
        assert!(!denied(&edit) && edit.stdout.contains("additionalContext"));
        assert!(!denied(&call("Edit", json!({"file_path": abs("Business/Notes.md")}))));
        assert!(call("Write", json!({"file_path": abs("System/_Other.md")})).stdout.is_empty());

        assert!(denied(&call("Bash", json!({"command": "mv draft.md Business/_Deal.md"}))));
        assert!(denied(&call("Bash", json!({"command": format!("cp x \"{}\"", abs("Business/_Deal.md"))}))));
        assert!(!denied(&call("Bash", json!({"command": "cat Business/_Plan.md"}))));
        assert!(call("Bash", json!({"command": "git -C Business log"})).stdout.is_empty());
        assert!(call("Bash", json!({"command": "ls L1_Sensing/"})).stdout.is_empty());
        assert!(decide(&root, &[], &json!({"tool_name": "Write", "tool_input": {"file_path": abs("Business/_New.md")}}), &t()).stdout.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
