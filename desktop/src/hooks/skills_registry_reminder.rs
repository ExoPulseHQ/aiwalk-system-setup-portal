//! `.claude/hooks/skills_registry_reminder.py`: at SessionStart, the vault's System/Skills_Registry.md table as a
//! checklist (first 40 rows), so the agent offers a registered skill instead of working ad hoc. Reads no stdin.

use super::pycompat::{context_json, fill, py_re, py_strip, read_text};
use super::{text, texts, Out};
use std::path::PathBuf;

const EMBEDDED: &str = include_str!("texts/skills_registry_reminder.json");

/// (skill, description cut to 60 characters) for each table row that names a skill in backticks.
pub fn rows(registry: &str) -> Vec<(String, String)> {
    let re = py_re(r"\A\|\s*\*{0,2}`([^`]+)`\*{0,2}[^|]*\|([^|]+)\|");
    registry.split_inclusive('\n').filter_map(|line| {
        let m = re.captures(line).ok()??;
        Some((m[1].to_string(), py_strip(&m[2]).chars().take(60).collect()))
    }).collect()
}

pub fn run(_stdin: &[u8]) -> Out {
    let reg = super::project_dir().unwrap_or_else(|| PathBuf::from(".")).join("System").join("Skills_Registry.md");
    let Some(rows) = read_text(&reg).map(|r| rows(&r)).filter(|r| !r.is_empty()) else { return Out::quiet() };
    let t = texts("skills_registry_reminder", EMBEDDED);
    let list = rows.iter().take(40).map(|(s, d)| fill(&text(&t, "row"), &[("skill", s), ("description", d)])).collect::<Vec<_>>().join("\n");
    Out { stdout: context_json("SessionStart", &fill(&text(&t, "context"), &[("rows", &list)]), ", \"suppressOutput\": true"), ..Out::quiet() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_rows() {
        let reg = "| Skill | What |\n|---|---|\n| **`ars-full`** (plugin) | Full pipeline  |x|\n|`a`|b|\n| plain | no backticks |\n";
        assert_eq!(rows(reg), [("ars-full".into(), "Full pipeline".into()), ("a".to_string(), "b".to_string())]);
        assert_eq!(rows(&format!("| `x` | {} |", "字".repeat(70)))[0].1.chars().count(), 60);
    }
}
