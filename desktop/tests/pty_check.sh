#!/usr/bin/env bash
# Checks `aiwalk-setup pty` against what the plugin's pty_bridge.py does, on Linux or macOS.
#   desktop/tests/pty_check.sh <aiwalk-setup binary> [pty_bridge.py]
set -u
BIN=$(readlink -f "$1"); PY=${2:-}
fail=0
say() { printf '%-46s %s\n' "$1" "$2"; }
check() { if [ "$2" = "$3" ]; then say "$1" "ok ($2)"; else say "$1" "DIFFERENT: got [$2] want [$3]"; fail=1; fi; }
clean() { tr -d '\r' | sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g'; }

# 1. a real terminal at the size asked for, and the command's exit status
# (stdin stays open while the command runs: a closed stdin hangs the command up, as in pty_bridge.py)
out=$(sleep 2 | PTY_COLS=100 PTY_ROWS=30 "$BIN" pty -- sh -c 'test -t 0 && test -t 1 && echo tty; stty size; echo "$TERM $COLORTERM"' | clean | paste -sd'|')
check "terminal, size, TERM" "$out" "tty|30 100|xterm-256color truecolor"
sleep 2 | PTY_COLS=100 PTY_ROWS=30 "$BIN" pty -- sh -c 'exit 7' >/dev/null; check "exit status" "${PIPESTATUS[1]}" "7"
"$BIN" pty -- /no/such/program </dev/null >/dev/null 2>&1; check "missing program" "$?" "127"

# 2. stdin goes to the terminal
out=$( (sleep 0.6; printf 'echo he""llo\n'; sleep 0.5; printf 'exit 3\n'; sleep 0.5) | "$BIN" pty -- sh 2>/dev/null | clean | grep -c '^hello$'); check "stdin reaches the command" "$out" "1"
out=$( (sleep 0.6; printf 'echo he""llo\n'; sleep 0.5; printf 'exit 3\n'; sleep 0.5) | "$BIN" pty -- sh 2>/dev/null | clean | grep -c 'echo he""llo'); check "what is typed is echoed, as a terminal does" "$out" "1"
(sleep 0.6; printf 'exit 3\n'; sleep 0.5) | "$BIN" pty -- sh >/dev/null 2>&1; check "exit status through stdin" "$?" "3"

# 3. resize inside the stdin stream, taken out before the command sees it
out=$( (sleep 0.3; printf '\033]777;resize;132;50\007'; sleep 0.3; printf 'stty size\n'; sleep 0.5; printf 'exit\n'; sleep 0.3) | PTY_COLS=80 PTY_ROWS=24 "$BIN" pty -- sh 2>/dev/null | clean | grep -E '^[0-9]+ [0-9]+$' | tail -1)
check "resize in the stream" "$out" "50 132"

# 4. resize on fd 3, as pty_bridge.py takes it
out=$( { (sleep 0.6; printf 'stty size\n'; sleep 0.5; printf 'exit\n'; sleep 0.3) | PTY_COLS=80 PTY_ROWS=24 "$BIN" pty -- sh 2>/dev/null 3< <(sleep 0.2; echo "resize 90 20"; sleep 2); } | clean | grep -E '^[0-9]+ [0-9]+$' | tail -1)
check "resize on fd 3" "$out" "20 90"

# 5. closing stdin hangs the command up
start=$(date +%s); "$BIN" pty -- sleep 30 </dev/null >/dev/null 2>&1; took=$(( $(date +%s) - start ))
[ "$took" -lt 5 ] && say "closed stdin ends the command" "ok (${took}s)" || { say "closed stdin ends the command" "DIFFERENT: took ${took}s"; fail=1; }

# 6. a lot of output arrives whole
out=$(sleep 20 | "$BIN" pty -- sh -c 'head -c 3000000 /dev/zero | tr "\0" "x"' | tr -d '\r\n' | wc -c); check "3 MB of output" "$out" "3000000"

# 7. the same through the Python bridge, where given. It is given fd 3, as the plugin always does: started without
#    one it takes its own terminal for the resize channel and loses output (the bug this program's fd 3 check avoids).
if [ -n "$PY" ]; then
  out=$(sleep 2 | PTY_COLS=100 PTY_ROWS=30 python3 "$PY" sh -c 'test -t 0 && test -t 1 && echo tty; stty size; echo "$TERM $COLORTERM"' 3< <(sleep 3) | clean | paste -sd'|')
  check "python: terminal, size, TERM" "$out" "tty|30 100|xterm-256color truecolor"
  sleep 2 | PTY_COLS=100 PTY_ROWS=30 python3 "$PY" sh -c 'exit 7' >/dev/null 3< <(sleep 3); check "python: exit status" "${PIPESTATUS[1]}" "7"
fi
exit $fail
