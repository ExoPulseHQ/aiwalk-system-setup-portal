// ?design=next: what the restyle needs that CSS cannot do. Runs after the app's scripts and reads the same globals
// they keep (lastTeam, local, termsNow, connectRule), so every number in a sentence comes from the data on screen.
// After each redraw it (1) writes the page's one sentence, (2) names states by their words (purple ok, amber, red),
// (3) turns logins and machine names into the shared chip, (4) picks the page's one primary button.
(() => {
  if (new URLSearchParams(location.search).get("theme") === "dark") document.documentElement.dataset.theme = "dark";
  const $ = (root, sel) => [...root.querySelectorAll(sel)];
  const n = (k, one, many = one + "s") => `${k} ${k === 1 ? one : many}`;
  const span = (cls, text) => { const s = document.createElement("span"); s.className = cls; s.textContent = text; return s; };
  const chip = (text, title) => { const c = span("nx-chip", text); if (title) c.title = title; return c; };
  const orgOf = s => s && s.user ? (s.vaults.find(v => v.access) || {}).access : null;
  const tl = () => typeof lastTeam === "undefined" ? null : lastTeam;
  const loginByName = (org, name) => org && Object.keys(org.people).find(l => org.people[l].name === name);

  // (2) states by their words: the app reuses its "Admin" style for "Can connect", so the class alone cannot tell
  const OK = /^(Can connect|Version \d)/, BAD = /^(Can't connect|Tunnel not set up)/, WARN = /^(Not yet|Finish sign-in)/;
  function states(root) {
    $(root, ".perm").forEach(p => {
      const t = p.textContent.trim();
      p.classList.toggle("nx-ok", OK.test(t) && !p.classList.contains("pending"));
      p.classList.toggle("nx-bad", BAD.test(t));
      p.classList.toggle("nx-warn", WARN.test(t) || p.classList.contains("pending"));
    });
  }

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
    // an invitation or an intern is a login without a profile name: the row's title is the login itself
    $(root, ".item").forEach(r => {
      const t = r.querySelector(".title");
      if (t && !t.classList.contains("nx-chip") && [...r.querySelectorAll(":scope > .tag")].some(g => /^(Invited|Intern)$/.test(g.textContent))) t.classList.add("nx-chip");
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
      $(document, ".page").forEach(page => { if (page.hidden) return; states(page); chips(page); summary(page); primary(page); });
    } finally { observer.observe(document.querySelector("main"), { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["class", "hidden"] }); }
  };
  const observer = new MutationObserver(() => { if (!queued) { queued = true; requestAnimationFrame(run); } });
  run();
})();
