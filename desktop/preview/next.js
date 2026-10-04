// ?design=next: what the restyle needs that CSS cannot do. Runs after the app's scripts and reads the same globals
// they keep (lastTeam, local, termsNow, connectRule), so every number in a sentence comes from the data on screen.
// After each redraw it (1) writes the page's one sentence, (2) names states by their words (purple ok, amber, red),
// (3) turns logins and machine names into the shared chip, (4) picks the page's one primary button, (5) sets the
// icons (icons.js), each beside a word that says the same, so no state rests on colour alone.
(() => {
  if (new URLSearchParams(location.search).get("theme") === "dark") document.documentElement.dataset.theme = "dark";
  const $ = (root, sel) => [...root.querySelectorAll(sel)];
  const n = (k, one, many = one + "s") => `${k} ${k === 1 ? one : many}`;
  const span = (cls, text) => { const s = document.createElement("span"); s.className = cls; s.textContent = text; return s; };
  const chip = (text, title) => { const c = span("nx-chip", text); if (title) c.title = title; return c; };
  const orgOf = s => s && s.user ? (s.vaults.find(v => v.access) || {}).access : null;
  const tl = () => typeof lastTeam === "undefined" ? null : lastTeam;
  const loginByName = (org, name) => org && Object.keys(org.people).find(l => org.people[l].name === name);

  // an icon: decoration beside its label, so hidden from assistive tech; the label or an aria-label carries the name
  function svg(name) {
    if (!window.NX_ICONS[name]) throw new Error(`next.js: no icon ${name} in icons.js`);
    const i = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    i.setAttribute("viewBox", "0 0 24 24"); i.setAttribute("aria-hidden", "true"); i.setAttribute("focusable", "false");
    i.setAttribute("class", "nx-i"); i.dataset.icon = name; i.innerHTML = window.NX_ICONS[name];
    return i;
  }
  // `name` first in `host`, once; null takes it away
  function mark(host, name) {
    if (!host) return;
    const have = host.querySelector(":scope > svg.nx-i");
    if (have && have.dataset.icon === name) return;
    if (have) have.remove();
    if (name) host.prepend(svg(name));
  }

  // (2) states by their words: the app reuses its "Admin" style for "Can connect", so the class alone cannot tell
  const OK = /^(Can connect|Version \d)/, BAD = /^(Can't connect|Tunnel not set up)/, WARN = /^(Not yet|Finish sign-in)/;
  function states(root) {
    $(root, ".perm").forEach(p => {
      const t = p.textContent.trim();
      p.classList.toggle("nx-ok", OK.test(t) && !p.classList.contains("pending"));
      p.classList.toggle("nx-bad", BAD.test(t));
      p.classList.toggle("nx-warn", WARN.test(t) || p.classList.contains("pending"));
      if (!p.classList.contains("checking")) mark(p, STATE_ICON(p, t));
    });
    // the sign-in lines of the identity block, while they are not drawing a ring or a spinner
    $(root, ".method").forEach(m => { if (!m.querySelector(".ring, .spinner")) mark(m, STATE_ICON(m, m.textContent.trim())); });
    $(root, ".specs .warn").forEach(w => mark(w, "circle-alert"));
  }
  const STATE_ICON = (p, t) => p.classList.contains("nx-ok") || p.classList.contains("m-key") || p.classList.contains("m-account") ? "check"
    : p.classList.contains("nx-bad") ? "x" : p.classList.contains("nx-warn") || p.classList.contains("m-temporary") ? "circle-alert"
    : t === "No access" ? "lock" : null;

  // (3) one chip for people and machines
  function chips(root) {
    const s = tl(), org = orgOf(s), people = org ? org.people : {};
    const hosts = new Set(org ? org.machines.map(m => m.host) : []);
    // a machine's row title, and a login standing alone as a row's second line ("amy-chen" or "@amy-chen")
    $(root, ".item .title").forEach(t => { if (hosts.has(t.textContent) && !t.classList.contains("nx-chip")) t.classList.add("nx-chip"); });
    $(root, ".item .sub").forEach(t => {
      if (t.dataset.nx || t.children.length) return;
      const m = /^@?([\w-]+)(.*)$/.exec(t.textContent);
      const title = t.parentNode.querySelector(".title");
      if (m && (people[m[1]] || (title && title.textContent === m[1])) && (!m[2] || m[2].startsWith(","))) {
        t.dataset.nx = 1; t.replaceChildren(chip(m[1]), m[2]);
      }
    });
    // a request is a person asking: "dee-park asks for read on docs-l4"
    $(root, ".item .title").forEach(t => {
      const m = /^([\w-]+)( asks for .*)$/.exec(t.textContent);
      if (m && !t.dataset.nx && people[m[1]]) { t.dataset.nx = 1; t.replaceChildren(chip(m[1]), m[2]); }
    });
    // who may connect to a machine, and who may merge: logins, beside the machine's own chip
    $(root, ".who-connect .spec > .k, .pr .tag").forEach(k => {
      if (k.dataset.nx) return;
      const login = loginByName(org, k.textContent) || (k.closest(".pr") && /^[\w-]+$/.test(k.textContent) && k.textContent);
      if (login) { k.dataset.nx = 1; k.className = k.className.replace(/\btag\b/, "") + " nx-chip"; k.title = k.textContent; k.textContent = login; }
    });
    const who = root.querySelector(".badge .who > .dim");
    if (who && s && who.textContent === `@${s.user}`) { who.textContent = s.user; who.className = "nx-chip"; }
    // no display name: the badge's name is the login, shown once as the chip; "Signed in on this computer" is what the
    // page's sentence already says
    const name = root.querySelector(".badge .who > .name");
    if (name && s && !s.name && name.textContent === s.user) {
      name.classList.add("nx-chip");
      const dim = name.nextElementSibling;
      if (dim && dim.classList.contains("dim") && !dim.classList.contains("nx-chip")) dim.hidden = true;
    }
    // a person known only by login: the login is the row's title, once, as the chip, and the second line says what
    // else there is, so every person row has the same two lines
    $(root, "#people .list .item, .terms .list .item").forEach(r => {
      const t = r.querySelector(".text > .title");
      if (!t || t.dataset.nx || !(t.textContent in people || r.querySelector(".text > .sub > .nx-chip"))) return;
      if (t.textContent in people && people[t.textContent].name !== t.textContent) return;
      let sub = r.querySelector(".text > .sub");
      const twin = sub && sub.firstChild && sub.firstChild.classList && sub.firstChild.classList.contains("nx-chip") && sub.firstChild.textContent === t.textContent;
      if (sub && !twin) return;   // an invitation or an intern line already says something else
      t.dataset.nx = 1; t.classList.add("nx-chip");
      if (!sub) { sub = document.createElement("div"); sub.className = "sub"; t.after(sub); }
      const rest = twin ? sub.textContent.slice(t.textContent.length).replace(/^,\s*/, "") : "";
      sub.dataset.nx = 1;
      sub.textContent = rest ? rest[0].toUpperCase() + rest.slice(1) : "No display name on GitHub";
    });
    // an invitation or an intern is a login without a profile name: the row's title is the login itself
    $(root, ".item").forEach(r => {
      const t = r.querySelector(".title");
      if (t && !t.classList.contains("nx-chip") && [...r.querySelectorAll(":scope > .tag")].some(g => /^(Invited|Intern)$/.test(g.textContent))) t.classList.add("nx-chip");
    });
  }

  // (5) icons: the sidebar, what a row is, the identity lines, and the few buttons whose sign is conventional
  const NAV = { team: "key-round", machines: "server", people: "users-round", android: "smartphone", vm: "app-window", terms: "scroll-text" };
  const LINE = { GitHub: "github", Machines: "server", "Claude Code": "square-terminal" };
  const BUTTON = { Copy: "copy", "Open viewer": "external-link", "Open in Obsidian": "external-link", "Check again": "refresh-cw",
    "Invite someone": "user-plus", "New desktop": "plus" };   // Remove stays a word: a column of icons on it only added weight
  function icons(page) {
    $(document, "nav button[data-page]").forEach(b => mark(b, NAV[b.dataset.page]));
    $(page, ".claude-line > .label").forEach(l => mark(l, LINE[l.textContent]));
    $(page, "details.vault > summary h2").forEach(h => mark(h, "book-open"));
    $(page, ".section.pr > header > h2").forEach(h => mark(h, "git-pull-request"));
    // rows: a machine, a person, an invitation, a request
    const heading = r => { const sec = r.closest(".section"); return sec ? (sec.querySelector("h2") || {}).textContent || "" : ""; };
    $(page, ".list > .item").forEach(r => {
      const title = r.querySelector(".text > .title"), h = heading(r);
      const kind = title && title.classList.contains("nx-chip") && r.querySelector(".meters") ? "server"
        : /^Waiting for you/.test(h) ? "inbox"
        : h === "People" || h === "Who accepted" ? ([...r.querySelectorAll(":scope > .tag")].some(g => g.textContent === "Invited") ? "mail" : "user-round")
        : null;
      if (kind) { mark(r, kind); r.classList.add("nx-row-i"); }
    });
    // desktops: the row says what it is; its buttons hold together and wrap as one group
    $(page, ".desk").forEach(d => {
      mark(d, /^Screen sharing/.test(d.querySelector(":scope > span").textContent) ? "screen-share" : "monitor");
      const loose = $(d, ":scope > button");
      if (loose.length) { let acts = d.querySelector(":scope > .nx-acts"); if (!acts) { acts = span("nx-acts", ""); d.append(acts); } acts.append(...loose); }
      const addr = d.querySelector("strong.cmd"), copy = $(d, "button").find(b => b.textContent.trim() === "Copy");
      if (copy && addr) { copy.classList.add("nx-icon-only"); copy.setAttribute("aria-label", `Copy ${addr.textContent}`); copy.title = `Copy ${addr.textContent}`; }
    });
    $(page, "button").forEach(b => {
      if (b.classList.contains("working") || b.closest(".lang, .segmented")) return;
      const t = b.textContent.trim();
      mark(b, BUTTON[t] || null);
    });
  }

  // (4) one primary button per page: the action the page is for, else its first filled button
  const PRIMARY = { team: /^(Open in Obsidian|Sign in with GitHub|Approve on GitHub|Finish signing in|I agree)$/, people: /^Invite someone$/, terms: /^I agree$/ };
  function primary(page) {
    const buttons = $(page, "button").filter(b => !b.closest("dialog, .lang, .segmented") && !b.classList.contains("tile"));
    const live = buttons.filter(b => !b.disabled || b.classList.contains("working"));
    const want = PRIMARY[page.id] && live.find(b => PRIMARY[page.id].test(b.textContent.trim()));
    const pick = want || live.find(b => !b.classList.contains("ghost"));
    buttons.forEach(b => b.classList.toggle("nx-primary", b === pick));
  }

  // (1) the page's one sentence: [tone, sentence, detail parts] or null while the page is still reading
  const dev = (text, bad) => span("nx-dev" + (bad ? " bad" : ""), text);
  function teamSentence(page) {
    const s = tl();
    if (page.querySelector(".stage") || !s) return null;
    if (!s.user) return ["warn", "You are not signed in.", ["Nothing is connected yet: no vaults, no machines."]];
    if (page.querySelector(".terms")) return ["warn", "The team's terms are waiting for you.", ["Read them below; you are asked once per version."]];
    const org = orgOf(s), machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
    const reach = [...new Set(machines.map(m => m.host))].filter(h => !org || connectRule(h, machines, org).may(s.user));
    const here = s.vaults.filter(v => local.copies[v.repo]).length;
    // what still needs doing, read from the badge's own state chips
    const todo = $(page, ".claude-line").map(l => [l.querySelector(".label"), l.querySelector(".label + .method")])
      .filter(([k, v]) => k && v && k.textContent !== "GitHub" && (v.classList.contains("m-temporary") || /not done/.test(v.textContent)))
      .map(([k]) => k.textContent);
    if ($(page, ".item .title").some(t => t.textContent === "Owner tools are locked")) todo.push("Owner tools");
    const detail = ["Signed in as ", chip(s.user), `. ${n(here, "vault")} on this computer, ${n(reach.length, "machine")} within reach.`];
    if (todo.length) return ["warn", "Almost everything is connected.", [...detail, " ", dev(`Still to do: ${todo.join(", ")}.`)]];
    return ["ok", "Everything is connected.", detail];
  }
  function machinesSentence(page) {
    const pills = $(page, ".list .item .perm");
    if (!pills.length) return null;
    const host = p => p.closest(".item").querySelector(".title").textContent;
    const up = pills.filter(p => p.classList.contains("nx-ok")), bad = pills.filter(p => p.classList.contains("nx-bad"));
    const off = pills.filter(p => p.textContent.trim() === "No access"), checking = pills.filter(p => p.classList.contains("checking"));
    if (checking.length === pills.length) return ["wait", "Checking the machines.", []];
    const warns = $(page, ".specs .warn").map(w => {
      const row = w.closest(".specs").previousElementSibling;
      return `${row ? row.querySelector(".title").textContent : "A machine"}: ${w.textContent.replace(/^\w/, c => c.toLowerCase()).replace(": ", ", ")}`;
    });
    const detail = [];
    if (up.length) detail.push("Within reach: ", ...up.flatMap((p, i) => [i ? " " : "", chip(host(p))]), ".");
    if (off.length) detail.push(` ${n(off.length, "other needs", "others need")} access from an owner.`);
    warns.forEach(w => detail.push(" ", dev(w)));
    if (bad.length) return ["bad", `${bad.length} of ${n(pills.length, "machine")} cannot be reached.`, [...detail, " ", dev(`Down: ${bad.map(host).join(", ")}.`, true)]];
    const lead = up.length === 1 ? "1 machine is up." : `${up.length} machines are up.`;
    return [warns.length ? "warn" : "ok", warns.length ? lead.replace(".", `, ${warns.length} needs a look.`) : lead, detail];
  }
  function peopleSentence(page) {
    const org = orgOf(tl());
    if (!org || !page.querySelector(".section") || (org.people[tl().user] || {}).grants !== null) return null;
    const people = Object.keys(org.people).length, waiting = org.requests.length;
    const invited = $(page, ".tag").filter(t => t.textContent === "Invited").length, interns = $(page, ".tag").filter(t => t.textContent === "Intern").length;
    const detail = [[invited, `${n(invited, "invitation")} not accepted yet.`], [interns, `${n(interns, "intern", "interns")} outside ${org.org}.`]]
      .filter(([k]) => k).map(([, t]) => t).join(" ");
    return ["ok", `${n(people, "person", "people")}, ${waiting ? `${waiting} waiting for you` : "nobody waiting"}.`, detail ? [detail] : []];
  }
  function termsSentence(page) {
    const t = typeof termsNow === "undefined" ? null : termsNow;
    if (!t || !page.querySelector(".terms")) return null;
    if (!t.accepted || t.accepted.version < t.version) return ["warn", `Version ${t.version} of the terms is waiting for you.`, []];
    const marks = $(page, ".list .perm"), behind = marks.filter(p => !p.classList.contains("nx-ok"));
    const detail = [`Version ${t.version}, agreed on ${t.accepted.date}.`];
    if (marks.length) detail.push(" ", behind.length ? dev(`${behind.length} of ${marks.length} have not agreed to it yet.`) : `All ${marks.length} have agreed to it.`);
    return [behind.length ? "warn" : "ok", "You agreed to the current terms.", detail];
  }
  const SENTENCE = { team: teamSentence, machines: machinesSentence, people: peopleSentence, terms: termsSentence };

  function summary(page) {
    const make = SENTENCE[page.id], got = make && make(page);
    const old = page.querySelector(".nx-summary");
    if (!got) { if (old) old.remove(); return; }
    const [tone, sentence, detail] = got;
    const key = JSON.stringify([tone, sentence, detail.map(d => typeof d === "string" ? d : d.textContent)]);
    if (old && old.dataset.key === key) return;
    const box = document.createElement("div"); box.className = "nx-summary"; box.dataset.key = key;
    box.setAttribute("role", "status");
    const p = document.createElement("p"); p.className = "nx-state"; p.dataset.tone = tone; p.textContent = sentence;
    p.prepend(svg({ ok: "circle-check", warn: "circle-alert", bad: "circle-x", wait: "circle-dashed" }[tone]));
    box.append(p);
    if (detail.length) { const d = document.createElement("p"); d.className = "nx-detail"; d.append(...detail); box.append(d); }
    const h1 = page.querySelector("h1");
    if (old) old.replaceWith(box); else if (h1) h1.after(box); else page.prepend(box);
  }

  // after every redraw, once per frame; our own edits are skipped by the key checks above, so this settles
  let queued = false;
  const run = () => {
    queued = false;
    observer.disconnect();
    try {
      $(document, ".page").forEach(page => { if (page.hidden) return; states(page); chips(page); summary(page); icons(page); primary(page); });
    } finally { observer.observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["class", "hidden"] }); }
  };
  const observer = new MutationObserver(() => { if (!queued) { queued = true; requestAnimationFrame(run); } });
  run();
})();
