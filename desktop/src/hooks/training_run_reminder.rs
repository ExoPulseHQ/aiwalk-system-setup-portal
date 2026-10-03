//! `.claude/hooks/training_run_reminder.py`: before a Bash call that runs an ExoPulse training, eval or SCONE entry
//! point (locally or ssh-wrapped), a reminder of the Function Validation Protocol, naming what each canonical fixture
//! the command mentions is expected to give.

use super::file_location_reminder::strip_msg;
use super::pycompat::{context_json, fill, py_re};
use super::{text, texts, Out};
use serde_json::Value;

const EMBEDDED: &str = include_str!("texts/training_run_reminder.json");

/// The entry points; the reminder names the ones that matched, as written here.
const PATTERNS: [&str; 6] = [r"\bdeprl\.main\b", r"\bdeprl\.play\b", r"\beval_cli\b", r"\bsconewalk_\w+", r"\bsconestand_\w+", "opensim_unified_runner"];

/// The canonical fixtures: pattern, and the texts key of what it is expected to give.
const FIXTURES: [(&str, &str); 8] = [
    ("stand_to_walk_from_scratch", "fact_stand_to_walk"),
    ("v12b_consol", "fact_v12b_consol"),
    (r"v8_kneeankle|deepmimic_v8\b", "fact_v8"),
    ("v13_emg", "fact_v13_emg"),
    ("v9a_knee3", "fact_v9a_knee3"),
    ("v15a", "fact_v15a"),
    (r"deepmimic_v7\b|v7_8p8M", "fact_v7"),
    ("_ev6_orig|moe_experiment/v6", "fact_ev6"),
];

/// Which of `pats` match `scan`, in order.
fn matching<'a>(pats: impl IntoIterator<Item = &'a str>, scan: &str) -> Vec<&'a str> {
    pats.into_iter().filter(|p| py_re(p).is_match(scan).unwrap_or(false)).collect()
}

pub fn reminder(cmd: &str, t: &Value) -> Option<String> {
    let scan = strip_msg(cmd);
    let hits = matching(PATTERNS, &scan);
    if hits.is_empty() { return None }
    let fixtures = matching(FIXTURES.iter().map(|f| f.0), &scan);
    let mut lines = vec![fill(&text(t, "header"), &[("matched", &hits.join(", "))])];
    if fixtures.is_empty() {
        lines.push(text(t, "no_fixture"));
    } else {
        lines.extend(["".into(), text(t, "fixtures_heading")]);
        for (pat, key) in FIXTURES.iter().filter(|f| fixtures.contains(&f.0)) {
            lines.push(fill(&text(t, "fixture_line"), &[("pattern", pat), ("fact", &text(t, key))]));
        }
        lines.extend(["".into(), text(t, "fixtures_footer")]);
    }
    lines.extend(["".into(), text(t, "fixture_files"), text(t, "env_canary"), text(t, "log_path")]);
    Some(lines.join("\n"))
}

pub fn run(stdin: &[u8]) -> Out {
    let Ok(Value::Object(data)) = serde_json::from_slice::<Value>(stdin) else { return Out::quiet() };
    let Some(cmd) = data.get("tool_input").and_then(|t| t.get("command")).and_then(Value::as_str).filter(|c| !c.is_empty()) else { return Out::quiet() };
    match reminder(cmd, &texts("training_run_reminder", EMBEDDED)) {
        Some(r) => Out { stdout: context_json("PreToolUse", &r, ""), ..Out::quiet() },
        None => Out::quiet(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Value { serde_json::from_str(EMBEDDED).unwrap() }

    #[test]
    fn entry_points() {
        assert_eq!(matching(PATTERNS, "ssh dragon 'python -m deprl.main x.yaml; python -m deprl.play'"), [PATTERNS[0], PATTERNS[1]]);
        assert_eq!(matching(PATTERNS, "sconewalk_h0918 eval_cli opensim_unified_runner.py"), [PATTERNS[2], PATTERNS[3], PATTERNS[5]]);
        assert!(matching(PATTERNS, "mydeprl.main_x deprl.mainx my_eval_cli sconewalk_ scene").is_empty());
        assert!(reminder(r#"git commit -m "run deprl.main""#, &t()).is_none());
    }

    #[test]
    fn fixtures_in_order() {
        let r = reminder("python -m deprl.play v15a/deepmimic_v8 stand_to_walk_from_scratch", &t()).unwrap();
        let (a, b, c) = (r.find("`stand_to").unwrap(), r.find("`v8_knee").unwrap(), r.find("`v15a`").unwrap());
        assert!(a < b && b < c, "{r}");
        assert!(!reminder("deprl.main deepmimic_v8x", &t()).unwrap().contains("v8_kneeankle"));
    }
}
