#!/usr/bin/env bash
# Parity of `aiwalk-setup hook writing_style_reminder` with the vault's .claude/hooks/writing_style_reminder.py.
#
#   hooks_parity_b.sh <binary> <vault> [work dir]
#
# Builds a fake vault in the work dir (the real vault's System/vault_rules.json plus fixture notes), then feeds the
# same hook JSON, with the same environment (CLAUDE_PROJECT_DIR = the fake vault, process cwd = the fake vault,
# TMPDIR = a fresh folder per implementation and per sequence), to `python3 <vault>/.claude/hooks/
# writing_style_reminder.py` and to `<binary> hook writing_style_reminder`. Each sequence runs once under Python
# alone and once under Rust alone; stdout, stderr, exit code and the state files left in TMPDIR (names and bytes) must
# be identical step by step. Mixed sequences then share one TMPDIR between the two implementations (Python first and
# Rust on the retry, and the other way round) and must answer exactly as the Python-only run did.
# The real vault is only read. Exit 0 when every step is identical, 1 otherwise.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <binary> <vault> [work dir]" >&2; exit 2; }
BIN=$(realpath "$1"); VAULT=$(realpath "$2"); WORK=${3:-$(mktemp -d)}
mkdir -p "$WORK"; WORK=$(realpath "$WORK")
exec python3 - "$BIN" "$VAULT" "$WORK" <<'PY'
import json, os, shutil, subprocess, sys

BIN, VAULT, WORK = sys.argv[1:4]
PYHOOK = os.path.join(VAULT, ".claude", "hooks", "writing_style_reminder.py")
V = os.path.join(WORK, "vault")
L1 = os.path.join(V, "L1_Sensing")
OUT = os.path.join(WORK, "outside")
SCRATCH = os.path.join(WORK, "scratchpad")
for d in (V, OUT, SCRATCH, os.path.join(WORK, "tmp")):
    shutil.rmtree(d, ignore_errors=True)

def put(rel, text="", root=V):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8", newline="") as f:
        f.write(text)

os.makedirs(os.path.join(V, "System"))
shutil.copy(os.path.join(VAULT, "System", "vault_rules.json"), os.path.join(V, "System", "vault_rules.json"))
put("CLAUDE.md", "# rules\n")
put("top.md", "top\n")
put("L1_Sensing/_L1_Sensing.md", "---\ntier: primary\n---\n# log\n")
put("L1_Sensing/Foo_Summary.md", "---\nparent_log: \"[[_L1_Sensing]]\"\n---\nbody\n")
put("L1_Sensing/Bar_Slide.md", "slide\n")
put("L1_Sensing/Baz_Slide.html", "<p>slide</p>\n")
put("L1_Sensing/Qux_Report.md", "---\nreport_type: technical\n---\n")
put("L1_Sensing/guessed.md", "---\nparent_log: \"[[_L1_Sensing]]\"\n---\nbody\n")
put("L1_Sensing/untyped.md", "no frontmatter\n")
put("L1_Sensing/中文筆記.md", "沒有 frontmatter\n")
put("L1_Sensing/crlf.md", "---\r\nparent_log: x\r\n---\r\nbody\r\n")
put("L1_Sensing/README_like_README.md", "readme\n")
put("L1_Sensing/fig.png", "png")
put("L1_Sensing/x.txt", "x")
put("L1_Sensing/sub/keep.txt", "k")
put("L2_Platform/old.md", "old\n")
put("scripts/x.py", "print()\n")
put("Papers/p.md", "paper\n")
put("Templates/T.md", "template\n")
put("System/Refbook_Style_Clarity_Grace_Index.md", "guide\n")
put("submod/.git", "gitdir: ../.git/modules/submod\n")
put("submod/inside.md", "sub\n")
put("x.md", "outside\n", OUT)

def hook(tool, ti, cwd=L1, sid="parity-session"):
    return json.dumps({"session_id": sid, "transcript_path": "/dev/null", "cwd": cwd, "permission_mode": "default",
                       "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": ti}, ensure_ascii=False).encode()
W = lambda p, content="x\n", **k: hook("Write", {"file_path": p, "content": content}, **k)
E = lambda p, **k: hook("Edit", {"file_path": p, "old_string": "a", "new_string": "b"}, **k)
ME = lambda p, **k: hook("MultiEdit", {"file_path": p, "edits": [{"old_string": "a", "new_string": "b"}]}, **k)
B = lambda c, **k: hook("Bash", {"command": c, "description": "parity"}, **k)
L = lambda n: os.path.join(L1, n)
twice = lambda name, inp: (name, [inp, inp])

SEQS = [
    ("gate: new, same again, other, first again", [W(L("A_Summary.md")), W(L("A_Summary.md")), W(L("B_Summary.md")), W(L("A_Summary.md"))]),
    twice("Write new Summary", W(L("New_Summary.md"))),
    twice("Write new Slide .md", W(L("New_Slide.md"))),
    twice("Write new Slide .html", W(L("New_Slide.html"))),
    twice("Write new Report .md", W(L("New_Report.md"))),
    twice("Write new primary log _*.md", W(L("_New_Log.md"))),
    twice("Write new untyped .md, frontmatter says summary", W(L("guess_me.md"), "---\nparent_log: \"[[_L1_Sensing]]\"\n---\n")),
    twice("Write new untyped .md, frontmatter says primary", W(L("tiered.md"), "---\ntier: primary\n---\n")),
    twice("Write new untyped .md, no frontmatter", W(L("plain.md"), "plain\n")),
    twice("Write new untyped .md, Chinese name", W(L("新筆記.md"), "內容\n")),
    twice("Write new Summary, Chinese name", W(L("步態分析_Summary.md"))),
    twice("Write new .png", W(L("new.png"))),
    twice("Write new .pdf", W(L("new.pdf"))),
    twice("Write new .json", W(L("new.json"), "{}\n")),
    twice("Write new .py", W(L("new.py"), "print()\n")),
    twice("Write new .canvas", W(L("new.canvas"), "{}\n")),
    twice("Write new top-level .md", W(os.path.join(V, "newtop.md"))),
    twice("Write new in Papers/", W(os.path.join(V, "Papers", "new.md"))),
    twice("Write new in scripts/ (skipped top)", W(os.path.join(V, "scripts", "new.py"))),
    twice("Write new in .obsidian/ (skipped dir)", W(os.path.join(V, ".obsidian", "new.json"))),
    twice("Write new in submodule folder", W(os.path.join(V, "submod", "new.md"))),
    twice("Write new outside the vault", W(os.path.join(OUT, "new_Summary.md"))),
    twice("Write new under /tmp/", W("/tmp/parity-b-never-made_Summary.md")),
    twice("Write new under a scratchpad", W(os.path.join(SCRATCH, "x_Summary.md"))),
    twice("Write new, relative path", W("rel_new.md")),
    twice("Write new, relative ../ path", W("../L2_Platform/rel_Summary.md")),
    twice("Write new, ~ path", W("~/parity-b-never-made.md")),
    twice("Write existing Summary", W(L("Foo_Summary.md"))),
    twice("Write existing untyped .md", W(L("untyped.md"))),
    ("Edit existing Summary", [E(L("Foo_Summary.md"))]),
    ("Edit existing Slide .md", [E(L("Bar_Slide.md"))]),
    ("Edit existing Slide .html", [E(L("Baz_Slide.html"))]),
    ("Edit existing Report", [E(L("Qux_Report.md"))]),
    ("Edit existing primary log", [E(L("_L1_Sensing.md"))]),
    ("Edit existing guessed .md", [E(L("guessed.md"))]),
    ("Edit existing CRLF guessed .md", [E(L("crlf.md"))]),
    twice("Edit existing untyped .md (once per session)", E(L("untyped.md"))),
    ("Edit existing untyped .md, other session", [E(L("untyped.md")), E(L("untyped.md"), sid="other-session")]),
    twice("Edit existing Chinese untyped .md", E(L("中文筆記.md"))),
    ("Edit existing _README.md", [E(L("README_like_README.md"))]),
    ("Edit existing .png", [E(L("fig.png"))]),
    ("Edit registered guide", [E(os.path.join(V, "System", "Refbook_Style_Clarity_Grace_Index.md"))]),
    ("Edit in Templates/", [E(os.path.join(V, "Templates", "T.md"))]),
    ("Edit in Papers/", [E(os.path.join(V, "Papers", "p.md"))]),
    ("Edit top-level .md", [E(os.path.join(V, "top.md"))]),
    ("Edit in submodule folder", [E(os.path.join(V, "submod", "inside.md"))]),
    ("Edit outside the vault", [E(os.path.join(OUT, "x.md"))]),
    ("Edit a missing untyped .md", [E(L("missing.md"))]),
    ("MultiEdit Summary", [ME(L("Foo_Summary.md"))]),
    twice("MultiEdit untyped .md", ME(L("untyped.md"))),
    ("Read tool on untyped .md", [hook("Read", {"file_path": L("untyped.md")})]),
    twice("Bash cp to new file", B("cp x.txt copied.md")),
    twice("Bash cp into a folder", B(f"cp {L('x.txt')} {L('fig.png')} sub")),
    twice("Bash mv", B("mv x.txt moved.md")),
    twice("Bash install", B("install -m 644 x.txt installed.sh")),
    twice("Bash touch, absolute", B(f"touch {L('touched.md')}")),
    twice("Bash touch, Chinese name", B("touch 觸碰_Summary.md")),
    twice("Bash tee", B("echo hi | tee -a teed.md")),
    twice("Bash > redirect", B("echo hi > redirected.md")),
    twice("Bash >> redirect", B("echo hi >> appended.md")),
    twice("Bash python open for write", B("python3 -c \"open('py_made.md','w').write('x')\"")),
    twice("Bash heredoc into new file", B("cat > here_Summary.md <<'EOF'\nbody with > and 'quotes'\nEOF")),
    twice("Bash heredoc python open", B("python3 - <<'PYX'\nopen('from_heredoc.md', 'w').write('x')\nPYX")),
    twice("Bash heredoc tail kept", B("git commit -F - <<'MSG' && touch tail_new.md\nmsg > x\nMSG")),
    twice("Bash cd then relative target", B("cd ../L2_Platform && touch cd_new.md")),
    twice("Bash cd to scratch then target", B(f"cd {SCRATCH} && echo x > preview.html")),
    twice("Bash two cds", B(f"cd {SCRATCH} && cd {L1} && touch twocd.md")),
    twice("Bash several new targets", B("touch one.md two.png && echo > three.json")),
    ("Bash touch existing file", [B("touch untyped.md")]),
    ("Bash ls", [B("ls -la")]),
    ("Bash 2>&1 and /dev/null", [B("make 2>&1 > /dev/null")]),
    ("Bash quoted > in prose", [B("git commit -m 'a > b.md'")]),
    ("Bash => arrow", [B("echo x=>y.md")]),
    ("Bash ssh", [B("ssh host 'echo > remote.md'")]),
    ("Bash env-prefixed ssh", [B("FOO=1 ssh host touch remote.md")]),
    ("Bash $VAR target", [B("touch $S/x.md")]),
    ("Bash target outside vault", [B(f"touch {OUT}/n.md")]),
    ("Bash target in scripts/", [B(f"touch {V}/scripts/n.py")]),
    ("Bash unbalanced quote", [B("touch 'unclosed.md")]),
    ("Bash cwd outside vault, relative target", [B("touch rel.md", cwd=OUT)]),
    twice("Bash touch -d takes its value as a file", B("touch -d 2020-01-01 dated.md")),
    ("Bash quoted file name", [B("touch \"quoted name.md\"")]),
    twice("Bash tee with an input redirect", B("tee teed2.md < x.txt")),
    twice("Write file name ending in a newline", W(L("nl_Summary.md\n"))),
    twice("Write backslash path", W("L1_Sensing\\back_Summary.md")),
    ("Bash empty command", [hook("Bash", {})]),
    ("empty stdin", [b""]),
    ("not JSON", [b"not json"]),
    ("JSON array", [b"[]"]),
    ("JSON null", [b"null"]),
    ("JSON {}", [b"{}"]),
    ("tool_input a string", [b'{"tool_name": "Write", "tool_input": "x"}']),
    ("file_path a number", [b'{"tool_name": "Write", "tool_input": {"file_path": 5}}']),
    ("session_id null, new file", [json.dumps({"session_id": None, "cwd": L1, "tool_name": "Write", "tool_input": {"file_path": L("nullsess.md")}}).encode()] * 2),
    ("no cwd field, relative path", [json.dumps({"session_id": "s", "tool_name": "Write", "tool_input": {"file_path": "L1_Sensing/nocwd_Summary.md"}}).encode()] * 2),
    ("path instead of file_path", [hook("Write", {"path": L("viapath_Summary.md")})] * 2),
    ("content not a string", [hook("Write", {"file_path": L("badcontent.md"), "content": 5})] * 2),
]
MIXED = ["gate: new, same again, other, first again", "Write new Summary", "Write new untyped .md, no frontmatter",
         "Edit existing untyped .md (once per session)", "Bash touch, absolute", "Bash heredoc into new file",
         "Write new .png", "Bash several new targets", "Edit existing Chinese untyped .md"]

def run(impl, stdin, tmpdir):
    env = dict(os.environ, CLAUDE_PROJECT_DIR=V, TMPDIR=tmpdir, PYTHONDONTWRITEBYTECODE="1")
    for k in ("TEMP", "TMP"):
        env.pop(k, None)
    cmd = ["python3", PYHOOK] if impl == "P" else [BIN, "hook", "writing_style_reminder"]
    r = subprocess.run(cmd, input=stdin, capture_output=True, cwd=V, env=env)
    return r.stdout, r.stderr, r.returncode

def state(tmpdir):
    out = []
    for dp, _, fs in os.walk(tmpdir):
        for f in fs:
            p = os.path.join(dp, f)
            out.append((os.path.relpath(p, tmpdir), open(p, "rb").read()))
    return sorted(out)

n = [0]
def fresh():
    n[0] += 1
    d = os.path.join(WORK, "tmp", str(n[0]))
    os.makedirs(d)
    return d

def short(r):
    o, e, c = r
    return f"exit {c}, stdout {o[:160]!r}{'...' if len(o) > 160 else ''}, stderr {e[-160:]!r}"

inputs = same = 0
diffs = []
def compare(label, ref, got):
    global inputs, same
    inputs += 1
    if ref == got:
        same += 1
    else:
        diffs.append((label, short(ref), short(got)))

reference = {}
for name, steps in SEQS:
    tp, tr = fresh(), fresh()
    refs = []
    for i, s in enumerate(steps):
        p, r = run("P", s, tp), run("R", s, tr)
        refs.append(p)
        compare(f"{name} [step {i + 1}]", p, r)
        print(f"{'same' if p == r else 'DIFF'}  {name} [step {i + 1}]: {short(p)}")
    sp, sr = state(tp), state(tr)
    if sp != sr:
        diffs.append((f"{name} [state files]", repr(sp), repr(sr)))
    print(f"{'same' if sp == sr else 'DIFF'}  {name} [state files: {len(sp)}]")
    reference[name] = (steps, refs, sp)

for name in MIXED:
    steps, refs, sp = reference[name]
    for pattern in ("PR", "RP"):
        t = fresh()
        for i, s in enumerate(steps):
            impl = pattern[i % 2]
            got = run(impl, s, t)
            compare(f"{name} mixed {pattern} [step {i + 1} by {impl}]", refs[i], got)
            print(f"{'same' if got == refs[i] else 'DIFF'}  {name} mixed {pattern} [step {i + 1} by {impl}]")
        st = state(t)
        if st != sp:
            diffs.append((f"{name} mixed {pattern} [state files]", repr(sp), repr(st)))

print(f"\n{inputs} inputs in {len(SEQS)} sequences + {2 * len(MIXED)} mixed runs: {same} identical, {len(diffs)} differ")
for label, p, r in diffs:
    print(f"\nDIFF {label}\n  python: {p}\n  rust:   {r}")
sys.exit(1 if diffs else 0)
PY
