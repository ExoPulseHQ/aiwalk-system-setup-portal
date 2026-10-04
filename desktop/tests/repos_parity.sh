#!/usr/bin/env bash
# Parity check: `aiwalk-setup vault owner|plan|manifest|assemble|push|clone|pull` against scripts/exo_repos.py.
#
#   desktop/tests/repos_parity.sh <aiwalk-setup binary> <scratch clone of the vault> [work dir]
#
# The scratch clone is only read (git show, ls-files, grep, and `plan <scratch>`); it may be marked root-only.
# Everything written goes under the work dir: the Python with its deleted hook restored from history, a synthetic
# flat vault (the real rules, every route given a file, the real notes that link PDFs, made-up PDFs), its split
# made by each side, local bare remotes, and clones of them. Both sides run with the same fixed git identity and
# dates, so commits made by assemble must come out with the same hashes. Exit 1 if any case differs unexpectedly.
set -u
BIN=$(readlink -f "$1"); SCR=$(readlink -f "$2"); W=${3:-$(dirname "$SCR")/parity}
REAL=/media/eddlai/DATA/ExoPulse_docs
[ "$SCR" = "$REAL" ] && { echo "pass a scratch clone, not the vault"; exit 2; }
case "$W" in "$SCR"*|"$REAL"*) echo "the work dir must be outside the vaults"; exit 2;; esac
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t \
       GIT_AUTHOR_DATE='2026-01-01T00:00:00Z' GIT_COMMITTER_DATE='2026-01-01T00:00:00Z'
G() { git -c protocol.file.allow=always "$@"; }
rm -rf "$W"; mkdir -p "$W"
FAIL=0; ROWS=()
row() { ROWS+=("| $1 | $2 |"); [ "$2" = same ] || [[ "$2" == deliberate* ]] || FAIL=1; }
mask() { sed -e "s#$W#W#g" -e 's/-py\b/-X/g; s/-rs\b/-X/g'; }
# run <tag> <dir> <cmd...>: stdout, stderr, exit into $W/<tag>.{out,err,code}
run() { local t=$1 d=$2; shift 2; (cd "$d" && "$@") >"$W/$t.out" 2>"$W/$t.err"; echo $? >"$W/$t.code"; }
same_run() {   # same_run <py tag> <rs tag>: exit, stdout, stderr all equal (paths masked)
  cmp -s "$W/$1.code" "$W/$2.code" && cmp -s <(mask <"$W/$1.out") <(mask <"$W/$2.out") && cmp -s <(mask <"$W/$1.err") <(mask <"$W/$2.err")
}
show() { for t in "$@"; do echo "---- $t: exit $(cat "$W/$t.code")"; sed 's/^/  | /' "$W/$t.out"; sed 's/^/  ! /' "$W/$t.err"; done; }
files() { (cd "$1" && find . -path '*/.git' -prune -o -type f ! -name .exo-frozen -print0 | sort -z | xargs -0 sha256sum); }

# ---- the Python, runnable: its hook came back from history; root = $W/py
mkdir -p "$W/py/scripts" "$W/py/.claude/hooks" "$W/py/System"
git -C "$SCR" show HEAD:scripts/exo_repos.py >"$W/py/scripts/exo_repos.py"
git -C "$SCR" show 3218d5a^:.claude/hooks/writing_style_reminder.py >"$W/py/.claude/hooks/writing_style_reminder.py"
git -C "$SCR" show HEAD:System/vault_rules.json >"$W/py/System/vault_rules.json"
PY="python3 $W/py/scripts/exo_repos.py"

# ---- owner: real rules, paths of every kind
mapfile -t OWN < <(python3 - "$W/py/System/vault_rules.json" <<'EOF'
import json, sys
r = json.load(open(sys.argv[1]))
paths = ["System/_Management.md", "L3_Simulation_AI/_Sim_Log.md", "CLAUDE.md", "llms.txt", "System/vault_rules.json", "main.md",
         "L3_Simulation_AI/a.md", "Papers/x.pdf", ".gdrive-mcp/k.json", "Nope/x.md", "scripts/a.py", "L4_Deployment/Xilinx_FPGA_Note/",
         "Templates/x.md", "main.mdx", "System/Refbook_html_effectiveness/a.md"]
paths += [g["path"] for g in r.get("guides", [])][:3] + [t["create"]["template"] for t in r["types"] if t["create"].get("template")][:2]
print("\n".join(paths))
EOF
)
run owner-py "$W/py" $PY owner "${OWN[@]}"
run owner-rs "$W/py" "$BIN" vault owner "${OWN[@]}"
same_run owner-py owner-rs && row "owner (${#OWN[@]} paths, real rules)" same || { row owner DIFFERENT; show owner-py owner-rs; }

# ---- plan on the scratch vault (read only), as it is split today
run plan-py "$W" $PY plan "$SCR"
run plan-rs "$W" "$BIN" vault plan "$SCR"
same_run plan-py plan-rs && row "plan on the scratch vault (exit $(cat "$W/plan-py.code"))" same || { row "plan scratch" DIFFERENT; show plan-py plan-rs; }

# ---- the synthetic flat vault
F=$W/flat
mkdir -p "$F"; cp -a "$W/py/." "$F/"
for n in nested; do mkdir -p "$W/$n-work" && echo "# $n" >"$W/$n-work/n.md" && git -C "$W/$n-work" init -q -b main && git -C "$W/$n-work" add -A && git -C "$W/$n-work" commit -qm n && git clone -q --bare "$W/$n-work" "$W/$n.git"; done
python3 - "$F" "$W" "$SCR" <<'EOF'
import json, os, re, subprocess, sys
F, W, SCR = sys.argv[1:]
def w(rel, data):
    p = os.path.join(F, rel); os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "wb").write(data if isinstance(data, bytes) else data.encode())
rp = os.path.join(F, "System/vault_rules.json"); rules = json.load(open(rp))
rules["repos"]["nested"] = [{"path": "L3_Simulation_AI/Nested", "url": f"{W}/nested.git"},
                            {"path": ".claude/skills/up", "url": f"{W}/nested.git"},
                            {"path": "System/Missing", "url": f"{W}/missing.git"}]
json.dump(rules, open(rp, "w"), ensure_ascii=False, indent=2)
for r in rules["repos"]["routes"]:   # one file per route
    p = r["prefix"]
    if not p.startswith(".git"):
        w(p + "f.md" if p.endswith("/") else p, "route file\n")
# the real notes that link PDFs, as data
notes = subprocess.run(["git", "-C", SCR, "-c", "core.quotepath=off", "grep", "-l", "-i", r"\.pdf", "--", "*.md"], capture_output=True, text=True).stdout.split("\n")
names = set()
for n in filter(None, notes):
    b = subprocess.run(["git", "-C", SCR, "show", "HEAD:" + n], capture_output=True).stdout
    w(n, b)
    names |= {os.path.basename(m.strip()) for m in re.findall(r"!?\[\[([^\]|#]+\.pdf)", b.decode("utf-8", "ignore"), re.I)}
for i, n in enumerate(sorted(names)):
    if "/" not in n and n.strip() and "\n" not in n:
        w(f"Papers/{['Linked', 'Other', 'Deep/er'][i % 3]}/{n}", b"%PDF" + b"x" * (1000 * i % 77777))
w("Papers/Sim/Big_2024.pdf", b"P" * 300000); w("Papers/A/Dup.PDF", b"a" * 1234); w("Papers/B/Dup.PDF", b"b" * 4321)
w("Papers/中文 論文.pdf", b"c" * 2500000); w("Papers/Unlinked.pdf", b"u"); w("Papers/Never.pdf", b"n" * 499999); w("Papers/gone.pdf", b"g"); w("Papers/p.md", "[[Big_2024.pdf]]")
w("Papers/img.png", b"\x89PNG"); w("Nowhere/x.md", "an orphan")
w("L3_Simulation_AI/n.md", "see ![[Big_2024.pdf]] and [[Sim/Big_2024.pdf|b]] [[Dup.PDF#p]] [[ Unlinked.pdf ]]")
w("L4_Deployment/bad.md", b"\xff\xfe[[Dup.PDF]] \xe4\xb8 [[\xe4\xb8\xad\xe6\x96\x87 \xe8\xab\x96\xe6\x96\x87.pdf]]")
w("System/crlf.md", b"[[a\r\nb.pdf]]\r\n[[Big_2024.pdf]]\r\n")
w(".gdrive-mcp/k.json", "{}")
EOF
G -C "$F" init -q -b main && git -C "$F" config user.name t && git -C "$F" config user.email t@t
G -C "$F" add -A && G -C "$F" commit -qm flat && rm "$F/Papers/gone.pdf"
echo "fixture: $(git -C "$F" ls-files | wc -l) files, $(git -C "$F" ls-files 'Papers/*' | grep -ic '\.pdf$') PDFs"

# ---- plan on the flat vault (orphans: exit 1)
cp -a "$F" "$W/flat-plan"
run plan2-py "$W" $PY plan "$W/flat-plan"
run plan2-rs "$W" "$BIN" vault plan "$W/flat-plan"
same_run plan2-py plan2-rs && row "plan on the flat fixture (exit $(cat "$W/plan2-py.code"))" same || { row "plan flat" DIFFERENT; show plan2-py plan2-rs; }

# ---- manifest on the flat vault
cp -a "$F" "$W/m-py"; cp -a "$F" "$W/m-rs"
run man-py "$W" python3 "$W/m-py/scripts/exo_repos.py" manifest
run man-rs "$W/m-rs" "$BIN" vault manifest
if same_run man-py man-rs && cmp -s "$W/m-py/Papers/_manifest.json" "$W/m-rs/Papers/_manifest.json"; then row "manifest, flat vault: $(cat "$W/man-py.out")" same
else row "manifest flat" DIFFERENT; show man-py man-rs; diff "$W/m-py/Papers/_manifest.json" "$W/m-rs/Papers/_manifest.json" | head -20; fi

# ---- assemble (one remote base per side; nested from a local bare repo, one that fails)
cp -a "$F" "$W/a-src-py"; cp -a "$F" "$W/a-src-rs"
EXO_REMOTE_BASE=$W/remote-py run asm-py "$W" python3 "$W/a-src-py/scripts/exo_repos.py" assemble "$W/asm-py"
EXO_REMOTE_BASE=$W/remote-rs run asm-rs "$W/a-src-rs" "$BIN" vault assemble "$W/asm-rs"
heads() { (cd "$1" && git rev-parse HEAD && git submodule foreach -q --recursive 'echo "$sm_path $(git rev-parse HEAD)"' && cat .gitmodules); }
if same_run asm-py asm-rs && cmp -s <(heads "$W/asm-py") <(heads "$W/asm-rs") && cmp -s <(files "$W/asm-py") <(files "$W/asm-rs"); then
  row "assemble: same output, same commit hashes in book and all $(git -C "$W/asm-py" submodule status | wc -l) submodules, same files" same
else row assemble DIFFERENT; show asm-py asm-rs; diff <(heads "$W/asm-py") <(heads "$W/asm-rs") | head; diff <(files "$W/asm-py") <(files "$W/asm-rs") | head; fi
# a second assemble onto an existing folder
run asm2-py "$W" python3 "$W/a-src-py/scripts/exo_repos.py" assemble "$W/asm-py"
run asm2-rs "$W/a-src-rs" "$BIN" vault assemble "$W/asm-py"
same_run asm2-py asm2-rs && row "assemble onto an existing folder" same || { row "assemble exists" DIFFERENT; show asm2-py asm2-rs; }

# ---- manifest in the split vault: the Python sees only the book; the port reads the repo submodules too
cp -a "$W/asm-py" "$W/ms-py"; cp -a "$W/asm-py" "$W/ms-rs"
run mans-py "$W" python3 "$W/ms-py/scripts/exo_repos.py" manifest
run mans-rs "$W/ms-rs" "$BIN" vault manifest
if cmp -s "$W/m-py/Papers/_manifest.json" "$W/ms-rs/Papers/_manifest.json" && cmp -s "$W/man-py.out" "$W/mans-rs.out"; then
  row "manifest, split vault: Python \"$(cat "$W/mans-py.out")\"; port = the Python's flat-vault file" "deliberate (Python bug)"
else row "manifest split" DIFFERENT; show mans-py mans-rs; fi

# ---- push: every repo a bare remote except exo-mgmt (that push fails)
for side in py rs; do
  for r in exo-book exo-l1 exo-l2 exo-l3 exo-l4 exo-l5 exo-l6 exo-papers exo-secrets exo-trailforge; do
    git init -q --bare -b main "$W/remote-$side/$r.git" && git -C "$W/remote-$side/$r.git" config uploadpack.allowFilter true
  done
done
EXO_REMOTE_BASE=$W/remote-py run push-py "$W" $PY push "$W/asm-py"
EXO_REMOTE_BASE=$W/remote-rs run push-rs "$W" "$BIN" vault push "$W/asm-rs"
refs() { for b in "$1"/*.git; do echo "$(basename "$b") $(git -C "$b" rev-parse -q --verify main)"; done; }
if same_run push-py push-rs && cmp -s <(refs "$W/remote-py") <(refs "$W/remote-rs"); then row "push (one remote missing: FAILED line)" same
else row push DIFFERENT; show push-py push-rs; diff <(refs "$W/remote-py") <(refs "$W/remote-rs"); fi

# ---- clone with exo-l3 unreadable too
for side in py rs; do mv "$W/remote-$side/exo-l3.git" "$W/remote-$side/exo-l3.hidden"; done
run clone-py "$W" $PY clone "$W/remote-py/exo-book.git" "$W/c-py"
run clone-rs "$W" "$BIN" vault clone "$W/remote-rs/exo-book.git" "$W/c-rs"
if same_run clone-py clone-rs && cmp -s <(files "$W/c-py") <(files "$W/c-rs"); then row "clone (2 unreadable, Papers on demand): $(grep -c . "$W/clone-py.out") lines" same
else row clone DIFFERENT; show clone-py clone-rs; diff <(files "$W/c-py") <(files "$W/c-rs") | head; fi

# ---- manifest where Papers is sparse: the Python writes an empty list, the port refuses
cp -a "$W/c-py" "$W/sp-py"; cp -a "$W/c-rs" "$W/sp-rs"
run mansp-py "$W" python3 "$W/sp-py/scripts/exo_repos.py" manifest
run mansp-rs "$W/sp-rs" "$BIN" vault manifest
[ "$(cat "$W/mansp-rs.code")" = 1 ] && cmp -s <(cat "$W/c-rs/Papers/_manifest.json" 2>/dev/null) <(cat "$W/sp-rs/Papers/_manifest.json" 2>/dev/null) \
  && row "manifest with Papers sparse: Python \"$(cat "$W/mansp-py.out")\" (overwrites the list); port exit 1, file kept" "deliberate (Python bug)" \
  || { row "manifest sparse" DIFFERENT; show mansp-py mansp-rs; }

# ---- pull: a new note in Secrets, access to exo-l1 taken away
for side in py rs; do
  echo new >"$W/asm-$side/Secrets/new.md"; G -C "$W/asm-$side/Secrets" add -A; G -C "$W/asm-$side/Secrets" commit -qm n
  G -C "$W/asm-$side/Secrets" push -q origin main; mv "$W/remote-$side/exo-l1.git" "$W/remote-$side/exo-l1.hidden"
done
run pull-py "$W" $PY pull "$W/c-py"
run pull-rs "$W" "$BIN" vault pull "$W/c-rs"
if same_run pull-py pull-rs && cmp -s <(files "$W/c-py") <(files "$W/c-rs") && [ -f "$W/c-rs/Secrets/new.md" ]; then row "pull (new note arrives, one folder lost)" same
else row pull DIFFERENT; show pull-py pull-rs; diff <(files "$W/c-py") <(files "$W/c-rs") | head; fi
[ -f "$W/c-rs/L1_Sensing/.exo-frozen" ] && [ ! -f "$W/c-py/L1_Sensing/.exo-frozen" ] && [ -z "$(git -C "$W/c-rs/L1_Sensing" status --porcelain)" ] \
  && row "pull marks the lost folder L1_Sensing/.exo-frozen, kept out of git ($(head -1 "$W/c-rs/L1_Sensing/.exo-frozen"))" "deliberate (the Python never wrote it)" \
  || row "frozen marker" DIFFERENT

# ---- pull when the book cannot fast-forward
for side in py rs; do
  echo mine >>"$W/c-$side/main.md"; G -C "$W/c-$side" commit -qam mine
  echo theirs >>"$W/asm-$side/main.md"; G -C "$W/asm-$side" commit -qam theirs; G -C "$W/asm-$side" push -q origin main
done
run pull2-py "$W" $PY pull "$W/c-py"
run pull2-rs "$W" "$BIN" vault pull "$W/c-rs"
same_run pull2-py pull2-rs && row "pull, book diverged: $(head -1 "$W/pull2-py.out" | cut -c1-60)" same || { row "pull diverged" DIFFERENT; show pull2-py pull2-rs; }

echo; echo "| case | result |"; echo "|---|---|"; printf '%s\n' "${ROWS[@]}"
exit $FAIL
