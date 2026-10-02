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
    try { await working(out, "Signing out", () => invoke("sign_out", { login: s.user })); toast(`Signed out ${s.user}`); } catch (e) { toast(`Could not sign out: ${e}`); }
    loadTeam();
  };
  const actions = el("div", "actions");
  // gh keeps several accounts; one is active for git and this app
  const others = s.accounts.filter(x => !x.active);
  if (others.length) {
    const pick = el("select");
    pick.append(new Option("Switch account", ""), ...others.map(x => new Option(x.login, x.login)));
    pick.onchange = async () => {
      pick.disabled = true;
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
      try { await working(send, "Sending", () => invoke("request_access", { org: a.org, repo, level: level.value, note: note.value })); toast("Request sent to the owners"); await loadTeam(); }
      catch (e) { msg.textContent = `GitHub refused: ${e}`; }
    };
    wrap.replaceChildren(level, note, send, msg);
    note.focus();
  };
  wrap.append(btn);
  return wrap;
}

// Set by vaultSection for owners: clicking a permission opens who-can-open-this for that repo.
let editAccess = null;

function node(title, where, rank, repo, extra, group) {
  const n = el("div", "node" + (group ? " group" : ""));
  n.append(el("span", "title", title), el("span", "where", where || ""));
  if (repo) {
    const p = pill(rank, repo);
    if (editAccess) { const ed = editAccess; p.classList.add("edit"); p.tabIndex = 0; p.title = `Who can open ${repo}`; p.onclick = () => ed(repo); p.onkeydown = e => e.key === "Enter" && ed(repo); }
    n.append(p);
    const x = extra(repo, rank); if (x) n.append(x);
  }
  return n;
}

// Owners: everyone's level on one repo, changed in place.
async function accessDialog(a, repo) {
  const box = el("div", "access-list");
  const msg = el("p", "sub");
  Object.entries(a.people).sort(([, x], [, y]) => x.name.localeCompare(y.name)).forEach(([login, p]) => {
    const row = el("div", "item");
    const text = el("div", "text");
    text.append(el("div", "title", p.name === login ? login : p.name), el("div", "sub", login));
    row.append(text);
    if (p.grants === null) { row.append(pill(4, "Owners can open every repo")); box.append(row); return; }
    const now = Math.min(p.grants[repo] || 0, 2);
    const pick = el("select");
    [["0", "No access"], ["1", "Read"], ["2", "Write"]].forEach(([v, l]) => pick.append(new Option(l, v)));
    pick.value = String(now);
    pick.onchange = async () => {
      pick.disabled = true; msg.textContent = "";
      const spin = el("span", "spinner"); pick.before(spin);
      try { toast(await invoke("set_access", { org: a.org, repo, login, level: +pick.value })); p.grants[repo] = +pick.value; }
      catch (e) { msg.textContent = e; pick.value = String(now); }
      spin.remove(); pick.disabled = false;
    };
    row.append(pick);
    box.append(row);
  });
  box.append(msg);
  await ask(`Who can open ${repo}`, "Changes apply on GitHub as soon as you pick them.", [["done", "Done", true]], box);
  loadTeam();
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

// Lab machines, one row each, and whether this computer can connect to it right now. Members only ever
// connect through Cloudflare (Access for the GitHub sign-in, a tunnel for SSH), never to a machine's address.
// Which project lives where is the vault plugin's job; here only the machine matters.
function machinesSection(machines) {
  const sec = el("div", "section");
  const head = el("header");
  const again = el("button", "small ghost", "Check again");
  head.append(el("h2", null, "Lab machines"), el("span", "grow"), again);
  sec.append(head, el("p", "sub", "Whether this computer can reach each machine right now, through Cloudflare."));
  const hosts = [...new Map(machines.map(m => [m.host, m])).values()];
  const byHost = Object.fromEntries(hosts.map(m => [m.host, m]));
  const carries = h => machines.filter(m => m.host === h).map(m => m.repo).join(", ");
  const list = el("div", "list"), pills = {};
  hosts.forEach(m => {
    const p = el("span", "perm checking"); p.append(el("span", "spinner"));
    pills[m.host] = p;
    const right = [];
    if (m.sometimes) right.push(el("span", "tag", "Often off"));
    if (m.via) right.push(el("span", "tag", `Through ${m.via}`));
    list.append(item(m.host, [m.note, `Carries ${carries(m.host)}`].filter(Boolean).join(". "), null, ...right, p));
  });
  const help = el("div");
  sec.append(help, list);

  const LOOK = { up: ["p4", "Can connect"], down: ["p0", "Can't connect"], "no-tunnel": ["p0", "Tunnel not set up"],
                 "sign-in": ["pending", "Sign in needed"], "no-cloudflared": ["p0", "cloudflared missing"] };
  // a machine reached through another one shares that one's way in
  const way = m => byHost[m.via] || m;
  const check = async () => {
    Object.values(pills).forEach(p => { p.className = "perm checking"; p.replaceChildren(el("span", "spinner")); });
    const targets = [...new Map(hosts.map(m => [way(m).host, way(m).tunnel || null])).entries()];
    const state = await working(again, "Checking", () => invoke("reachable", { machines: targets }));
    hosts.forEach(m => {
      const [cls, text] = LOOK[state[way(m).host]] || LOOK.down;
      pills[m.host].className = "perm " + cls; pills[m.host].textContent = text;
    });
    await showHelp(state);
  };

  // what this computer still needs: a Cloudflare sign-in, the ssh aliases
  async function showHelp(state) {
    help.replaceChildren();
    const needSignIn = hosts.find(m => state[way(m).host] === "sign-in");
    if (needSignIn) {
      const b = el("button", "small", "Sign in to Cloudflare");
      b.onclick = async () => { try { toast(await working(b, "Waiting for the browser", () => invoke("access_login", { tunnel: way(needSignIn).tunnel }))); } catch (e) { toast(e); } check(); };
      help.append(item("Sign in to Cloudflare once on this computer", "A browser opens; sign in with GitHub. Cloudflare lets in members of the team only.", null, b));
    }
    const ssh = await invoke("ssh_status", { machines });
    if (ssh === "missing") {
      const b = el("button", "small", "Set up connections");
      b.onclick = async () => {
        if (await ask("Set up connections on this computer?", "The app adds the lab machines to your SSH settings (~/.ssh/config) as one marked block, so ssh <account>@host-20 goes through Cloudflare. The rest of the file stays as it is, and the old file is kept as config.bak.",
          [["cancel", "Cancel"], ["ok", "Set up connections", true]]) !== "ok") return;
        try { toast(await working(b, "Setting up", () => invoke("ssh_setup", { machines }))); } catch (e) { toast(e); }
        showHelp(state);
      };
      help.append(item("Connections are not set up on this computer", "Needed once, so ssh knows to go through Cloudflare.", null, b));
    } else if (ssh === "current") {
      help.append(el("p", "sub", "Connections are set up on this computer."));
    }
  }
  again.onclick = check;
  check();
  return sec;
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
        const others = [...b.parentElement.querySelectorAll("button")].filter(x => x !== b);
        others.forEach(x => x.disabled = true);
        try { await working(b, cmd === "approve_request" ? "Approving" : "Declining", () => invoke(cmd, { org: a.org, number: r.number })); toast(cmd === "approve_request" ? `${r.author} now has ${r.level} on ${r.repo}` : "Request declined"); await loadTeam(); }
        catch (err) { msg.textContent = `GitHub refused: ${err}`; others.forEach(x => x.disabled = false); }
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
      const spin = el("span", "spinner"); box.before(spin);
      try { await invoke("set_team", { org: a.org, team: t.slug, login, member: on }); await loadTeam(login); }
      catch (e) { box.checked = !on; box.disabled = false; msg.textContent = `GitHub refused: ${e}`; }
      finally { spin.remove(); }
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
  editAccess = owner ? repo => accessDialog(a, repo) : null;
  const docs = el("ul", "tree"), teams = el("div");
  const show = login => {
    const person = a.people[login] || { name: login, grants: {} };
    // only a member looking at their own view can ask for more
    const extra = (repo, rank) => !owner && login === user && rank < 2 ? requestControl(a, repo, rank) : null;
    docs.replaceChildren(tree(a.tree, person.grants, extra));
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
  const render = show;
  // keep pills editable for re-renders under View as
  const showEditable = login => { editAccess = owner ? repo => accessDialog(a, repo) : null; render(login); editAccess = null; };
  if (owner) head.querySelector("select").onchange = e => showEditable(e.target.value);
  showEditable(viewAs in a.people ? viewAs : user);
  sec.append(el("h3", null, "Documents"), docs, teams);
  if (owner) sec.insertBefore(el("p", "sub", "Click a permission to choose who can open that repo."), docs);
  return sec;
}

function signInView(error, adding) {
  const box = el("div", "empty");
  box.append(el("h1", null, adding ? "Add another GitHub account" : "Sign in with GitHub"),
    el("p", "lede", adding ? "The page that opens approves whichever account your browser is signed in to. Sign in to the other account there first, or use a private window. The new account becomes the active one; switch back from the badge."
                           : "Your team access follows your GitHub account. Sign in once on this computer; the app keeps no token of its own."));
  const code = el("p", "sub"), btn = el("button", null, "Sign in with GitHub");
  btn.onclick = async () => {
    code.textContent = "";
    const stop = await listen("gh-code", e => {
      code.replaceChildren("Enter this code on the GitHub page that just opened, then approve: ", el("strong", "cmd", e.payload));
    });
    const ok = await working(btn, "Waiting for GitHub", () => invoke("sign_in"));
    stop();
    ok ? loadTeam() : (code.textContent = "Sign-in was not finished. Try again.");
  };
  box.append(btn, code);
  if (adding) { const back = el("button", "ghost", "Cancel"); back.style.marginLeft = "8px"; back.onclick = () => loadTeam(); btn.after(back); }
  if (error && !/not logged|auth login/i.test(error)) box.append(el("p", "sub", error));
  return box;
}

// This computer's copy of a vault: download it, bring it up to date, open it in Obsidian.
let local = { copies: {}, obsidian: true };
const stages = {};   // repo -> the stage box of a running download or update, kept across re-renders
listen("vault-progress", e => {
  const [repo, f, text] = e.payload;
  if (stages[repo]) stages[repo].set(f, text);
});

function downloadRow(v) {
  const path = local.copies[v.repo];
  const msg = el("span", "sub");
  // a long git job: the button says what it is doing and a stage box under the text follows its steps
  const busy = (b, label, work) => async () => {
    msg.textContent = "";
    stages[v.repo] = stage(label);
    text.append(stages[v.repo]);
    try { toast(await working(b, label, work)); } catch (e) { msg.textContent = e; }
    delete stages[v.repo];
    await refreshLocal(); loadTeam();
  };
  const text = el("div", "text");
  const useCopy = el("button", "small ghost", path ? "Change folder" : "Use a copy I already have");
  useCopy.onclick = async () => {
    const p = await invoke("pick_folder", { title: `Pick your copy of ${v.name}` });
    if (!p) return;
    try { toast(await working(useCopy, "Checking that folder", () => invoke("vault_link", { repo: v.repo, path: p }))); await refreshLocal(); loadTeam(); }
    catch (e) { msg.textContent = e; }
  };
  if (path) {
    const update = el("button", "small ghost", "Get latest"), open = el("button", "small", "Open in Obsidian");
    update.onclick = busy(update, "Getting the latest", () => invoke("vault_update", { repo: v.repo, path }));
    open.disabled = !local.obsidian;
    open.onclick = () => invoke("vault_open", { path });
    const box = el("div", "place");
    text.append(el("div", "title", "On this computer"), el("div", "sub path", path), msg);
    if (stages[v.repo]) text.append(stages[v.repo]);
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

  const get = el("button", "small", "Download");
  get.onclick = busy(get, "Downloading", () => invoke("vault_download", { repo: v.repo, dest: chosen[v.repo] || null }));
  const change = el("button", "small ghost", "Change folder");
  change.onclick = async () => {
    const parent = await invoke("pick_folder", { title: `Where should ${v.name} go?` });
    if (!parent) return;
    // the vault gets its own folder inside the one picked
    chosen[v.repo] = parent.replace(/[\\/]+$/, "") + (parent.includes("\\") ? "\\" : "/") + name;
    where.textContent = chosen[v.repo];
  };
  box.className = "place";
  text.append(el("div", "title", "Not on this computer yet"), sub, msg);
  if (stages[v.repo]) text.append(stages[v.repo]);
  const buttons = el("div", "buttons");
  buttons.append(change, useCopy, get);
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
  b.onclick = async () => { try { toast(await working(b, "Installing Obsidian, a few minutes", () => invoke("obsidian_install"))); } catch (e) { toast(e); } await refreshLocal(); loadTeam(); };
  const box = el("div", "list");
  box.append(item("Obsidian is not installed", "The vaults are read and edited in Obsidian.", null, b));
  return box;
}

// gh and git come with the app (git only on Windows); what is still missing blocks sign-in, with the fix.
function missingTools(t) {
  const box = el("div", "empty");
  box.append(el("h1", null, "One more thing before signing in"),
    el("p", "lede", "This app talks to GitHub through two small programs. This computer is missing:"));
  const list = el("div", "list");
  if (!t.gh) list.append(item("GitHub CLI (gh)", "It normally comes with this app. Reinstall the app, or install gh from cli.github.com.", null));
  if (!t.git) {
    if (t.os === "macos") {
      const b = el("button", "small", "Install git");
      b.onclick = async () => { b.disabled = true; try { await invoke("install_git"); toast("Follow the installer macOS opened, then come back"); } catch (e) { toast(e); b.disabled = false; } };
      list.append(item("git", "macOS installs it with its command line tools. It takes a few minutes.", null, b));
    } else {
      list.append(item("git", t.os === "linux" ? "Install it from a terminal: sudo apt install git (Ubuntu, Debian), then come back."
                                              : "It normally comes with this app. Reinstall the app.", null));
    }
  }
  const again = el("button", null, "Check again");
  again.onclick = () => loadTeam();
  box.append(list, again);
  return box;
}

// Owners: the organisation's people, invitations and owner role.
async function peopleSection(org, user, teams, tree) {
  const sec = el("div", "section");
  const head = el("header");
  head.append(el("h2", null, "People"), el("span", "grow"));
  sec.append(head, el("p", "sub", `Everyone in ${org} on GitHub. Owners can open every repo and change everyone's access.`));
  let p;
  try { p = await invoke("org_people", { org }); } catch (e) { sec.append(el("p", "sub", `Could not read the organisation: ${e}`)); return sec; }
  const msg = el("p", "sub");
  const act = (fn, done, button, label) => async () => {
    msg.textContent = "";
    try { toast(await (button ? working(button, label, fn) : fn())); await loadTeam(); } catch (e) { msg.textContent = e; done && done(); }
  };

  const list = el("div", "list");
  p.members.sort((x, y) => (y.owner - x.owner) || x.name.localeCompare(y.name)).forEach(m => {
    const role = el("select");
    role.append(new Option("Member", "member"), new Option("Owner", "owner"));
    role.value = m.owner ? "owner" : "member";
    role.disabled = m.login === user;   // an owner does not demote themselves here
    role.onchange = async () => {
      const toOwner = role.value === "owner";
      if (await ask(toOwner ? `Make ${m.name} an owner?` : `Make ${m.name} a member?`,
          toOwner ? "Owners can open every repo, change everyone's access, invite and remove people." : "They keep only the access their teams and grants give them.",
          [["cancel", "Cancel"], ["ok", toOwner ? "Make owner" : "Make member", true]]) !== "ok") { role.value = m.owner ? "owner" : "member"; return; }
      act(() => invoke("set_role", { org, login: m.login, owner: toOwner }), () => role.value = m.owner ? "owner" : "member")();
    };
    const right = [role];
    if (m.login !== user) {
      const rm = el("button", "small ghost", "Remove");
      rm.onclick = async () => {
        if (await ask(`Remove ${m.name} from ${org}?`, "Their teams and repo access end now. Copies already on their computer stay there.",
            [["cancel", "Cancel"], ["rm", "Remove", true]]) === "rm") act(() => invoke("remove_member", { org, login: m.login }), null, rm, "Removing")();
      };
      right.push(rm);
    }
    list.append(item(m.name === m.login ? m.login : m.name, m.name === m.login ? null : m.login, null, ...right));
  });
  p.invites.forEach(i => {
    const cancel = el("button", "small ghost", "Cancel invitation");
    cancel.onclick = () => act(() => invoke("cancel_invite", { org, id: i.id }), null, cancel, "Cancelling")();
    list.append(item(i.login, `Invited ${i.created}${i.owner ? " as owner" : ""}, not accepted yet`, null, el("span", "tag", "Invited"), cancel));
  });
  sec.append(list);

  // invite: a GitHub username, a role, and the layers they start in; everyone also gets the shared team
  const form = el("div", "invite");
  form.append(el("h3", null, "Invite someone"));
  const who = el("input", "who"); who.placeholder = "Their GitHub username"; who.autocomplete = "off"; who.spellcheck = false;
  form.append(who);

  let asOwner = false;
  const seg = el("div", "segmented"); seg.setAttribute("role", "radiogroup"); seg.setAttribute("aria-label", "Role");
  const roleBtn = (label, owner) => {
    const b = el("button", null, label); b.type = "button"; b.setAttribute("role", "radio");
    b.onclick = () => { asOwner = owner; paint(); };
    return b;
  };
  const memberBtn = roleBtn("Member", false), ownerBtn = roleBtn("Owner", true);
  seg.append(memberBtn, ownerBtn);
  const roleRow = el("div", "field");
  roleRow.append(el("span", "label", "Role"), seg);
  form.append(roleRow);

  // the layer a team opens, by the name people know (L1 Sensing), from the access tree
  const names = {};
  const walk = n => { if (n.repo) names[n.repo] = n.title; n.children.forEach(walk); };
  if (tree) walk(tree);
  const shared = teams.find(t => t.slug === "members");
  const tiles = el("div", "tiles"), chosenTeams = new Set();
  teams.filter(t => t !== shared).forEach(t => {
    const repos = t.repos.map(([r]) => r);
    const title = repos.map(r => names[r]).find(Boolean) || t.slug;
    const tile = el("button", "tile"); tile.type = "button"; tile.setAttribute("aria-pressed", "false");
    tile.append(el("span", "tile-title", title), el("span", "tile-sub", repos.join(", ")));
    tile.onclick = () => { chosenTeams.has(t.slug) ? chosenTeams.delete(t.slug) : chosenTeams.add(t.slug); paint(); };
    tile.dataset.slug = t.slug;
    tiles.append(tile);
  });
  const startsRow = el("div", "field top");
  const ownerNote = el("p", "sub");
  startsRow.append(el("span", "label", "Starts in"), tiles);
  form.append(startsRow, ownerNote);

  const send = el("button", null, "Send invitation");
  const foot = el("div", "foot"); foot.append(send);
  form.append(foot);

  function paint() {
    memberBtn.setAttribute("aria-checked", String(!asOwner)); ownerBtn.setAttribute("aria-checked", String(asOwner));
    tiles.querySelectorAll(".tile").forEach(tile => {
      tile.disabled = asOwner;
      tile.setAttribute("aria-pressed", String(!asOwner && chosenTeams.has(tile.dataset.slug)));
    });
    ownerNote.textContent = asOwner ? "Owners open every repo, so they need no layers."
      : shared ? "Everyone also gets the shared notes and papers, and can ask the owners for more." : "";
  }
  paint();

  send.onclick = async () => {
    if (!who.value.trim()) return who.focus();
    const picked = asOwner ? [] : [...chosenTeams, ...(shared ? [shared.slug] : [])];
    await act(() => invoke("invite", { org, login: who.value, owner: asOwner, teams: picked }), null, send, "Inviting")();
  };
  sec.append(form, msg);
  return sec;
}

let lastTeam = null;
async function loadTeam(viewAs) {
  const page = teamPage();
  // first load fills the page with the steps; later reloads keep the page and show the top line only
  const first = !page.querySelector(".badge");
  const progress = stage("Reading your access from GitHub");
  if (first) page.replaceChildren(el("h1", null, "Team access"), progress);
  const t = await invoke("tools");
  if (!t.gh || !t.git) return page.replaceChildren(missingTools(t));
  const stop = await listen("team-stage", e => { const [i, n, text] = e.payload; progress.set(i / n, text); });
  const s = lastTeam = await invoke("team_access").finally(stop);
  if (!s.user) return page.replaceChildren(signInView(s.error));
  await refreshLocal();
  const owner = s.vaults.some(v => v.access && (v.access.people[s.user] || {}).grants === null);
  const parts = [el("h1", null, "Team access"),
    el("p", "lede", owner ? "You are an owner: you can see everyone's access and approve requests."
                          : "What your GitHub account reaches. Ask the owners for anything you need that is not here."),
    badge(s), obsidianNotice()];
  const org = (s.vaults.find(v => v.access) || {}).access;
  if (owner && org) { progress.set(1, "Reading the organisation's people"); parts.push(requestsSection(org), await peopleSection(org.org, s.user, org.teams, org.tree)); }
  const machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
  if (machines.length) parts.push(machinesSection(machines));
  s.vaults.forEach(v => parts.push(vaultSection(v, s.user, viewAs)));
  page.replaceChildren(...parts);
}
