//! `skill_usage`: counts the skills used in this vault's Claude sessions, so the team sees which ones people reach for,
//! registered or not. PreToolUse on the Skill tool gives the skill Claude called; UserPromptSubmit gives the one the
//! person typed as `/name`. Each goes into `.claude/skill_usage.local.json` (gitignored), skill to { n, at }: a small
//! write and no git or network, since it runs on every call. `vault ship` copies the file into
//! `System/skill_usage/<login>.json` under this computer's name, and the vault plugin compares it with the registry.

use super::Out;
use serde_json::{json, Value};
use std::path::Path;

/// The skill this hook input is about, or None.
fn skill(input: &Value) -> Option<String> {
    if input["tool_name"] == "Skill" { return input["tool_input"]["skill"].as_str().map(|s| s.trim_start_matches('/').to_string()).filter(|s| !s.is_empty()) }
    // `/name args`; `/media/x` is a path, not a command
    let p = input["prompt"].as_str()?.strip_prefix('/')?;
    let name: String = p.chars().take_while(|c| c.is_ascii_alphanumeric() || "_:.-".contains(*c)).collect();
    (!name.is_empty() && !p[name.len()..].starts_with('/')).then_some(name)
}

/// The counts with one more use of `skill` at `now_ms`.
fn count(old: Value, skill: &str, now_ms: u64) -> Value {
    let mut all = if old.is_object() { old } else { json!({}) };
    let n = all[skill]["n"].as_u64().unwrap_or(0) + 1;
    all[skill] = json!({ "n": n, "at": now_ms });
    all
}

pub fn run(stdin: &[u8]) -> Out {
    let (Ok(input), Some(dir)) = (serde_json::from_slice::<Value>(stdin), super::project_dir()) else { return Out::quiet() };
    let Some(s) = skill(&input) else { return Out::quiet() };
    let path = dir.join(".claude").join("skill_usage.local.json");
    if !path.parent().is_some_and(Path::exists) { return Out::quiet() }
    let old = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&count(old, &s, now)).unwrap_or_default());
    Out::quiet()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_skill() {
        assert_eq!(skill(&json!({ "tool_name": "Skill", "tool_input": { "skill": "opentikz:using-opentikz" } })).as_deref(), Some("opentikz:using-opentikz"));
        assert_eq!(skill(&json!({ "prompt": "/office-hours an idea" })).as_deref(), Some("office-hours"));
        assert_eq!(skill(&json!({ "prompt": "/media/eddlai/x.png look" })), None, "a path");
        assert_eq!(skill(&json!({ "prompt": "hello /review" })), None);
        assert_eq!(skill(&json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } })), None);
    }

    #[test]
    fn counts_add_up() {
        let a = count(Value::Null, "review", 1);
        let b = count(a, "review", 2);
        assert_eq!(b["review"], json!({ "n": 2, "at": 2 }));
        assert_eq!(count(json!("junk"), "x", 3)["x"]["n"], 1);
    }

    #[test]
    fn never_fails() { assert_eq!(run(b"not json"), Out::quiet()); }
}
