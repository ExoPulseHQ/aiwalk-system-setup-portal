#!/usr/bin/env bash
# Parity check: `aiwalk-setup hook git_discipline_reminder | new_repo_worktree_check | primary_log_reminder`
# against the vault's .claude/hooks/<name>.py, on the same inputs.
#
#   desktop/tests/hooks_parity_a.sh <aiwalk-setup binary> <vault> [work dir]
#
# Each case feeds the same Claude Code hook JSON on stdin to both, from the same working directory and with the same
# CLAUDE_PROJECT_DIR and HOME, and compares stdout, stderr and exit code byte for byte. The vault is only read: its
# hooks, System/vault_rules.json and, for one case, sync_ownership.py --check on one of its primary logs. Fixtures
# (repos, a worktree, fake vaults, a fake HOME) live in the work dir, which is deleted first. Identity questions go
# to GitHub read-only, as whoever gh is signed in as; under the fake HOME (and no keyring) gh is signed in as nobody.
# Cases where a difference is known and explained in the port are marked "known"; every other difference fails.
set -u
BIN=$(readlink -f "$1"); VAULT=$(readlink -f "$2"); TMP=${3:-/media/eddlai/DATA/tmp-hooks-a-test}
case "$TMP" in "$VAULT"*) echo "the work dir must be outside the vault"; exit 2;; esac
HOOKS=$VAULT/.claude/hooks
G() { git -c user.name=fixture -c user.email=fixture@example.com "$@"; }
rm -rf "$TMP"; mkdir -p "$TMP/out"
unset CLAUDE_PROJECT_DIR GH_TOKEN GITHUB_TOKEN GIT_DIR GIT_WORK_TREE

# ---- fixtures
V=$TMP/vault; P=$TMP/plain; WT=$TMP/wt; N=$TMP/nogit; FH=$TMP/fakehome; U=$TMP/金庫
mkdir -p "$V/System" "$V/scripts" "$V/L1" "$V/_dir.md" "$P/src" "$N" "$FH" "$U"
G -C "$V" init -q && G -C "$V" commit -q --allow-empty -m init
G -C "$P" init -q && G -C "$P" commit -q --allow-empty -m init
G -C "$P" worktree add -q "$WT" -b feat/x
mkdir -p "$V/sub" && G -C "$V/sub" init -q && G -C "$V/sub" commit -q --allow-empty -m init
G -C "$U" init -q && G -C "$U" commit -q --allow-empty -m init
ln -s "$P" "$TMP/plainlink"; ln -s "$V" "$TMP/vaultlink"
printf '{"machines": {"hosts": [{"host": "lab-1"}, {"host": "dragon.x"}]}}' > "$V/System/vault_rules.json"
B=$TMP/brokenvault; mkdir -p "$B/System"; printf '{"machines": ' > "$B/System/vault_rules.json"
# a stand-in for sync_ownership.py --check whose answer the log under check chooses
cat > "$V/scripts/sync_ownership.py" <<'EOF'
import sys
t = open(sys.argv[-1], encoding="utf-8").read()
if "STALE" in t: sys.exit(1)
if "CRLF" in t: sys.stderr.write("  first\r\nsecond\r\n"); sys.exit(2)
if "LONG" in t: sys.stderr.write("錯" * 150); sys.exit(2)
if "SILENT" in t: sys.exit(3)
if "FAIL" in t: sys.stderr.write("Traceback: boom\n"); sys.exit(2)
print("unchanged: x")
EOF
for k in OK STALE CRLF LONG SILENT FAIL; do printf '# 2026/10/01 t\n%s\n<!-- ownership:start -->\n' "$k" > "$V/L1/_$k.md"; done
printf '# no block\n' > "$V/L1/_Plain.md"
printf 'x\n<!-- ownership:start -->\n' > "$V/_Root.md"
printf '<!-- ownership:start -->\n\xff\xfe\n' > "$V/L1/_Bin.md"
printf 'STALE\n<!-- ownership:start -->\n' > "$TMP/_Outside.md"
printf 'note\n' > "$V/L1/Note.md"
REALLOG=$(grep -l -- '<!-- ownership:start -->' "$VAULT"/System/_*.md 2>/dev/null | head -1)

fail=0; total=0; same_n=0
row() { printf '| %s | %s | %s | %s |\n' "$1" "$2" "$3" "$4"; }
summ() {   # exit code, stdout file, stderr file -> one cell
  python3 - "$@" "$TMP" <<'EOF'
import json, sys
code, out, err, tmp = sys.argv[1], open(sys.argv[2], encoding="utf-8", errors="replace").read(), open(sys.argv[3], encoding="utf-8", errors="replace").read(), sys.argv[4]
s = f"exit {code}"
if not out: s += ", quiet"
else:
    try:
        h = json.loads(out)["hookSpecificOutput"]
        if h.get("permissionDecision") == "deny": s += ", deny: " + h["permissionDecisionReason"].split("\n")[1][:70]
        else:
            c = h["additionalContext"]
            s += f", context ({len(c)} chars): " + " / ".join(l[:14] for l in c.split("\n") if l.startswith("•")) if c.startswith("⚠ Git 規則") else f", context ({len(c)} chars): " + c.replace("\n", " ")[-60:]
    except Exception: s += ", out: " + out[:60]
if err: s += ", stderr: " + err.strip().splitlines()[-1][:60]
print(s.replace(tmp, "T").replace("|", "\\|"))
EOF
}
# hook NAME CWD PROJECT HOME STDIN [known]: PROJECT "-" leaves CLAUDE_PROJECT_DIR unset, HOME "-" keeps the real one
hook() {
  local name=$1 cwd=$2 proj=$3 home=$4 input=$5 known=${6:-} env=()
  [ "$proj" != - ] && env+=(CLAUDE_PROJECT_DIR="$proj")
  # a fake HOME also keeps gh away from the login keyring, which `gh auth token` reads even without a config
  [ "$home" != - ] && env+=(HOME="$home" DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent)
  (cd "$cwd" && printf '%s' "$input" | env "${env[@]}" python3 "$HOOKS/$name.py" >"$TMP/out/po" 2>"$TMP/out/pe"); local pc=$?
  (cd "$cwd" && printf '%s' "$input" | env "${env[@]}" "$BIN" hook "$name" >"$TMP/out/ro" 2>"$TMP/out/re"); local rc=$?
  total=$((total+1))
  local same
  if cmp -s "$TMP/out/po" "$TMP/out/ro" && cmp -s "$TMP/out/pe" "$TMP/out/re" && [ "$pc" = "$rc" ]; then same=same; same_n=$((same_n+1))
  elif [ -n "$known" ]; then same="DIFFERENT (known: $known)"
  else same=DIFFERENT; fail=1; fi
  row "$total. $CASE" "$(summ "$pc" "$TMP/out/po" "$TMP/out/pe")" "$(summ "$rc" "$TMP/out/ro" "$TMP/out/re")" "$same"
}
# the PreToolUse JSON Claude Code sends: j TOOL KEY VALUE (VALUE is JSON when it starts with @)
j() { python3 -c 'import json,sys; v=sys.argv[3]; v=json.loads(v[1:]) if v.startswith("@") else v; print(json.dumps({"session_id":"s","hook_event_name":"PreToolUse","tool_name":sys.argv[1],"tool_input":{sys.argv[2]:v}}))' "$@"; }
bash_() { j Bash command "$1"; }

echo "## git_discipline_reminder"; row case python rust result; row --- --- --- ---
g() { CASE=$1; hook git_discipline_reminder "$TMP" "${3:-$VAULT}" "${4:--}" "$2" "${5:-}"; }
g "empty stdin" ""
g "malformed JSON" '{"tool_input": {"command": "git push"'
g "non-object JSON" '["git push"]' "" - "Python raises AttributeError (traceback, exit 1); the port says nothing"
g "tool_input is a string" '{"tool_input": "git push"}' "" - "Python raises AttributeError; the port says nothing"
g "tool_input null" '{"tool_input": null}'
g "no command" '{"tool_input": {}}'
g "command is a number" '{"tool_input": {"command": 5}}' "" - "Python raises TypeError in re.sub; the port says nothing"
g "NaN elsewhere in the JSON" '{"tool_input": {"command": "git push"}, "x": NaN}' "" - "Python's json accepts NaN, serde_json does not"
g "git status" "$(bash_ 'git status && git log --oneline -3')"
g "git commit" "$(bash_ 'git commit -m "x"')"
g "add then commit" "$(bash_ 'git add . && git commit -am x')"
g "git push" "$(bash_ 'git push origin main')"
g "git merge" "$(bash_ 'git merge --no-ff feat/x')"
g "git -C merge && push" "$(bash_ 'git -C /r merge feat/x && git -C /r push origin main')"
g "message names push" "$(bash_ 'git commit -m "fix push race"')"
g "single-quoted message names merge" "$(bash_ "git commit -m 'merge stuff'")"
g "--message=push" "$(bash_ 'git commit --message=push')"
g "-m then \\x1c then a quoted push" "$(bash_ $'git commit -m\x1c"push now"')"
g "push after ; is another segment" "$(bash_ 'echo git; echo push')"
g "git log --grep=merge" "$(bash_ 'git log --grep=merge')"
g "word boundary: pushé" "$(bash_ 'git pushé')"
g "CJK message" "$(bash_ 'git commit -m "修好 推送" && git push')"
g "ssh dragon commit (deny)" "$(bash_ "ssh ntk@dragon 'cd ~/ExoPulse && git commit -m \"x\" a.py'")"
g "ssh horse git -C push (deny)" "$(bash_ 'ssh ntk@horse "git -C ~/depRL push"')"
g "ssh tiger commit && push (deny both)" "$(bash_ "ssh tiger 'git commit -am x && git push'")"
g "ssh dragon EXO_RAW=1" "$(bash_ "ssh ntk@dragon 'EXO_RAW=1 git commit -m x'")"
g "ssh dragon merge (not denied)" "$(bash_ "ssh ntk@dragon 'git -C ~/x merge feat/y'")"
g "ssh dragonfly (not a lab machine)" "$(bash_ "ssh ntk@dragonfly 'git push'")"
g "ssh ox (vault_rules.json)" "$(bash_ "ssh ntk@ox 'git push'")"
g "ssh ox, CLAUDE_PROJECT_DIR unset" "$(bash_ "ssh ntk@ox 'git push'")" - - "unset: the Python reads the vault it lives in, the port falls back to dragon/horse/tiger"
g "ssh dragon, CLAUDE_PROJECT_DIR unset" "$(bash_ "ssh ntk@dragon 'git push'")" -
g "fake vault host lab-1" "$(bash_ 'ssh me@lab-1 git push')" "$V"
g "fake vault host with a dot" "$(bash_ 'ssh dragon.x git commit -m y')" "$V"
g "fake vault: dragon not listed" "$(bash_ 'ssh ntk@dragon git push')" "$V"
g "broken vault_rules.json" "$(bash_ 'ssh ntk@horse git push')" "$B"
g "heredoc over ssh" "$(bash_ $'ssh ntk@tiger bash <<EOF\ngit push\nEOF')"
g "ssh status; local push" "$(bash_ 'ssh ntk@dragon git status; git push')"
g "gh signed out: commit" "$(bash_ 'git commit -m x')" "$VAULT" "$FH"
g "gh signed out: deny" "$(bash_ "ssh ntk@dragon 'git commit -m x'")" "$VAULT" "$FH"

echo; echo "## new_repo_worktree_check"; row case python rust result; row --- --- --- ---
w() { CASE=$1; hook new_repo_worktree_check "${3:-$TMP}" "${4:-$V}" - "$2" "${5:-}"; }
w "empty stdin" ""
w "malformed JSON" '{'
w "non-object JSON" '5' "" "" "Python raises AttributeError; the port says nothing"
w "tool_input is a list" '{"tool_input": [1]}' "" "" "Python raises AttributeError; the port says nothing"
w "file_path is a number" '{"tool_input": {"file_path": 7}}' "" "" "Python raises TypeError in abspath; the port says nothing"
w "no file_path" '{"tool_input": {"content": "x"}}'
w "file in the vault" "$(j Write file_path "$V/L1/a.md")"
w "file in a repo under the vault" "$(j Edit file_path "$V/sub/x.py")"
w "file in a plain repo" "$(j Edit file_path "$P/src/main.py")"
w "new file, missing folders" "$(j Write file_path "$P/a/b/c/new.py")"
w "path key, file_path empty" "$(python3 -c 'import json,sys; print(json.dumps({"tool_input":{"file_path":"","path":sys.argv[1]}}))' "$P/x")"
w "file in a linked worktree" "$(j Edit file_path "$WT/x.py")"
w "file outside any repo" "$(j Write file_path "$N/x.py")"
w "relative path from the repo" "$(j Edit file_path src/main.py)" "$P"
w "relative path with .." "$(j Edit file_path ../plain/src/main.py)" "$N"
w "through a symlink" "$(j Edit file_path "$TMP/plainlink/src/m.py")"
w "vault named through a symlink" "$(j Edit file_path "$V/L1/a.md")" "$TMP" "$TMP/vaultlink"
w "rooted elsewhere, editing the vault" "$(j Edit file_path "$V/L1/a.md")" "$TMP" "$N"
w "CLAUDE_PROJECT_DIR unset" "$(j Edit file_path "$V/L1/a.md")" "$TMP" -
w "non-ASCII repo path" "$(j Edit file_path "$U/筆記.md")"
w "a .. that climbs out of a missing folder" "$(j Write file_path "$P/missing/../../nogit/x.py")"

echo; echo "## primary_log_reminder"; row case python rust result; row --- --- --- ---
p() { CASE=$1; hook primary_log_reminder "${3:-$TMP}" "${4:-$V}" - "$2" "${5:-}"; }
p "empty stdin" ""
p "malformed JSON" 'nope'
p "non-object JSON" '"x"' "" "" "Python raises AttributeError; the port says nothing"
p "file_path is a list" '{"tool_input": {"file_path": ["_a.md"]}}' "" "" "Python raises TypeError in basename; the port says nothing"
p "no file_path" '{"tool_input": {}}'
p "ordinary note" "$(j Edit file_path "$V/L1/Note.md")"
p "primary log, missing file" "$(j Write file_path "$V/L1/_New.md")"
p "primary log, no ownership block" "$(j Edit file_path "$V/L1/_Plain.md")"
p "ownership: up to date" "$(j Edit file_path "$V/L1/_OK.md")"
p "ownership: stale" "$(j Edit file_path "$V/L1/_STALE.md")"
p "ownership: check fails with stderr" "$(j Edit file_path "$V/L1/_FAIL.md")"
p "ownership: check fails silently" "$(j Edit file_path "$V/L1/_SILENT.md")"
p "ownership: CRLF stderr" "$(j Edit file_path "$V/L1/_CRLF.md")"
p "ownership: 150-char CJK stderr" "$(j Edit file_path "$V/L1/_LONG.md")"
p "ownership: log at the vault root" "$(j Edit file_path "$V/_Root.md")"
p "ownership: log outside the vault" "$(j Edit file_path "$TMP/_Outside.md")"
p "ownership: not UTF-8" "$(j Edit file_path "$V/L1/_Bin.md")"
p "a folder named _dir.md" "$(j Edit file_path "$V/_dir.md")"
p "relative path" "$(j Edit file_path L1/_STALE.md)" "$V"
p "path key" "$(j MultiEdit path "$V/L1/_STALE.md")"
p "CLAUDE_PROJECT_DIR unset" "$(j Edit file_path "$V/L1/_STALE.md")" "$TMP" -
p "vault without the script" "$(j Edit file_path "$N/_x.md")" "$TMP" "$N"
p "basename ends in a newline" "$(j Write file_path $'/x/_a.md\n')"
p "not .md" "$(j Edit file_path "$V/L1/_a.mdx")"
[ -n "$REALLOG" ] && p "the vault's own log, real sync_ownership.py" "$(j Edit file_path "$REALLOG")" "$VAULT" "$VAULT"

echo; echo "$same_n of $total identical"
rm -rf "$TMP"
exit $fail
