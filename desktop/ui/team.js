// Team access: who you are on GitHub, what that reaches, and asking for more. Every OS gets this page.

const LABEL = { 4: "Admin", 3: "Write", 2: "Write", 1: "Read", 0: "No access" };
const teamPage = () => document.getElementById("team");

function pill(rank, title) { const p = el("span", "perm p" + rank, LABEL[rank]); p.title = title; return p; }

function initials(name) {
  const parts = name.trim().split(/\s+/);
  return (parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : name.slice(0, 2)).toUpperCase();
}

// The badge: the signed-in person, how this computer is signed in, and Sign out.
function badge(s) {
  const b = el("div", "badge");
  const name = s.name || s.user;
  const who = el("div", "who");
  who.append(el("div", "name", name), el("div", "dim", s.name ? `@${s.user}` : "Signed in on this computer"));
  const methods = el("div", "methods");
  const a = s.auth || {};
  methods.append(
    el("span", "method " + (a.method === "account" ? "m-account" : "m-off"), "GitHub account"),
    el("span", "method " + (a.method === "temporary" ? "m-temporary" : "m-off"), "Temporary credential"),
    el("span", "method m-off", "Security key"));
  if (a.protocol) methods.append(el("span", "sub", `git over ${a.protocol.toUpperCase()}`));
  who.append(methods);
  const out = el("button", "ghost small", "Sign out");
  out.onclick = async () => {
    if (await ask("Sign out of GitHub on this computer?", "git and this app stop reaching your team's repos until you sign in again. Nothing on GitHub changes.",
      [["cancel", "Cancel"], ["out", "Sign out", true]]) !== "out") return;
    try { await invoke("sign_out"); toast("Signed out"); } catch (e) { toast(`Could not sign out: ${e}`); }
    loadTeam();
  };
  const actions = el("div", "actions"); actions.append(out);
  b.append(el("div", "band"), el("div", "face", initials(name)), who, actions);
  return b;
}

// A member asks for more on one repo: pick read or write, add a note, it lands as an issue the owners see.
function requestControl(a, repo, rank) {
  const pending = a.requests.find(r => r.repo === repo);
  if (pending) return el("span", "tag", `Requested ${pending.level}`);
  const wrap = el("span", "req"), btn = el("button", "small ghost", "Request access");
  btn.onclick = () => {
    const level = el("select");
    if (rank < 1) level.append(new Option("Read", "read"));
    level.append(new Option("Write", "write"));
    const note = el("input"); note.placeholder = "What you need it for";
    const send = el("button", "small", "Send request"), msg = el("span", "sub");
    send.onclick = async () => {
      send.disabled = true;
      try { await invoke("request_access", { org: a.org, repo, level: level.value, note: note.value }); toast("Request sent to the owners"); await loadTeam(); }
      catch (e) { msg.textContent = `GitHub refused: ${e}`; send.disabled = false; }
    };
    wrap.replaceChildren(level, note, send, msg);
    note.focus();
  };
  wrap.append(btn);
  return wrap;
}

function node(title, where, rank, repo, extra, group) {
  const n = el("div", "node" + (group ? " group" : ""));
  n.append(el("span", "title", title), el("span", "where", where || ""));
  if (repo) { n.append(pill(rank, repo)); const x = extra(repo, rank); if (x) n.append(x); }
  return n;
}

function tree(t, grants, extra) {
  const li = el("li");
  const rank = t.repo ? (grants === null ? 4 : (grants[t.repo] || 0)) : 0;
  li.append(node(t.title, t.detail, rank, t.repo, extra, t.children.length));
  if (t.children.length) {
    const ul = el("ul");
    t.children.forEach(c => ul.append(tree(c, grants, extra)));
    li.append(ul);
  }
  return li;
}

// Code repos and the machines that carry them; write access to the repo is what opens its machine account.
function codeTree(a, login, grants, extra) {
  const reach = new Set(a.reach[login] || []);
  const root = el("ul", "tree");
  [...new Set(a.machines.map(m => m.repo))].forEach(repo => {
    const rank = grants === null ? 4 : (grants[repo] || 0);
    const li = el("li");
    li.append(node(repo, "", rank, repo, extra, true));
    const hosts = el("ul");
    a.machines.forEach((m, i) => {
      if (m.repo !== repo) return;
      const ok = reach.has(i);
      const n = el("div", "node");
      const where = el("span", "where" + (ok ? " cmd" : ""), ok ? `ssh ${m.account}@${m.host}` : `account ${m.account}`);
      n.append(el("span", "title", m.host), where,
        el("span", "perm " + (!ok ? "p0" : m.ready ? "p1" : "pending"), !ok ? "No login" : m.ready ? "Ready" : "Not set up"));
      const h = el("li"); h.append(n); hosts.append(h);
    });
    li.append(hosts);
    root.append(li);
  });
  return root;
}

// Owners: open requests from everyone, approve (grant and close) or decline (close).
function requestsSection(a) {
  if (!a.requests.length) return el("p", "quiet", "No access requests waiting. New ones show up here.");
  const sec = el("div", "section");
  const head = el("header");
  head.append(el("h2", null, `Waiting for you (${a.requests.length})`));
  sec.append(head);
  const list = el("div", "list");
  a.requests.forEach(r => {
    const msg = el("span", "sub");
    const note = r.body.split("\n").slice(3).join(" ").replace(/<!--.*?-->/g, "").trim();
    const act = (cmd, label, ghost) => {
      const b = el("button", "small" + (ghost ? " ghost" : ""), label);
      b.onclick = async e => {
        e.stopPropagation();
        b.parentElement.querySelectorAll("button").forEach(x => x.disabled = true);
        try { await invoke(cmd, { org: a.org, number: r.number }); toast(cmd === "approve_request" ? `${r.author} now has ${r.level} on ${r.repo}` : "Request declined"); await loadTeam(); }
        catch (err) { msg.textContent = `GitHub refused: ${err}`; b.parentElement.querySelectorAll("button").forEach(x => x.disabled = false); }
      };
      return b;
    };
    list.append(item(`${r.author} asks for ${r.level} on ${r.repo}`, note || null, null, msg,
      act("approve_request", "Approve"), act("decline_request", "Decline", true)));
  });
  sec.append(list);
  return sec;
}

function teamsList(a, login, person) {
  const sec = el("div");
  sec.append(el("h3", null, `Teams for ${person.name}`), el("p", "sub", "Ticking a team adds them on GitHub right away."));
  const list = el("div", "list"), msg = el("p", "sub");
  a.teams.forEach(t => {
    const sw = switchBox(t.members.includes(login), async (on, box) => {
      box.disabled = true;
      try { await invoke("set_team", { org: a.org, team: t.slug, login, member: on }); await loadTeam(login); }
      catch (e) { box.checked = !on; box.disabled = false; msg.textContent = `GitHub refused: ${e}`; }
    });
    list.append(item(t.slug, t.repos.map(([r, rank]) => `${r} ${LABEL[rank].toLowerCase()}`).join(", "), null, sw));
  });
  sec.append(list, msg);
  return sec;
}

function vaultSection(v, user, viewAs) {
  const sec = el("div", "section");
  const head = el("header");
  head.append(el("h2", null, v.name), el("span", "grow"));
  sec.append(head);
  const a = v.access;
  if (!a) {
    sec.append(el("p", "sub", "This account cannot read it, or it is not split into repos yet."));
    return sec;
  }
  const owner = (a.people[user] || {}).grants === null;
  const docs = el("ul", "tree"), code = el("div"), teams = el("div");
  const show = login => {
    const person = a.people[login] || { name: login, grants: {} };
    // only a member looking at their own view can ask for more
    const extra = (repo, rank) => !owner && login === user && rank < 2 ? requestControl(a, repo, rank) : null;
    docs.replaceChildren(tree(a.tree, person.grants, extra));
    code.replaceChildren(el("h3", null, "Code and machines"),
      el("p", "sub", "Write access to a code repo lets you sign in to its account on every machine that carries it. Read access does not include a login."),
      codeTree(a, login, person.grants, extra));
    teams.replaceChildren(owner && person.grants !== null && a.teams.length ? teamsList(a, login, person) : "");
  };
  if (owner) {   // owners may look through any member's eyes
    const pick = el("select");
    Object.keys(a.people).sort((x, y) => (x !== user) - (y !== user) || a.people[x].name.localeCompare(a.people[y].name))
      .forEach(l => pick.append(new Option(l === user ? "Me" : a.people[l].name === l ? l : `${a.people[l].name} (${l})`, l)));
    pick.onchange = () => show(pick.value);
    if (viewAs in a.people) pick.value = viewAs;
    const label = el("label", "sub", "View as ");
    label.append(pick);
    head.append(label);
  }
  show(viewAs in a.people ? viewAs : user);
  sec.append(el("h3", null, "Documents"), docs, code, teams);
  return sec;
}

function signInView(error) {
  const box = el("div", "empty");
  box.append(el("h1", null, "Sign in with GitHub"),
    el("p", "lede", "Your team access follows your GitHub account. Sign in once on this computer; the app keeps no token of its own."));
  const code = el("p", "sub"), btn = el("button", null, "Sign in with GitHub");
  btn.onclick = async () => {
    btn.disabled = true;
    code.textContent = "Waiting for GitHub…";
    const stop = await listen("gh-code", e => {
      code.replaceChildren("Enter this code on the GitHub page that just opened, then approve: ", el("strong", "cmd", e.payload));
    });
    const ok = await invoke("sign_in");
    stop();
    ok ? loadTeam() : (code.textContent = "Sign-in was not finished. Try again.", btn.disabled = false);
  };
  box.append(btn, code);
  if (error && !/not logged|auth login/i.test(error)) box.append(el("p", "sub", error));
  return box;
}

async function loadTeam(viewAs) {
  const page = teamPage();
  if (!page.childElementCount) page.append(el("p", "dim", "Reading GitHub…"));
  const s = await invoke("team_access");
  if (!s.user) return page.replaceChildren(signInView(s.error));
  const owner = s.vaults.some(v => v.access && (v.access.people[s.user] || {}).grants === null);
  const parts = [el("h1", null, "Team access"),
    el("p", "lede", owner ? "You are an owner: you can see everyone's access and approve requests."
                          : "What your GitHub account reaches. Ask the owners for anything you need that is not here."),
    badge(s)];
  s.vaults.forEach(v => { if (v.access && owner) parts.push(requestsSection(v.access)); parts.push(vaultSection(v, s.user, viewAs)); });
  page.replaceChildren(...parts);
}
