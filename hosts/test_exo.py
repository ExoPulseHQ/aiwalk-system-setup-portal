"""Builds a throwaway origin and clone, then checks every state `exo code status` reports. Run: python3 hosts/test_exo.py"""
import json, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def sh(cwd, *cmd):
    subprocess.run(cmd, cwd=cwd, check=True, capture_output=True)


with tempfile.TemporaryDirectory() as t:
    env = {**os.environ, "GIT_AUTHOR_NAME": "Ann", "GIT_COMMITTER_NAME": "Ann", "GIT_AUTHOR_EMAIL": "a@x", "GIT_COMMITTER_EMAIL": "a@x", "HOME": t}
    os.environ.update(env)
    origin, repo = f"{t}/origin.git", f"{t}/proj"
    sh(t, "git", "init", "-q", "--bare", "-b", "main", origin)
    sh(t, "git", "clone", "-q", origin, repo)
    open(f"{repo}/a", "w").write("1"); sh(repo, "git", "add", "a"); sh(repo, "git", "commit", "-qm", "a"); sh(repo, "git", "push", "-q", "origin", "main")
    # merged: a branch merged into main with a merge commit, its worktree still here
    sh(repo, "git", "worktree", "add", "-q", "-b", "feat/done", f"{t}/proj-done")
    open(f"{t}/proj-done/b", "w").write("2"); sh(f"{t}/proj-done", "git", "add", "b"); sh(f"{t}/proj-done", "git", "commit", "-qm", "b")
    sh(repo, "git", "merge", "-q", "--no-ff", "-m", "Merge feat/done", "feat/done"); sh(repo, "git", "push", "-q", "origin", "main")
    # behind: someone else pushes to main
    other = tempfile.mkdtemp() + "/other"; sh(t, "git", "clone", "-q", origin, other)   # outside $HOME, so it is not scanned
    open(f"{other}/c", "w").write("3"); sh(other, "git", "add", "c"); sh(other, "git", "commit", "-qm", "c"); sh(other, "git", "push", "-q", "origin", "main")
    # dirty and unpushed: a fresh branch with one local commit and one changed file; a fresh branch with nothing yet
    sh(repo, "git", "worktree", "add", "-q", "-b", "feat/wip", f"{t}/proj-wip")
    open(f"{t}/proj-wip/d", "w").write("4"); sh(f"{t}/proj-wip", "git", "add", "d"); sh(f"{t}/proj-wip", "git", "commit", "-qm", "d")
    open(f"{t}/proj-wip/a", "w").write("changed")
    sh(repo, "git", "worktree", "add", "-q", "-b", "feat/new", f"{t}/proj-new")
    open(f"{t}/proj-wip/run.log", "w").write("out")   # a new file: listed apart from edits
    sh(repo, "git", "branch", "loose")   # a branch with no worktree
    open(f"{repo}/a", "w").write("edited on main")   # a change straight on the trunk

    out = subprocess.run([sys.executable, f"{HERE}/exo", "code", "status"], capture_output=True, text=True, check=True).stdout
    r = json.loads(out)["repos"]
    assert [x["path"] for x in r] == [repo], r            # found by scanning $HOME; worktrees and the bare origin are not repos
    r = r[0]; w = {x["branch"]: x for x in r["worktrees"]}
    assert r["default"] == "main" and r["fetch"] == "ok", r
    assert w["main"]["main"] and w["main"]["behind"] == 1 and w["main"]["changed"] == ["a"], w["main"]
    assert w["feat/done"]["merged"] is True, w["feat/done"]
    assert w["feat/wip"]["merged"] is False and w["feat/wip"]["pushed"] is False and w["feat/wip"]["unpushed"] == 1 and w["feat/wip"]["changed"] == ["a"], w["feat/wip"]
    assert w["feat/wip"]["new"] == ["run.log"] and w["main"]["new_count"] == 0, w
    assert w["feat/new"]["merged"] is False and w["feat/new"]["unpushed"] == 0, w["feat/new"]
    assert r["no_worktree"] == ["loose"], r["no_worktree"]
    only = json.loads(subprocess.run([sys.executable, f"{HERE}/exo", "code", "status", "--no-fetch", "--repos", "origin"], capture_output=True, text=True, check=True).stdout)["repos"]
    assert [x["repo"] for x in only] == ["proj"], only     # matched by the remote's name, not the folder's
    assert json.loads(subprocess.run([sys.executable, f"{HERE}/exo", "code", "status", "--no-fetch", "--repos", "nope"], capture_output=True, text=True, check=True).stdout)["repos"] == []

    X = lambda *a: json.loads(subprocess.run([sys.executable, f"{HERE}/exo", "code", *a], capture_output=True, text=True).stdout)
    who = ["--name", "Bob", "--email", "b@x"]
    assert not X("commit", repo, *who, "-m", "x", "a")["ok"] and not X("push", repo)["ok"]          # the trunk is never committed or pushed
    open(f"{t}/proj-wip/api_token.txt", "w").write("s")
    r = X("commit", f"{t}/proj-wip", *who, "-m", "m", "a", "api_token.txt"); assert not r["ok"] and "token" in r["error"], r
    r = X("commit", f"{t}/proj-wip", *who, "-m", "feat: a", "--body", "why", "a"); assert r["ok"], r   # only the ticked file goes in
    log = subprocess.run(["git", "-C", f"{t}/proj-wip", "log", "-1", "--format=%an|%s|%b"], capture_output=True, text=True).stdout.strip()
    assert log == "Bob|feat: a|why", log
    d = X("diff", f"{t}/proj-wip"); assert d["ok"] and "run.log" in d["new"] and d["unpushed"] == ["feat: a", "d"], d
    assert X("push", f"{t}/proj-wip")["ok"]
    assert subprocess.run(["git", "--git-dir", origin, "rev-parse", "--verify", "-q", "refs/heads/feat/wip"], capture_output=True).returncode == 0
    assert not X("finish", f"{t}/proj-wip")["ok"]                     # files not in git: never removed
    assert not X("finish", f"{t}/proj-new")["ok"]                     # clean but not merged
    r = X("finish", f"{t}/proj-done"); assert r["ok"] and not os.path.exists(f"{t}/proj-done"), r
    assert "feat/done" not in subprocess.run(["git", "-C", repo, "branch"], capture_output=True, text=True).stdout
    assert not X("pull", f"{t}/proj-new")["ok"]                       # only the main copy brings in updates
    r = X("pull", repo); assert r["ok"] and r["from"] != r["to"], r
    print("exo code status: ok")
