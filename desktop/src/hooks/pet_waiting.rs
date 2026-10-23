//! `pet_waiting`: the Notification hook. When Claude Code asks the person for something (a tool to allow, an answer),
//! it writes `<session>.waiting.json` beside the session's transcript, so the vault plugin's pet can show that a
//! session waits. The pet drops it once the transcript has a newer line. The idle reminder Claude Code also sends
//! after a turn ends ("waiting for your input") is not a question and writes nothing.
//!
//! Like every hook it never gets in the way: anything it cannot read or write ends in "say nothing, exit 0".

use super::Out;
use std::path::Path;

/// The waiting file's path and contents, or None for a notification the pet should not show.
fn waiting(input: &serde_json::Value, now_ms: u128) -> Option<(std::path::PathBuf, String)> {
    let kind = input["notification_type"].as_str().unwrap_or_default();
    let message = input["message"].as_str().unwrap_or_default();
    if kind == "idle_prompt" || (kind.is_empty() && message.to_lowercase().contains("waiting for your input")) { return None }
    let session = input["session_id"].as_str().filter(|s| !s.is_empty() && !s.contains(['/', '\\']))?;
    let dir = Path::new(input["transcript_path"].as_str()?).parent()?;
    Some((dir.join(format!("{session}.waiting.json")), serde_json::json!({ "message": message, "at": now_ms as u64 }).to_string()))
}

pub fn run(stdin: &[u8]) -> Out {
    let Ok(input) = serde_json::from_slice::<serde_json::Value>(stdin) else { return Out::quiet() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    if let Some((path, body)) = waiting(&input, now) { let _ = std::fs::write(path, body); }
    Out::quiet()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_are_written_and_idle_is_not() {
        let n = |kind: &str, msg: &str| serde_json::json!({ "session_id": "abc", "transcript_path": "/h/.claude/projects/v/abc.jsonl", "message": msg, "notification_type": kind });
        let (p, body) = waiting(&n("permission_prompt", "Claude needs your permission to use Bash"), 5).unwrap();
        assert_eq!(p, Path::new("/h/.claude/projects/v/abc.waiting.json"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["at"], 5);
        assert!(waiting(&n("idle_prompt", "Claude is waiting for your input"), 5).is_none());
        assert!(waiting(&n("", "Claude is waiting for your input"), 5).is_none(), "older Claude Code without notification_type");
        assert!(waiting(&n("", "Claude needs your permission to use Edit"), 5).is_some());
        assert!(waiting(&serde_json::json!({ "session_id": "../x", "transcript_path": "/t/a.jsonl" }), 5).is_none(), "no path tricks");
    }

    #[test]
    fn never_fails() {
        assert_eq!(run(b"not json"), Out::quiet());
        assert_eq!(run(br#"{"session_id":"s","transcript_path":"/nonexistent/dir/s.jsonl","message":"x"}"#), Out::quiet());
    }
}
