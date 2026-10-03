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
  const methods = el("div", "methods claude-line");
  const a = s.accounts.find(x => x.active) || {};
  methods.append(el("span", "label", "GitHub"),
    el("span", "method " + (a.method === "account" ? "m-account" : "m-off"), "GitHub account"),
    el("span", "method " + (a.method === "temporary" ? "m-temporary" : "m-off"), "Temporary credential"),
    el("span", "method m-off", "Security key"));
  if (a.protocol) methods.append(el("span", "sub", `git over ${a.protocol.toUpperCase()}`));
  who.append(methods, labLine(s), claudeLine());
  const out = el("button", "ghost small", "Sign out of GitHub");
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
    pick.append(new Option("Switch GitHub account", ""), ...others.map(x => new Option(x.login, x.login)));
    pick.onchange = async () => {
      pick.disabled = true;
      try { await invoke("switch_account", { login: pick.value }); toast(`Now using ${pick.value}; step 2 signs this account in to the machines`); labSignInNext = true; }
      catch (e) { toast(`Could not switch: ${e}`); }
      loadTeam();
    };
    actions.append(pick);
  }
  const add = el("button", "ghost small", "Add a GitHub account");
  add.onclick = () => teamPage().replaceChildren(signInView(null, true));
  actions.append(add, out);
  who.append(actions);
  b.append(el("div", "band"), el("div", "face", initials(name)), who);
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
  line.append(el("span", "label", "Machines"), state, msg);
  const step2 = async b => {
    msg.textContent = "";
    try { await working(b, "Step 2 of 2: waiting for the browser", () => invoke("access_login", { tunnels })); }
    catch (e) { msg.textContent = e; }
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
      const b = el("button", "small", "Finish signing in");
      b.onclick = () => step2(b);
      line.append(b);
      if (labSignInNext) { labSignInNext = false; step2(b); }
      return;
    }
    const until = new Date(id.expires * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });
    if (id.matches === false) {
      // someone else's GitHub account in the browser: the machines would see a different person than this app
      state.className = "method m-temporary"; state.textContent = `Signed in as ${id.email}`;
      msg.textContent = `That is not ${s.user}. Sign out of GitHub in the browser (or use the right account there), then sign in again.`;
      const b = el("button", "small", "Sign in again");
      b.onclick = async () => { await invoke("lab_sign_out"); step2(b); };
      line.append(b);
      return;
    }
    // a machine added or split off since the last sign-in: the team sign-in covers it without the browser
    if (id.missing && !topped) { topped = true; const b = el("button", "small", "Finish signing in"); line.append(b); return step2(b); }
    state.className = "method m-key"; state.textContent = `Signed in as ${id.email}`;
    msg.textContent = `until ${until}, ` + (id.matches ? `same person as @${s.user}` : `not checked against @${s.user}: this GitHub sign-in predates the email check, sign out and in once`);
  };
  paint();
  return line;
}

// Claude Code on this computer: the team's vault workflow runs through it, so the badge says whether it is ready.
function claudeLine() {
  const line = el("div", "claude-line");
  const label = el("span", "label", "Claude Code");
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
      state.className = "method m-temporary"; state.textContent = `Installed ${c.version}, not signed in`;
      const b = el("button", "small", "Sign in to Claude");
      b.onclick = () => run(b, "Waiting for the browser", "claude_login");
      line.append(b);
    } else {
      state.className = "method m-key";
      state.textContent = `Signed in${c.method === "claude.ai" ? " with Claude.ai" : c.method ? ` with ${c.method}` : ""}${PLAN[c.plan] ? `, ${PLAN[c.plan]} plan` : ""}`;
      msg.textContent = c.version;
    }
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

// Set after step 1 (GitHub) succeeds or the account switches, so the badge runs step 2 at once.
let labSignInNext = false;

// Machines, one row each, and whether this computer can connect to it right now. Members only ever
// connect through Cloudflare (Access for the GitHub sign-in, a tunnel for SSH), never to a machine's address.
// Which project lives where is the vault plugin's job; here only the machine matters.
function machinesSection(machines, org, user) {
  const sec = el("div", "section");
  const head = el("header");
  const again = el("button", "small ghost", "Check again");
  head.append(el("h2", null, "Reachable now"), el("span", "grow"), again);
  sec.append(head, el("p", "sub", "Whether this computer can reach each machine right now, through Cloudflare."));
  const hosts = [...new Map(machines.map(m => [m.host, m])).values()];
  const byHost = Object.fromEntries(hosts.map(m => [m.host, m]));
  const carries = h => machines.filter(m => m.host === h).map(m => m.repo).join(", ");
  const list = el("div", "list"), pills = {}, meters = {}, specs = {};
  const live = new Set();   // machines whose status page answered: signed in and reachable, whatever an older check said
  let lastState = null;
  hosts.forEach(m => {
    const p = el("span", "perm checking"); p.append(el("span", "spinner"));
    pills[m.host] = p;
    const right = [];
    if (m.sometimes) right.push(el("span", "tag", "Often off"));
    if (m.via) right.push(el("span", "tag", `Through ${m.via}`));
    meters[m.host] = el("span", "meters");
    const row = item(m.host, [m.note, `Carries ${carries(m.host)}`].filter(Boolean).join(". "), null, ...right, p);
    row.querySelector(".text").append(meters[m.host]);   // live numbers sit under the name, the right side keeps the status
    // what the machine has, opened by clicking its row once its status is known
    // the row opens to who may connect (always known) and, once the machine answers, its hardware and desktops
    const detail = el("div", "specs"); detail.hidden = true;
    specs[m.host] = el("div");
    detail.append(specs[m.host]);
    if (org) detail.append(whoCanConnect(m.host, machines, org, user));
    row.classList.add("click");
    row.onclick = () => { detail.hidden = !detail.hidden; };
    list.append(row, detail);
  });
  const help = el("div");
  sec.append(help, list);

  const LOOK = { up: ["p4", "Can connect"], down: ["p0", "Can't connect"], "no-tunnel": ["p0", "Tunnel not set up"],
                 "sign-in": ["pending", "Finish sign-in on Team access"], "no-cloudflared": ["p0", "cloudflared missing"] };
  // a machine reached through another one shares that one's way in
  const way = m => byHost[m.via] || m;
  const check = async () => {
    Object.values(pills).forEach(p => { p.className = "perm checking"; p.replaceChildren(el("span", "spinner")); });
    const targets = [...new Map(hosts.map(m => [way(m).host, way(m).tunnel || null])).entries()];
    const state = await working(again, "Checking", () => invoke("reachable", { machines: targets }));
    for (const h of Object.keys(state)) if (live.has(h)) state[h] = "up";
    lastState = state;
    hosts.forEach(m => {
      if (org && !connectRule(way(m).host, machines, org).may(user)) { pills[m.host].className = "perm p0"; pills[m.host].textContent = "No access"; return; }
      const [cls, text] = LOOK[state[way(m).host]] || LOOK.down;
      pills[m.host].className = "perm " + cls; pills[m.host].textContent = text;
    });
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
  }
  // live numbers from each machine's status page, every 15 s while this page is on screen
  const bar = (label, pct, title) => {
    const b = el("span", "meter-mini"); b.title = title;
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
      const [cls, text] = LOOK.up; pills[host].className = "perm " + cls; pills[host].textContent = text;
      const memPct = s.mem_gb ? 100 * s.mem_used_gb / s.mem_gb : 0;
      meters[host].replaceChildren(bar("CPU", s.cpu_pct, `CPU ${s.cpu_pct}% of ${s.threads} threads, load ${s.load.join(" ")}`),
        bar("RAM", memPct, `Memory ${s.mem_used_gb} of ${s.mem_gb} GB in use`),
        ...(gpu == null ? [] : [bar("GPU", gpu, g.map(x => `${x.name}: ${x.util}%, ${Math.round(x.mem_used_mb / 1024)}/${Math.round(x.mem_total_mb / 1024)} GB`).join("\n"))]));
      const d = [["Processor", `${s.cpu} (${s.threads} threads)`], ["Memory", `${s.mem_used_gb} of ${s.mem_gb} GB in use`],
        ...g.map((x, i) => [`GPU ${g.length > 1 ? i : ""}`.trim(), `${x.name}, ${x.util}% busy, ${(x.mem_used_mb / 1024).toFixed(1)} of ${Math.round(x.mem_total_mb / 1024)} GB, ${x.temp_c} °C`]),
        ["Disk", `${gb(s.disk_free_gb)} free of ${gb(s.disk_gb)}`], ["System", `${s.os}, up ${Math.round(s.uptime_h / 24)} days, ${s.users} signed in`]];
      specs[host].replaceChildren(...d.map(([k, v]) => { const r = el("div", "spec"); r.append(el("span", "k", k), el("span", null, v)); return r; }));
      if (s.disk_free_gb != null && s.disk_free_gb < 20) specs[host].append(el("p", "warn", `Disk almost full: ${gb(s.disk_free_gb)} left.`));
      if ((s.desktops || []).length) specs[host].append(desktopList(byHost[host], s.desktops, open));
      // a cluster reached through this machine (rooster through horse) comes in the same answer
      if (s.cluster) hosts.filter(x => x.via === host).forEach(x => clusterView(x.host, s.cluster));
    }
    // a check that finished before these numbers arrived may still be asking for a sign-in
    if (lastState && [...live].some(h => lastState[h] !== "up")) { live.forEach(h => lastState[h] = "up"); showHelp(lastState); }
  };
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
    const add = el("button", "small ghost", "New desktop");
    add.onclick = async e => {
      e.stopPropagation(); msg.textContent = "";
      try { toast(`Desktop ${await working(add, "Starting", () => invoke("desktop", { tunnel: m.tunnel, user: desktops[0]?.user || "ntk", action: "start", display: 0 }))} is ready`); status(); }
      catch (err) { msg.textContent = err; }
    };
    head.append(el("span", "k", "Desktops"), add, msg);
    box.append(head);
    desktops.forEach(d => {
      const r = el("div", "desk"), msg = el("span", "sub"), num = +d.display.slice(1);
      const name = `${d.display}  ${d.geometry || ""}  (${d.user})${d.socket ? "" : "  password"}`;
      const show = port => {
        r.replaceChildren(el("span", null, name));
        // closing ends the desktop for everyone on it, since desktops are shared by the account
        const close = el("button", "small ghost", "Close");
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
          r.append(b, close);
          return;
        }
        const addr = `127.0.0.1:${port}`;
        const copy = el("button", "small ghost", "Copy"), view = el("button", "small ghost", "Open viewer"), stop = el("button", "small ghost", "Disconnect");
        copy.onclick = e => { e.stopPropagation(); navigator.clipboard.writeText(addr).then(() => toast(`Copied ${addr}`), () => toast(addr)); };
        view.onclick = e => { e.stopPropagation(); invoke("open_viewer", { port }); };
        stop.onclick = async e => { e.stopPropagation(); await invoke("close_forward", { host: m.host, display: num }); show(null); };
        r.append(el("span", "sub", "VNC at"), el("strong", "cmd", addr), copy, view, stop, close);
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
  // core is the owners' team and machine-* the machines' extras (set on the Machines page): not layers to tick here
  a.teams.filter(t => t.slug !== "core" && !t.slug.startsWith("machine-")).forEach(t => {
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
  head.append(el("h2", null, v.name), el("span", "tag", here ? "On this computer" : "Not downloaded"),
    pill(Math.min(v.permission, 4), v.repo), el("span", "grow"));
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
  const tools = el("div", "row view-as");
  if (owner) {   // owners may look through any member's eyes
    const pick = el("select");
    Object.keys(a.people).sort((x, y) => (x !== user) - (y !== user) || a.people[x].name.localeCompare(a.people[y].name))
      .forEach(l => pick.append(new Option(l === user ? "Me" : a.people[l].name === l ? l : `${a.people[l].name} (${l})`, l)));
    pick.onchange = () => show(pick.value);
    if (viewAs in a.people) pick.value = viewAs;
    const label = el("label", "sub", "View as ");
    label.append(pick);
    tools.append(el("span", "sub grow", "Click a permission to choose who can open that repo."), label);
  }
  show(viewAs in a.people ? viewAs : user);
  body.append(el("h3", null, "Documents"), tools, docs, teams);
  return sec;
}

function signInView(error, adding) {
  const box = el("div", "empty");
  box.append(el("h1", null, adding ? "Add another GitHub account" : "Sign in with GitHub"),
    el("p", "lede", adding ? "The page that opens approves whichever account your browser is signed in to. Sign in to the other account there first, or use a private window. The new account becomes the active one; switch back from the badge."
                           : "Your team access follows your GitHub account. Signing in has two steps in the browser: GitHub for the repos, then the same account for the machines. The app keeps no token of its own."));
  const code = el("p", "sub"), btn = el("button", null, "Sign in with GitHub");
  btn.onclick = async () => {
    code.textContent = "";
    let got = false;
    const stop = await listen("gh-code", e => {
      got = true;
      // shown and copyable: gh copies it too, but a button does not depend on that having worked
      const copy = el("button", "small ghost", "Copy");
      copy.style.marginLeft = "8px";
      copy.onclick = () => navigator.clipboard.writeText(e.payload).then(() => toast(`Copied ${e.payload}`), () => toast(e.payload));
      code.replaceChildren("Enter this code on the GitHub page that just opened, then approve. It is already copied, so pasting works: ",
        el("strong", "cmd", e.payload), copy);
    });
    // until the code is found, whatever gh says is shown as it is
    const stopLine = await listen("gh-line", e => { if (!got) code.textContent = e.payload; });
    const ok = await working(btn, "Step 1 of 2: waiting for GitHub", () => invoke("sign_in"));
    stop(); stopLine();
    if (ok) { labSignInNext = true; loadTeam(); } else code.textContent = "Sign-in was not finished. Try again.";
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
  // the vault gets its own folder inside the one picked
  const into = parent => parent.replace(/[\\/]+$/, "") + (parent.includes("\\") ? "\\" : "/") + v.repo.split("/").pop();
  const text = el("div", "text");
  const useCopy = el("button", "small ghost", path ? "Change folder" : "Use a copy I already have");
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
  // the team's terms come before anything else, once per version
  const termsOrg = (s.vaults.find(v => v.access) || {}).access;
  if (termsOrg && (termsNow = await invoke("terms_state", { org: termsOrg.org })) && !termsOk()) {
    return page.replaceChildren(termsView(termsNow, termsOrg.org, true));
  }
  await refreshLocal();
  const owner = s.vaults.some(v => v.access && (v.access.people[s.user] || {}).grants === null);
  const parts = [el("h1", null, "Team access"),
    el("p", "lede", owner ? "You are an owner: you can see everyone's access and approve requests."
                          : "What your GitHub account reaches. Ask the owners for anything you need that is not here."),
    badge(s), obsidianNotice()];
  const org = (s.vaults.find(v => v.access) || {}).access;
  if (owner && org) { progress.set(1, "Reading the organisation's people"); parts.push(requestsSection(org), await peopleSection(org.org, s.user, org.teams.filter(t => t.slug !== "core" && !t.slug.startsWith("machine-")), org.tree)); }
  const orgs = new Set(s.vaults.filter(v => v.access).map(v => v.access.org));
  s.vaults.forEach(v => parts.push(vaultSection(v, s.user, viewAs, orgs)));
  page.replaceChildren(...parts);
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
    el("p", "lede", "The lab machines, reached only through Cloudflare after you sign in with GitHub. Click a machine for its hardware and desktops.")];
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
  page.replaceChildren(...head, machines.length ? machinesSection(machines, org, s.user) : el("p", "sub", "No machines are listed for your team yet."));
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
  const paint = () => {
    const extra = new Set(team(extraSlug).members);
    const head = el("div", "row");
    head.append(el("span", "k", "Who can connect"), el("span", "sub", `Teams: ${teamsFor.join(", ")}, plus extra people`));
    box.replaceChildren(head);
    if (!owner) {
      const v = via(user);
      box.append(el("p", "sub", v.length ? `You can connect, through ${v.join(", ")}.` : extra.has(user) ? "You can connect: an owner added you."
        : "You can't connect to this machine. Ask an owner: write access to one of its code repos, or an extra place here."));
      return;
    }
    const people = Object.keys(org.people).sort((a, b) => org.people[a].name.localeCompare(org.people[b].name));
    const can = people.filter(l => via(l).length || extra.has(l));
    for (const login of can) {
      const row = el("div", "spec");
      const tags = el("span");
      via(login).forEach(t => tags.append(el("span", "tag", t)));
      if (extra.has(login)) {
        tags.append(el("span", "tag extra", "Extra"));
        const rm = el("button", "small ghost", "Remove");
        rm.onclick = e => { e.stopPropagation(); change(login, false, rm); };
        tags.append(rm);
      }
      row.append(el("span", "k", org.people[login].name), tags);
      box.append(row);
    }
    const others = people.filter(l => !can.includes(l));
    if (others.length) {
      const pick = el("select");
      pick.append(new Option("Let someone connect", ""), ...others.map(l => new Option(`${org.people[l].name} (@${l})`, l)));
      pick.onclick = e => e.stopPropagation();
      pick.onchange = () => pick.value && change(pick.value, true, pick);
      box.append(pick);
    }
    box.append(el("p", "sub", "Changes reach the machine at that person's next sign-in, within 24 hours."));
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
  const box = el("div", "text");
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

function termsView(t, org, asking) {
  const wrap = el("div", "terms");
  wrap.append(el("h1", null, asking ? "Before you start" : "Terms"),
    el("p", "lede", asking ? (t.accepted ? `The team's terms changed (version ${t.version}). Please read them again.` : "Please read the team's terms. You are asked once; they stay under Terms in the sidebar.")
                           : `Version ${t.version}.` + (t.accepted ? ` You accepted version ${t.accepted.version} on ${t.accepted.date}.` : "")));
  const text = termsText(t.text);
  const agree = el("button", null, "I agree"), note = el("span", "sub", "Scroll to the end to agree.");
  // reaching the end of either language's copy is enough: they say the same thing
  const atEnd = () => { if (text.scrollTop + text.clientHeight >= text.scrollHeight - 8) { agree.disabled = false; note.textContent = ""; } };
  wrap.append(termsSwitch(text, () => setTimeout(atEnd, 0)), text);
  if (asking) {
    agree.disabled = true;
    text.onscroll = atEnd; setTimeout(atEnd, 0);   // a short text fits without scrolling
    agree.onclick = async () => {
      try { await working(agree, "Recording", () => invoke("terms_accept", { org, version: t.version })); termsNow = null; loadTeam(); }
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
  const t = termsNow || await invoke("terms_state", { org: access.org });
  if (!t) return page.replaceChildren(el("h1", null, "Terms"), el("p", "sub", "Your team has no terms yet."));
  const view = termsView(t, access.org, false);
  page.replaceChildren(view);
  // owners see who accepted which version
  if ((access.people[s.user] || {}).grants === null) {
    const who = await invoke("terms_everyone", { org: access.org });
    const sec = el("div", "section"), list = el("div", "list");
    sec.append(el("h2", null, "Who accepted"));
    Object.keys(access.people).sort((a, b) => access.people[a].name.localeCompare(access.people[b].name)).forEach(login => {
      const a = who[login];
      const tag = a ? el("span", "perm " + (a.version >= t.version ? "p4" : "pending"), `Version ${a.version}, ${a.date}`) : el("span", "perm p0", "Not yet");
      list.append(item(access.people[login].name, `@${login}`, null, tag));
    });
    sec.append(list); view.append(sec);
  }
}
