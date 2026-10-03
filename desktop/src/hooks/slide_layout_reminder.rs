//! `.claude/hooks/slide_layout_reminder.py`: before Edit/Write/MultiEdit on an internal `*_Slide.md` deck, the two
//! layout rules that only show once the deck is rendered, and how many of its figure pages use the two-column layout.
//! The Python wrote it to stderr with exit 0, which Claude Code shows to nobody; this one answers as additionalContext.

use super::pycompat::{basename, context_json, fill, read_text};
use super::{text, texts, Out};
use serde_json::Value;
use std::path::Path;

const EMBEDDED: &str = include_str!("texts/slide_layout_reminder.json");

/// (pages holding a figure, of those how many use the two-column grid); the first part (frontmatter and cover) skipped.
pub fn page_stats(body: &str) -> (usize, usize) {
    let figs: Vec<&str> = body.split("\n---\n").skip(1).filter(|p| p.contains("![[")).collect();
    (figs.len(), figs.iter().filter(|p| p.contains("drag=\"96 10\"")).count())
}

pub fn run(stdin: &[u8]) -> Out {
    let Ok(Value::Object(data)) = serde_json::from_slice::<Value>(stdin) else { return Out::quiet() };
    let ti = data.get("tool_input");
    let field = |k| ti.and_then(|t| t.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty());
    let fp = field("file_path").or_else(|| field("path")).unwrap_or("");
    let base = basename(fp);
    if !base.ends_with("_Slide.md") { return Out::quiet() }
    let t = texts("slide_layout_reminder", EMBEDDED);
    let mut lines = vec![fill(&text(&t, "header"), &[("file", base)])];
    lines.extend(["rule_grid_heading", "rule_font_size", "rule_two_column", "rule_whole_deck"].map(|k| text(&t, k)));
    if let Some((figs, two)) = read_text(Path::new(fp)).map(|b| page_stats(&b)).filter(|s| s.0 > 0) {
        let state = text(&t, if two == figs { "state_all" } else if two == 0 { "state_none" } else { "state_half" });
        lines.push(fill(&text(&t, "status"), &[("figures", &figs.to_string()), ("two_column", &two.to_string()), ("state", &state)]));
    }
    Out { stdout: context_json("PreToolUse", &lines.join("\n"), ""), ..Out::quiet() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats() {
        let deck = "---\nbg: x\n---\n# cover ![[c.png]]\n---\n![[a.png]] <grid drag=\"96 10\">\n---\ntext\n---\n![[b.png]]\n";
        assert_eq!(page_stats(deck), (3, 1));   // the Python drops only the part before the first `---`: the cover counts
        assert_eq!(page_stats("no pages ![[a.png]]"), (0, 0));
    }
}
