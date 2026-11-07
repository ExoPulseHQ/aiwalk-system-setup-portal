#!/usr/bin/env python3
"""exo-keys: the machine's side of the Machines page's repo keys. The app sends this file over ssh and runs it with
`python3 - <command>`; nothing is installed. Each GitHub repo a machine works with gets a key of its own (design: vault
Project_Management/Team_Access_Identity_Design_Summary.md, "access belongs to the repo"), a submodule's repo included.

    scan                      JSON: the repos under $HOME with their origin and submodule URLs, the deploy keys in ~/.ssh,
                              the ssh aliases and the git URL rewrites
    add NAME REPO [URL ...]   makes ~/.ssh/deploy_NAME if missing, the alias github-NAME, and rewrites each URL (and
                              github.com's own forms of REPO) to that alias; JSON with the public key
    verify NAME               JSON: what GitHub answers to the alias github-NAME (a repo key answers "Hi owner/repo!")
"""
import glob, json, os, socket, subprocess, sys

HOME = os.path.expanduser("~")
SSH = os.path.join(HOME, ".ssh")
CONFIG = os.path.join(SSH, "config")


def run(*args, timeout=20):
    r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    return r.returncode, (r.stdout + r.stderr).strip()


def git(*args):
    code, out = run("git", *args)
    return out if code == 0 else ""


def repos():
    """Main copies (a .git folder, so not worktrees) up to two levels below $HOME."""
    found = []
    for pattern in ("*/.git", "*/*/.git"):
        for g in sorted(glob.glob(os.path.join(HOME, pattern))):
            if not os.path.isdir(g):
                continue
            top = os.path.dirname(g)
            subs = []
            if os.path.exists(os.path.join(top, ".gitmodules")):
                for l in git("-C", top, "config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.url$").splitlines():
                    k, _, url = l.partition(" ")
                    subs.append({"name": k[len("submodule."):-len(".url")], "url": url})
            found.append({"path": top, "origin": git("-C", top, "remote", "get-url", "origin"), "submodules": subs})
    return found


def aliases():
    """{ssh Host alias: IdentityFile} from ~/.ssh/config (the first IdentityFile of each Host line's names)."""
    out, names = {}, []
    try:
        lines = open(CONFIG, encoding="utf-8").read().splitlines()
    except OSError:
        return out
    for l in lines:
        k, _, v = l.strip().partition(" ")
        if k.lower() == "host":
            names = v.split()
        elif k.lower() == "match":
            names = []
        elif k.lower() == "identityfile":
            for n in names:
                out.setdefault(n, v.strip())
    return out


def rewrites():
    """{URL prefix: what git rewrites it to}, from the global url.<base>.insteadOf settings."""
    out = {}
    for l in git("config", "--global", "--get-regexp", r"^url\..*\.insteadof$").splitlines():
        k, _, frm = l.partition(" ")
        out[frm] = k[len("url."):-len(".insteadof")]
    return out


def keys():
    out = {}
    for p in glob.glob(os.path.join(SSH, "deploy_*.pub")):
        try:
            out[os.path.basename(p)[len("deploy_"):-len(".pub")]] = open(p, encoding="utf-8").read().strip()
        except OSError:
            pass
    return out


def add(name, repo, urls):
    key = os.path.join(SSH, f"deploy_{name}")
    os.makedirs(SSH, mode=0o700, exist_ok=True)
    if not os.path.exists(key):
        code, out = run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", f"{socket.gethostname()}-{name}-deploy", "-f", key)
        if code:
            raise SystemExit(json.dumps({"ok": False, "error": out or "ssh-keygen failed"}))
    alias = f"github-{name}"
    if alias not in aliases():
        block = f"\nHost {alias}\n  HostName github.com\n  User git\n  IdentityFile ~/.ssh/deploy_{name}\n  IdentitiesOnly yes\n"
        with open(CONFIG, "a", encoding="utf-8") as f:
            f.write(block)
        os.chmod(CONFIG, 0o600)
    target = f"git@{alias}:{repo}.git"
    have = set(git("config", "--global", "--get-all", f"url.{target}.insteadOf").splitlines())
    forms = [f"git@github.com:{repo}.git", f"git@github.com:{repo}", f"ssh://git@github.com/{repo}.git", f"https://github.com/{repo}.git", f"https://github.com/{repo}"]
    for u in dict.fromkeys(forms + urls):
        if u != target and u not in have:
            run("git", "config", "--global", "--add", f"url.{target}.insteadOf", u)
    return {"ok": True, "key": open(key + ".pub", encoding="utf-8").read().strip(), "alias": alias}


def verify(name):
    _, out = run("ssh", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new", "-o", "ConnectTimeout=15", "-T", f"git@github-{name}", timeout=30)
    return {"ok": True, "answer": out.splitlines()[-1] if out else ""}


def main(argv):
    if argv[:1] == ["scan"]:
        res = {"ok": True, "host": socket.gethostname(), "repos": repos(), "keys": keys(), "aliases": aliases(), "rewrites": rewrites()}
    elif argv[:1] == ["add"] and len(argv) >= 3:
        res = add(argv[1], argv[2], argv[3:])
    elif argv[:1] == ["verify"] and len(argv) == 2:
        res = verify(argv[1])
    else:
        print(__doc__.strip(), file=sys.stderr); return 2
    print(json.dumps(res, ensure_ascii=False)); return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
