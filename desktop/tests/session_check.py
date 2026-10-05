#!/usr/bin/env python3
"""Checks `aiwalk-setup session` the same way on Linux, macOS and Windows: what the vault plugin relies on in place
of tmux. session_check.sh covers more, but only where bash and FIFOs exist; this runs wherever Python does.

    python3 desktop/tests/session_check.py <aiwalk-setup binary>

A session is opened with a client on pipes, typed into, its client killed, listed, attached to again (the earlier
output must come back), sent text from another process, and ended. Uses its own names and ends what it started.
"""
import json, os, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(sys.argv[1])
WIN = os.name == "nt"
SHELL = ["cmd.exe", "/Q", "/K", "prompt $G"] if WIN else ["sh"]
NL = b"\r\n" if WIN else b"\n"
NAME = f"check-{os.getpid()}"
env = dict(os.environ, PTY_COLS="100", PTY_ROWS="30")
if not WIN:   # a scratch place for the sockets, so real sessions are neither seen nor touched
    env["XDG_RUNTIME_DIR"] = tempfile.mkdtemp()
    os.chmod(env["XDG_RUNTIME_DIR"], 0o700)
failed = []


def check(what, ok, detail=""):
    print(f"{what:<58} {'ok' if ok else 'DIFFERENT ' + str(detail)[:300]}", flush=True)
    if not ok: failed.append(what)


class Client:
    """`session open` on pipes; everything it prints is collected as it comes."""
    def __init__(self, *args):
        self.p = subprocess.Popen([BIN, "session", "open", NAME, *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, env=env)
        self.out = b""
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            d = self.p.stdout.read1(65536)
            if not d: return
            self.out += d
            # Windows' console asks where the cursor is before it prints anything, and waits for the answer; a real
            # terminal (the plugin's) answers by itself, so this stand-in must too
            if b"\x1b[6n" in d:
                try: self.type(b"\x1b[1;1R")
                except OSError: pass

    def type(self, data):
        self.p.stdin.write(data); self.p.stdin.flush()

    def saw(self, text, within=8):
        end = time.time() + within
        while time.time() < end:
            if text in self.out: return True
            time.sleep(0.1)
        return False

    def kill(self):
        self.p.kill(); self.p.wait()


def run(*args, data=None):
    return subprocess.run([BIN, "session", *args], input=data, capture_output=True, env=env, timeout=30)


def listed():
    r = run("list")
    try: rows = json.loads(r.stdout.decode() or "[]")
    except ValueError: rows = None
    return r, rows


try:
    c = Client("--", *SHELL)
    time.sleep(1.5)
    c.type(b"echo fir" + (b"" if WIN else b'""') + b"st-marker" + NL)   # typed so the echo of the keys is not the answer
    check("1 a command typed into the session answers", c.saw(b"first-marker"), c.out[-300:] + c.p.stderr.read1(300) if c.p.poll() is not None else c.out[-300:])
    c.kill(); time.sleep(0.5)

    r, rows = listed()
    mine = [x for x in (rows or []) if x.get("name") == NAME]
    check("2 listed after its client is killed", bool(mine), r.stdout[:300] + r.stderr[:300])
    check("2 nobody attached now", bool(mine) and mine[0].get("attached") == 0, mine)

    c2 = Client()
    check("3 attaching again shows the earlier output", c2.saw(b"first-marker"), c2.out[-300:])
    c2.type(b"echo second-marker" + NL)
    check("3 the session still takes commands", c2.saw(b"second-marker"), c2.out[-300:])

    r = run("send", NAME, data=b"echo sent-marker")
    check("4 send exits 0", r.returncode == 0, r.stderr[:300])
    c2.type(b"\r")
    check("4 sent text runs on Enter", c2.saw(b"sent-marker"), c2.out[-300:])
    c2.kill()

    r = run("end", NAME)
    check("5 end exits 0", r.returncode == 0, r.stderr[:300])
    time.sleep(1)
    _, rows = listed()
    check("5 gone from the list", rows is not None and not [x for x in rows if x.get("name") == NAME], rows)
finally:
    try: run("end", NAME)
    except Exception: pass

print("ALL OK" if not failed else f"{len(failed)} DIFFERENT")
sys.exit(1 if failed else 0)
