#!/usr/bin/env bash
# Parity check: `aiwalk-setup hook <name>` against the vault's `.claude/hooks/<name>.py`, for file_location_reminder,
# training_run_reminder, slide_layout_reminder and skills_registry_reminder.
#
#   desktop/tests/hooks_parity_c.sh <aiwalk-setup binary> <vault> [work dir]
#
# Each case feeds the same stdin, with the same CLAUDE_PROJECT_DIR, HOME and cwd, to `python3 <vault>/.claude/hooks/
# <name>.py` and to `<binary> hook <name>`, and compares stdout, stderr and exit code byte for byte. Every case is
# printed with both results (exit code, stdout length and hash, last line of stderr). Nothing is written in the vault:
# decks and registries are made in the work dir, which is deleted at the end. Exit 1 when any case differs.
set -u
BIN=$(readlink -f "$1"); VAULT=$(readlink -f "$2"); TMP=${3:-/media/eddlai/DATA/tmp-hooks-c-test}
rm -rf "$TMP"; mkdir -p "$TMP/home" "$TMP/cwd"
unset CLAUDE_PROJECT_DIR
export HOME=$TMP/home
fail=0; n=0; diffs=0

row() { printf '| %s | %s | %s | %s |\n' "$1" "$2" "$3" "$4"; }
summ() {   # exit code, stdout file, stderr file -> one cell
  local s="exit $1"
  if [ -s "$2" ]; then s="$s, stdout $(wc -c <"$2")B $(md5sum <"$2" | cut -c1-8)"; else s="$s, no stdout"; fi
  [ -s "$3" ] && s="$s, stderr $(wc -c <"$3")B: $(tail -1 "$3" | tr '|' '/' | cut -c1-60)"
  printf '%s' "$s"
}
# hook NAME CASE PROJECT CWD STDIN: PROJECT "-" leaves CLAUDE_PROJECT_DIR unset, "''" sets it empty.
hook() {
  local name=$1 label=$2 proj=$3 cwd=$4 input=$5 env=() pc rc
  case "$proj" in -) ;; "''") env=(CLAUDE_PROJECT_DIR=) ;; *) env=(CLAUDE_PROJECT_DIR="$proj") ;; esac
  printf '%s' "$input" > "$TMP/in"
  (cd "$cwd" && env "${env[@]}" python3 "$VAULT/.claude/hooks/$name.py" <"$TMP/in" >"$TMP/po" 2>"$TMP/pe"); pc=$?
  (cd "$cwd" && env "${env[@]}" "$BIN" hook "$name" <"$TMP/in" >"$TMP/ro" 2>"$TMP/re"); rc=$?
  n=$((n+1))
  if cmp -s "$TMP/po" "$TMP/ro" && cmp -s "$TMP/pe" "$TMP/re" && [ "$pc" = "$rc" ]; then same=same; else same=DIFFERENT; fail=1; diffs=$((diffs+1)); fi
  row "$n. $label" "$(summ "$pc" "$TMP/po" "$TMP/pe")" "$(summ "$rc" "$TMP/ro" "$TMP/re")" "$same"
}
# the PreToolUse JSON Claude Code sends: j TOOL KEY VALUE (a string value)
j() { python3 -c 'import json,sys; print(json.dumps({"session_id":"s","hook_event_name":"PreToolUse","cwd":"/x","tool_name":sys.argv[1],"tool_input":{sys.argv[2]:sys.argv[3]}}))' "$@"; }
b() { j Bash command "$1"; }
section() { echo; echo "## $1 ($2)"; row case python rust result; row --- --- --- ---; n=0; diffs=0; }
total() { echo; echo "$1: $n cases, $((n-diffs)) identical, $diffs different"; }
malformed() {   # the input every hook has to survive: name, project, cwd
  hook "$1" "malformed JSON" "$2" "$3" '{"tool_name": "Bash",'
  hook "$1" "empty stdin" "$2" "$3" ''
  hook "$1" "JSON array" "$2" "$3" '[]'
  hook "$1" "JSON string" "$2" "$3" '"x"'
  hook "$1" "tool_input null" "$2" "$3" '{"tool_name": "Bash", "tool_input": null}'
  hook "$1" "tool_input a string" "$2" "$3" '{"tool_name": "Bash", "tool_input": "x"}'
  hook "$1" "invalid UTF-8" "$2" "$3" "$(printf '{"tool_input": {"command": "\xff > ~/a", "file_path": "\xff_Slide.md"}}')"
}

# ---------------------------------------------------------------------------------------------------------------------
H=file_location_reminder; C=$TMP/cwd
section "$H" "PreToolUse on Bash"
hook $H "> ~/run.log" "$VAULT" "$C" "$(b 'python x.py > ~/run.log')"
hook $H ">> ~/run.log" "$VAULT" "$C" "$(b 'python x.py >> ~/run.log 2>&1')"
hook $H "> \$HOME/out.txt" "$VAULT" "$C" "$(b 'conda list > $HOME/out.txt')"
hook $H "-o ~/f.png" "$VAULT" "$C" "$(b 'curl -o ~/f.png https://x')"
hook $H "--output ~/f, --output=~/g" "$VAULT" "$C" "$(b 'tool --output ~/f; tool --output=~/g')"
hook $H "nohup ... > ~/train.log &" "$VAULT" "$C" "$(b 'nohup python -m deprl.main c.yaml > ~/train.log 2>&1 &')"
hook $H "ssh-wrapped > ~/x" "$VAULT" "$C" "$(b "ssh ntk@dragon 'cd ~/ExoPulse && nohup python run.py > ~/nohup_run.out 2>&1 &'")"
hook $H "ssh-wrapped \$HOME" "$VAULT" "$C" "$(b 'ssh ntk@horse "pip freeze > $HOME/freeze.txt"')"
hook $H "five targets, four listed" "$VAULT" "$C" "$(b 'a > ~/1; b > ~/2; c > ~/3; d > ~/4; e > ~/5')"
hook $H "exotic.log (not exo/)" "$VAULT" "$C" "$(b 'x > ~/exotic.log')"
hook $H "canonical ~/ExoPulse/depRL/logs (not allowed list)" "$VAULT" "$C" "$(b 'x > ~/ExoPulse/depRL/logs/2026-10/a.log')"
hook $H "allowed ~/exo/ ~/archive/ ~/logs/ ~/thesis_results/" "$VAULT" "$C" "$(b 'a > ~/exo/a.log; b > ~/archive/env_snapshots/2026-10-03/c.txt; c > ~/logs/x; d > ~/thesis_results/figures/staging/a.png')"
hook $H "allowed dotfile ~/.bashrc" "$VAULT" "$C" "$(b 'echo x >> ~/.bashrc')"
hook $H "allowed \$HOME/SCONE/" "$VAULT" "$C" "$(b 'x > $HOME/SCONE/r.sto')"
hook $H "read from ~/ (no redirect)" "$VAULT" "$C" "$(b 'cat ~/run.log; ls ~/')"
hook $H "commit message names > ~/x" "$VAULT" "$C" "$(b 'git commit -m "write > ~/x.log"')"
hook $H "--message='...' names > ~/x" "$VAULT" "$C" "$(b "git commit --message='a > ~/x.log'")"
hook $H "> /tmp/x (absolute)" "$VAULT" "$C" "$(b 'x > /tmp/x.log')"
hook $H "\${HOME} braces (not matched)" "$VAULT" "$C" "$(b 'x > ${HOME}/a.log')"
hook $H "~/ then non-ASCII name" "$VAULT" "$C" "$(b 'x > ~/結果.txt')"
hook $H "> ~/ then \\x1c (Python \\s)" "$VAULT" "$C" "$(b "$(printf 'x >\x1c~/a.log')")"
hook $H "new doc > notes.md (Python crashes)" "$VAULT" "$C" "$(b 'echo hi > notes.md')"
hook $H "new doc > ~/notes.md (Python crashes)" "$VAULT" "$C" "$(b 'echo hi > ~/notes.md')"
hook $H "existing doc tee CLAUDE.md (Python crashes)" "$VAULT" "$C" "$(b 'cat x | tee CLAUDE.md')"
hook $H "empty command" "$VAULT" "$C" "$(b '')"
hook $H "no command key" "$VAULT" "$C" "$(j Bash description x)"
hook $H "command a number" "$VAULT" "$C" '{"tool_input": {"command": 5}}'
hook $H "NaN elsewhere in the JSON" "$VAULT" "$C" '{"tool_input": {"command": "x > ~/a"}, "x": NaN}'
malformed $H "$VAULT" "$C"
total $H

# ---------------------------------------------------------------------------------------------------------------------
H=training_run_reminder
section "$H" "PreToolUse on Bash"
hook $H "python -m deprl.main" "$VAULT" "$C" "$(b 'python -m deprl.main experiments/x.yaml')"
hook $H "python -m deprl.play" "$VAULT" "$C" "$(b 'python -m deprl.play --path runs/x')"
hook $H "eval_cli" "$VAULT" "$C" "$(b 'python tools/eval_cli.py --ckpt x')"
hook $H "sconewalk_h0918" "$VAULT" "$C" "$(b 'scone sconewalk_h0918_v1.scone')"
hook $H "sconestand_h0918" "$VAULT" "$C" "$(b 'ls results/sconestand_h0918_v1/')"
hook $H "opensim_unified_runner" "$VAULT" "$C" "$(b 'python opensim_unified_runner.py')"
hook $H "ssh-wrapped deprl.main + nohup" "$VAULT" "$C" "$(b "ssh ntk@dragon 'cd ~/ExoPulse/depRL && nohup python -m deprl.main c.yaml > logs/a.log 2>&1 &'")"
hook $H "all six entry points" "$VAULT" "$C" "$(b 'deprl.main; deprl.play; eval_cli; sconewalk_a; sconestand_b; opensim_unified_runner')"
hook $H "fixture stand_to_walk_from_scratch" "$VAULT" "$C" "$(b 'python -m deprl.play stand_to_walk_from_scratch --max_steps 1000')"
hook $H "fixture v12b_consol" "$VAULT" "$C" "$(b 'python -m deprl.play runs/v12b_consol/')"
hook $H "fixture v8_kneeankle and deepmimic_v8" "$VAULT" "$C" "$(b 'eval_cli v8_kneeankle; eval_cli deepmimic_v8/x')"
hook $H "fixture v13_emg, v9a_knee3, v15a" "$VAULT" "$C" "$(b 'eval_cli v15a v13_emg v9a_knee3')"
hook $H "fixture deepmimic_v7 and v7_8p8M" "$VAULT" "$C" "$(b 'eval_cli deepmimic_v7 v7_8p8M')"
hook $H "fixture _ev6_orig, moe_experiment/v6" "$VAULT" "$C" "$(b 'eval_cli x_ev6_orig moe_experiment/v6')"
hook $H "deepmimic_v8x (no \\b, no fixture)" "$VAULT" "$C" "$(b 'eval_cli deepmimic_v8x')"
hook $H "fixture name without an entry point" "$VAULT" "$C" "$(b 'ls runs/v12b_consol')"
hook $H "look-alike mydeprl.main_x" "$VAULT" "$C" "$(b 'python mydeprl.main_x')"
hook $H "look-alike deprl.mainx / my_eval_cli" "$VAULT" "$C" "$(b 'deprl.mainx; my_eval_cli; eval_cli2')"
hook $H "look-alike sconewalk_ alone / scene" "$VAULT" "$C" "$(b 'echo sconewalk_ scene sconewalkx')"
hook $H "deprl.main inside a commit message" "$VAULT" "$C" "$(b 'git commit -m "run deprl.main"')"
hook $H "deprl.main inside --message=" "$VAULT" "$C" "$(b 'git commit --message=deprl.main')"
hook $H "deprl.main then non-ASCII (é is \\w)" "$VAULT" "$C" "$(b 'deprl.mainé')"
hook $H "deprl.main then ² (Python \\w, not Rust's)" "$VAULT" "$C" "$(b 'deprl.main²')"
hook $H "deprl.main then a combining mark" "$VAULT" "$C" "$(b "$(printf 'deprl.main\xcc\x81')")"
hook $H "empty command" "$VAULT" "$C" "$(b '')"
hook $H "command a number" "$VAULT" "$C" '{"tool_input": {"command": 5}}'
malformed $H "$VAULT" "$C"
total $H

# ---------------------------------------------------------------------------------------------------------------------
H=slide_layout_reminder; D=$TMP/decks; mkdir -p "$D/Dir_Slide.md"
fig='![[f.png]]'; two='<grid drag="96 10" drop="2 2">'
printf -- '---\nbg: x\n---\n# cover\n---\n%s %s\n---\n%s %s\n' "$fig" "$two" "$fig" "$two" > "$D/All_Slide.md"
printf -- '---\nbg: x\n---\n# cover\n---\n%s\n---\n%s\n' "$fig" "$fig" > "$D/None_Slide.md"
printf -- '---\nbg: x\n---\n# cover\n---\n%s %s\n---\n%s\n---\ntext\n' "$fig" "$two" "$fig" > "$D/Half_Slide.md"
printf -- '---\nbg: x\n---\n# cover %s\n---\ntext\n' "$fig" > "$D/Cover_Slide.md"
printf -- '---\nbg: x\n---\n# cover\n---\ntext only\n' > "$D/NoFig_Slide.md"
printf -- '---\r\nbg: x\r\n---\r\n# cover\r\n---\r\n%s %s\r\n---\r\n%s\r\n' "$fig" "$two" "$fig" > "$D/Crlf_Slide.md"
printf -- '---\nbg: x\n---\n\xff\n---\n%s\n' "$fig" > "$D/Latin1_Slide.md"
e() { python3 -c 'import json,sys; print(json.dumps({"tool_name":sys.argv[1],"tool_input":json.loads(sys.argv[2])}))' "$@"; }
section "$H" "PreToolUse on Edit/Write/MultiEdit; writes stderr"
hook $H "Edit, all two-column" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/All_Slide.md\"}")"
hook $H "Edit, none two-column" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/None_Slide.md\"}")"
hook $H "Edit, half two-column" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/Half_Slide.md\"}")"
hook $H "MultiEdit, figure only on the cover" "$VAULT" "$C" "$(e MultiEdit "{\"file_path\": \"$D/Cover_Slide.md\"}")"
hook $H "Edit, no figure pages" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/NoFig_Slide.md\"}")"
hook $H "Edit, CRLF deck" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/Crlf_Slide.md\"}")"
hook $H "Write, new deck (missing)" "$VAULT" "$C" "$(e Write "{\"file_path\": \"$D/New_Slide.md\", \"content\": \"x\"}")"
hook $H "Edit, relative path from cwd" "$VAULT" "$D" "$(e Edit '{"file_path": "Half_Slide.md"}')"
hook $H "Edit, relative path, other cwd" "$VAULT" "$C" "$(e Edit '{"file_path": "Half_Slide.md"}')"
hook $H "path key instead of file_path" "$VAULT" "$C" "$(e Edit "{\"path\": \"$D/All_Slide.md\"}")"
hook $H "empty file_path falls to path" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"\", \"path\": \"$D/All_Slide.md\"}")"
hook $H "a directory named X_Slide.md" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/Dir_Slide.md\"}")"
hook $H "non-ASCII deck name" "$VAULT" "$C" "$(e Edit '{"file_path": "/x/週報_Slide.md"}')"
hook $H "_Slide.html (not internal)" "$VAULT" "$C" "$(e Edit '{"file_path": "/x/A_Slide.html"}')"
hook $H "_Slide.md.bak" "$VAULT" "$C" "$(e Edit '{"file_path": "/x/A_Slide.md.bak"}')"
hook $H "a Summary" "$VAULT" "$C" "$(e Edit '{"file_path": "/x/A_Summary.md"}')"
hook $H "trailing slash" "$VAULT" "$C" "$(e Edit '{"file_path": "/x/A_Slide.md/"}')"
hook $H "no file_path" "$VAULT" "$C" "$(e Edit '{"old_string": "a"}')"
hook $H "Edit, deck not UTF-8 (Python crashes)" "$VAULT" "$C" "$(e Edit "{\"file_path\": \"$D/Latin1_Slide.md\"}")"
hook $H "file_path a number" "$VAULT" "$C" '{"tool_input": {"file_path": 5}}'
malformed $H "$VAULT" "$C"
total $H

# ---------------------------------------------------------------------------------------------------------------------
H=skills_registry_reminder; P=$TMP/proj
mk() { rm -rf "$P"; mkdir -p "$P/System"; }
ss='{"session_id": "s", "hook_event_name": "SessionStart", "source": "startup"}'
section "$H" "SessionStart; reads no stdin"
mk; hook $H "no registry" "$P" "$C" "$ss"
mk; cp "$VAULT/System/Skills_Registry.md" "$P/System/"; hook $H "copy of the vault's registry" "$P" "$C" "$ss"
hook $H "the real vault" "$VAULT" "$C" "$ss"
mk; printf '# R\n\n| skill | what |\n|---|---|\n| plain | no backticks |\n| `a` |\n|x| `b` | c |\n' > "$P/System/Skills_Registry.md"; hook $H "only rows the parser skips" "$P" "$C" "$ss"
mk; python3 -c '
import sys
rows = ["| **`bold`** (plugin) | Bold name, padded   |", "|`tight`|t|", "| `long` | " + "字" * 70 + " |", "| `a` `b` | two ticks |", "x | `notfirst` | n |"]
rows += ["| `s%02d` | row %d |" % (i, i) for i in range(45)]
open(sys.argv[1], "w", encoding="utf-8").write("\n".join(rows))' "$P/System/Skills_Registry.md"
hook $H "bold, long, >40 rows, no final newline" "$P" "$C" "$ss"
mk; printf '| `crlf` | ends in CR LF |\r\n| `cr` | old Mac |\r| `x1c` |\x1c pad \x1c|\n' > "$P/System/Skills_Registry.md"; hook $H "CRLF, CR, \\x1c padding" "$P" "$C" "$ss"
mk; printf '| `a` | \xff |\n' > "$P/System/Skills_Registry.md"; hook $H "registry not UTF-8 (Python crashes)" "$P" "$C" "$ss"
mk; mkdir "$P/System/Skills_Registry.md"; hook $H "registry is a directory" "$P" "$C" "$ss"
mk; cp "$VAULT/System/Skills_Registry.md" "$P/System/"; hook $H "CLAUDE_PROJECT_DIR unset, cwd has registry" - "$P" "$ss"
hook $H "CLAUDE_PROJECT_DIR unset, cwd without" - "$C" "$ss"
hook $H "CLAUDE_PROJECT_DIR empty, cwd has registry" "''" "$P" "$ss"
mk; cp "$VAULT/System/Skills_Registry.md" "$P/System/"
hook $H "malformed stdin" "$P" "$C" '{"x'
hook $H "empty stdin" "$P" "$C" ''
hook $H "non-object stdin" "$P" "$C" '[1]'
total $H

rm -rf "$TMP"
exit $fail
