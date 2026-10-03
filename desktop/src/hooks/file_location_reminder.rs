//! `.claude/hooks/file_location_reminder.py`: before a Bash call, a reminder when the command redirects output to
//! the home root (`> ~/x`, `>> $HOME/x`, `-o ~/x`, `--output ~/x`, also inside an ssh-wrapped command) instead of
//! the canonical places. Folders already sorted (`~/exo/`, `~/archive/`, dotdirs, ...) stay silent.
//!
//! Not ported: the Python's new-document reminder (DOC_CREATE_RE with writing_style_reminder's is_new_doc). It calls
//! is_new_doc with one argument where that function takes two, so every command it matches ends in a TypeError
//! traceback (exit 1) and the reminder never reaches anyone; writing_style_reminder covers Bash-created documents itself.

use super::pycompat::{context_json, fill, py_re};
use super::{text, texts, Out};
use fancy_regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const EMBEDDED: &str = include_str!("texts/file_location_reminder.json");

const ALLOWED_DIRS: &str = concat!(
    r"exo/|NTKCAP/|thesis_results/|archive/|Documents/|Downloads/|Desktop/|",
    r"Pictures/|Videos/|Music/|Templates/|Public/|bin/|logs/|snap/|",
    r"claude_scratch_archive/|SCONE/|models/|work/|Xilinx_projects/|",
    r"miniconda3/|mambaforge/|",
    r"\.",
);

/// The Python's _strip_msg: drop the text of `-m "..."`, `--message '...'`, `-F ...`, `--message=...` so a commit
/// message that mentions `> ~/x` does not count. Shared with training_run_reminder, which has the same function.
pub fn strip_msg(cmd: &str) -> String {
    static RES: LazyLock<[(Regex, &str); 3]> = LazyLock::new(|| [
        (py_re(r#"(-m|--message|-F)\s+"[^"]*""#), "$1"),
        (py_re(r#"(-m|--message|-F)\s+'[^']*'"#), "$1"),
        (py_re(r#"--message=("[^"]*"|'[^']*'|\S+)"#), "--message"),
    ]);
    RES.iter().fold(cmd.to_string(), |c, (re, to)| re.replace_all(&c, *to).into_owned())
}

/// The names written straight into the home root, in order (REDIR_RE.findall).
pub fn home_root_targets(scan: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| py_re(&format!(r"(?:>>?|-o|--output(?:=|\s+))\s*(?:~/|\$HOME/)(?!(?:{ALLOWED_DIRS}))([\w.\-]+)")));
    RE.captures_iter(scan).filter_map(|m| m.ok()?.get(1).map(|g| g.as_str().to_string())).collect()
}

pub fn run(stdin: &[u8]) -> Out {
    let Ok(Value::Object(data)) = serde_json::from_slice::<Value>(stdin) else { return Out::quiet() };
    let Some(cmd) = data.get("tool_input").and_then(|t| t.get("command")).and_then(Value::as_str).filter(|c| !c.is_empty()) else { return Out::quiet() };
    let hits = home_root_targets(&strip_msg(cmd));
    if hits.is_empty() { return Out::quiet() }
    let targets = hits.iter().take(4).map(|t| format!("~/{t}")).collect::<Vec<_>>().join(", ");
    let t = texts("file_location_reminder", EMBEDDED);
    Out { stdout: context_json("PreToolUse", &fill(&text(&t, "reminder"), &[("targets", &targets)]), ""), ..Out::quiet() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_root_only() {
        assert_eq!(home_root_targets("nohup python x.py > ~/run.log 2>&1 &"), ["run.log"]);
        assert_eq!(home_root_targets("ssh h 'cmd >> $HOME/a.txt; curl -o ~/b --output=~/c --output  ~/d'"), ["a.txt", "b", "c", "d"]);
        assert!(home_root_targets("x > ~/exo/a.log; y > ~/.bashrc; z > ~/archive/e/f; cat ~/a").is_empty());
        assert_eq!(home_root_targets("x > ~/exotic.log"), ["exotic.log"]);
    }

    #[test]
    fn messages_are_not_scanned() {
        assert_eq!(strip_msg(r#"git commit -m "x > ~/a" --message='y' -F 'z'"#), "git commit -m --message -F");
        assert!(home_root_targets(&strip_msg(r#"git commit -m "echo > ~/a""#)).is_empty());
    }
}
