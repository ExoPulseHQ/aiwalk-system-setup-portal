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
  const a = s.accounts.find(x => x.active) || {};
  methods.append(
    el("span", "method " + (a.method === "account" ? "m-account" : "m-off"), "GitHub account"),
    el("span", "method " + (a.method === "temporary" ? "m-temporary" : "m-off"), "Temporary credential"),
    el("span", "method m-off", "Security key"));
  if (a.protocol) methods.append(el("span", "sub", `git over ${a.protocol.toUpperCase()}`));
  who.append(methods);
  const out = el("button", "ghost small", "Sign out");
  out.onclick = async () => {
    const others = s.accounts.filter(x => !x.active).map(x => x.login);
    if (await ask(`Sign out ${s.user} on this computer?`, (others.length ? `${others.join(", ")} stays signed in and takes over.`
        : "git and this app stop reaching your team's repos until you sign in again.") + " Nothing on GitHub changes.",
      [["cancel", "Cancel"], ["out", "Sign out", true]]) !== "out") return;
    try { await invoke("sign_out", { login: s.user }); toast(`Signed out ${s.user}`); } catch (e) { toast(`Could not sign out: ${e}`); }
    loadTeam();
  };
  const actions = el("div", "actions");
  // gh keeps several accounts; one is active for git and this app
  const others = s.accounts.filter(x => !x.active);
  if (others.length) {
    const pick = el("select");
    pick.append(new Option("Switch account", ""), ...others.map(x => new Option(x.login, x.login)));
    pick.onchange = async () => {
      try { await invoke("switch_account", { login: pick.value }); toast(`Now using ${pick.value}`); } catch (e) { toast(`Could not switch: ${e}`); }
      loadTeam();
    };
    actions.append(pick);
  }
  const add = el("button", "ghost small", "Add account");
  add.onclick = () => teamPage().replaceChildren(signInView(null, true));
  actions.append(add, out);
  who.append(actions);
  b.append(el("div", "band"), el("div", "face", initials(name)), who);
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
  sec.append(downloadRow(v));
  const a = v.access;
  if (!a) {
    // one repo, not split: what counts is the permission on the repo itself
    const t = el("ul", "tree"), li = el("li");
    li.append(node(v.name, v.repo, v.permission, v.repo, () => null, false));
    t.append(li);
    sec.append(el("h3", null, "Documents"), t,
      el("p", "sub", v.permission ? "This vault is one repo, so your access is the same everywhere in it." : "This account cannot open this vault. Ask an owner if you need it."));
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

function signInView(error, adding) {
  const box = el("div", "empty");
  box.append(el("h1", null, adding ? "Add another GitHub account" : "Sign in with GitHub"),
    el("p", "lede", adding ? "The page that opens approves whichever account your browser is signed in to. Sign in to the other account there first, or use a private window. The new account becomes the active one; switch back from the badge."
                           : "Your team access follows your GitHub account. Sign in once on this computer; the app keeps no token of its own."));
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
  if (adding) { const back = el("button", "ghost", "Cancel"); back.style.marginLeft = "8px"; back.onclick = () => loadTeam(); btn.after(back); }
  if (error && !/not logged|auth login/i.test(error)) box.append(el("p", "sub", error));
  return box;
}

// This computer's copy of a vault: download it, bring it up to date, open it in Obsidian.
let local = { copies: {}, obsidian: true };
const downloading = {};
listen("vault-progress", e => {
  const [repo, pct] = e.payload;
  downloading[repo] = pct;
  const bar = document.getElementById("dl-" + repo);
  if (bar) { bar.hidden = false; bar.value = pct / 100; }
});

function downloadRow(v) {
  const path = local.copies[v.repo];
  const msg = el("span", "sub");
  const busy = (b, work) => async () => {
    b.disabled = true; msg.textContent = "";
    try { toast(await work()); } catch (e) { msg.textContent = e; }
    await refreshLocal(); loadTeam();
  };
  const useCopy = el("button", "small ghost", path ? "Change folder" : "Use a copy I already have");
  useCopy.onclick = async () => {
    const p = await invoke("pick_folder", { title: `Pick your copy of ${v.name}` });
    if (!p) return;
    try { toast(await invoke("vault_link", { repo: v.repo, path: p })); await refreshLocal(); loadTeam(); }
    catch (e) { msg.textContent = e; }
  };
  if (path) {
    const update = el("button", "small ghost", "Get latest"), open = el("button", "small", "Open in Obsidian");
    update.onclick = busy(update, () => invoke("vault_update", { path }));
    open.disabled = !local.obsidian;
    open.onclick = () => invoke("vault_open", { path });
    const box = el("div", "place");
    const text = el("div", "text");
    text.append(el("div", "title", "On this computer"), el("div", "sub path", path), msg);
    const buttons = el("div", "buttons");
    buttons.append(useCopy, update, open);
    box.append(text, buttons);
    return box;
  }
  if (!v.permission) return el("span");
  const name = v.repo.split("/").pop();
  const box = el("div");
  const where = el("span", "cmd");
  const sub = el("div", "sub");
  sub.append("Goes to ", where, ". Large files such as papers stay on GitHub until you open them.");
  (chosen[v.repo] ? Promise.resolve(chosen[v.repo]) : invoke("default_folder", { repo: v.repo })).then(p => where.textContent = p);

  const bar = el("progress"); bar.id = "dl-" + v.repo; bar.max = 1; bar.hidden = !(v.repo in downloading);
  const get = el("button", "small", "Download");
  get.onclick = busy(get, () => invoke("vault_download", { repo: v.repo, dest: chosen[v.repo] || null }));
  const change = el("button", "small ghost", "Change folder");
  change.onclick = async () => {
    const parent = await invoke("pick_folder", { title: `Where should ${v.name} go?` });
    if (!parent) return;
    // the vault gets its own folder inside the one picked
    chosen[v.repo] = parent.replace(/[\\/]+$/, "") + (parent.includes("\\") ? "\\" : "/") + name;
    where.textContent = chosen[v.repo];
  };
  box.className = "place";
  const text = el("div", "text");
  text.append(el("div", "title", "Not on this computer yet"), sub, msg);
  const buttons = el("div", "buttons");
  buttons.append(change, useCopy, bar, get);
  box.append(text, buttons);
  return box;
}
const chosen = {};

async function refreshLocal() {
  const s = lastTeam;
  if (s) local = await invoke("vault_local", { repos: s.vaults.map(v => v.repo) });
}

function obsidianNotice() {
  if (local.obsidian) return "";
  const b = el("button", "small", "Install Obsidian");
  b.onclick = async () => { b.disabled = true; try { toast(await invoke("obsidian_install")); } catch (e) { toast(e); } await refreshLocal(); loadTeam(); };
  const box = el("div", "list");
  box.append(item("Obsidian is not installed", "The vaults are read and edited in Obsidian.", null, b));
  return box;
}

let lastTeam = null;
async function loadTeam(viewAs) {
  const page = teamPage();
  if (!page.childElementCount) page.append(el("p", "dim", "Reading GitHub…"));
  const s = lastTeam = await invoke("team_access");
  if (!s.user) return page.replaceChildren(signInView(s.error));
  await refreshLocal();
  const owner = s.vaults.some(v => v.access && (v.access.people[s.user] || {}).grants === null);
  const parts = [el("h1", null, "Team access"),
    el("p", "lede", owner ? "You are an owner: you can see everyone's access and approve requests."
                          : "What your GitHub account reaches. Ask the owners for anything you need that is not here."),
    badge(s), obsidianNotice()];
  s.vaults.forEach(v => { if (v.access && owner) parts.push(requestsSection(v.access)); parts.push(vaultSection(v, s.user, viewAs)); });
  page.replaceChildren(...parts);
}
