#!/usr/bin/env python3
"""What this lab machine has and how busy it is, as one small JSON answer for aIwalk System Setup.

Listens on 127.0.0.1 only; the machine's Cloudflare tunnel publishes it as status-<name>.aiwalkcorp.com, behind
Cloudflare Access, so only members of the team can read it. Work happens only when asked, and an answer is reused
for 5 seconds, so a page polling every 15 seconds costs a few /proc reads and one nvidia-smi call (~0.1 s).

  exo-status.py              serve on 127.0.0.1:9101
  exo-status.py --once       print one answer and exit
  exo-status.py --selftest   check the parsers on canned input
  --cluster HPC              also report a SLURM cluster reached through this machine (an ssh alias in its
                             ~/.ssh/config), from the cluster's own status service that its `hpcs` command reads
"""
import json, os, shutil, subprocess, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

VERSION = "1"   # raise by hand when a change here should reach the machines; the app's Machines page compares it
PORT = 9101
CACHE_SECONDS = 5
CLUSTER = None          # ssh alias of a cluster this machine reaches, from --cluster
CLUSTER_SECONDS = 60    # the school's service is asked at most once a minute, in the background
CLUSTER_API = "http://10.141.255.254:8050"   # NYCU HPC's status service, behind its login node (what `hpcs` reads)


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


def desktops(ps_out):
    """VNC desktops from `ps -eo user=,args=`: [{user, display, port, geometry}] (TigerVNC Xvnc / Xtigervnc)."""
    out = []
    for line in ps_out.splitlines():
        parts = line.split()
        if len(parts) < 3 or os.path.basename(parts[1]) not in ("Xtigervnc", "Xvnc"):
            continue
        disp = next((p for p in parts[2:] if p.startswith(":") and p[1:].isdigit()), None)
        if disp is None:
            continue
        args = parts[2:]
        opt = lambda k: args[args.index(k) + 1] if k in args and args.index(k) + 1 < len(args) else None
        port = int(opt("-rfbport") or 5900 + int(disp[1:]))
        # a desktop on a Unix socket has no TCP port; the portal forwards to the socket instead
        out.append({"user": parts[0], "display": disp, "port": port if port > 0 else None,
                    "socket": opt("-rfbunixpath"), "geometry": opt("-geometry") or ""})
    return sorted(out, key=lambda d: int(d["display"][1:]))


def static():
    cpu = next((l.split(":", 1)[1].strip() for l in read("/proc/cpuinfo").splitlines() if l.startswith("model name")), "")
    os_name = next((l.split("=", 1)[1].strip('"') for l in read("/etc/os-release").splitlines() if l.startswith("PRETTY_NAME=")), "")
    return {"hostname": os.uname().nodename, "os": os_name, "cpu": cpu, "threads": os.cpu_count(),
            "mem_gb": round(meminfo(read("/proc/meminfo")).get("MemTotal", 0) / 1048576, 1)}


def version_of(text):
    """The VERSION line near the top of a host tool (`VERSION = "1"` in Python, `VERSION=1` in bash) as a number;
    0 for a tool from before versions. Read from the file, never by running it."""
    for line in text.splitlines():
        k, eq, v = line.partition("=")
        if eq and k.strip() == "VERSION":
            v = v.split("#")[0].strip().strip("\"'")
            return int(v) if v.isdigit() else 0
    return 0


def tools(bindir=os.path.expanduser("~/.local/bin")):
    """{name: version} of the host tools installed in ~/.local/bin; None for one that is missing."""
    return {n: version_of(read(os.path.join(bindir, n))) if os.path.isfile(os.path.join(bindir, n)) else None
            for n in ("exo-status.py", "exo", "exo-desktop")}


_last = {"t": 0.0, "answer": None, "cpu": None}
_lock = threading.Lock()



def cluster_parse(text):
    """Two JSON lines from the cluster's /nodes and /queue: nodes with CPU/GPU in use, jobs running and waiting."""
    lines = [l for l in text.splitlines() if l.strip()]
    nodes, queue = json.loads(lines[0]), json.loads(lines[1])
    return {"time": queue.get("last_update"), "running": queue.get("running_job", 0), "pending": queue.get("pending_job", 0),
            "running_by_queue": queue.get("running_by_partition", {}), "pending_by_queue": queue.get("pending_by_partition", {}),
            "nodes": [{"name": n["name"], "cpu_used": n["cpu_used"], "cpu_total": n["cpu_total"], "gpu_used": n["gpu_used"],
                       "gpu_total": n["gpu_total"], "state": "+".join(n.get("state", [])), "reason": n.get("reason", "")} for n in nodes]}


_cluster = {"t": 0, "data": None, "busy": False}

def cluster():
    """The last cluster reading; starts a fresh one in the background when it is older than CLUSTER_SECONDS."""
    if not CLUSTER:
        return None
    if not _cluster["busy"] and time.time() - _cluster["t"] > CLUSTER_SECONDS:
        _cluster["busy"] = True
        threading.Thread(target=_fetch_cluster, daemon=True).start()
    return _cluster["data"]

def _fetch_cluster():
    try:
        out = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", CLUSTER,
                              f"curl -s --max-time 10 {CLUSTER_API}/nodes; echo; curl -s --max-time 10 {CLUSTER_API}/queue"],
                             capture_output=True, text=True, timeout=30).stdout
        _cluster["data"] = cluster_parse(out)
    except (OSError, subprocess.TimeoutExpired, ValueError, IndexError, KeyError):
        pass   # keep the last reading; the page shows its time
    finally:
        _cluster.update(t=time.time(), busy=False)

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
        ps = subprocess.run(["ps", "-eo", "user=,args="], capture_output=True, text=True).stdout
        ans = {**static(), "time": int(now), "cpu_pct": cpu_pct,
               "load": [round(x, 2) for x in os.getloadavg()],
               "mem_used_gb": round((m.get("MemTotal", 0) - m.get("MemAvailable", 0)) / 1048576, 1),
               "gpus": gpus(smi),
               "disk_gb": round(disk.total / 1e9), "disk_free_gb": round(disk.free / 1e9),
               "uptime_h": round(float(read("/proc/uptime").split()[0] or 0) / 3600, 1),
               "users": len(set(users[::5])) if users else 0,
               "desktops": desktops(ps), "cluster": cluster(), "tools": tools()}
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
    d = desktops("ntk /usr/bin/Xtigervnc :2 -localhost=0 -desktop x -geometry 1920x1080 -rfbport 5902\n"
                 "ntk bash -c grep Xtigervnc :9\nroot /usr/bin/Xvnc :7\n")
    assert d == [{"user": "ntk", "display": ":2", "port": 5902, "socket": None, "geometry": "1920x1080"},
                 {"user": "root", "display": ":7", "port": 5907, "socket": None, "geometry": ""}], d
    s6 = desktops("ntk /usr/bin/Xtigervnc :6 -rfbport -1 -rfbunixpath /home/ntk/.vnc/desk-6.sock -geometry 1920x1080\n")
    assert s6 == [{"user": "ntk", "display": ":6", "port": None, "socket": "/home/ntk/.vnc/desk-6.sock", "geometry": "1920x1080"}], s6
    c = cluster_parse('[{"cpu_total":224,"cpu_used":96,"gpu_total":8,"gpu_used":8,"name":"DGX-CN01","reason":"","state":["ALLOCATED"]}]\n'
                      '{"last_update":1,"pending_by_partition":{"defq":2},"pending_job":7,"running_by_partition":{"defq":12},"running_job":27}\n')
    assert c["running"] == 27 and c["pending"] == 7 and c["nodes"][0] == {"name": "DGX-CN01", "cpu_used": 96, "cpu_total": 224,
        "gpu_used": 8, "gpu_total": 8, "state": "ALLOCATED", "reason": ""}, c
    assert version_of('#!/bin/bash\nVERSION=3   # raise by hand\n') == 3 and version_of('x = 1\nVERSION = "12"\n') == 12
    assert version_of("#!/usr/bin/env python3\nprint(1)\n") == 0
    import tempfile
    with tempfile.TemporaryDirectory() as t:
        open(os.path.join(t, "exo"), "w").write('VERSION = "2"\n')
        open(os.path.join(t, "exo-desktop"), "w").write("#!/bin/bash\n")
        assert tools(t) == {"exo-status.py": None, "exo": 2, "exo-desktop": 0}, tools(t)
    here = os.path.dirname(os.path.abspath(__file__))
    assert tools(here)["exo-status.py"] == int(VERSION), "this file's own VERSION line parses"
    a = answer(); assert "tools" in a and {"cpu", "threads", "mem_gb", "cpu_pct", "gpus", "disk_free_gb"} <= set(a) and 0 <= a["cpu_pct"] <= 100
    assert answer() is a, "answers are reused within the cache window"
    print("exo-status: all checks passed")


if __name__ == "__main__":
    if "--cluster" in sys.argv:
        i = sys.argv.index("--cluster"); CLUSTER = sys.argv[i + 1]; del sys.argv[i:i + 2]
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    elif sys.argv[1:] == ["--once"]:
        print(json.dumps(answer(), indent=1))
    else:
        ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
