//! What the ported hooks need to match Python byte for byte: `json.dumps` output, `{name}` placeholders filled in
//! one pass, Python `re` character classes for fancy-regex, and the text Python reads from a file or a match.

use fancy_regex::Regex;

/// A JSON string as Python's json.dumps writes it: ensure_ascii, so every character outside space..~ is a \u escape
/// (UTF-16 pairs above the BMP), lowercase hex.
pub fn dumps_str(s: &str) -> String {
    let mut out = String::new();
    for c in serde_json::to_string(s).unwrap().chars() {
        if (c as u32) < 0x7f { out.push(c) } else { for u in c.encode_utf16(&mut [0; 2]) { out += &format!("\\u{u:04x}") } }
    }
    out
}

/// `print(json.dumps({"hookSpecificOutput": {"hookEventName": event, "additionalContext": ctx}, **extra}))`;
/// `extra` is the rest of the object, already written (`, "suppressOutput": true`), or "".
pub fn context_json(event: &str, ctx: &str, extra: &str) -> String {
    format!("{{\"hookSpecificOutput\": {{\"hookEventName\": {}, \"additionalContext\": {}}}{extra}}}\n", dumps_str(event), dumps_str(ctx))
}

/// `{key}` in `t` replaced by its value, in one pass: a value that itself holds `{key}` stays as it is.
pub fn fill(t: &str, vars: &[(&str, &str)]) -> String {
    let (mut out, mut rest) = (String::new(), t);
    'scan: while let Some(i) = rest.find('{') {
        out += &rest[..i];
        for (k, v) in vars {
            if let Some(after) = rest[i + 1..].strip_prefix(*k).and_then(|r| r.strip_prefix('}')) { out += v; rest = after; continue 'scan }
        }
        out.push('{');
        rest = &rest[i + 1..];
    }
    out + rest
}

/// Python's `\w` (str.isalnum() or `_`) as class members, and `\s` (str.isspace(): Unicode White_Space plus \x1c-\x1f).
const W: &str = r"\p{L}\p{N}_";
const S: &str = r"\s\x1c-\x1f";

/// A Python `re` pattern on str, made to mean the same in fancy-regex: `\w`, `\s`, `\S` and `\b` become explicit
/// classes, since the Rust ones differ on marks, connector punctuation, `²` and \x1c-\x1f.
pub fn py_re(p: &str) -> Regex {
    let (mut out, mut class, mut it) = (String::new(), false, p.chars());
    while let Some(c) = it.next() {
        match (c, class) {
            ('\\', _) => match (it.next(), class) {
                (Some('w'), true) => out += W,
                (Some('s'), true) => out += S,
                (Some('w'), false) => out += &format!("[{W}]"),
                (Some('s'), false) => out += &format!("[{S}]"),
                (Some('S'), false) => out += &format!("[^{S}]"),
                (Some('b'), false) => out += &format!("(?:(?<=[{W}])(?![{W}])|(?<![{W}])(?=[{W}]))"),
                (Some(e), _) => { out.push('\\'); out.push(e) }
                (None, _) => out.push('\\'),
            },
            ('[', false) => { class = true; out.push(c) }
            (']', true) if !out.ends_with('[') && !out.ends_with("[^") => { class = false; out.push(c) }
            _ => out.push(c),
        }
    }
    Regex::new(&out).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// A file's text as Python's open(encoding="utf-8") reads it: universal newlines, so \r\n and \r are \n.
/// None where Python raises (missing, a directory, not UTF-8).
pub fn read_text(path: &std::path::Path) -> Option<String> {
    Some(std::fs::read_to_string(path).ok()?.replace("\r\n", "\n").replace('\r', "\n"))
}

/// str.strip(): Python's whitespace, which counts \x1c-\x1f too.
pub fn py_strip(s: &str) -> &str { s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)) }

/// os.path.basename.
pub fn basename(p: &str) -> &str { p.rsplit(|c| c == '/' || (cfg!(windows) && c == '\\')).next().unwrap_or("") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_like_python() {
        assert_eq!(dumps_str("a\"\\\n\t\x01\x7f é ⚠ 😀"), concat!(r#""a\"\\\n\t\u0001\u007f "#, r"\u00e9 \u26a0 \ud83d\ude00", "\""));
        assert_eq!(context_json("E", "x", ", \"suppressOutput\": true"),
                   "{\"hookSpecificOutput\": {\"hookEventName\": \"E\", \"additionalContext\": \"x\"}, \"suppressOutput\": true}\n");
    }

    #[test]
    fn fill_is_one_pass() {
        assert_eq!(fill("{a}-{b}-{c}-{", &[("a", "{b}"), ("b", "2")]), "{b}-2-{c}-{");
    }

    #[test]
    fn classes_are_pythons() {
        assert!(py_re(r"\bab\b").is_match("x ab.").unwrap());
        assert!(!py_re(r"\bab\b").is_match("xab").unwrap());
        assert!(!py_re(r"ab\b").is_match("ab²").unwrap());        // ² is \w in Python, not in Rust
        assert!(py_re(r"ab\b").is_match("ab\u{301}").unwrap());    // a combining mark is not \w in Python
        assert!(py_re(r"a\sb").is_match("a\x1cb").unwrap());
        assert!(py_re(r"[^\s;]+x").is_match("ux").unwrap());
        assert!(py_re(r"[\w.\-]+$").is_match("a.b-c").unwrap());
    }

    #[test]
    fn strip_and_basename() {
        assert_eq!(py_strip("\x1f a \u{3000}"), "a");
        assert_eq!(basename("a/b_Slide.md"), "b_Slide.md");
        assert_eq!(basename("a/"), "");
    }
}
