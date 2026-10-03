#!/usr/bin/env bash
# Parity check: `aiwalk-setup hook root-only-guard` and `aiwalk-setup guard ...` against the vault's
# .claude/hooks/root_only_guard.py, on the same inputs in throwaway folders.
#
#   desktop/tests/guard_parity.sh <aiwalk-setup binary> <root_only_guard.py> [work dir]
#
# Part 1 feeds each hook input (Claude Code's PreToolUse JSON) to both and compares stdout, stderr and exit code
# byte for byte. Part 2 runs install, uninstall and status under a fake HOME (never the real one) and compares the
# status word and the settings.json each writes, with the binary's command put back to the Python's so the rest of
# the file can be compared byte for byte. Status words that are meant to differ are marked "by design".
set -u
BIN=$(readlink -f "$1"); PY=$(readlink -f "$2"); TMP=${3:-/media/eddlai/DATA/tmp-guard-test}
rm -rf "$TMP"; mkdir -p "$TMP"
V=$TMP/vault; O=$TMP/other; U=$TMP/金庫
mkdir -p "$V/.claude" "$V/L1" "$O" "$U/.claude"
touch "$V/.claude/root_only" "$U/.claude/root_only"
ln -s "$V" "$O/link"
F=$V/L1/a.md
unset APPIMAGE CLAUDE_PROJECT_DIR
export GUARDV=$V
fail=0; n=0
row() { printf '| %s | %s | %s | %s |\n' "$1" "$2" "$3" "$4"; }
summ() {   # exit code, stdout, stderr -> one cell
  local s="exit $1"
  case "$2" in *'"deny"'*) s="$s, deny ($(printf '%s' "$2" | sed -E 's/.*"permissionDecisionReason": "([^ ]*) is marked.*/\1/' | sed "s#$TMP#T#"))";; "") s="$s, allow";; *) s="$s, out: $(printf '%s' "$2" | head -c 60)";; esac
  [ -n "$3" ] && s="$s, stderr: $(printf '%s' "$3" | tail -1 | head -c 70)"
  printf '%s' "$s"
}

# hook NAME PROJECT STDIN: PROJECT "-" leaves CLAUDE_PROJECT_DIR unset. Both run from $O (their os.getcwd()).
hook() {
  local name=$1 proj=$2 input=$3 env=()
  [ "$proj" != - ] && env=(CLAUDE_PROJECT_DIR="$proj")
  [ -n "${HOOKHOME:-}" ] && env+=(HOME="$HOOKHOME")
  local po pe pc ro re rc
  po=$(cd "$O" && printf '%s' "$input" | env "${env[@]}" python3 "$PY" 2>"$TMP/pe"; echo "x$?"); pc=${po##*x}; po=${po%x*}; pe=$(cat "$TMP/pe")
  ro=$(cd "$O" && printf '%s' "$input" | env "${env[@]}" "$BIN" hook root-only-guard 2>"$TMP/re"; echo "x$?"); rc=${ro##*x}; ro=${ro%x*}; re=$(cat "$TMP/re")
  n=$((n+1))
  if [ "$po" = "$ro" ] && cmp -s "$TMP/pe" "$TMP/re" && [ "$pc" = "$rc" ]; then same=same; else same=DIFFERENT; fail=1; fi
  row "$n. $name" "$(summ "$pc" "$po" "$pe")" "$(summ "$rc" "$ro" "$re")" "$same"
}
# j TOOL INPUT_JSON [CWD]: the PreToolUse JSON Claude Code sends
j() { python3 -c 'import json,sys; d={"session_id":"s","hook_event_name":"PreToolUse","tool_name":sys.argv[1],"tool_input":json.loads(sys.argv[2])}; d.update({"cwd":sys.argv[3]} if len(sys.argv)>3 else {}); print(json.dumps(d))' "$@"; }
b() { j Bash "$(python3 -c 'import json,sys; print(json.dumps({"command":sys.argv[1]}))' "$1")" "$2"; }   # b COMMAND CWD
e() { python3 -c 'import json,sys; print(json.dumps({sys.argv[1]:sys.argv[2]}))' "$1" "$2"; }

echo "## Hook"
row case python rust result; row --- --- --- ---
# the Python's --selftest
hook "Edit, rooted in the vault" "$V" "$(j Edit "$(e file_path "$F")" "$V")"
hook "Edit, rooted elsewhere" "$O" "$(j Edit "$(e file_path "$F")" "$O")"
hook "Write, relative path out of another root" "$O" "$(j Write "$(e file_path ../vault/L1/a.md)" "$O")"
hook "Edit, rooted in a vault subfolder" "$V/L1" "$(j Edit "$(e file_path "$F")" "$V")"
hook "Edit, unmarked repo" "$V" "$(j Edit "$(e file_path "$O/x.py")" "$V")"
hook "cat (read only)" "$O" "$(b "cat $F" "$O")"
hook "append redirect >>" "$O" "$(b "echo hi >> $F" "$O")"
hook "git -C vault commit" "$O" "$(b "git -C $V commit -am x" "$O")"
hook "sed -i" "$O" "$(b "sed -i s/a/b/ $F" "$O")"
hook ">> rooted in the vault" "$V" "$(b "echo hi >> $F" "$V")"
hook "heredoc text names the vault" "$O" "$(b "python3 - <<'EOF'
s = s.replace('a', 'built in $V/L1')
open(p, 'w').write(s)
EOF" "$O")"
hook "commit message names a vault file" "$O" "$(b "git commit -m 'trial at $F' && git push" "$O")"
hook "echo text names a vault file" "$O" "$(b "echo 'see $F' > $O/note.txt" "$O")"
hook "heredoc open(vault, 'w')" "$O" "$(b "python3 - <<'EOF'
open('$F', 'w').write('x')
EOF" "$O")"
hook "ls && rm -rf vault" "$O" "$(b "ls $V/.repos && rm -rf $V" "$O")"
hook "cp into vault" "$O" "$(b "cp /tmp/x $F" "$O")"
hook "cd vault then relative >" "$O" "$(b "cd $V && echo hi > L1/a.md" "$O")"
hook "git --git-dir --work-tree add" "$O" "$(b "git --git-dir $V/.repos/x.git --work-tree $V add -f L1/a.md" "$O")"
hook "2>/dev/null > other" "$O" "$(b "python3 x.py 2>/dev/null > $O/out.txt" "$O")"
hook "N=vault; cp \$N/..." "$O" "$(b "N=$V; cp /tmp/x \$N/L1/a.md" "$O")"
hook "export N=vault && cd \$N && git pull" "$O" "$(b "export N=$V && cd \$N && git pull" "$O")"
hook "N=other; cp \${N}/b.md" "$O" "$(b "N=$O; cp /tmp/x \${N}/b.md" "$O")"
# other tools and fields
hook "NotebookEdit notebook_path" "$O" "$(j NotebookEdit "$(e notebook_path "$V/n.ipynb")" "$O")"
hook "MultiEdit" "$O" "$(j MultiEdit "$(e file_path "$F")" "$O")"
hook "Read (any non-Bash tool is checked)" "$O" "$(j Read "$(e file_path "$F")" "$O")"
hook "Glob path" "$O" "$(j Glob "$(e path "$V")" "$O")"
hook "no CLAUDE_PROJECT_DIR, cwd vault" - "$(j Edit "$(e file_path "$F")" "$V")"
hook "no CLAUDE_PROJECT_DIR, cwd other" - "$(j Edit "$(e file_path "$F")" "$O")"
hook "empty CLAUDE_PROJECT_DIR" "" "$(j Edit "$(e file_path "$F")" "$O")"
hook "no cwd field (process cwd)" "$O" "$(j Write "$(e file_path ../vault/L1/a.md)")"
hook "symlink into the vault" "$O" "$(j Write "$(e file_path "$O/link/L1/b.md")" "$O")"
hook "non-ASCII guarded root" "$O" "$(j Edit "$(e file_path "$U/a.md")" "$O")"
# Bash variants
hook "tee" "$O" "$(b "echo x | tee -a $F" "$O")"
hook "2>&1 | tee" "$O" "$(b "make 2>&1 | tee $F" "$O")"
hook "mv" "$O" "$(b "mv /tmp/x $F" "$O")"
hook "rm -rf --" "$O" "$(b "rm -rf -- $V/L1" "$O")"
hook "touch, mkdir -p" "$O" "$(b "mkdir -p $V/new && touch $V/new/x" "$O")"
hook "sudo tee" "$O" "$(b "sudo tee $F < /dev/null" "$O")"
hook "sed -n (not in place)" "$O" "$(b "sed -n p $F" "$O")"
hook "sed --in-place" "$O" "$(b "sed --in-place=.bak s/a/b/ $F" "$O")"
hook "sed -Ei" "$O" "$(b "sed -Ei s/a/b/ $F" "$O")"
hook "git push, cwd vault" "$O" "$(b "git push" "$V")"
hook "git -C vault status (read verb)" "$O" "$(b "git -C $V status" "$O")"
hook "git -c k=v -C vault log" "$O" "$(b "git -c a.b=c -C $V log" "$O")"
hook "git --work-tree=vault checkout" "$O" "$(b "git --work-tree=$V checkout ." "$O")"
hook "git -C other commit" "$O" "$(b "git -C $O commit -m x" "$O")"
hook "cd other/.. relative cp" "$O" "$(b "cd $O && cp x ../vault/L1/a.md" "$O")"
hook "Path().write_text" "$O" "$(b "python3 -c \"from pathlib import Path; Path('$F').write_text('x')\"" "$O")"
hook "open(..., 'a')" "$O" "$(b "python3 -c \"open('$F', 'a').write('x')\"" "$O")"
hook "open(...) read only" "$O" "$(b "python3 -c \"print(open('$F').read())\"" "$O")"
hook "open(..., 'r')" "$O" "$(b "python3 -c \"print(open('$F', 'r').read())\"" "$O")"
hook "environment variable \$GUARDV" "$O" "$(b "cp x \$GUARDV/L1/a.md" "$O")"
HOOKHOME=$O hook "~ in cd (HOME=other)" "$O" "$(b "cd ~ && cp x link/L1/a.md" "$O")"
HOOKHOME=$O hook "N=~/link; cp \$N/x" "$O" "$(b "N=~/link; cp x \$N/L1/a.md" "$O")"
hook "<<- heredoc body has > vault" "$O" "$(b "cat <<-'X' > $O/a
	> $F
	X" "$O")"
hook "unterminated heredoc" "$O" "$(b "cat <<EOF
> $F" "$O")"
hook "unclosed quote, then >" "$O" "$(b "echo \"abc > $F" "$O")"
hook "dd of=vault (Python limit)" "$O" "$(b "dd if=/dev/zero of=$F" "$O")"
hook "script run by path (Python limit)" "$O" "$(b "$V/do.sh" "$O")"
hook "|| and ; separators" "$O" "$(b "false || true; rm $F" "$O")"
hook "backslash at end (shlex error)" "$O" "$(b "rm $F \\" "$O")"
# malformed input
hook "malformed JSON" "$O" '{"tool_name": "Edit",'
hook "empty stdin" "$O" ''
hook "JSON array" "$O" '[]'
hook "JSON string" "$O" '"x"'
hook "tool_input null" "$O" '{"tool_name": "Edit", "tool_input": null}'
hook "tool_input a string" "$O" '{"tool_name": "Edit", "tool_input": "x"}'
hook "file_path a number" "$O" '{"tool_name": "Edit", "tool_input": {"file_path": 5}}'
hook "command a number" "$O" '{"tool_name": "Bash", "tool_input": {"command": 5}}'
hook "invalid UTF-8" "$O" "$(printf '{"tool_name": "Edit", "tool_input": {"file_path": "\xff"}}')"
hook "NaN in JSON" "$O" "{\"tool_name\": \"Edit\", \"tool_input\": {\"file_path\": \"$F\"}, \"x\": NaN}"

echo
echo "## Install and status (fake HOME)"
row case python rust result; row --- --- --- ---
OWN="\"$BIN\" hook root-only-guard"
PYCMD='python3 \"$HOME/.claude/hooks/root_only_guard.py\"'
H=$TMP/home; P=$TMP/hp; R=$TMP/hr   # P: Python's home, R: the binary's
fresh() { rm -rf "$P" "$R"; mkdir -p "$P/.claude" "$R/.claude"; if [ -n "${1:-}" ]; then printf '%s' "$1" > "$P/.claude/settings.json"; cp "$P/.claude/settings.json" "$R/.claude/settings.json"; fi; }
py() { HOME=$P python3 "$PY" "$@" 2>&1; echo "exit $?"; }
rs() { HOME=$R "$BIN" guard "$@" 2>&1; echo "exit $?"; }
last() { printf '%s' "$1" | grep -v '^exit' | tail -1 | head -c 70 | tr -d '\n'; printf ', %s' "$(printf '%s' "$1" | tail -1)"; }
# the binary's settings with its command written as the Python's, against the Python's
cmp_settings() { python3 - "$P/.claude/settings.json" "$R/.claude/settings.json" "$OWN" <<'EOF'
import json, sys
p, r, own = sys.argv[1:]
try: a = open(p, encoding="utf-8").read()
except OSError: a = None
try: b = open(r, encoding="utf-8").read()
except OSError: b = None
if b is not None: b = b.replace(json.dumps(own)[1:-1], 'python3 \\"$HOME/.claude/hooks/root_only_guard.py\\"')
print("identical" if a == b else "DIFFERENT")
if a != b: open(r + ".normalized", "w", encoding="utf-8").write(b or "")
EOF
}
want() {   # NAME WORD RSOUT: a status only the binary has
  local got; got=$(printf '%s' "$3" | grep -v '^exit' | tail -1)
  row "$1" "n/a (want $2)" "$(last "$3")" "$([ "$got" = "$2" ] && echo "as wanted" || { fail=1; echo DIFFERENT; })"
}
st() {   # NAME PYOUT RSOUT [by design]
  local same; [ "$(last "$2")" = "$(last "$3")" ] && same=same || { same=${4:-DIFFERENT}; [ "$same" = DIFFERENT ] && fail=1; }
  row "$1" "$(last "$2")" "$(last "$3")" "$same"
}
OTHER='{
  "z_first": true,
  "permissions": {"allow": ["Bash(ls:*)"], "deny": []},
  "hooks": {
    "Stop": [{"hooks": [{"type": "command", "command": "echo stop"}]}],
    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo mine"}]}]
  },
  "env": {"B": "2", "A": "1"},
  "note": "中文 ok",
  "n": [1, -2, 3.5, 0.1]
}'

fresh; st "status, nothing installed" "$(py --status)" "$(rs --status)"
fresh; st "install on a home without settings.json" "$(py --install)" "$(rs --install)"
row "  settings.json written" "" "" "$(cmp_settings)"
st "  then status" "$(py --status)" "$(rs --status)"
fresh "$OTHER"; st "install, settings with other hooks and keys" "$(py --install)" "$(rs --install)"
row "  settings.json written (other hooks, key order)" "" "" "$(cmp_settings)"
bak() { printf '%s' "$OTHER" | cmp -s - "$1/.claude/settings.json.bak" && echo yes || echo no; }
row "  settings.json.bak is the old file" "$(bak "$P")" "$(bak "$R")" "$([ "$(bak "$P")" = "$(bak "$R")" ] && echo same || echo DIFFERENT)"
st "uninstall" "$(py --uninstall)" "$(rs --uninstall)"
row "  settings.json written" "" "" "$(cmp_settings)"
fresh; st "uninstall without settings.json" "$(py --uninstall)" "$(rs --uninstall)"
row "  settings.json written" "" "" "$(cmp_settings)"

# install over the Python's registration: the binary replaces it
fresh "$OTHER"; HOME=$R python3 "$PY" --install >/dev/null; HOME=$P python3 "$PY" --install >/dev/null
st "Python registered (copy current): status" "$(py --status)" "$(rs --status)" "by design: python on, binary stale"
want "  binary install over it" on "$(rs --install)"
row "  Python entries left after binary install" "" "$(grep -c root_only_guard.py "$R/.claude/settings.json")" ""
row "  guard entries after binary install" "" "$(grep -c 'root-only-guard' "$R/.claude/settings.json")" ""
row "  other hooks kept" "" "$(grep -c 'echo mine\|echo stop' "$R/.claude/settings.json")" ""
row "  Python --status on that home" "$(last "$(HOME=$R python3 "$PY" --status 2>&1; echo "exit $?")")" "" "python sees no Python entry"
want "  binary install twice" on "$(rs --install)"
row "  guard entries after second install" "" "$(grep -c 'root-only-guard' "$R/.claude/settings.json")" ""
fresh; HOME=$P python3 "$PY" --install >/dev/null; HOME=$R python3 "$PY" --install >/dev/null
printf '\n# changed\n' >> "$P/.claude/hooks/root_only_guard.py"; cp "$P/.claude/hooks/root_only_guard.py" "$R/.claude/hooks/root_only_guard.py"
st "Python registered, copy differs" "$(py --status)" "$(rs --status)"
rm "$P/.claude/hooks/root_only_guard.py" "$R/.claude/hooks/root_only_guard.py"
st "Python registered, copy gone" "$(py --status)" "$(rs --status)"
# the binary's own words
fresh; HOME=$R "$BIN" guard --install >/dev/null
mkdir -p "$TMP/elsewhere"; cp "$BIN" "$TMP/elsewhere/aiwalk-setup"
sed -i "s#$BIN#$TMP/elsewhere/aiwalk-setup#" "$R/.claude/settings.json"
want "binary registered at another existing path" stale "$(rs --status)"
rm "$TMP/elsewhere/aiwalk-setup"
want "binary registered at a path that is gone" missing-copy "$(rs --status)"
want "  install repairs it" on "$(rs --install)"
# broken settings.json: nothing is written
fresh '{"hooks": '; st "settings.json not JSON: status" "$(py --status)" "$(rs --status)"
st "settings.json not JSON: install" "$(py --install)" "$(rs --install)" "by design: same exit, different stderr"
row "  settings.json left as it was" "$(cat "$P/.claude/settings.json")" "$(cat "$R/.claude/settings.json")" ""
fresh '{"x": 1e16, "y": 12345678901234567890123}'; py --install >/dev/null; rs --install >/dev/null
row "numbers Python and serde_json print differently" "$(tr -d '\n ' < "$P/.claude/settings.json" | head -c 40)" "$(tr -d '\n ' < "$R/.claude/settings.json" | head -c 40)" "$(cmp_settings)"

echo
[ $fail = 0 ] && echo "all hook cases and status words agree (differences marked by design are expected)" || echo "DIFFERENCES FOUND (see rows marked DIFFERENT)"
exit $fail
