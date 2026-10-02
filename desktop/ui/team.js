
const LABEL = { 4: "Admin", 3: "Write", 2: "Write", 1: "Read", 0: "No access" };

function pill(rank, title) { const p = el("span", "perm p" + rank, LABEL[rank]); p.title = title; return p; }

// A member asks for more on one repo: pick read or write, add a note, it lands as an issue the owners see.
function requestControl(a, repo, rank) {
  const pending = a.requests.find(r => r.repo === repo);
  if (pending) return el("span", "tag", `Requested ${pending.level}`);
  const wrap = el("span", "req"), btn = el("button", "small", "Request");
  btn.onclick = () => {
    const level = el("select");
    if (rank < 1) level.append(new Option("Read", "read"));
    level.append(new Option("Write", "write"));
    const note = el("input"); note.placeholder = "What you need it for";
    const send = el("button", "small", "Send"), msg = el("span", "dim");
    send.onclick = async () => {
      send.disabled = true;
      try { await invoke("request_access", { org: a.org, repo, level: level.value, note: note.value }); await loadTeam(); }
      catch (e) { msg.textContent = `GitHub refused: ${e}`; send.disabled = false; }
    };
    wrap.replaceChildren(level, note, send, msg);
  };
  wrap.append(btn);
  return wrap;
}

function tree(node, grants, extra) {
  const li = el("li", node.children.length ? "group" : "");
  const row = el("div", "row");
  row.append(el("span", "title", node.title), el("span", "detail", node.detail));
  if (node.repo) {
    const rank = grants === null ? 4 : (grants[node.repo] || 0);
    row.append(pill(rank, node.repo));
    const x = extra(node.repo, rank);
    if (x) row.append(x);
  }
  li.append(row);
  if (node.children.length) {
    const ul = el("ul");
    node.children.forEach(c => ul.append(tree(c, grants, extra)));
    li.append(ul);
  }
  return li;
}

// Code repos and the machines that carry them; write access to the repo is what opens its machine account.
function codeSection(a, login, grants, extra) {
  const box = el("div");
  box.append(el("h3", null, "Code and machines"),
    el("p", "dim", "Write access to a code repo lets you sign in to its account on every machine that carries it. Read access does not include a login."));
  const reach = new Set(a.reach[login] || []);
  const ul = el("ul", "tree");
  [...new Set(a.machines.map(m => m.repo))].forEach(repo => {
    const rank = grants === null ? 4 : (grants[repo] || 0);
    const li = el("li", "group"), row = el("div", "row");
    row.append(el("span", "title", repo), el("span", "detail", ""), pill(rank, repo));
    const x = extra(repo, rank);
    if (x) row.append(x);
    const hosts = el("ul");
    a.machines.forEach((m, i) => {
      if (m.repo !== repo) return;
      const r = el("div", "row");
      const ok = reach.has(i);
      r.append(el("span", "title", m.host), el("span", "detail", ok ? `ssh ${m.account}@${m.host}` : m.account),
        el("span", "perm " + (!ok ? "p0" : m.ready ? "p1" : "pending"), !ok ? "No login" : m.ready ? "Ready" : "Not set up yet"));
      const h = el("li"); h.append(r); hosts.append(h);
    });
    li.append(row, hosts);
    ul.append(li);
  });
  box.append(ul);
  return box;
}

// Owners: open requests from everyone, approve (grant and close) or decline (close).
function requestsSection(a) {
  const box = el("div", "requests");
  box.append(el("h3", null, a.requests.length ? `Requests (${a.requests.length})` : "No open requests"));
  a.requests.forEach(r => {
    const row = el("div", "row"), msg = el("span", "dim");
    const note = r.body.split("\n").slice(3).join(" ").replace(/<!--.*?-->/g, "").trim();
    row.append(el("span", "title", r.author), el("span", "detail", `${r.level} on ${r.repo}${note ? "  " + note : ""}`));
    const act = (cmd, label) => {
      const b = el("button", "small" + (cmd === "decline_request" ? " ghost" : ""), label);
      b.onclick = async () => {
        row.querySelectorAll("button").forEach(x => x.disabled = true);
        try { await invoke(cmd, { org: a.org, number: r.number }); await loadTeam(); }
        catch (e) { msg.textContent = `GitHub refused: ${e}`; row.querySelectorAll("button").forEach(x => x.disabled = false); }
      };
      return b;
    };
    row.append(act("approve_request", "Approve"), act("decline_request", "Decline"), msg);
    box.append(row);
  });
  return box;
}

function vaultCard(v, user, viewAs) {
  const card = el("section", "card");
  card.append(el("h2", null, v.name), el("div", "dim", v.about));
  const a = v.access;
  if (!a) { card.append(el("p", "dim", "No access tree: this account cannot read the vault, or it is not split into repos.")); return card; }
  const owner = (a.people[user] || {}).grants === null;
  if (owner) card.append(requestsSection(a));
  const box = el("ul", "tree"), code = el("div"), teams = el("div", "teams");
  const show = login => {
    const person = a.people[login] || { grants: {} };
    // only a member looking at their own view can ask for more
    const extra = (repo, rank) => !owner && login === user && rank < 2 ? requestControl(a, repo, rank) : null;
    box.replaceChildren(tree(a.tree, person.grants, extra));
    code.replaceChildren(codeSection(a, login, person.grants, extra));
    teams.replaceChildren();
    if (!a.teams.length || person.grants === null) return;   // owners already reach everything
    const alertLine = el("p", "dim");
    teams.append(el("h3", null, `Teams for ${person.name}`));
    a.teams.forEach(t => {
      const cb = el("input"); cb.type = "checkbox"; cb.checked = t.members.includes(login);
      cb.onchange = async () => {
        cb.disabled = true;
        try {
          await invoke("set_team", { org: a.org, team: t.slug, login, member: cb.checked });
          await loadTeam(login);
        } catch (e) { cb.checked = !cb.checked; cb.disabled = false; alertLine.textContent = `GitHub refused: ${e}`; }
      };
      const label = el("label", "team");
      label.append(cb, el("span", "title", " " + t.slug), el("span", "detail",
        t.repos.map(([r, rank]) => `${r} ${LABEL[rank].toLowerCase()}`).join("  ")));
      teams.append(label);
    });
    teams.append(alertLine);
  };
  if (owner) {   // owners may preview any member's view
    const pick = el("select");
    Object.keys(a.people).sort((x, y) => (x !== user) - (y !== user) || a.people[x].name.localeCompare(a.people[y].name))
      .forEach(l => pick.append(new Option(a.people[l].name === l ? l : `${a.people[l].name} (${l})`, l)));
    pick.onchange = () => show(pick.value);
    if (viewAs in a.people) pick.value = viewAs;
    const label = el("label", null, "View as ");
    label.append(pick);
    card.append(label);
  }
  show(viewAs in a.people ? viewAs : user);
  card.append(el("h3", null, "Documents"), box, code, teams);
  return card;
}

const who = document.getElementById("who"), vaults = document.getElementById("vaults");

function signInCard(error) {
  const card = el("section", "card");
  card.append(el("h2", null, "Sign in with GitHub"),
    el("p", "dim", "Everything here follows your GitHub account. Sign in once; the app keeps no token of its own."));
  if (error) card.append(el("p", "dim", error));
  const code = el("p", "code"), btn = el("button", null, "Sign in");
  btn.onclick = async () => {
    btn.disabled = true;
    code.textContent = "Waiting for GitHub…";
    const stop = await window.__TAURI__.event.listen("gh-code", e => {
      code.textContent = `Your one-time code is ${e.payload}. Paste it on the GitHub page that just opened and approve.`;
    });
    const ok = await invoke("sign_in");
    stop();
    ok ? loadTeam() : (code.textContent = "Sign-in was not completed.", btn.disabled = false);
  };
  card.append(btn, code);
  return card;
}

async function loadTeam(viewAs) {
  who.textContent = "Reading GitHub…";
  const s = await invoke("team_access");
  if (!s.user) { who.textContent = ""; vaults.replaceChildren(signInCard(s.error)); return; }
  who.textContent = `Signed in to GitHub as ${s.user}`;
  vaults.replaceChildren(...s.vaults.map(v => vaultCard(v, s.user, viewAs)));
}
