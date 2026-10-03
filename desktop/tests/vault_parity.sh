#!/usr/bin/env bash
# Parity check: `aiwalk-setup vault ...` against scripts/vault_ship.py, on identical throwaway vaults.
#
#   desktop/tests/vault_parity.sh <aiwalk-setup binary> <vault with scripts/vault_ship.py> [work dir]
#
# For every case it builds a fresh vault twice (local bare remotes for the vault and two submodules, Sub and
# Secrets), runs the Python in one and the binary in the other, and compares exit code, stdout (commit hashes
# masked), stderr, and what landed: the remotes' trees, authors and subjects, the local status, the primary logs
# and System/vault_index.json. The real vault is only read: its two scripts, the team table, the primary logs that
# have an ownership block and the sub-docs they link to are copied in. Identity questions go to GitHub read-only,
# as whoever gh is signed in as. Nothing is pushed anywhere but the bare remotes under the work dir.
set -u
BIN=$(readlink -f "$1"); REAL=$(readlink -f "$2"); TMP=${3:-/media/eddlai/DATA/tmp-vaultcli-test}
case "$TMP" in "$REAL"*) echo "the work dir must be outside the vault"; exit 2;; esac
G() { git -c user.name=fixture -c user.email=fixture@example.com -c protocol.file.allow=always "$@"; }
TODAY=$(date +%Y/%m/%d)
rm -rf "$TMP"; mkdir -p "$TMP/content"

# ---- what every fixture vault holds, copied once from the real vault
python3 - "$REAL" "$TMP/content" <<'EOF'
import os, re, shutil, sys
real, out = sys.argv[1:]
def cp(rel):
    os.makedirs(os.path.dirname(os.path.join(out, rel)), exist_ok=True)
    shutil.copyfile(os.path.join(real, rel), os.path.join(out, rel))
for f in ["scripts/vault_ship.py", "scripts/sync_ownership.py", "System/ExoPulse_Task_Assignment.md"]:
    cp(f)
logs = [f"System/{f}" for f in sorted(os.listdir(os.path.join(real, "System")))
        if f.startswith("_") and f.endswith(".md") and "<!-- ownership:auto:start -->" in open(os.path.join(real, "System", f), encoding="utf-8").read()]
names = {}
for root, dirs, files in os.walk(real):
    dirs[:] = [d for d in dirs if not d.startswith(".") and d != "node_modules"]
    for f in files:
        if f.endswith(".md"):
            names.setdefault(f, os.path.relpath(os.path.join(root, f), real))
linked = set()
for l in logs:
    cp(l)
    for t in re.findall(r"\[\[([^\]|#]+)", open(os.path.join(real, l), encoding="utf-8").read()):
        b = os.path.basename(t.strip())
        p = names.get(b) or names.get(b + ".md")
        if p and p not in logs and not p.startswith(("scripts/", "Secrets/", "Sub/")):   # the fixture's own submodules
            linked.add(p)
for p in sorted(linked):
    cp(p)
open(os.path.join(out, "logs.txt"), "w").write("\n".join(logs) + "\n")
print(f"fixture: {len(logs)} primary logs, {len(linked)} linked sub-docs")
EOF
mkdir -p "$TMP/content/notes"
printf 'line one\nline two\n' > "$TMP/content/notes/a.md"
printf 'b\n' > "$TMP/content/notes/b.md"
mapfile -t LOGS < "$TMP/content/logs.txt"

# ---- one fresh vault in $1: remotes $1/{vault,Sub,Secrets}.git, working copy $1/v
mk() {
  local D=$1 r
  mkdir -p "$D"
  for r in vault Sub Secrets; do git init -q --bare -b main "$D/$r.git"; done
  for r in Sub Secrets; do
    git clone -q "$D/$r.git" "$D/seed-$r" 2>/dev/null
    printf '# %s\n' "$r" > "$D/seed-$r/s.md"
    G -C "$D/seed-$r" add -A && G -C "$D/seed-$r" commit -qm init && G -C "$D/seed-$r" push -q origin HEAD:main
  done
  git clone -q "$D/vault.git" "$D/v" 2>/dev/null
  G -C "$D/v" symbolic-ref HEAD refs/heads/main
  cp -a "$TMP/content/." "$D/v/"; rm "$D/v/logs.txt"
  G -C "$D/v" submodule add -q ../Sub.git Sub 2>/dev/null && G -C "$D/v" submodule add -q ../Secrets.git Secrets 2>/dev/null \
    || { echo "fixture: submodule add failed in $D"; exit 2; }
  (cd "$D/v" && python3 scripts/vault_ship.py index >/dev/null)
  G -C "$D/v" add -A && G -C "$D/v" commit -qm fixture && G -C "$D/v" push -q -u origin main 2>/dev/null
}

# what landed, with commit hashes that differ between the two runs reduced to what they point at
state() {
  local D=$1 r
  for r in vault Sub Secrets; do
    echo "== remote $r: $(git -C "$D/$r.git" log -1 --format='%an <%ae> | %s' main)"
    git -C "$D/$r.git" ls-tree -r main | while read -r mode type hash path; do
      if [ "$type" = commit ]; then
        [ "$hash" = "$(git -C "$D/$path.git" rev-parse main)" ] && echo "$path -> $path remote head" || echo "$path -> other commit"
      else echo "$hash $path"; fi
    done | sha256sum | cut -c1-12
  done
  echo "== local: ahead $(git -C "$D/v" rev-list --count origin/main..HEAD 2>/dev/null), status: $(git -C "$D/v" status --porcelain | tr '\n' ';')"
  for r in Sub Secrets; do echo "== $r local: ahead $(git -C "$D/v/$r" rev-list --count origin/main..HEAD), status: $(git -C "$D/v/$r" status --porcelain | tr '\n' ';')"; done
  echo "== files: $(cd "$D/v" && sha256sum "${LOGS[@]}" System/vault_index.json | cut -c1-12 | tr '\n' ' ')"
}

IMPL=
tool() { if [ "$IMPL" = py ]; then python3 scripts/vault_ship.py "$@"; else "$BIN" vault "$@"; fi; }
mask() { sed -E 's/\b[0-9a-f]{7,40}\b/HASH/g'; }

ROWS=()
# case <name> <setup shell code, run in the working copy; $D is the case folder> <env assignments> -- <args>
case_() {
  local name=$1 setup=$2 envs=$3; shift 4
  for IMPL in py rs; do
    D="$TMP/$name-$IMPL"; mk "$D"
    (cd "$D/v" && D=$D && eval "$setup") >/dev/null 2>&1
    (cd "$D/v" && env $envs bash -c "$(declare -f tool); IMPL=$IMPL BIN='$BIN'; tool \"\$@\"" _ "$@") >"$D/out" 2>"$D/err"
    echo $? > "$D/code"
    state "$D" > "$D/state"
  done
  local p="$TMP/$name-py" r="$TMP/$name-rs" same=same
  cmp -s "$p/code" "$r/code" || same=DIFFERENT
  cmp -s <(mask < "$p/out") <(mask < "$r/out") || same=DIFFERENT
  cmp -s "$p/err" "$r/err" || same=DIFFERENT
  cmp -s "$p/state" "$r/state" || same=DIFFERENT
  echo "################ $name: $same"
  for i in py rs; do
    echo "---- $i: exit $(cat "$TMP/$name-$i/code")"
    echo "stdout:"; mask < "$TMP/$name-$i/out" | sed 's/^/  | /'
    echo "stderr:"; sed 's/^/  | /' "$TMP/$name-$i/err"
  done
  [ $same = same ] && echo "state (identical):" && sed 's/^/  | /' "$p/state" || { echo "state diff (py < > rs):"; diff "$p/state" "$r/state" | sed 's/^/  | /'; }
  ROWS+=("$name|$(cat "$p/code")|$(mask < "$p/out" | tail -1)|$(cat "$r/code")|$(mask < "$r/out" | tail -1)|$same")
}

ADD_TOPIC='for f in '"${LOGS[*]}"'; do python3 -c "
import sys; p=sys.argv[1]; s=open(p,encoding=\"utf-8\").read(); i=s.index(\"\n# 20\")
open(p,\"w\",encoding=\"utf-8\").write(s[:i]+\"\n# '"$TODAY"' Parity test topic (PT-EL)\n\nA topic added by the parity test.\n\n---\n\"+s[i:])" "$f"; done'

case_ identity ':' '' -- identity
case_ plain_file 'echo more >> notes/a.md' '' -- ship -m "docs(test): plain file" notes/a.md
case_ submodule_file 'echo more >> Sub/s.md' '' -- ship -m "docs(test): file in a submodule" Sub/s.md
case_ token_held_back 'echo secret > notes/my_token.txt; echo more >> notes/b.md' '' -- ship -m "docs(test): token" notes/my_token.txt notes/b.md
case_ secret_named_submodule_pointer 'echo more >> Secrets/s.md' '' -- ship -m "docs(test): Secrets pointer" Secrets/s.md
case_ nothing_staged ':' '' -- ship -m "docs(test): nothing"
case_ already_in_git ':' '' -- ship -m "docs(test): already" notes/a.md
case_ rebase_conflict 'git clone -q "$D/vault.git" "$D/other" && printf "theirs\n" > "$D/other/notes/a.md" && git -C "$D/other" -c user.name=o -c user.email=o@x commit -qam theirs && git -C "$D/other" push -q origin main; printf "ours\n" > notes/a.md' '' -- ship -m "docs(test): conflict" notes/a.md
case_ unresolved_wikilink 'echo "[[No_Such_Note]]" >> notes/a.md' '' -- ship -m "docs(test): bad link" notes/a.md
case_ check_author_right ':' 'GIT_AUTHOR_NAME=eddLai GIT_AUTHOR_EMAIL=86520518+eddLai@users.noreply.github.com' -- check-author
case_ check_author_wrong ':' 'GIT_AUTHOR_NAME=someone GIT_AUTHOR_EMAIL=someone@example.com' -- check-author
case_ can_push_local_remote ':' '' -- can-push Sub
case_ can_push_own_repo 'git -C Sub remote set-url origin https://github.com/eddLai/myohand.git' '' -- can-push Sub
case_ can_push_third_party 'git -C Sub remote set-url origin git@github.com:torvalds/linux.git' '' -- can-push Sub
case_ index_updated 'echo n > Sub/new.md && git -C Sub add new.md' '' -- index
case_ index_then_unchanged 'echo n > Sub/new.md && git -C Sub add new.md && tool index' '' -- index
case_ primary_log_sync "$ADD_TOPIC" '' -- ship -m "docs(test): primary logs" "${LOGS[@]}"
case_ unknown_command ':' '' -- frobnicate

echo
echo "| case | Python exit | Python last stdout line | Rust exit | Rust last stdout line | same? |"
echo "|---|---|---|---|---|---|"
for r in "${ROWS[@]}"; do IFS='|' read -r n pc po rc ro s <<< "$r"; echo "| $n | $pc | ${po:-} | $rc | ${ro:-} | $s |"; done
printf '%s\n' "${ROWS[@]}" | grep -q DIFFERENT && exit 1 || exit 0
