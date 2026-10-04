#!/usr/bin/env bash
# Checks `aiwalk-setup session` on Linux or macOS: sessions that outlive their client, replay, send, title, resize,
# two clients, end, exit codes, a killed keeper, bad names, and a full-screen program's repaint when one is installed.
#   desktop/tests/session_check.sh <aiwalk-setup binary>
# Runs in a scratch XDG_RUNTIME_DIR, so it never sees or touches real sessions, and ends everything it started.
set -u
BIN=$(readlink -f "$1")
W=$(mktemp -d); export XDG_RUNTIME_DIR=$W/run; mkdir -m 700 "$XDG_RUNTIME_DIR"; mkdir "$W/cwd"
export INPUTRC=/dev/null PS1='$ '
fail=0 n=0
say() { printf '%-60s %s\n' "$1" "$2"; }
check() { if [ "$2" = "$3" ]; then say "$1" "ok"; else say "$1" "DIFFERENT: got [$2] want [$3]"; fail=1; fi; }
clean() { tr -d '\r' | sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g; s/\x1b\][^\x07]*\x07//g'; }
lines() { clean < "$1" | grep -c "^$2\$"; }
list() { "$BIN" session list; }
field() { list | grep -o "\"name\":\"$1\"[^}]*\|{[^}]*\"name\":\"$1\"[^}]*" | grep -o "\"$2\":[^,}]*" | head -1 | cut -d: -f2-; }

# a client attached through a FIFO we hold open (fd 10+n), its output in $W/out<id>; `keys <id> <text>` writes to it
declare -A CPID FD
attach() {   # attach <id> <session> [open args...]
  local id=$1 s=$2; shift 2
  mkfifo "$W/in$id"
  PTY_COLS=80 PTY_ROWS=24 "$BIN" session open "$s" "$@" < "$W/in$id" > "$W/out$id" 2>"$W/err$id" & CPID[$id]=$!
  n=$((n + 1)); FD[$id]=$((10 + n)); eval "exec ${FD[$id]}>\"$W/in$id\""
}
keys() { printf "$2" >&"${FD[$1]}"; }
detach() { { kill -9 "${CPID[$1]}"; wait "${CPID[$1]}"; } 2>/dev/null; eval "exec ${FD[$1]}>&-"; }

cleanup() {
  for id in "${!CPID[@]}"; do kill -9 "${CPID[$id]}" 2>/dev/null; done
  for s in $(list | grep -o '"name":"[^"]*"' | cut -d'"' -f4); do "$BIN" session end "$s"; done
  rm -rf "$W"
}
trap cleanup EXIT

# 1. open, type, see output; a killed client leaves the session running and listed
attach 1 t1 --cwd "$W/cwd" -- sh
sleep 0.8; keys 1 'echo he""llo\n'; sleep 0.6
check "1 output seen" "$(lines "$W/out1" hello)" "1"
detach 1; sleep 0.3
pid=$(field t1 pid)
check "1 listed after the client is killed" "$(field t1 attached)|$(field t1 cwd)|$(field t1 command)" "0|\"$W/cwd\"|[\"sh\"]"
kill -0 "$pid" 2>/dev/null && say "1 its command still runs (pid $pid)" "ok" || { say "1 its command still runs" "DIFFERENT: pid [$pid] gone"; fail=1; }

# 2. attaching again replays what was there, and the session still works
attach 2 t1; sleep 0.6
check "2 replay shows the earlier output" "$(lines "$W/out2" hello)" "1"
keys 2 'echo wor""ld2\n'; sleep 0.5
check "2 a new command works" "$(lines "$W/out2" world2)" "1"

# 3. send: pasted into bash's input, run only on Enter
attach 3 t3 -- bash --norc --noprofile -i; sleep 1
printf 'echo sent""text' | "$BIN" session send t3; check "3 send exits 0" "$?" "0"
sleep 0.6
check "3 pasted text is not run" "$(lines "$W/out3" senttext)" "0"
clean < "$W/out3" | grep -q 'echo sent""text' && say "3 pasted text is in the input line" "ok" || { say "3 pasted text is in the input line" "DIFFERENT"; fail=1; }
keys 3 '\r'; sleep 0.5
check "3 Enter runs it" "$(lines "$W/out3" senttext)" "1"
detach 3; "$BIN" session end t3

# 4. title from OSC 0
keys 2 "printf '\\\\033]0;hello\\\\007'\n"; sleep 0.5
check "4 title" "$(field t1 title)" '"hello"'

# 5. resize in the stream
keys 2 '\033]777;resize;132;50\007'; sleep 0.3; keys 2 'stty size\n'; sleep 0.5
check "5 resize in the stream" "$(clean < "$W/out2" | grep -E '^[0-9]+ [0-9]+$' | tail -1)" "50 132"

# 6. two clients mirror; killing one leaves the other working
attach 4 t1; sleep 0.6
check "6 two attached" "$(field t1 attached)" "2"
keys 2 'echo two""clients\n'; sleep 0.5
check "6 both see output" "$(lines "$W/out2" twoclients)$(lines "$W/out4" twoclients)" "11"
detach 2; sleep 0.3
keys 4 'echo still""here\n'; sleep 0.5
check "6 the other still works" "$(lines "$W/out4" stillhere)|$(field t1 attached)" "1|1"

# 7. end: gone from the list, its command gone; ending a missing session is fine, sending to one is not
pid=$(field t1 pid)
"$BIN" session end t1; check "7 end exits 0" "$?" "0"
check "7 not listed" "$(field t1 name)" ""
kill -0 "$pid" 2>/dev/null && { say "7 its command is gone" "DIFFERENT: pid $pid alive"; fail=1; } || say "7 its command is gone" "ok"
sleep 0.3; kill -0 "${CPID[4]}" 2>/dev/null && { say "7 the attached client exited" "DIFFERENT"; fail=1; } || say "7 the attached client exited" "ok"
"$BIN" session end nosuch; check "7 ending a missing session" "$?" "0"
echo x | "$BIN" session send nosuch 2>/dev/null; check "7 sending to a missing session" "$?" "1"

# 8. the command exiting by itself: the attached client exits with its code, the session is gone
PTY_COLS=80 PTY_ROWS=24 "$BIN" session open t8 -- sh -c 'sleep 1; echo bye; exit 7' < <(sleep 5) > "$W/out8"; code=$?
check "8 client exits with the command's code" "$code|$(lines "$W/out8" bye)" "7|1"
check "8 session gone" "$(field t8 name)" ""
PTY_COLS=80 PTY_ROWS=24 "$BIN" session open t8b -- sh -c 'echo quick; exit 5' < <(sleep 5) > "$W/out8b"; code=$?
check "8 also when it exits before the client attaches" "$code|$(lines "$W/out8b" quick)" "5|1"

# 9. a keeper killed with SIGKILL leaves no ghost
attach 5 t9 -- sh; sleep 0.8; detach 5
cpid=$(field t9 pid); kpid=$(ps -o ppid= -p "$cpid" | tr -d ' ')
kill -9 "$kpid"; sleep 0.3
check "9 killed keeper not listed" "$(field t9 name)" ""
[ -e "$XDG_RUNTIME_DIR/aiwalk-setup/sessions/t9.sock" ] && { say "9 its socket is cleaned up" "DIFFERENT"; fail=1; } || say "9 its socket is cleaned up" "ok"

# 10. bad names are refused
bad=0; for name in "" "a/b" ".." "a b" "$(printf 'x%.0s' {1..65})"; do "$BIN" session open "$name" -- sh </dev/null >/dev/null 2>&1; [ $? -eq 2 ] || bad=1; done
check "10 bad names refused" "$bad" "0"

# full-screen program: detach, attach again, and the screen comes back (replay, then a repaint at the new size)
VIM=$(command -v vim || command -v vi)
if [ -n "$VIM" ]; then
  attach 6 tv -- "$VIM" -u NONE -N "$W/cwd/file.txt"; sleep 1
  # ESC leaves insert mode, but ResizeScanner holds a lone ESC until the next byte (it may begin a resize request):
  # Ctrl-L, a harmless redraw, releases it
  keys 6 'iREPAINT_MARK\033'; sleep 0.3; keys 6 '\014'; sleep 0.5; detach 6
  attach 7 tv; sleep 1.2
  repaints=$(grep -c $'\x1b\[?1049h' "$W/out7")   # the alternate screen comes back first (from the kept modes)
  marks=$(grep -o REPAINT_MARK "$W/out7" | wc -l)
  [ "$repaints" -ge 1 ] && [ "$marks" -ge 2 ] && say "vim: alt screen restored, text replayed and repainted" "ok ($marks copies)" || { say "vim repaint" "DIFFERENT: 1049h=$repaints marks=$marks"; fail=1; }
  keys 7 ':q!\r'; sleep 0.5
  check "vim: quitting ends the session" "$(field tv name)" ""
fi
exit $fail
