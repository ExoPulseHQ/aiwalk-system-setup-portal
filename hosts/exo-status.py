#!/usr/bin/env python3
"""What this lab machine has and how busy it is, as one small JSON answer for aIwalk System Setup.

Listens on 127.0.0.1 only; the machine's Cloudflare tunnel publishes it as status-<name>.aiwalkcorp.com, behind
Cloudflare Access, so only members of the team can read it. Work happens only when asked, and an answer is reused
for 5 seconds, so a page polling every 15 seconds costs a few /proc reads and one nvidia-smi call (~0.1 s).

  exo-status.py              serve on 127.0.0.1:9101
  exo-status.py --once       print one answer and exit
  exo-status.py --selftest   check the parsers on canned input
"""
import json, os, shutil, subprocess, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 9101
CACHE_SECONDS = 5


def read(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return ""


def cpu_times(stat):
    """(busy, total) jiffies from the first line of /proc/stat."""
    v = [int(x) for x in stat.splitlines()[0].split()[1:]]
    idle = v[3] + (v[4] if len(v) > 4 else 0)   # idle + iowait
    return sum(v) - idle, sum(v)


def meminfo(text):
    """{field: kB} from /proc/meminfo."""
    out = {}
    for line in text.splitlines():
        k, _, rest = line.partition(":")
        if rest.split():
            out[k] = int(rest.split()[0])
    return out


def gpus(csv):
    """nvidia-smi --query-gpu=name,memory.total,memory.used,utilization.gpu,temperature.gpu --format=csv,noheader,nounits"""
    out = []
    for line in csv.strip().splitlines():
        f = [x.strip() for x in line.split(",")]
        if len(f) >= 5:
            num = lambda x: float(x) if x.replace(".", "", 1).isdigit() else None
            out.append({"name": f[0], "mem_total_mb": num(f[1]), "mem_used_mb": num(f[2]), "util": num(f[3]), "temp_c": num(f[4])})
    return out


def static():
    cpu = next((l.split(":", 1)[1].strip() for l in read("/proc/cpuinfo").splitlines() if l.startswith("model name")), "")
    os_name = next((l.split("=", 1)[1].strip('"') for l in read("/etc/os-release").splitlines() if l.startswith("PRETTY_NAME=")), "")
    return {"hostname": os.uname().nodename, "os": os_name, "cpu": cpu, "threads": os.cpu_count(),
            "mem_gb": round(meminfo(read("/proc/meminfo")).get("MemTotal", 0) / 1048576, 1)}


_last = {"t": 0.0, "answer": None, "cpu": None}
_lock = threading.Lock()


def answer():
    with _lock:
        now = time.time()
        if _last["answer"] and now - _last["t"] < CACHE_SECONDS:
            return _last["answer"]
        busy, total = cpu_times(read("/proc/stat"))
        prev = _last["cpu"]
        if prev is None:   # first call: measure over a short interval
            time.sleep(0.2); prev = (busy, total); busy, total = cpu_times(read("/proc/stat"))
        cpu_pct = round(100 * (busy - prev[0]) / max(total - prev[1], 1), 1)
        m = meminfo(read("/proc/meminfo"))
        try:
            smi = subprocess.run(["nvidia-smi", "--query-gpu=name,memory.total,memory.used,utilization.gpu,temperature.gpu",
                                  "--format=csv,noheader,nounits"], capture_output=True, text=True, timeout=5).stdout
        except (OSError, subprocess.TimeoutExpired):
            smi = ""
        disk = shutil.disk_usage("/")
        users = subprocess.run(["who"], capture_output=True, text=True).stdout.split()
        ans = {**static(), "time": int(now), "cpu_pct": cpu_pct,
               "load": [round(x, 2) for x in os.getloadavg()],
               "mem_used_gb": round((m.get("MemTotal", 0) - m.get("MemAvailable", 0)) / 1048576, 1),
               "gpus": gpus(smi),
               "disk_gb": round(disk.total / 1e9), "disk_free_gb": round(disk.free / 1e9),
               "uptime_h": round(float(read("/proc/uptime").split()[0] or 0) / 3600, 1),
               "users": len(set(users[::5])) if users else 0}
        _last.update(t=now, answer=ans, cpu=(busy, total))
        return ans


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(answer()).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def log_message(self, *_):   # quiet: this runs forever
        pass


def selftest():
    assert cpu_times("cpu  10 0 10 70 10 0 0 0 0 0\n") == (20, 100)
    assert meminfo("MemTotal:       16000000 kB\nMemAvailable:    4000000 kB\n") == {"MemTotal": 16000000, "MemAvailable": 4000000}
    g = gpus("NVIDIA GeForce RTX 5080, 16303, 7, 0, 41\nTesla, [N/A], 1, 2, 3\n")
    assert g[0] == {"name": "NVIDIA GeForce RTX 5080", "mem_total_mb": 16303.0, "mem_used_mb": 7.0, "util": 0.0, "temp_c": 41.0}
    assert g[1]["mem_total_mb"] is None
    a = answer(); assert {"cpu", "threads", "mem_gb", "cpu_pct", "gpus", "disk_free_gb"} <= set(a) and 0 <= a["cpu_pct"] <= 100
    assert answer() is a, "answers are reused within the cache window"
    print("exo-status: all checks passed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    elif sys.argv[1:] == ["--once"]:
        print(json.dumps(answer(), indent=1))
    else:
        ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
