// Team access: who you are on GitHub, what that reaches, and asking for more. Every OS gets this page.

const LABEL = { 4: "Admin", 3: "Write", 2: "Write", 1: "Read", 0: "No access" };
const teamPage = () => document.getElementById("team");
// core is the owners' team, machine-* the machines' extras and merge-* the merge rights: not layers to tick or join
const layerTeam = t => t.slug !== "core" && !/^(machine|merge)-/.test(t.slug);

// Each level has a glyph; its tooltip says what the level allows.
const LEVEL_SAYS = { Admin: ["shield", "Admin: opens the repo and changes who else can"], Write: ["pencil", "Write: opens the repo and changes its files"],
  Read: ["eye", "Read: opens and downloads the repo, no changes"], "No access": ["lock", "No access: ask an owner"] };
// A level: its glyph alone (the access tree), "worded" with its glyph (a vault's heading), or "plain" words (dialogs).
function pill(rank, look = "glyph", tip) {
  const word = LABEL[rank], [name, says] = LEVEL_SAYS[word];
  if (look === "glyph") return glyph("perm p" + rank, name, word, says);
  const p = el("span", "perm p" + rank);
  p.append(...(look === "worded" ? [icon(name)] : []), word);
  p.dataset.tip = tip || says;
  return p;
}
// The icon and words before a line of the identity block.
const lineLabel = (text, name) => { const l = el("span", "label"); l.append(icon(name), text); return l; };

// A person in a list: the profile name over the login chip, or the login alone (as the title chip) with a second line
// that says there is no profile name, so every person row has the same two lines.
function personRow(login, name, ...right) {
  const known = name && name !== login;
  const r = kind(item(known ? name : login, known ? chip(login) : "No display name on GitHub", null, ...right), "user-round");
  if (!known) r.querySelector(".title").classList.add("chip");
  return r;
}

// Who may use a thing, as one wrapping line of people grouped by why: the reason is a quiet label said once, the
// person is the chip. people: [{login, name, why: [group, ...], rm: {tip, run(button)} | null}], groups in `order`.
// Added-by-hand people carry their own remove mark; `pick` (a select whose first option names the act) opens from a
// "+" at the end of the line. Used for who can connect to a machine and who can merge on a repo.
function whoLine(people, order, pick, label) {
  const line = el("div", "who-line"); line.setAttribute("role", "list"); line.setAttribute("aria-label", label);
  const groups = new Map(order.map(g => [g, []]));
  people.forEach(p => { const g = p.why[0]; if (!groups.has(g)) groups.set(g, []); groups.get(g).push(p); });
  groups.forEach((ps, g) => {
    if (!ps.length) return;
    const grp = el("span", "who-g"); grp.setAttribute("role", "listitem");
    grp.append(el("span", "who-k", g));
    ps.forEach(p => {
      const c = chip(p.login, [p.name !== p.login ? p.name : "", p.why.length > 1 ? `also ${p.why.slice(1).join(", ")}` : ""].filter(Boolean).join(", ") || p.login);
      c.tabIndex = 0;
      if (p.rm) {
        const x = el("button", "x"); x.type = "button"; x.append(icon("x"));
        x.dataset.tip = p.rm.tip; x.setAttribute("aria-label", p.rm.tip);
        x.onclick = e => { e.stopPropagation(); p.rm.run(x); };
        c.classList.add("has-x"); c.append(x);
      }
      grp.append(c);
    });
    line.append(grp);
  });
  if (!people.length) line.append(el("span", "who-k", "Nobody yet"));
  if (pick) {
    const wrap = el("span", "who-add"), b = iconButton("user-plus", pick.options[0].text, "small ghost add", true);
    pick.hidden = true;
    b.onclick = e => { e.stopPropagation(); pick.hidden = false; b.hidden = true; pick.focus(); try { pick.showPicker(); } catch {} };
    pick.addEventListener("blur", () => { if (!pick.value) { pick.hidden = true; b.hidden = false; } });
    wrap.append(b, pick); line.append(wrap);
  }
  return line;
}

// The badge: the signed-in person, how this computer is signed in, and Sign out.
function badge(s) {
  const b = el("div", "badge");
  const who = el("div", "who");
  // the name in the display treatment over the login chip; without a profile name, the login alone, as the chip
  if (s.name) who.append(el("div", "name", s.name), chip(s.user)); else who.append(el("div", "name chip", s.user));
  const methods = el("div", "methods claude-line");
  const a = s.accounts.find(x => x.active) || {};
  // the way in use keeps its words; the other ways to sign in are their icon
  const way = (m, word, cls, mark, other) => a.method === m ? stateText(el("span", "method " + cls), mark, word)
    : glyph("method m-off", other, word, `${word}: another way to sign in, not in use`);
  methods.append(lineLabel("GitHub", "github"),
    way("account", "GitHub account", "m-account", "check", "user-round"),
    way("temporary", "Temporary credential", "m-temporary", "circle-alert", "clock"),
    glyph("method m-off", "key-round", "Security key", "Security key: another way to sign in, not in use"));
  if (a.protocol) methods.append(el("span", "sub", `git over ${a.protocol.toUpperCase()}`));
  who.append(methods, labLine(s), claudeLine());
  const out = iconButton("log-out", "Sign out of GitHub", "ghost small", true);
  out.onclick = async () => {
    const others = s.accounts.filter(x => !x.active).map(x => x.login);
    if (await ask(`Sign out ${s.user} on this computer?`, (others.length ? `${others.join(", ")} stays signed in and takes over.`
        : "git and this app stop reaching your team's repos until you sign in again.") + " Nothing on GitHub changes.",
      [["cancel", "Cancel"], ["out", "Sign out", true]]) !== "out") return;
    try { await working(out, "Signing out", () => invoke("sign_out", { login: s.user })); termsNow = null; toast(`Signed out ${s.user}`); } catch (e) { toast(`Could not sign out: ${e}`); }
    loadTeam();
  };
  const actions = el("div", "actions");
  // gh keeps several accounts; one is active for git and this app
  const others = s.accounts.filter(x => !x.active);
  if (others.length) {
    const pick = el("select");
    pick.append(new Option("Switch GitHub account", ""), ...others.map(x => new Option(x.login, x.login)));
    pick.onchange = async () => {
      pick.disabled = true;
      try { await invoke("switch_account", { login: pick.value }); termsNow = null; toast(`Now using ${pick.value}; step 2 signs this account in to the machines`); labSignInNext = true; }
      catch (e) { toast(`Could not switch: ${e}`); }
      loadTeam();
    };
    actions.append(pick);
  }
  const add = iconButton("user-plus", "Add a GitHub account", "ghost small", true);
  add.onclick = () => teamPage().replaceChildren(signInView(null, "add"));
  actions.append(add, out);
  who.append(actions);
  b.append(who);
  return b;
}

// Sign-in is one thing in two steps: 1 GitHub (gh, for repos), 2 the machines (Cloudflare Access, its own
// GitHub sign-in in the browser, because Access cannot take gh's token). Both must be the same person, so step 2 lives
// here next to step 1, shows whom Cloudflare knows, and is redone whenever the GitHub account changes.
function labLine(s) {
  const tunnels = myTunnels(s);
  const line = el("div", "claude-line");
  if (!tunnels.length) return line;
  let topped = false;
  const state = el("span", "method m-off"), msg = el("span", "sub");
  line.append(lineLabel("Machines", "server"), state, msg);
  const step2 = async b => {
    msg.textContent = "";
    // the machines are signed in to one after another: a ring fills as each one is done
    const ring = el("span", "ring"), count = el("span");
    const show = (done, total, now) => {
      ring.style.setProperty("--p", total ? 100 * done / total : 0);
      ring.dataset.tip = `${done} of ${total} machines signed in`;
      count.textContent = now ? ` ${done} of ${total}, now ${now}` : ` ${done} of ${total}`;
    };
    show(0, tunnels.length, "");
    state.className = "method m-off"; state.replaceChildren(ring, count);
    const stop = await listen("lab-progress", e => show(...e.payload));
    try { await working(b, "Step 2 of 2: waiting for the browser", () => invoke("access_login", { tunnels })); }
    catch (e) { msg.textContent = e; }
    stop();
    window.dispatchEvent(new Event("lab-signed-in"));
    paint();
  };
  const paint = async () => {
    line.querySelectorAll("button").forEach(b => b.remove());
    state.className = "method m-off"; state.replaceChildren(el("span", "spinner"));
    const id = await invoke("lab_identity", { tunnels });
    if (!id) {
      state.textContent = "Step 2 of 2 not done";
      msg.textContent = "A browser opens with your GitHub account; no Cloudflare account is needed.";
      const b = el("button", "small", "Finish signing in"); b.dataset.primary = 1;
      b.onclick = () => step2(b);
      line.append(b);
      todo("Machines", true);
      if (labSignInNext) { labSignInNext = false; step2(b); }
      return;
    }
    const until = new Date(id.expires * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });
    if (id.matches === false) {
      // someone else's GitHub account in the browser: the machines would see a different person than this app
      state.className = "method m-temporary"; stateText(state, "circle-alert", id.email, "Signed in as ");
      msg.textContent = `That is not ${s.user}. Sign out of GitHub in the browser (or use the right account there), then sign in again.`;
      const b = el("button", "small", "Sign in again");
      b.onclick = async () => { await invoke("lab_sign_out"); step2(b); };
      line.append(b);
      todo("Machines", true);
      return;
    }
    // a machine added or split off since the last sign-in: the team sign-in covers it without the browser
    if (id.missing && !topped) { topped = true; const b = el("button", "small", "Finish signing in"); b.dataset.primary = 1; line.append(b); return step2(b); }
    state.className = "method m-key"; stateText(state, "check", id.email, "Signed in as ");
    todo("Machines", false);
    msg.textContent = `until ${until}, ` + (id.matches ? `same person as @${s.user}` : `not checked against @${s.user}: this GitHub sign-in predates the email check, sign out and in once`);
  };
  paint();
  return line;
}

// Claude Code on this computer: the team's vault workflow runs through it, so the badge says whether it is ready.
function claudeLine() {
  const line = el("div", "claude-line");
  const label = lineLabel("Claude Code", "square-terminal");
  const state = el("span", "method m-off"); state.append(el("span", "spinner"));
  const msg = el("span", "sub");
  line.append(label, state, msg);
  const PLAN = { max: "Max", pro: "Pro", team: "Team", enterprise: "Enterprise" };
  const paint = async () => {
    const c = await invoke("claude_state");
    line.querySelectorAll("button").forEach(b => b.remove());
    if (!c.path) {
      state.className = "method m-off"; state.textContent = "Not installed";
      const b = el("button", "small", "Install Claude Code");
      b.onclick = () => run(b, "Installing", "claude_install");
      line.append(b);
    } else if (!c.signed_in) {
      state.className = "method m-temporary"; stateText(state, "circle-alert", `Installed ${c.version}, not signed in`);
      const b = el("button", "small", "Sign in to Claude");
      b.onclick = () => run(b, "Waiting for the browser", "claude_login");
      line.append(b);
    } else {
      state.className = "method m-key";
      const plan = PLAN[c.plan] ? `, ${PLAN[c.plan]} plan` : "";
      if (c.method) stateText(state, "check", (c.method === "claude.ai" ? "Claude.ai" : c.method) + plan, "Signed in with ");
      else stateText(state, "check", `Signed in${plan}`);
      msg.textContent = c.version;
    }
    todo("Claude Code", !!c.path && !c.signed_in);
  };
  async function run(b, label, cmd) {
    msg.textContent = "";
    const stop = await listen("claude-step", e => { msg.textContent = e.payload; });
    try { toast(await working(b, label, () => invoke(cmd))); } catch (e) { msg.textContent = e; }
    stop(); paint();
  }
  paint();
  return line;
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
    let p = pill(rank);
    if (editAccess) {
      // an owner changes who can open the repo from its level: a real button, so keyboard and screen readers reach it
      const ed = editAccess, word = LABEL[rank];
      p = el("button", `perm p${rank} glyph edit`); p.type = "button";
      p.append(icon(LEVEL_SAYS[word][0]), el("span", "sr", word));
      p.dataset.tip = `${LEVEL_SAYS[word][1]}. Click to change who can open it.`;
      p.setAttribute("aria-label", `${word}. Who can open ${repo}`);
      p.onclick = () => ed(repo);
    }
    n.append(p);
    const x = extra(repo, rank); if (x) n.append(x);
  }
  return n;
}

// The interns the People list last read (outside collaborators and people invited to single repos), so the
// who-can-open dialog can show them without asking GitHub again.
let internsNow = [];

// Owners: everyone's level on one repo, changed in place.
async function accessDialog(a, repo) {
  const box = el("div", "access-list");
  const msg = el("p", "sub");
  let changed = false;   // the page is read again only when something was changed here
  // one row with a level to pick; `after(level)` keeps the caller's own record in step
  const row = (title, sub, tag, now, login, after) => {
    const r = el("div", "item"), text = el("div", "text");
    text.append(el("div", "title", title), el("div", "sub", sub));
    r.append(text);
    if (tag) r.append(el("span", "tag", tag));
    const pick = el("select");
    [["0", "No access"], ["1", "Read"], ["2", "Write"]].forEach(([v, l]) => pick.append(new Option(l, v)));
    pick.value = String(now);
    pick.onchange = async () => {
      pick.disabled = true; msg.textContent = "";
      const spin = el("span", "spinner"); pick.before(spin);
      try { toast(await invoke("set_access", { org: a.org, repo, login, level: +pick.value })); now = +pick.value; after(now); changed = true; }
      catch (e) { msg.textContent = e; pick.value = String(now); }
      spin.remove(); pick.disabled = false;
    };
    r.append(pick);
    return r;
  };
  Object.entries(a.people).sort(([, x], [, y]) => x.name.localeCompare(y.name)).forEach(([login, p]) => {
    if (p.grants === null) {
      const r = el("div", "item"), text = el("div", "text");
      text.append(el("div", "title", p.name === login ? login : p.name), el("div", "sub", login));
      r.append(text, pill(4, "plain", "Owners can open every repo"));
      return box.append(r);
    }
    box.append(row(p.name === login ? login : p.name, login, null, Math.min(p.grants[repo] || 0, 2), login, level => { p.grants[repo] = level; }));
  });
  // interns are outside the organisation: their access is per repo, given here like anyone's
  const RANK = { admin: 2, maintain: 2, write: 2, triage: 1, read: 1 };
  internsNow.forEach(i => {
    const has = i.repos.find(r => r.repo === repo);
    const now = has ? (RANK[has.level] ?? 1) : 0;
    box.append(row(i.login, has && has.invite ? `${i.login}, invited, not accepted yet` : i.login, "Intern", now, i.login, level => {
      i.repos = i.repos.filter(r => r.repo !== repo);
      if (level) i.repos.push({ repo, level: level === 2 ? "write" : "read", invite: null });
    }));
  });
  box.append(msg);
  await ask(`Who can open ${repo}`, "Changes apply on GitHub as soon as you pick them.", [["done", "Done", true]], box);
  if (changed) loadTeam();
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

// Set after step 1 (GitHub) succeeds or the account switches, so the badge runs step 2 at once.
let labSignInNext = false;

// Machines, one row each, and whether this computer can connect to it right now. Members only ever
// connect through Cloudflare (Access for the GitHub sign-in, a tunnel for SSH), never to a machine's address.
// Which project lives where is the vault plugin's job; here only the machine matters.
function machinesSection(machines, org, user) {
  const sec = el("div", "section");
  const head = el("header");
  const again = iconButton("refresh-cw", "Check again", "small ghost", true);
  const title = el("h2", null, "Reachable now");
  title.append(info("Whether this computer can reach each machine right now, through Cloudflare. The team's machines, reached only through Cloudflare after you sign in with GitHub. Click a machine for its hardware and desktops."));
  head.append(title, el("span", "grow"), again);
  sec.append(head);
  const hosts = [...new Map(machines.map(m => [m.host, m])).values()];
  const byHost = Object.fromEntries(hosts.map(m => [m.host, m]));
  const carries = h => machines.filter(m => m.host === h).map(m => m.repo).join(", ");
  const list = el("div", "list"), pills = {}, meters = {}, specs = {};
  const live = new Set();   // machines whose status page answered: signed in and reachable, whatever an older check said
  let lastState = null;
  const look = {}, diskLow = {};   // per host: what its state chip says, and a disk running out (for the opening strip)
  hosts.forEach(m => {
    const p = el("span", "perm checking"); p.append(el("span", "spinner"));
    pills[m.host] = p; look[m.host] = "checking";
    const right = [];
    if (m.sometimes) { const t = el("span", "tag worded"); t.append(icon("moon"), "Often off"); t.dataset.tip = "Often off: this machine is switched off at times"; right.push(t); }
    if (m.via) right.push(el("span", "tag", `Through ${m.via}`));
    meters[m.host] = el("span", "meters");
    const row = kind(item(m.host, [m.note, `Carries ${carries(m.host)}`].filter(Boolean).join(". "), null, ...right, p), "server");
    row.classList.add("machine", "click");
    row.querySelector(".title").classList.add("chip");
    row.querySelector(".text").append(meters[m.host]);   // live numbers sit under the name, the right side keeps the status
    // the row is a disclosure: open by default, it shows who may connect (always known) and, once the machine
    // answers, its hardware and desktops; a click or Enter or Space folds it
    const detail = el("div", "specs"); detail.id = `specs-${m.host}`;
    specs[m.host] = el("div");
    detail.append(specs[m.host]);
    if (org) detail.append(whoCanConnect(m.host, machines, org, user));
    row.tabIndex = 0; row.setAttribute("role", "button"); row.setAttribute("aria-expanded", "true"); row.setAttribute("aria-controls", detail.id);
    const fold = () => { detail.hidden = !detail.hidden; row.setAttribute("aria-expanded", String(!detail.hidden)); };
    row.onclick = fold;
    row.onkeydown = e => { if (e.target === row && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); fold(); } };
    list.append(row, detail);
  });
  // a state chip: a glyph when all is as it should be, words where it deviates
  const LOOK = { up: ["s-ok", "check", "Can connect"], down: ["p0 s-bad", "x", "Can't connect"], "no-tunnel": ["p0 s-bad", "x", "Tunnel not set up"],
                 "sign-in": ["s-warn", "circle-alert", "Finish sign-in on Team access"], none: ["p0", "lock", "No access"] };
  const setLook = (host, key) => {
    const p = pills[host], [cls, name, word] = LOOK[key] || LOOK.down;
    look[host] = LOOK[key] ? key : "down";
    p.className = "perm " + cls;
    if (key === "up" || key === "none") {
      p.classList.add("glyph"); p.tabIndex = 0;
      p.replaceChildren(icon(name), el("span", "sr", word));
      p.dataset.tip = key === "up" ? `Can connect: this computer reaches ${host} now, through Cloudflare` : LEVEL_SAYS["No access"][1];
    } else { p.removeAttribute("tabindex"); delete p.dataset.tip; p.replaceChildren(icon(name), word); }
  };
  // the page's opening strip, from the same states
  const paintOpening = () => {
    const page = document.getElementById("machines");
    if (!sec.isConnected) return;
    const of = k => hosts.map(m => m.host).filter(h => look[h] === k);
    // a machine with no tunnel yet is not set up, which is not the same as down: it is said quietly, not in red
    const up = of("up"), off = of("none"), bad = of("down"), unset = of("no-tunnel");
    if (of("checking").length === hosts.length)
      return setOpening(page, ["wait", "Checking the machines.", [], [{ icon: "circle-dashed", text: "Checking", tone: "calm", tip: "Checking the machines" }]]);
    const warns = hosts.map(m => m.host).filter(h => diskLow[h]).map(h => [h, diskLow[h]]);
    const items = [], detail = [];
    if (up.length) { items.push({ icon: "server", num: up.length, chips: up, tip: `${up.length === 1 ? "1 machine is" : `${up.length} machines are`} up: ${up.join(", ")}` });
      detail.push(`Within reach: ${up.join(", ")}.`); }
    if (off.length) { const t = `${plural(off.length, "other needs", "others need")} access from an owner`; items.push({ icon: "lock", num: off.length, unit: "no access", tone: "calm", tip: t }); detail.push(` ${t}.`); }
    warns.forEach(([h, w]) => { items.push({ icon: "circle-alert", chips: [h], text: w, tone: "warn", tip: `${h}: ${w}` }); detail.push(` ${h}: ${w}.`); });
    if (unset.length) { const t = `Not set up yet: ${unset.join(", ")}`; items.push({ icon: "circle-dashed", num: unset.length, unit: "not set up", tone: "calm", tip: t }); detail.push(` ${t}.`); }
    if (bad.length) {
      items.push({ icon: "circle-x", chips: bad, text: "down", tone: "bad", tip: `Cannot be reached: ${bad.join(", ")}` });
      return setOpening(page, ["bad", `${bad.length} of ${plural(hosts.length, "machine")} cannot be reached.`, [...detail, ` Down: ${bad.join(", ")}.`], items]);
    }
    const lead = up.length === 1 ? "1 machine is up." : `${up.length} machines are up.`;
    setOpening(page, [warns.length ? "warn" : "ok", warns.length ? lead.replace(".", `, ${warns.length} needs a look.`) : lead, detail, items]);
  };
  queueMicrotask(paintOpening);
  const help = el("div");
  sec.append(help, list);

  // a machine reached through another one shares that one's way in
  const way = m => byHost[m.via] || m;
  const check = async () => {
    Object.entries(pills).forEach(([h, p]) => { look[h] = "checking"; p.className = "perm checking"; p.removeAttribute("tabindex"); delete p.dataset.tip; p.replaceChildren(el("span", "spinner")); });
    paintOpening();
    const targets = [...new Map(hosts.map(m => [way(m).host, way(m).tunnel || null])).entries()];
    const state = await working(again, "Checking", () => invoke("reachable", { machines: targets }));
    for (const h of Object.keys(state)) if (live.has(h)) state[h] = "up";
    lastState = state;
    hosts.forEach(m => setLook(m.host, org && !connectRule(way(m).host, machines, org).may(user) ? "none" : state[way(m).host]));
    paintOpening();
    await showHelp(state);
  };

  // what this computer still needs: the ssh aliases (the sign-in itself is step 2 on the badge)
  async function showHelp(state) {
    help.replaceChildren();
    const ssh = await invoke("ssh_status", { machines });
    if (ssh === "missing") {
      const b = el("button", "small", "Set up connections");
      b.onclick = async () => {
        if (await ask("Set up connections on this computer?", "The app adds the machines to your SSH settings (~/.ssh/config) as one marked block, so ssh <account>@host-20 goes through Cloudflare. The rest of the file stays as it is, and the old file is kept as config.bak.",
          [["cancel", "Cancel"], ["ok", "Set up connections", true]]) !== "ok") return;
        try { toast(await working(b, "Setting up", () => invoke("ssh_setup", { machines }))); } catch (e) { toast(e); }
        showHelp(state);
      };
      help.append(item("Connections are not set up on this computer", "Needed once, so ssh knows to go through Cloudflare.", null, b));
    } else if (ssh === "current") {
      help.append(el("p", "sub", "Connections are set up on this computer."));
    }
    pickPrimary(document.getElementById("machines"));
  }
  // live numbers from each machine's status page, every 15 s while this page is on screen
  const bar = (label, pct, title) => {
    const b = el("span", "meter-mini"); b.dataset.tip = title;
    const v = Math.max(0, Math.min(100, pct || 0));
    const fill = el("i"); fill.style.width = `${v}%`;
    if (v >= 85) fill.className = "hot";
    const track = el("span", "track"); track.append(fill);
    b.append(el("span", "k", label), track, el("span", "v", `${Math.round(v)}%`));
    return b;
  };
  const gb = x => x == null ? "?" : `${Math.round(x)} GB`;
  const status = async () => {
    const tunnels = hosts.filter(m => m.tunnel).map(m => [m.host, m.tunnel]);
    if (!tunnels.length) return;
    const [all, open] = await Promise.all([invoke("machine_status", { tunnels }), invoke("forwards")]);
    for (const [host, s] of Object.entries(all)) {
      const g = s.gpus || [];
      const gpu = g.length ? Math.max(...g.map(x => x.util || 0)) : null;
      // its status page answered through Cloudflare, so this computer is signed in and the tunnel is up
      live.add(host);
      setLook(host, "up");
      const memPct = s.mem_gb ? 100 * s.mem_used_gb / s.mem_gb : 0;
      meters[host].replaceChildren(bar("CPU", s.cpu_pct, `CPU ${s.cpu_pct}% of ${s.threads} threads, load ${s.load.join(" ")}`),
        bar("RAM", memPct, `Memory ${s.mem_used_gb} of ${s.mem_gb} GB in use`),
        ...(gpu == null ? [] : [bar("GPU", gpu, g.map(x => `${x.name}: ${x.util}%, ${Math.round(x.mem_used_mb / 1024)}/${Math.round(x.mem_total_mb / 1024)} GB`).join("\n"))]));
      // hardware: [label, its icon, which of several, value]; the label is its icon, the words in its tooltip
      const d = [["Processor", "cpu", "", `${s.cpu} (${s.threads} threads)`], ["Memory", "memory-stick", "", `${s.mem_used_gb} of ${s.mem_gb} GB in use`],
        ...g.map((x, i) => ["GPU", "gpu", g.length > 1 ? String(i) : "", `${x.name}, ${x.util}% busy, ${(x.mem_used_mb / 1024).toFixed(1)} of ${Math.round(x.mem_total_mb / 1024)} GB, ${x.temp_c} °C`]),
        ["Disk", "hard-drive", "", `${gb(s.disk_free_gb)} free of ${gb(s.disk_gb)}`], ["System", "server-cog", "", `${s.os}, up ${Math.round(s.uptime_h / 24)} days, ${s.users} signed in`]];
      specs[host].replaceChildren(...d.map(([k, name, n, v]) => {
        const r = el("div", "spec hw"), label = el("span", "k");
        label.append(icon(name), el("span", "sr", n ? `${k} ` : k), n);
        label.dataset.tip = n ? `${k} ${n}` : k;
        r.append(label, el("span", null, v)); return r; }));
      diskLow[host] = s.disk_free_gb != null && s.disk_free_gb < 20 ? `disk almost full, ${gb(s.disk_free_gb)} left` : null;
      if (diskLow[host]) { const w = el("p", "warn"); w.append(icon("circle-alert"), `Disk almost full: ${gb(s.disk_free_gb)} left.`); specs[host].append(w); }
      if (s.host_tools) specs[host].append(hostTools(byHost[host], s.host_tools));
      if ((s.desktops || []).length) specs[host].append(desktopList(byHost[host], s.desktops, open));
      // a cluster reached through this machine (rooster through horse) comes in the same answer
      if (s.cluster) hosts.filter(x => x.via === host).forEach(x => clusterView(x.host, s.cluster));
    }
    // a check that finished before these numbers arrived may still be asking for a sign-in
    if (lastState && [...live].some(h => lastState[h] !== "up")) { live.forEach(h => lastState[h] = "up"); showHelp(lastState); }
    paintOpening();
  };
  // Whether the machine runs the exo, exo-status.py and exo-desktop this app carries; owners can copy them over.
  const owner = org && (org.people[user] || {}).grants === null;
  function hostTools(m, line) {
    const r = el("div", "row"), msg = el("span", "sub");
    r.append(el("span", null, line));
    if (owner) {
      const b = iconButton("wrench", "Update host tools", "small ghost", true);
      b.onclick = async e => {
        e.stopPropagation(); msg.textContent = "";
        const account = m.account || "ntk";
        if (await ask(`Update host tools on ${m.host}?`, `The app copies exo, exo-status.py and exo-desktop into ~/.local/bin of ${account} and restarts the status page. Desktops and running work are left alone.`,
          [["cancel", "Cancel"], ["ok", "Update host tools", true]]) !== "ok") return;
        try { toast(await working(b, "Updating", () => invoke("update_host_tools", { tunnel: m.tunnel, user: account }))); status(); }
        catch (err) { msg.textContent = err; }
      };
      r.append(b, msg);
    }
    return r;
  }
  // A SLURM cluster: totals as meters, then each node and the queues. The reading is at most a minute old.
  function clusterView(host, c) {
    const sum = k => c.nodes.reduce((a, n) => a + (n[k] || 0), 0);
    const [gu, gt, cu, ct] = ["gpu_used", "gpu_total", "cpu_used", "cpu_total"].map(sum);
    meters[host].replaceChildren(bar("GPU", gt ? 100 * gu / gt : 0, `${gu} of ${gt} GPUs allocated`),
      bar("CPU", ct ? 100 * cu / ct : 0, `${cu} of ${ct} cores allocated`),
      el("span", "sub", `${c.running} jobs running, ${c.pending} waiting`));
    const q = o => Object.entries(o || {}).map(([k, v]) => `${k} ${v}`).join(", ") || "none";
    const rows = [...c.nodes.map(n => [n.name, `GPU ${n.gpu_used}/${n.gpu_total}, CPU ${n.cpu_used}/${n.cpu_total}, ${n.state.toLowerCase()}${n.reason ? ` (${n.reason})` : ""}`]),
      ["Running", q(c.running_by_queue)], ["Waiting", q(c.pending_by_queue)],
      ["Updated", c.time ? new Date(c.time * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "?"]];
    specs[host].replaceChildren(...rows.map(([k, v]) => { const r = el("div", "spec"); r.append(el("span", "k", k), el("span", null, v)); return r; }));
  }
  // Each VNC desktop listens on the machine itself only; opening one forwards a local port to it through
  // Cloudflare, and the person's own VNC viewer connects to 127.0.0.1:<that port>.
  function desktopList(m, desktops, open) {
    const box = el("div", "desktops");
    const head = el("div", "row"), msg = el("span", "sub");
    const add = iconButton("plus", "New desktop", "small ghost", true);
    add.onclick = async e => {
      e.stopPropagation(); msg.textContent = "";
      try { toast(`Desktop ${await working(add, "Starting", () => invoke("desktop", { tunnel: m.tunnel, user: desktops[0]?.user || "ntk", action: "start", display: 0 }))} is ready`); status(); }
      catch (err) { msg.textContent = err; }
    };
    head.append(el("span", "k", "Desktops"), add, msg);
    box.append(head);
    desktops.forEach(d => {
      const r = el("div", "desk"), msg = el("span", "sub"), num = +d.display.slice(1);
      // the machine's own screen sharing over RDP is one more row: nothing to start or close, its sign-in is the machine's
      const rdp = d.kind === "rdp";
      const name = rdp ? "Screen sharing (RDP)" : `${d.display}  ${d.geometry || ""}  (${d.user})${d.socket ? "" : "  password"}`;
      const show = port => {
        r.replaceChildren(icon(rdp ? "screen-share" : "monitor"), el("span", "desk-name", name));
        // closing ends the desktop for everyone on it, since desktops are shared by the account; disconnecting only
        // stops this computer's view: two different glyphs, and tooltips that say the difference
        const close = iconButton("power", `Close desktop ${d.display}: ends it for everyone on it`, "small ghost", true);
        const acts = el("span", "acts");
        close.onclick = async e => {
          e.stopPropagation();
          if (await ask(`Close desktop ${d.display} on ${m.host}?`, "Anyone working on this desktop loses it, along with any unsaved work in it.",
            [["cancel", "Cancel"], ["close", "Close desktop", true]]) !== "close") return;
          try {
            await invoke("close_forward", { host: m.host, display: num });
            toast(await working(close, "Closing", () => invoke("desktop", { tunnel: m.tunnel, user: d.user, action: "stop", display: num })));
            status();
          } catch (err) { msg.textContent = err; r.append(msg); }
        };
        if (!port) {
          const b = el("button", "small ghost", "Open desktop");
          b.onclick = async e => {
            e.stopPropagation();
            try { show(await working(b, "Connecting", () => invoke("open_forward", { host: m.host, tunnel: m.tunnel, user: d.user, display: num, socket: d.socket || null, port: d.port || null }))); }
            catch (err) { msg.textContent = err; r.append(msg); }
          };
          acts.append(b, ...(rdp ? [] : [close]));
          r.append(acts);
          return;
        }
        const addr = `127.0.0.1:${port}`;
        const copy = iconButton("copy", `Copy ${addr}`, "small ghost", true), view = iconButton("external-link", "Open viewer");
        const stop = iconButton("unplug", rdp ? "Disconnect: it keeps running" : `Disconnect from ${d.display}: it keeps running`, "small ghost", true);
        copy.onclick = e => { e.stopPropagation(); navigator.clipboard.writeText(addr).then(() => toast(`Copied ${addr}`), () => toast(addr)); };
        view.onclick = e => { e.stopPropagation(); invoke("open_viewer", { port, kind: d.kind || "vnc" }); };
        stop.onclick = async e => { e.stopPropagation(); await invoke("close_forward", { host: m.host, display: num }); show(null); };
        acts.append(copy, view, stop, ...(rdp ? [] : [close]));
        r.append(el("span", "sub", rdp ? "RDP at" : "VNC at"), el("strong", "cmd", addr), acts);
      };
      show(open[`${m.host}:${num}`]);
      box.append(r);
    });
    return box;
  }

  const timer = setInterval(() => { if (!sec.isConnected) return clearInterval(timer); if (current === "machines" && !document.hidden) status(); }, 15000);
  again.onclick = () => { check(); status(); };
  const onLab = () => { if (!sec.isConnected) return window.removeEventListener("lab-signed-in", onLab); check(); status(); };
  window.addEventListener("lab-signed-in", onLab);
  check(); status();
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
    list.append(kind(item([chip(r.author), ` asks for ${r.level} on ${r.repo}`], note || null, null, msg,
      act("approve_request", "Approve"), act("decline_request", "Decline", true)), "inbox"));
  });
  sec.append(list);
  return sec;
}

function teamsList(a, login, person) {
  const sec = el("div");
  sec.append(el("h3", null, `Teams for ${person.name}`), el("p", "sub", "Ticking a team adds them on GitHub right away."));
  const list = el("div", "list"), msg = el("p", "sub");
  a.teams.filter(layerTeam).forEach(t => {
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

// Somebody who cannot open a vault yet gets one way forward: ask the owners. If even that cannot reach them
// (the account is on no team, so it cannot see the request repo), say who has to act.
function askForVault(v, orgs) {
  const box = el("div", "ask");
  const owner = v.repo.split("/")[0];
  // requests travel inside an organisation; a vault kept under a person's own account has no request repo
  if (!orgs.has(owner)) {
    box.append(el("p", null, `This account cannot open ${v.name}. It belongs to ${owner} personally, so ask ${owner} to add you on GitHub.`));
    return box;
  }
  const msg = el("p", "sub");
  const b = el("button", "small", "Ask the owners for access");
  b.onclick = async () => {
    msg.textContent = "";
    try {
      await working(b, "Sending", () => invoke("request_access", { org: v.repo.split("/")[0], repo: v.repo.split("/")[1], level: "read", note: `Asked from aIwalk System Setup: cannot open ${v.name} yet.` }));
      toast("Request sent to the owners"); loadTeam();
    } catch (e) {
      msg.textContent = /not found|404|could not resolve/i.test(String(e))
        ? "Your account is not on a team yet, so requests cannot reach the owners. Ask an owner to add you to the team."
        : `GitHub refused: ${e}`;
    }
  };
  box.append(el("p", null, `This account cannot open ${v.name} yet.`), b, msg);
  return box;
}

// One collapsible section per vault; the summary line says where it is and what you can do, even when folded.
function vaultSection(v, user, viewAs, orgs) {
  const sec = el("details", "section vault");
  const key = "open:" + v.repo;
  try { sec.open = localStorage.getItem(key) !== "0"; } catch { sec.open = true; }
  sec.ontoggle = () => { try { localStorage.setItem(key, sec.open ? "1" : "0"); } catch {} };
  const summary = el("summary");
  const head = el("header");
  const here = local.copies[v.repo];
  head.append(el("h2", null, v.name),
    here ? glyph("tag", "monitor-check", "On this computer", "On this computer: a copy of this vault is here")
         : glyph("tag", "cloud", "Not downloaded", "Not downloaded: it stays on GitHub until you download it"),
    pill(Math.min(v.permission, 4), "worded"), el("span", "grow"));
  summary.append(head);
  sec.append(summary);
  const body = el("div", "vault-body");
  sec.append(body);
  if (!v.permission) { if (here) body.append(downloadRow(v)); body.append(askForVault(v, orgs)); return sec; }
  body.append(downloadRow(v));
  const a = v.access;
  if (!a) {
    // one repo, not split: what counts is the permission on the repo itself
    body.append(el("p", "sub", "This vault is one repo, so your access is the same everywhere in it."));
    return sec;
  }
  const owner = (a.people[user] || {}).grants === null;
  const docs = el("ul", "tree"), teams = el("div");
  const show = login => {
    const person = a.people[login] || { name: login, grants: {} };
    // only a member looking at their own view can ask for more
    const extra = (repo, rank) => !owner && login === user && rank < 2 ? requestControl(a, repo, rank) : null;
    editAccess = owner ? repo => accessDialog(a, repo) : null;   // pills are editable for owners, in every re-render
    docs.replaceChildren(tree(a.tree, person.grants, extra));
    editAccess = null;
    teams.replaceChildren(owner && person.grants !== null && a.teams.length ? teamsList(a, login, person) : "");
  };
  const tools = el("div", "row view-as"), docsHead = el("h3", null, "Documents");
  if (owner) {   // owners may look through any member's eyes
    const pick = el("select");
    Object.keys(a.people).sort((x, y) => (x !== user) - (y !== user) || a.people[x].name.localeCompare(a.people[y].name))
      .forEach(l => pick.append(new Option(l === user ? "Me" : a.people[l].name === l ? l : `${a.people[l].name} (${l})`, l)));
    pick.onchange = () => show(pick.value);
    if (viewAs in a.people) pick.value = viewAs;
    const label = el("label", "sub", "View as ");
    label.append(pick);
    docsHead.append(info("Click a permission to choose who can open that repo."));
    tools.append(label);
  }
  show(viewAs in a.people ? viewAs : user);
  body.append(docsHead, tools, docs, teams);
  return sec;
}

// mode: undefined for the first sign-in, "add" for another account, "owner" to add the permission owners' tools need
function signInView(error, mode) {
  const box = el("div", "empty");
  const TEXT = {
    add: ["Add another GitHub account", "The page that opens approves whichever account your browser is signed in to. Sign in to the other account there first, or use a private window. The new account becomes the active one; switch back from the badge."],
    owner: ["Unlock the owner tools", "Inviting people, changing roles and moving people between teams need GitHub's permission to manage the organisation. Approve it once with your owner account; members never need it."],
  }[mode] || ["Sign in with GitHub", "Your team access follows your GitHub account. Signing in has two steps in the browser: GitHub for the repos, then the same account for the machines."];
  box.append(el("h1", null, TEXT[0]));
  const code = el("p", "sub"), btn = el("button", null, mode === "owner" ? "Approve on GitHub" : "Sign in with GitHub");
  btn.dataset.primary = 1;
  btn.onclick = async () => {
    code.textContent = "";
    const stop = await listen("gh-code", e => {
      const copy = iconButton("copy", `Copy ${e.payload}`, "small ghost", true);
      copy.style.marginLeft = "8px";
      copy.onclick = () => navigator.clipboard.writeText(e.payload).then(() => toast(`Copied ${e.payload}`), () => toast(e.payload));
      const hint = el("span");
      code.replaceChildren("Enter this code on the GitHub page that just opened, then approve: ", el("strong", "cmd", e.payload), copy, hint);
      // copied for the person when the system allows it without a click
      navigator.clipboard.writeText(e.payload).then(() => hint.textContent = " It is already copied, so pasting works.", () => {});
    });
    let why = "";
    const stopLine = await listen("gh-line", e => { why = e.payload; });
    const ok = await working(btn, mode === "owner" ? "Waiting for GitHub" : "Step 1 of 2: waiting for GitHub", () => invoke("sign_in", { owner: mode === "owner" }));
    stop(); stopLine();
    if (ok) { labSignInNext = mode !== "owner"; termsNow = null; loadTeam(); } else code.textContent = why || "Sign-in was not finished. Try again.";
  };
  // the first sign-in: the strip says what is missing; adding an account or the owner tools keep their explanation
  if (mode) box.append(el("p", "lede", TEXT[1]));
  else setOpening(box, ["warn", "You are not signed in.", ["Nothing is connected yet: no vaults, no machines.", TEXT[1]],
    [{ icon: "github", text: "Not signed in", tone: "warn", tip: "You are not signed in: no vaults, no machines yet" }]]);
  box.append(btn, code);
  if (mode) { const back = el("button", "ghost", "Cancel"); back.style.marginLeft = "8px"; back.onclick = () => loadTeam(); btn.after(back); }
  if (error && !/not logged|auth login/i.test(error)) box.append(el("p", "sub", error));
  pickPrimary(box);
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
  // the vault gets its own folder inside the one picked
  const into = parent => parent.replace(/[\\/]+$/, "") + (parent.includes("\\") ? "\\" : "/") + v.repo.split("/").pop();
  const text = el("div", "text");
  const useCopy = path ? iconButton("folder-open", "Change folder", "small ghost", true) : el("button", "small ghost", "Use a copy I already have");
  useCopy.onclick = async () => {
    const p = await invoke("pick_folder", { title: `Pick the folder that holds your copy of ${v.name}` });
    if (!p) return;
    try { toast(await working(useCopy, "Checking that folder", () => invoke("vault_link", { repo: v.repo, path: p }))); await refreshLocal(); loadTeam(); return; }
    catch (e) { if (!/^No copy of/.test(String(e))) return msg.textContent = e; }
    // nothing there yet: the picked folder is where the vault should live, so offer a download into it
    const dest = into(p);
    const other = path ? ` The copy in ${path} stays where it is; delete it yourself once you no longer need it.` : "";
    if (await ask(`Download ${v.name} into ${p}?`, `There is no copy there yet. A new one goes to ${dest}.${other}`,
      [["cancel", "Cancel"], ["get", "Download here", true]]) !== "get") return;
    await busy(useCopy, "Downloading", () => invoke("vault_download", { repo: v.repo, dest }))();
  };
  if (path) {
    const update = iconButton("arrow-down-to-line", "Get latest"), open = iconButton("external-link", "Open in Obsidian", "small");
    open.dataset.primary = 1;
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
  const box = el("div");
  const where = el("span", "cmd");
  const sub = el("div", "sub");
  sub.append("Goes to ", where, ". Large files such as papers stay on GitHub until you open them.");
  (chosen[v.repo] ? Promise.resolve(chosen[v.repo]) : invoke("default_folder", { repo: v.repo })).then(p => where.textContent = p);

  const get = el("button", "small", "Download");
  get.onclick = busy(get, "Downloading", () => invoke("vault_download", { repo: v.repo, dest: chosen[v.repo] || null }));
  const change = iconButton("folder-open", "Change folder", "small ghost", true);
  change.onclick = async () => {
    const parent = await invoke("pick_folder", { title: `Where should ${v.name} go?` });
    if (!parent) return;
    chosen[v.repo] = into(parent);
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

// git comes with the app on Windows only; where it is missing, sign-in waits for it, with the fix.
function missingTools(t) {
  const box = el("div", "empty");
  box.append(el("h1", null, "One more thing before signing in"),
    el("p", "lede", "This app downloads and uploads through git. This computer is missing it:"));
  const list = el("div", "list");
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
  const again = iconButton("refresh-cw", "Check again", "", true);
  again.onclick = () => loadTeam();
  box.append(list, again);
  pickPrimary(box);
  return box;
}

// Owners: the organisation's people, invitations and owner role.
async function peopleSection(org, user, teams, tree) {
  const sec = el("div", "section");
  const head = el("header");
  // inviting is an occasional act: its form stays folded away until this button opens it, right under the heading
  const inviteBtn = iconButton("user-plus", "Invite someone", "small");
  inviteBtn.setAttribute("aria-expanded", "false"); inviteBtn.dataset.primary = 1;
  const title = el("h2", null, "People");
  head.append(title, el("span", "grow"), inviteBtn);
  const lede = el("p", "sub", `Everyone in ${org} on GitHub.`);
  sec.append(head, lede);
  let p;
  try { p = await invoke("org_people", { org }); } catch (e) { inviteBtn.remove(); sec.append(el("p", "sub", `Could not read the organisation: ${e}`)); return sec; }
  internsNow = p.interns || [];
  sec.counts = { invited: p.invites.length, interns: p.interns.length };
  if (!p.can_edit) {
    // read-only: names, logins and roles, and whom to ask; GitHub shows invitations and interns to owners only
    inviteBtn.remove();
    const rows = el("div", "list");
    p.members.slice().sort((x, y) => (y.owner - x.owner) || x.name.localeCompare(y.name)).forEach(m =>
      rows.append(personRow(m.login, m.name, el("span", "sub", m.owner ? "Owner" : "Member"))));
    sec.append(rows);
    const owners = p.members.filter(m => m.owner);
    if (owners.length) {
      const note = el("p", "sub", "Owners make changes here. Ask ");
      owners.forEach((m, i) => { if (i) note.append(i === owners.length - 1 ? " or " : ", "); note.append(el("span", "tag", m.name)); });
      sec.append(note);
    }
    return sec;
  }
  // an owner's view: what the list is says the info mark
  lede.remove();
  title.append(info(`Everyone in ${org} on GitHub. Owners can open every repo and change everyone's access.`));
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
      const rm = iconButton("user-minus", `Remove ${m.login} from ${org}`, "small ghost row-act", true);
      rm.onclick = async () => {
        if (await ask(`Remove ${m.name} from ${org}?`, "Their teams and repo access end now. Copies already on their computer stay there.",
            [["cancel", "Cancel"], ["rm", "Remove", true]]) === "rm") act(() => invoke("remove_member", { org, login: m.login }), null, rm, "Removing")();
      };
      right.push(rm);
    }
    list.append(personRow(m.login, m.name, ...right));
  });
  // an invitation is a login without a profile yet: the login is the row's title
  p.invites.forEach(i => {
    const cancel = el("button", "small ghost", "Cancel invitation");
    cancel.onclick = () => act(() => invoke("cancel_invite", { org, id: i.id }), null, cancel, "Cancelling")();
    const r = kind(item(i.login, `Invited ${i.created}${i.owner ? " as owner" : ""}, not accepted yet`, null,
      glyph("tag", "hourglass", "Invited", "Invited: has not accepted yet"), cancel), "mail");
    r.querySelector(".title").classList.add("chip");
    list.append(r);
  });
  sec.append(list);

  // interns: outside the organisation, on single repos; one row per person, accepted and pending repos apart
  if (p.interns.length) {
    const ilist = el("div", "list");
    const levels = rs => rs.map(r => `${r.level} on ${r.repo}`).join(", ");
    p.interns.forEach(n => {
      const has = n.repos.filter(r => r.invite == null), invited = n.repos.filter(r => r.invite != null);
      const sub = [has.length && `Has ${levels(has)}`, invited.length && `Invited, not accepted yet: ${levels(invited)}`].filter(Boolean).join(". ");
      const rm = iconButton("user-minus", `Remove ${n.login} from every repo of ${org}`, "small ghost row-act", true);
      rm.onclick = async () => {
        if (await ask(`Remove ${n.login} from every repo of ${org}?`, "This removes them from every repo of the organisation and cancels their pending repo invitations. Copies already on their computer stay there.",
            [["cancel", "Cancel"], ["rm", "Remove", true]]) === "rm")
          act(() => invoke("remove_intern", { org, login: n.login, invites: invited.map(r => [r.repo, r.invite]) }), null, rm, "Removing")();
      };
      const r = kind(item(n.login, sub || "No repos", null, glyph("tag", "graduation-cap", "Intern", "Intern: outside the organisation, single repos only"), rm), "user-round");
      r.querySelector(".title").classList.add("chip");
      ilist.append(r);
    });
    const h = el("h3", null, "Interns");
    h.append(info(`Not in ${org}; they have single repos only.`));
    sec.append(h, ilist);
  }

  // invite: a GitHub username, a role, and the layers they start in; everyone also gets the shared team
  const form = el("div", "invite");
  const who = el("input", "who"); who.placeholder = "Their GitHub username"; who.autocomplete = "off"; who.spellcheck = false;
  const nameRow = el("label", "field");
  nameRow.append(el("span", "label", "Username"), who);
  form.append(nameRow);

  let role = "member";
  const seg = el("div", "segmented"); seg.setAttribute("role", "radiogroup"); seg.setAttribute("aria-label", "Role");
  const roleBtn = (label, r) => {
    const b = el("button", null, label); b.type = "button"; b.setAttribute("role", "radio");
    b.onclick = () => { role = r; paint(); };
    return b;
  };
  const memberBtn = roleBtn("Member", "member"), ownerBtn = roleBtn("Owner", "owner"), internBtn = roleBtn("Intern (not in the organisation)", "intern");
  seg.append(memberBtn, ownerBtn, internBtn);
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

  const send = el("button", null, "Send invitation"); send.dataset.primary = 1;
  const foot = el("div", "foot"); foot.append(send);
  form.append(foot);

  function paint() {
    [[memberBtn, "member"], [ownerBtn, "owner"], [internBtn, "intern"]].forEach(([b, r]) => b.setAttribute("aria-checked", String(role === r)));
    const noTeams = role !== "member";
    tiles.querySelectorAll(".tile").forEach(tile => {
      tile.disabled = noTeams;
      tile.setAttribute("aria-pressed", String(!noTeams && chosenTeams.has(tile.dataset.slug)));
    });
    ownerNote.textContent = role === "owner" ? "Owners open every repo, so they need no layers."
      : role === "intern" ? "Interns get read access to the app, the access requests, the shared notes and the papers, as repo invitations. Raise single repos for them in the access tree afterwards."
      : shared ? "Everyone also gets the shared notes and papers, and can ask the owners for more." : "";
  }
  paint();

  send.onclick = async () => {
    if (!who.value.trim()) return who.focus();
    if (role === "intern") return act(() => invoke("invite_intern", { org, login: who.value }), null, send, "Inviting")();
    const picked = role === "owner" ? [] : [...chosenTeams, ...(shared ? [shared.slug] : [])];
    await act(() => invoke("invite", { org, login: who.value, owner: role === "owner", teams: picked }), null, send, "Inviting")();
  };
  // the drawer: closed it takes no room and no focus; open it grows out under the heading and the name field is ready
  const drawer = el("div", "drawer"), inner = el("div", "drawer-in");
  inner.append(form);
  inner.inert = true;
  drawer.append(inner);
  const setOpen = open => {
    drawer.classList.toggle("open", open);
    inner.inert = !open;
    inviteBtn.setAttribute("aria-expanded", String(open));
    inviteBtn.replaceChildren(icon(open ? "x" : "user-plus"), open ? "Close" : "Invite someone");
    inviteBtn.classList.toggle("ghost", open);
    // open, sending the invitation is the page's action, not the Close that took Invite someone's place
    if (open) delete inviteBtn.dataset.primary; else inviteBtn.dataset.primary = 1;
    pickPrimary(sec.closest(".page"));
    if (open) who.focus();
  };
  inviteBtn.onclick = () => setOpen(!drawer.classList.contains("open"));
  form.addEventListener("keydown", e => { if (e.key === "Escape") { setOpen(false); inviteBtn.focus(); } });
  head.after(drawer);   // right under the heading
  sec.append(msg);
  return sec;
}

// Who may merge pull requests on each code repo (the repos the machines carry). The vault plugin offers Merge at
// maintain or admin: org owners, core, and merge-<repo>, the people an owner added here. Listing a repo's people
// needs write there; anyone else sees only their own level.
function prSection(org, user, machines) {
  const sec = el("div", "section pr");
  const head = el("header");
  const title = el("h2");
  title.append(icon("git-pull-request"), "Pull request permissions",
    info("On GitHub's Free plan anyone with write can still merge on the website. Until the Team plan adds rulesets, merge rights hold by convention: a pre-push hook and this list."));
  head.append(title);
  sec.append(head);
  const list = el("div", "list");
  sec.append(list);
  const owner = (org.people[user] || {}).grants === null;
  const repos = [...new Set(machines.map(m => m.repo))];
  const name = l => (org.people[l] || {}).name || l;
  const MINE = { 4: "You can merge", 3: "You can merge", 2: "You can open pull requests", 1: "You can read", 0: "No access" };
  const read = () => invoke("pr_permissions", { org: org.org, repos }).then(paint, e => list.replaceChildren(el("p", "sub", `Could not read: ${e}`)));
  const paint = rows => {
    list.replaceChildren();
    if (!rows.length) list.append(el("p", "sub", "None of the code repos are open to you."));
    rows.forEach(r => {
      list.append(item(r.repo, MINE[r.mine]));
      if (!r.listed) return;
      // one line of people: who can merge, then who opens pull requests; the extras an owner added can be removed
      const box = el("div", "specs"), people = [], seen = new Map();
      [["can merge", r.merge], ["opens pull requests", r.write]].forEach(([g, logins]) => logins.forEach(l => {
        if (seen.has(l)) return seen.get(l).why.push(g);
        const rm = owner && r.extra.includes(l) ? { tip: `Remove ${l}'s merge right on ${r.repo}`, run: b => change(r.repo, l, false, b) } : null;
        const p = { login: l, name: name(l), why: [g], rm };
        seen.set(l, p); people.push(p);
      }));
      const others = Object.keys(org.people).filter(l => !r.merge.includes(l)).sort((a, b) => name(a).localeCompare(name(b)));
      let pick = null;
      if (owner && others.length) {
        pick = el("select");
        pick.append(new Option("Let someone merge", ""), ...others.map(l => new Option(`${name(l)} (@${l})`, l)));
        pick.onchange = () => pick.value && change(r.repo, pick.value, true, pick);
      }
      box.append(whoLine(people, ["can merge", "opens pull requests"], pick, `Who can merge on ${r.repo}`));
      list.append(box);
    });
  };
  async function change(repo, login, add, ctl) {
    try { toast(await working(ctl, add ? "Adding" : "Removing", () => invoke("merge_right", { org: org.org, repo, login, add }))); }
    catch (e) { toast(`Could not change ${repo}: ${e}`); }
    read();
  }
  list.append(el("p", "sub", "Reading who can merge"));
  read();
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
  if (!t.git) return page.replaceChildren(missingTools(t));
  // the team's terms come first, once per version: two quick questions to GitHub, before the long read of access
  if (!termsOk() || !termsNow) { progress.set(null, "Reading the team's terms"); termsNow = await invoke("terms_state"); }
  if (!termsOk()) return page.replaceChildren(termsView(termsNow, termsNow.org, true));
  const stop = await listen("team-stage", e => { const [i, n, text] = e.payload; progress.set(i / n, text); });
  const s = lastTeam = await invoke("team_access").finally(stop);
  if (!s.user) return page.replaceChildren(signInView(s.error));
  await refreshLocal();
  const owner = s.vaults.some(v => v.access && (v.access.people[s.user] || {}).grants === null);
  teamTodo.clear();
  const parts = [el("h1", null, "Team access"), badge(s), obsidianNotice()];
  const org = (s.vaults.find(v => v.access) || {}).access;
  // owners get a page of their own for the organisation; its entry in the sidebar carries the count of open requests
  const peopleNav = document.querySelector('nav [data-page="people"]');
  peopleNav.hidden = !org;   // everyone in the organisation sees People; only owners get the count of open requests
  if (owner && org) navLabel(peopleNav, "People", org.requests.length ? String(org.requests.length) : null);
  // an owner signed in without the permission to manage the organisation: the tools below would be refused
  if (owner && org && !(s.scopes || []).includes("admin:org")) {
    const b = el("button", "small", "Unlock");
    b.onclick = () => page.replaceChildren(signInView(null, "owner"));
    parts.push(item("Owner tools are locked", "Inviting people and changing access need one more approval on GitHub.", null, b));
    teamTodo.add("Owner tools");
  }
  // a member sees their own merge rights here; an owner sets everyone's on the People page
  if (org && !owner) parts.push(prSection(org, s.user, s.vaults.flatMap(v => (v.access && v.access.machines) || [])));
  const orgs = new Set(s.vaults.filter(v => v.access).map(v => v.access.org));
  s.vaults.forEach(v => parts.push(vaultSection(v, s.user, viewAs, orgs)));
  page.replaceChildren(...parts);
  paintTeamOpening();
  pickPrimary(page);
  if (current === "people") loadPeople();   // a change made there reloads access through here
}

// What still needs doing on Team access, set by the identity block's lines as they learn it: Machines (step 2 of the
// sign-in), Claude Code (installed, not signed in), Owner tools (locked).
const teamTodo = new Set();
function todo(what, on) {
  if (teamTodo.has(what) !== on) { teamTodo[on ? "add" : "delete"](what); paintTeamOpening(); }
  pickPrimary(teamPage());
}
// Team access opens with who is signed in, how many vaults are here and how many machines are within reach.
function paintTeamOpening() {
  const page = teamPage(), s = lastTeam;
  if (!s || !s.user || !page.querySelector(".badge")) return;
  const org = (s.vaults.find(v => v.access) || {}).access, machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
  const reach = [...new Set(machines.filter(m => m.tunnel).map(m => m.host))].filter(h => !org || connectRule(h, machines, org).may(s.user));
  const here = s.vaults.filter(v => local.copies[v.repo]).length;
  const left = ["Machines", "Claude Code", "Owner tools"].filter(t => teamTodo.has(t));
  const detail = [`Signed in as ${s.user}. ${plural(here, "vault")} on this computer, ${plural(reach.length, "machine")} within reach.`];
  const items = [{ icon: "check", chips: [s.user], tip: `Signed in as ${s.user}` },
    { icon: "monitor-check", num: here, unit: here === 1 ? "vault here" : "vaults here", tip: `${plural(here, "vault")} on this computer` },
    { icon: "server", num: reach.length, unit: "in reach", tip: `${plural(reach.length, "machine")} within reach` }];
  setOpening(page, left.length
    ? ["warn", "Almost everything is connected.", [...detail, ` Still to do: ${left.join(", ")}.`],
       [...items, { icon: "circle-alert", text: `To do: ${left.join(", ")}`, tone: "warn", tip: `Still to do: ${left.join(", ")}` }]]
    : ["ok", "Everything is connected.", detail, items]);
}

// People, the owners' page: requests waiting, everyone in the organisation with what they reach, and who may merge.
// It draws from Team access's last read, so every change made here reloads that and comes back.
async function loadPeople() {
  const page = document.getElementById("people");
  const title = el("h1", null, "People");
  if (!lastTeam) {
    page.replaceChildren(title, stage("Reading your access from GitHub"));
    await loadTeam();
    if (lastTeam) return;   // loadTeam drew this page on its way out
  }
  const s = lastTeam, org = s && s.user ? (s.vaults.find(v => v.access) || {}).access : null;
  const toTeam = el("button", null, "Go to Team access"); toTeam.onclick = () => go("team");
  if (!org) return page.replaceChildren(title, el("p", "sub", "Sign in on Team access first."), toTeam);
  if ((org.people[s.user] || {}).grants !== null) {
    // a member: the same page without edits; the requests listed are their own (the backend sends no one else's)
    const mine = org.requests.length ? (() => {
      const sec = el("div", "section"), list = el("div", "list");
      const h = el("header"); h.append(el("h2", null, "Your requests"));
      sec.append(h, list);
      org.requests.forEach(r => list.append(item(`${r.level} on ${r.repo}`, "Waiting for an owner")));
      return sec;
    })() : el("p", "quiet", "You have no open access requests.");
    const first = !page.querySelector(".section");
    if (first) page.replaceChildren(title, stage("Reading the organisation's people"));
    const members = await peopleSection(org.org, s.user, [], null);
    if (current !== "people") return;
    page.replaceChildren(title, el("p", "lede", "Everyone in the organisation and what you can ask for."), mine, members,
      prSection(org, s.user, s.vaults.flatMap(v => (v.access && v.access.machines) || [])));
    return pickPrimary(page);
  }
  const parts = [title];
  if (!(s.scopes || []).includes("admin:org")) {
    toTeam.className = "small"; toTeam.textContent = "Unlock on Team access";
    parts.push(item("Owner tools are locked", "Inviting people and changing access need one more approval on GitHub.", null, toTeam));
  }
  const first = !page.querySelector(".section");
  if (first) page.replaceChildren(...parts, stage("Reading the organisation's people"));
  const people = await peopleSection(org.org, s.user, org.teams.filter(layerTeam), org.tree);
  parts.push(requestsSection(org), people, prSection(org, s.user, s.vaults.flatMap(v => (v.access && v.access.machines) || [])));
  if (current !== "people") return;
  page.replaceChildren(...parts);
  // the page opens with how many people, how many wait for you, and the invitations and interns
  const all = Object.keys(org.people).length, waiting = org.requests.length, { invited = 0, interns = 0 } = people.counts || {};
  const items = [{ icon: "users-round", num: all, unit: all === 1 ? "person" : "people", tip: plural(all, "person", "people") },
    waiting ? { icon: "inbox", num: waiting, text: "waiting", tone: "act", tip: `${waiting} waiting for you to approve or decline` }
      : { icon: "inbox", num: 0, unit: "waiting", tone: "calm", tip: "Nobody waiting for you" }];
  if (invited) items.push({ icon: "mail", num: invited, unit: "invited", tone: "calm", tip: `${plural(invited, "invitation")} not accepted yet` });
  if (interns) items.push({ icon: "graduation-cap", num: interns, unit: interns === 1 ? "intern" : "interns", tone: "calm", tip: `${plural(interns, "intern")} outside ${org.org}` });
  const detail = [[invited, `${plural(invited, "invitation")} not accepted yet.`], [interns, `${plural(interns, "intern")} outside ${org.org}.`]]
    .filter(([k]) => k).map(([, t]) => t).join(" ");
  setOpening(page, ["ok", `${plural(all, "person", "people")}, ${waiting ? `${waiting} waiting for you` : "nobody waiting"}.`, detail ? [detail] : [], items]);
  pickPrimary(page);
}

// Machines, its own page on every OS: which lab machines this computer can reach, their load, and their desktops.
// The list comes from the vaults' rules, read with the team's access, so it reuses Team access's last read.
async function loadMachines() {
  const page = document.getElementById("machines");
  let s = lastTeam;
  if (!s) {
    const progress = stage("Reading your access from GitHub");
    page.replaceChildren(el("h1", null, "Machines"), progress);
    const stop = await listen("team-stage", e => { const [i, n, text] = e.payload; progress.set(i / n, text); });
    s = lastTeam = await invoke("team_access").finally(stop);
  }
  if (!termsOk()) {
    const b = el("button", null, "Go to Team access"); b.onclick = () => go("team");
    return page.replaceChildren(el("h1", null, "Machines"), el("p", "sub", "Read and accept the team's terms on Team access first."), b);
  }
  const head = [el("h1", null, "Machines"),
    el("p", "lede", "The team's machines, reached only through Cloudflare after you sign in with GitHub. Click a machine for its hardware and desktops.")];
  if (!s.user) {
    const b = el("button", null, "Go to Team access");
    b.onclick = () => go("team");
    return page.replaceChildren(...head, el("p", "sub", "Sign in on Team access first: the machines follow your GitHub account."), b);
  }
  const machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
  const org = (s.vaults.find(v => v.access) || {}).access;
  // coming back to the page keeps it as it was (its numbers refresh on their own); rebuild only when what it
  // shows changed: another account, other machines or other teams
  const key = JSON.stringify([s.user, machines, org && org.teams]);
  if (page.dataset.key === key && page.querySelector(".section")) return;
  page.dataset.key = key;
  const owner = org && (org.people[s.user] || {}).grants === null;
  // with machines listed, the opening strip says how they are and the lede is behind the section's info mark
  page.replaceChildren(...(machines.length ? [head[0]] : head), machines.length ? machinesSection(machines, org, s.user) : el("p", "sub", "No machines are listed for your team yet."),
    ...(owner ? [cloudflareSection()] : []));
  pickPrimary(page);
}

// Owners: the Cloudflare API token that lets this app add machines, sign SSH certificates and end people's
// sign-ins. Pasted once and kept in the system keyring; it is never shown again and never written to a file.
function cloudflareSection() {
  const sec = el("div", "section"), body = el("div", "list");
  const title = el("h2", null, "Cloudflare");
  title.append(info("For owners. The machines sit behind the team's Cloudflare account, the one that owns aiwalkcorp.com (not your personal one). With an API token made in that account, this app adds machines and ends a removed person's sign-in. Only the owner who does those things needs one."));
  sec.append(title, body);
  // the team key, shown to be told to another owner in person
  const showKey = key => ask("The team key", "Tell this key to the other owners in person or by a channel you trust. With it their app opens the team's token; without the owners' repo it opens nothing.",
    [["done", "Done", true]], (() => {
      const box = el("div"), k = el("strong", "cmd", key), copy = iconButton("copy", "Copy the team key", "small ghost", true);
      copy.style.marginLeft = "8px";
      copy.onclick = () => navigator.clipboard.writeText(key).then(() => toast("Copied the team key"), () => {});
      box.append(k, copy); return box;
    })());
  const paint = async () => {
    const st = await invoke("cf_state");
    if (st.connected) {
      const forget = iconButton("unplug", "Disconnect");
      forget.onclick = async () => { await invoke("cf_forget"); paint(); };
      const renew = el("button", "small ghost", "Renew");
      renew.dataset.tip = "Cloudflare gives the token a new value; the old one stops working at once";
      renew.onclick = async () => {
        if (await ask("Renew the token?", "The token gets a new value and the old one stops working at once, on every computer. Owners who joined with the team key get the new one by themselves.",
          [["cancel", "Cancel"], ["ok", "Renew", true]]) !== "ok") return;
        try { toast(await working(renew, "Renewing", () => invoke("cf_renew"))); } catch (e) { toast(e); }
        paint();
      };
      const rows = [item("Connected", st.expires ? `The token runs until ${st.expires}.` : "The token does not expire.", null, renew, forget)];
      // sharing: one token for all owners, kept locked in the owners' repo
      const share = el("button", "small", st.shared ? "Share again" : "Share with the other owners");
      share.onclick = async () => {
        try { showKey(await working(share, "Sharing", () => invoke("cf_share", { fresh: false }))); } catch (e) { toast(e); }
        paint();
      };
      const extra = [share];
      if (st.has_key) {
        const see = el("button", "small ghost", "Show the team key");
        see.onclick = async () => { const k = await invoke("cf_key"); if (k) showKey(k); };
        const change = el("button", "small ghost", "New key");
        change.dataset.tip = "After an owner left: a new key, and renew the token as well";
        change.onclick = async () => {
          if (await ask("Make a new team key?", "The other owners must enter the new key before their app gets the token again. Do this after an owner left, and renew the token as well.",
            [["cancel", "Cancel"], ["ok", "New key", true]]) !== "ok") return;
          try { showKey(await working(change, "Sharing", () => invoke("cf_share", { fresh: true }))); } catch (e) { toast(e); }
          paint();
        };
        extra.unshift(see, change);
      }
      if ((st.left || []).length) {
        // someone who knew the token and the key is an owner no longer: say so until both are changed
        const w = el("p", "warn", `${st.left.join(", ")} ${st.left.length > 1 ? "are" : "is"} no longer an owner but knew the team's token and key. Renew the token (or paste a new one), then make a new key and tell it to the owners.`);
        rows.push(w);
      }
      rows.push(item(st.shared ? "Shared with the other owners" : "Only on this computer",
        st.shared ? "The token is in the owners' repo, locked with the team key. The other owners enter that key once."
                  : "Share it and the other owners connect with one key instead of making tokens of their own.", null, ...extra));
      body.replaceChildren(...rows);
      return pickPrimary(sec.closest(".page"));
    }
    const msg = el("p", "sub", st.problem ? `The kept token no longer works: ${st.problem}` : "");
    const rows = [];
    if (st.shared) {
      // another owner already shared the team's token: the key is all this computer needs
      const key = el("input"); key.placeholder = "XXXX-XXXX-XXXX-XXXX-XXXX"; key.autocomplete = "off";
      const join = el("button", "small", "Connect");
      join.onclick = async () => {
        if (!key.value.trim()) return;
        try { toast(await working(join, "Opening", () => invoke("cf_join", { key: key.value }))); paint(); }
        catch (e) { msg.textContent = String(e); }
      };
      rows.push(item("Connect with the team key", "Another owner shared the team's token. Enter the key they told you.", null, key, join));
    }
    const input = el("input"); input.type = "password"; input.placeholder = "Paste the API token"; input.autocomplete = "off";
    const go = el("button", st.shared ? "small ghost" : "small", "Connect");
    go.onclick = async () => {
      if (!input.value.trim()) return;
      try { toast(await working(go, "Checking", () => invoke("cf_connect", { token: input.value }))); paint(); }
      catch (e) { msg.textContent = `Cloudflare did not take it: ${e}`; }
    };
    // what the token must be allowed to do, so nobody has to guess in Cloudflare's long list
    const needs = el("details", "sub"), list = el("ul");
    ["Account: Cloudflare Tunnel, Edit", "Account: Access: Apps and Policies, Edit", "Account: Access: Organizations, Identity Providers, and Groups, Edit",
     "Account: Access: SSH Auditing, Edit", "Account: Access: Audit Logs, Read", "Zone aiwalkcorp.com: DNS, Edit",
     "Optional, for the Renew button: API Tokens, Edit. It lets the token make tokens with any of the account's rights, so grant it only if you want one-click renewal."].forEach(t => list.append(el("li", null, t)));
    needs.append(el("summary", null, "What the token needs"), list);
    rows.push(item(st.shared ? "Or paste a token of your own" : "Not connected", "Sign in to Cloudflare as the team's account, create a custom token (Manage Account, API Tokens) and paste it here. It goes to this computer's keyring only.", null, input, go));
    body.replaceChildren(...rows, needs, msg);
    pickPrimary(sec.closest(".page"));
  };
  paint();
  return sec;
}

// Who may connect to a machine: exactly what its Cloudflare Access policy lets through. That is the members of
// core, of the teams the machine's code repos name in vault_rules.json (machines.teams), and of machine-<host>,
// the extra people an owner added here. Owners see everyone and add or remove extras; others see only themselves.
function connectRule(host, machines, org) {
  const teamsFor = [...new Set(["core", ...machines.filter(m => m.host === host).flatMap(m => m.teams || [])])];
  const team = slug => org.teams.find(t => t.slug === slug) || { slug, members: [] };
  const via = login => teamsFor.filter(t => team(t).members.includes(login));
  const extra = login => team(`machine-${host}`).members.includes(login);
  return { teamsFor, team, via, extra, may: login => via(login).length > 0 || extra(login) };
}

// The tunnels this person may use, so signing in never asks Cloudflare for a machine it would refuse.
function myTunnels(s) {
  const machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
  const org = (s.vaults.find(v => v.access) || {}).access;
  const hosts = [...new Map(machines.filter(m => m.tunnel).map(m => [m.host, m.tunnel])).entries()];
  return hosts.filter(([h]) => !org || connectRule(h, machines, org).may(s.user)).map(([, t]) => t);
}

function whoCanConnect(host, machines, org, user) {
  const box = el("div", "who-connect");
  const owner = (org.people[user] || {}).grants === null;
  const rule = connectRule(host, machines, org);
  const { teamsFor, team, via } = rule;
  const extraSlug = `machine-${host}`;
  // why people may connect, said once behind the label's info mark; the line's group labels say who
  const why = `Members of ${teamsFor.join(", ")} can connect, and anyone an owner adds by hand.`;
  const HAND = "added by hand";
  const paint = () => {
    const extra = new Set(team(extraSlug).members);
    const head = el("div", "row"), k = el("span", "k", "Who can connect");
    head.append(k);
    box.replaceChildren(head);
    if (!owner) {
      // a member's copy of the rule knows only their own teams, so they see their own sentence, not a line of one
      k.append(info(why));
      const v = via(user);
      if (v.length || extra.has(user)) box.append(el("p", "sub", v.length ? `You can connect, through ${v.join(", ")}.` : "You can connect: an owner added you."));
      else { const p = el("p", "ask-line"); p.append(icon("lock"), "You can't connect to this machine. Ask an owner: write access to one of its code repos, or an extra place here."); box.append(p); }
      return;
    }
    k.append(info(`${why} Changes reach the machine at that person's next sign-in, within 24 hours.`));
    const name = l => org.people[l].name;
    const people = Object.keys(org.people).sort((a, b) => name(a).localeCompare(name(b)))
      .filter(l => via(l).length || extra.has(l))
      .map(l => ({ login: l, name: name(l), why: [...via(l), ...(extra.has(l) ? [HAND] : [])],
        rm: extra.has(l) ? { tip: `Remove ${l}'s ${via(l).length ? "extra " : ""}access to ${host}`, run: b => change(l, false, b) } : null }));
    const others = Object.keys(org.people).filter(l => !via(l).length && !extra.has(l)).sort((a, b) => name(a).localeCompare(name(b)));
    let pick = null;
    if (others.length) {
      pick = el("select");
      pick.append(new Option("Let someone connect", ""), ...others.map(l => new Option(`${name(l)} (@${l})`, l)));
      pick.onclick = e => e.stopPropagation();
      pick.onchange = () => pick.value && change(pick.value, true, pick);
    }
    box.append(whoLine(people, [...teamsFor, HAND], pick, `Who can connect to ${host}`));
  };
  async function change(login, add, ctl) {
    try {
      toast(await working(ctl, add ? "Adding" : "Removing", () => invoke("machine_extra", { org: org.org, host, login, add })));
      const t = team(extraSlug);
      if (!org.teams.includes(t)) org.teams.push(t);
      t.members = add ? [...t.members, login] : t.members.filter(x => x !== login);
    } catch (e) { toast(`Could not change ${host}: ${e}`); }
    paint();
  }
  paint();
  return box;
}

// ---------------------------------------------------------------- Terms
// The team's terms (exo-access-requests/TERMS.md): asked once per version right after sign-in, readable any time
// from Terms in the sidebar, where owners also see who accepted which version.
let termsNow = null;   // {version, text, accepted: {version, date} | null}, or null when the team has no terms
const termsOk = () => !termsNow || (termsNow.accepted && termsNow.accepted.version >= termsNow.version);

// The terms file is plain: "# " headings, "N. " numbered items, paragraphs. Built as elements, never as HTML.
// Each "# " heading starts one language's copy; one is shown at a time, picked with the switch above the text.
function termsText(text) {
  const box = el("div", "doc");
  const parts = [];
  let part = null, list = null;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("<!--")) { list = null; continue; }
    if (line.startsWith("# ") || !part) {
      part = el("div"); parts.push(part); box.append(part); list = null;
      if (line.startsWith("# ")) { part.append(el("h2", null, line.slice(2))); continue; }
    }
    const item = line.match(/^\d+\.\s+(.*)$/);
    if (item) { if (!list) { list = el("ol"); part.append(list); } list.append(el("li", null, item[1])); continue; }
    list = null; part.append(el("p", null, line));
  }
  box.parts = parts;
  return box;
}

// The language switch: 中文 for a copy written in Chinese characters, English otherwise; this computer's language first.
function termsSwitch(text, onChange) {
  const zh = p => /[\u4e00-\u9fff]/.test(p.textContent.slice(0, 40));
  const row = el("div", "lang");
  if (text.parts.length < 2) return row;
  const show = i => {
    text.parts.forEach((p, k) => p.hidden = k !== i);
    [...row.children].forEach((b, k) => { b.classList.toggle("on", k === i); b.setAttribute("aria-pressed", k === i); });
    text.scrollTop = 0; onChange();
  };
  text.parts.forEach((p, i) => { const b = el("button", zh(p) ? "zh" : "en", zh(p) ? "中文" : "English"); b.onclick = () => show(i); row.append(b); });
  const mine = text.parts.findIndex(p => zh(p) === navigator.language.toLowerCase().startsWith("zh"));
  show(mine < 0 ? 0 : mine);
  return row;
}

// The terms page's opening strip: the version, and for owners how many have agreed to it (marks: one true per person
// on the current version, or null before that list is read).
function termsOpening(t, marks) {
  if (!t.accepted || t.accepted.version < t.version) return ["warn", `Version ${t.version} of the terms is waiting for you.`, [],
    [{ icon: "scroll-text", text: `Agree to version ${t.version}`, tone: "warn", tip: `Version ${t.version} of the terms is waiting for you` }]];
  const detail = [`Version ${t.version}, agreed on ${t.accepted.date}.`];
  const items = [{ icon: "scroll-text", num: `v${t.version}`, tip: `You agreed to version ${t.version} on ${t.accepted.date}` }];
  const behind = marks ? marks.filter(ok => !ok).length : 0;
  if (marks && marks.length) {
    items.push({ icon: "users-round", num: `${marks.length - behind}/${marks.length}`, unit: "agreed", tip: `${marks.length - behind} of ${marks.length} have agreed to version ${t.version}` });
    if (behind) items.push({ icon: "circle-alert", num: behind, text: "not yet", tone: "warn", tip: `${behind} of ${marks.length} have not agreed to version ${t.version} yet` });
    detail.push(behind ? ` ${behind} of ${marks.length} have not agreed to it yet.` : ` All ${marks.length} have agreed to it.`);
  }
  return [behind ? "warn" : "ok", "You agreed to the current terms.", detail, items];
}

function termsView(t, org, asking) {
  const wrap = el("div", "terms");
  wrap.append(el("h1", null, asking ? "Before you start" : "Terms"));
  if (asking) {
    const said = t.accepted ? `The team changed its terms. Please agree to version ${t.version}.` : "The team asks you to agree to its terms.";
    setOpening(wrap, ["warn", said, [`Version ${t.version}. Read it to the end, then press I agree. You are asked once per version; they stay under Terms in the sidebar.`],
      [{ icon: "scroll-text", text: `Agree to version ${t.version}`, tone: "warn", tip: `${said} Read it to the end, then press I agree.` }]]);
  } else setOpening(wrap, termsOpening(t, null));
  const text = termsText(t.text);
  // the page's one action, even while it waits for the end of the text
  const agree = el("button", "primary", "I agree"), note = el("span", "sub", "Scroll to the end to agree.");
  // reaching the end of either language's copy is enough: they say the same thing
  const atEnd = () => { if (text.scrollTop + text.clientHeight >= text.scrollHeight - 8) { agree.disabled = false; note.textContent = ""; } };
  wrap.append(termsSwitch(text, () => setTimeout(atEnd, 0)), text);
  if (asking) {
    agree.disabled = true;
    text.onscroll = atEnd; setTimeout(atEnd, 0);   // a short text fits without scrolling
    agree.onclick = async () => {
      try {
        await working(agree, "Recording", () => invoke("terms_accept", { org, version: t.version }));
        t.accepted = { version: t.version, date: new Date().toISOString().slice(0, 10) };   // recorded: no second look-up
        loadTeam();
      }
      catch (e) { note.textContent = `Could not record it: ${e}`; }
    };
    const row = el("div", "agree"); row.append(agree, note); wrap.append(row);
  }
  return wrap;
}

async function loadTerms() {
  const page = document.getElementById("terms");
  const s = lastTeam, access = s && (s.vaults.find(v => v.access) || {}).access;
  if (!access) return page.replaceChildren(el("h1", null, "Terms"), el("p", "sub", "Sign in on Team access to read the team's terms."));
  page.replaceChildren(el("h1", null, "Terms"), stage("Reading the terms"));
  const t = termsNow || await invoke("terms_state");
  if (!t) return page.replaceChildren(el("h1", null, "Terms"), el("p", "sub", "Your team has no terms yet."));
  const view = termsView(t, access.org, false);
  page.replaceChildren(view);
  // owners see who accepted which version
  if ((access.people[s.user] || {}).grants === null) {
    const who = await invoke("terms_everyone", { org: access.org });
    const sec = el("div", "section"), list = el("div", "list");
    sec.append(el("h2", null, "Who accepted"));
    // on the current version: the date, the version in the tooltip; an older version or none: words, in amber
    const marks = [];
    const accepted = a => {
      marks.push(!!a && a.version >= t.version);
      if (!a) return stateText(el("span", "perm p0 s-warn"), "circle-alert", "Not yet");
      if (a.version < t.version) return stateText(el("span", "perm pending"), "circle-alert", `Version ${a.version}, ${a.date}`);
      const p = stateText(el("span", "perm s-ok"), "check", a.date, `Version ${a.version}, `);
      p.dataset.tip = `Agreed to version ${a.version} on ${a.date}`; p.tabIndex = 0;
      return p;
    };
    Object.keys(access.people).sort((a, b) => access.people[a].name.localeCompare(access.people[b].name)).forEach(login => {
      list.append(personRow(login, access.people[login].name, accepted(who[login])));
    });
    // interns after the members; they record acceptance in the same requests repo
    const interns = await invoke("org_people", { org: access.org }).then(p => p.interns, () => []);
    interns.forEach(n => {
      const r = kind(item(n.login, `Intern${n.repos.every(r => r.invite != null) ? ", invitation not accepted yet" : ""}`, null, accepted(who[n.login])), "user-round");
      r.querySelector(".title").classList.add("chip");
      list.append(r);
    });
    sec.append(list); view.append(sec);
    setOpening(view, termsOpening(t, marks));
  }
}
