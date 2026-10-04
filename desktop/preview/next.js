// ?design=next: what the restyle needs that CSS cannot do. Runs after the app's scripts and reads the same globals
// they keep (lastTeam, local, termsNow, connectRule), so every number in a sentence comes from the data on screen.
// After each redraw it (1) writes the page's one sentence, (2) names states by their words (purple ok, amber, red),
// (3) turns logins and machine names into the shared chip, (4) picks the page's one primary button, (5) sets the
// icons (icons.js), each beside a word that says the same, so no state rests on colour alone.
(() => {
  const q = new URLSearchParams(location.search);
  if (q.get("theme") === "dark") document.documentElement.dataset.theme = "dark";
  // the display face for the name, the sentence and the wordmark: three candidates, a by default (next.css)
  document.documentElement.dataset.display = /^[abc]$/.test(q.get("display")) ? q.get("display") : "a";
  const mark0 = document.querySelector("nav .app");
  if (mark0 && mark0.firstChild && mark0.firstChild.nodeType === 3 && mark0.firstChild.textContent === "aIwalk") {
    const i = document.createElement("span"); i.className = "nx-cap-i"; i.textContent = "I";
    mark0.firstChild.replaceWith("a", i, "walk");
  }
  const $ = (root, sel) => [...root.querySelectorAll(sel)];
  const n = (k, one, many = one + "s") => `${k} ${k === 1 ? one : many}`;
  const span = (cls, text) => { const s = document.createElement("span"); s.className = cls; s.textContent = text; return s; };
  const chip = (text, title) => { const c = span("nx-chip", text); if (title) c.dataset.tip = title; return c; };
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
  const OK = /^(Can connect|Version \d|Connected over)/, BAD = /^(Can't connect|Tunnel not set up)/, WARN = /^(Not yet|Finish sign-in)/;
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
    : LEVEL[t] ? LEVEL[t][0] : p.classList.contains("m-off") && METHOD[t] ? METHOD[t] : null;

  // access levels: one glyph each, wherever a level shows (the word stays in the vault heading, the tooltip says it)
  const LEVEL = { Admin: ["shield", "Admin: opens the repo and changes who else can"], Write: ["pencil", "Write: opens the repo and changes its files"],
    Read: ["eye", "Read: opens and downloads the repo, no changes"], "No access": ["lock", "No access: ask an owner"] };

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
      if (login) { k.dataset.nx = 1; k.className = k.className.replace(/\btag\b/, "") + " nx-chip"; k.dataset.tip = k.textContent; k.textContent = login; }
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
    "Invite someone": "user-plus", "New desktop": "plus", "Look again": "refresh-cw", "Add a GitHub account": "user-plus",
    "Sign out of GitHub": "log-out", "Change folder": "folder-open", "Get latest": "arrow-down-to-line", "Update host tools": "wrench",
    Disconnect: "unplug", Close: "x", Remove: "user-minus", "Change password": "key-round" };
  // (6) the buttons that are their icon alone, the words in the tooltip and the accessible name. Disconnect and Close only on
  // a desktop row (the Cloudflare panel's Disconnect has no confirmation, and Close on the invite drawer is a word).
  // Kept as words: each page's one primary action, Open viewer and Open desktop (what a desktop row is for), and actions
  // that act at once without asking (Decline, Cancel invitation, the Cloudflare panel).
  // Get latest keeps its word: its arrow alone reads as the Download beside an absent vault, and it is the daily action.
  const ICON_ONLY = new Set(["Copy", "Add a GitHub account", "Sign out of GitHub", "Change folder", "Check again",
    "Update host tools", "New desktop", "Remove", "Change password", "Look again"]);
  const iconOnly = (b, t) => ICON_ONLY.has(t) || (/^(Disconnect|Close)$/.test(t) && !!b.closest(".desk"));
  function icons(page) {
    // the sidebar: below 700px it is its icons alone (next.css), so each button also carries its words as a tooltip
    $(document, "nav button[data-page]").forEach(b => { mark(b, NAV[b.dataset.page]);
      const label = [...b.childNodes].filter(x => x.nodeType === 3).map(x => x.textContent).join("").trim(), os = b.querySelector(".os");
      b.dataset.tip = label + (os && /^\d+$/.test(os.textContent) ? `, ${os.textContent} waiting` : ""); b.dataset.tipNarrow = 1; });
    $(document, "nav button.update").forEach(b => { mark(b, "arrow-down-to-line"); b.dataset.tip = b.textContent.trim(); b.dataset.tipNarrow = 1; });
    $(page, ".claude-line > .label").forEach(l => mark(l, LINE[l.textContent]));
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
      if (copy && addr) { copy.classList.add("nx-icon-only"); copy.setAttribute("aria-label", `Copy ${addr.textContent}`); copy.dataset.tip = `Copy ${addr.textContent}`; }
      // Close ends the desktop for everyone on it, Disconnect only stops this computer's view: two different glyphs, and
      // tooltips that say the difference (an x read as "hide this row")
      const disp = (d.querySelector(":scope > span").textContent.match(/^:\d+/) || [""])[0];
      $(d, "button").forEach(b => {
        const t = b.textContent.trim();
        if (t === "Close") { b.dataset.tip = `Close desktop ${disp}: ends it for everyone on it`; b.dataset.nxIcon = "power"; }
        if (t === "Disconnect") b.dataset.tip = `Disconnect from ${disp}: it keeps running`;
      });
    });
    $(page, "button").forEach(b => {
      if (b.classList.contains("working") || b.classList.contains("nx-info") || b.closest(".lang, .segmented, .nx-who")) return;
      const t = b.textContent.trim();
      mark(b, b.dataset.nxIcon || BUTTON[t] || null);
      // a column of Remove marks names whom each one removes, for the tooltip and for a screen reader's list of buttons
      if (t === "Remove" && !b.dataset.tip) {
        const r = b.closest(".item"), who = r && (r.querySelector(".text .nx-chip") || r.querySelector(".text > .title"));
        if (who) { b.dataset.tip = `Remove ${who.textContent.trim()}`; b.classList.add("nx-row-act"); }
      }
      if (iconOnly(b, t)) {
        b.classList.add("nx-icon-only");
        if (!b.dataset.tip) b.dataset.tip = t;
        if (!b.getAttribute("aria-label")) b.setAttribute("aria-label", b.dataset.tip);
      }
    });
  }

  // the Windows VM and phone rows, and the member's line on a machine they cannot reach
  const ROW = { "Card reader": "credit-card", "Windows account": "user-round", "VM folder": "folder", "Memory (GB)": "memory-stick",
    "CPU cores": "cpu", "Disk size (GB)": "hard-drive", "Clean up space": "trash-2",
    "Type with this keyboard": "keyboard", "Show the phone screen here": "screen-share", Desktop: "monitor", "Switch to Wi-Fi": "wifi", "Disconnect Wi-Fi": "wifi-off" };
  // the app's own shortcut icons, as the desktop shows them (serve.mjs serves icons/ at /__icons/)
  const APP_ICON = { "Open Windows": "windows-open", "Shut down Windows": "windows-stop" };
  function rows(page) {
    if (page.id === "vm" || page.id === "android") $(page, ".list > .item").forEach(r => {
      const t = (r.querySelector(".text > .title") || {}).textContent;
      if (APP_ICON[t]) {
        if (!r.querySelector(":scope > img.nx-app-i")) {
          const img = document.createElement("img"); img.className = "nx-app-i"; img.alt = ""; img.width = img.height = 28;
          img.src = `/__icons/${APP_ICON[t]}.svg`; r.prepend(img);
        }
      } else if (ROW[t]) { mark(r, ROW[t]); r.classList.add("nx-row-i"); }
      else r.classList.add("nx-row-pad");
      const sub = r.querySelector(".text > .sub");
      if (t === "VM folder" && sub) sub.classList.add("nx-mono");
      if (t === "Windows account" && sub && !sub.dataset.nx) { sub.dataset.nx = 1; sub.replaceChildren(chip(sub.textContent)); }
    });
    if (page.id === "android") $(page, ".card > .row").forEach(head => {
      const draw = head.querySelector("svg"), link = head.querySelector(".dim");
      if (draw && !draw.getAttribute("viewBox")) { draw.setAttribute("viewBox", "0 0 96 112"); draw.setAttribute("width", 72); draw.setAttribute("height", 84); }
      if (link && /^Connected over/.test(link.textContent)) link.className = "perm";
    });
    $(page, ".who-connect > p.sub").forEach(p => {
      if (/^You can't connect/.test(p.textContent)) { p.className = "nx-ask"; mark(p, "lock"); }
    });
  }

  // (7) states as glyphs, explanations behind an info mark, hardware labels as icons. The word stays in the DOM (hidden
  // from the eye, read by assistive tech) and in the tooltip, so the page's sentences and counts still read it.
  const sr = text => span("nx-sr", text);
  function glyph(el, icon, tipText) {
    if (icon) mark(el, icon);
    el.classList.add("nx-glyph"); el.dataset.tip = tipText;
    if (!el.hasAttribute("tabindex")) el.tabIndex = 0;   // so the tooltip also opens from the keyboard
  }
  const TAG = { "Often off": ["moon", "Often off: this machine is switched off at times"],
    "On this computer": ["monitor-check", "On this computer: a copy of this vault is here"],
    "Not downloaded": ["cloud", "Not downloaded: it stays on GitHub until you download it"],
    Invited: ["hourglass", "Invited: has not accepted yet"], Intern: ["graduation-cap", "Intern: outside the organisation, single repos only"] };
  const METHOD = { "GitHub account": "user-round", "Temporary credential": "clock", "Security key": "key-round" };
  const HW = { Processor: "cpu", Memory: "memory-stick", Disk: "hard-drive", System: "server-cog", GPU: "gpu" };
  // explanations that move behind an info mark: [the sentence's start, where the mark goes]
  // the heading a sentence sits under: the nearest h2/h3 before it, or its section's
  const headingBefore = el => {
    for (let x = el.previousElementSibling; x; x = x.previousElementSibling) {
      if (/^H[23]$/.test(x.tagName)) return x;
      const h = x.matches("header") && x.querySelector("h2, h3");
      if (h) return h;
    }
    return el.parentElement && !el.parentElement.classList.contains("page") ? headingBefore(el.parentElement) : null;
  };
  const INFO = [[/^Click a permission to choose/, el => headingBefore(el.closest(".view-as") || el)],
    [/^Whether this computer can reach/, headingBefore],
    [/^Changes reach the machine at that person's next sign-in/, el => el.closest(".who-connect").querySelector(".k")],
    [/^For owners\. The machines sit behind/, headingBefore],
    [/ on GitHub\. Owners can open every repo/, headingBefore],
    [/; they have single repos only\.$/, headingBefore],
    [/^(Added to the desktop and the app menu|Applied while Windows is off)$/, headingBefore]];
  // `extra` follows the sentence's own words; `instead` replaces them
  function info(sentence, host, extra, instead) {
    if (!host || sentence.classList.contains("nx-moved")) return;
    const text = instead || sentence.textContent.trim() + (extra ? " " + extra : "");
    sentence.classList.add("nx-moved");
    if ([...host.querySelectorAll(":scope > .nx-info")].some(i => i.dataset.tip === text)) return;   // the sentence was redrawn
    const b = document.createElement("button"); b.type = "button"; b.className = "nx-info"; b.dataset.tip = text; b.setAttribute("aria-label", text);
    b.append(svg("info"));
    host.append(b);
  }
  function compact(page) {
    $(page, ".perm").forEach(p => {
      if (p.classList.contains("checking") || p.closest(".badge")) return;
      const t = p.textContent.trim(), host = (p.closest(".item") || {}).querySelector ? p.closest(".item").querySelector(".title") : null;
      if (LEVEL[t] && !p.closest("summary")) glyph(p, null, LEVEL[t][1] + (p.classList.contains("edit") ? ". Click to change who can open it." : ""));
      else if (LEVEL[t]) p.dataset.tip = LEVEL[t][1];
      else if (t === "Can connect") glyph(p, null, `Can connect: this computer reaches ${host ? host.textContent : "it"} now, through Cloudflare`);
      else p.classList.remove("nx-glyph");
      // terms: who agreed to the current version shows the date; the version is in the tooltip
      const v = /^Version (\d+), (\S+)$/.exec(t);
      if (v && p.classList.contains("nx-ok") && !p.querySelector(".nx-sr")) {
        p.dataset.tip = `Agreed to version ${v[1]} on ${v[2]}`;
        p.replaceChildren(...[p.querySelector(":scope > svg")].filter(Boolean), sr(`Version ${v[1]}, `), v[2]);
        if (!p.hasAttribute("tabindex")) p.tabIndex = 0;
      }
    });
    // a tag that explains a deviation keeps its words beside the glyph: "Often off" is why a machine does not answer
    const WORDED = new Set(["Often off"]);
    $(page, ".tag").forEach(g => { const t = g.textContent.trim(); if (!TAG[t]) return;
      if (WORDED.has(t)) { mark(g, TAG[t][0]); g.dataset.tip = TAG[t][1]; g.classList.add("nx-worded"); } else glyph(g, ...TAG[t]); });
    // the identity block: the method in use keeps its word, the other ways to sign in are their icon
    $(page, ".claude-line .method").forEach(m => {
      const t = m.textContent.trim();
      if (METHOD[t] && m.classList.contains("m-off")) glyph(m, METHOD[t], `${t}: another way to sign in, not in use`);
      const said = /^(Signed in (?:as|with) )(.+)$/.exec(t);
      if (said && !m.querySelector(".nx-sr")) { m.dataset.tip = t; m.replaceChildren(...[m.querySelector(":scope > svg")].filter(Boolean), sr(said[1]), said[2]); }
    });
    $(page, ".specs .spec > .k").forEach(k => {
      if (k.classList.contains("nx-hw") || k.classList.contains("nx-chip")) return;
      const m = /^(Processor|Memory|Disk|System|GPU)(?: (\d+))?$/.exec(k.textContent.trim());
      if (!m) return;
      k.classList.add("nx-hw"); k.dataset.tip = k.textContent.trim();
      k.replaceChildren(svg(HW[m[1]]), sr(m[1] + (m[2] ? " " : "")), ...(m[2] ? [m[2]] : []));
    });
    $(page, ".sub, p").forEach(p => {
      if (p.classList.contains("nx-moved") || p.closest(".nx-summary, .terms > .text")) return;
      const t = p.textContent.trim(), hit = INFO.find(([re]) => re.test(t));
      if (!hit) return;
      // the Machines page's opening paragraph joins the section's info mark
      const lede = /^Whether this computer/.test(t) && page.querySelector(":scope > .lede");
      if (lede) lede.classList.add("nx-moved");
      info(p, hit[1](p), lede ? lede.textContent.trim() : "");
    });
  }

  // (8) who may use a thing, as one wrapping line of people grouped by why: the reason is a quiet label said once, the
  // person is the chip. Added-by-hand people carry their own remove mark; the add control is a "+" at the end of the
  // line that opens the page's own select in place. One component for "Who can connect" and the merge lists.
  // people: [{login, name, why: [group, ...], rm: button | null}], groups in the order given
  function whoLine(people, order, add, label) {
    const line = document.createElement("div"); line.className = "nx-who"; line.setAttribute("role", "list"); line.setAttribute("aria-label", label);
    const byGroup = new Map(order.map(g => [g, []]));
    people.forEach(p => { const g = p.why[0]; if (!byGroup.has(g)) byGroup.set(g, []); byGroup.get(g).push(p); });
    byGroup.forEach((ps, g) => {
      if (!ps.length) return;
      const grp = span("nx-who-g", ""); grp.setAttribute("role", "listitem");
      grp.append(span("nx-who-k", g));
      ps.forEach(p => {
        const c = chip(p.login, [p.name !== p.login ? p.name : "", p.why.length > 1 ? `also ${p.why.slice(1).join(", ")}` : ""].filter(Boolean).join(", ") || p.login);
        c.tabIndex = 0;
        if (p.rm) {
          c.classList.add("nx-has-x");
          p.rm.classList.add("nx-x"); p.rm.dataset.tip = p.rmTip; p.rm.setAttribute("aria-label", p.rmTip);
          mark(p.rm, "x"); c.append(p.rm);
        }
        grp.append(c);
      });
      line.append(grp);
    });
    if (!people.length) line.append(span("nx-who-k", "Nobody yet"));
    if (add) {
      const wrap = span("nx-who-add", ""), b = document.createElement("button");
      b.type = "button"; b.className = "small ghost nx-icon-only nx-add"; b.textContent = add.options[0].text;
      b.dataset.tip = add.options[0].text; b.setAttribute("aria-label", add.options[0].text); mark(b, "user-plus");
      add.classList.add("nx-off");
      b.onclick = e => { e.stopPropagation(); add.classList.remove("nx-off"); b.hidden = true; add.focus(); try { add.showPicker(); } catch {} };
      add.addEventListener("blur", () => { if (!add.value) { add.classList.add("nx-off"); b.hidden = false; } });
      wrap.append(b, add); line.append(wrap);
    }
    return line;
  }
  function whoLists(page) {
    const s = tl(), org = orgOf(s);
    if (!org) return;
    const nameOf = l => (org.people[l] || {}).name || l;
    // Who can connect, owner and member alike, read from the same rule the app uses
    $(page, ".who-connect").forEach(box => {
      if (box.querySelector(":scope > .nx-who") || typeof connectRule === "undefined") return;
      const row = box.closest(".specs") && box.closest(".specs").previousElementSibling;
      const host = row && row.querySelector(".text > .title") && row.querySelector(".text > .title").textContent;
      const machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
      if (!host || !machines.some(m => m.host === host)) return;
      const rule = connectRule(host, machines, org), extra = new Set(rule.team(`machine-${host}`).members);
      const HAND = "added by hand";
      const rms = new Map($(box, ":scope > .spec").map(sp => [((sp.querySelector(".k") || {}).textContent || ""), sp.querySelector("button")]));
      const people = Object.keys(org.people).sort((a, b) => nameOf(a).localeCompare(nameOf(b)))
        .filter(l => rule.via(l).length || extra.has(l))
        .map(l => { const rm = rms.get(l) || rms.get(nameOf(l)) || null;
          return { login: l, name: nameOf(l), why: [...rule.via(l), ...(extra.has(l) ? [HAND] : [])], rm, rmTip: `Remove ${l}'s ${rule.via(l).length ? "extra " : ""}access to ${host}` }; });
      // a member's copy of the rule may know only their own teams: then their own sentence says more than a line of one
      const owner = (org.people[s.user] || {}).grants === null;
      const solo = !owner && !people.some(p => p.login !== s.user);
      const pick = box.querySelector(":scope > select");
      const head = box.querySelector(":scope > .row"), teams = head && head.querySelector(":scope > .sub");
      const changes = $(box, ":scope > p.sub").find(p => /^Changes reach/.test(p.textContent));
      if (solo && teams && !teams.classList.contains("nx-moved")) {
        const m = /^Teams: (.*?)(, plus extra people)?$/.exec(teams.textContent.trim());
        info(teams, head.querySelector(".k"), "", m ? `Members of ${m[1]} can connect${m[2] ? ", and anyone an owner adds by hand" : ""}.` : "");
      }
      if (solo) return;
      // the reason, said once behind the heading's info mark; the group labels say who
      if (teams && head) {
        const m = /^Teams: (.*?)(, plus extra people)?$/.exec(teams.textContent.trim());
        info(teams, head.querySelector(".k"), "", (m ? `Members of ${m[1]} can connect${m[2] ? ", and anyone an owner adds by hand" : ""}.` : teams.textContent) + (changes ? " " + changes.textContent.trim() : ""));
        if (changes) changes.classList.add("nx-moved");
      }
      $(box, ":scope > .spec").forEach(sp => sp.remove());
      const yours = $(box, ":scope > p.sub").find(p => /^You can connect/.test(p.textContent));
      if (yours) yours.classList.add("nx-moved");
      (head || box.firstChild).after(whoLine(people, [...rule.teamsFor, HAND], pick, `Who can connect to ${host}`));
    });
    // who may merge: the same line, "can merge" and "opens pull requests" as its two groups
    $(page, ".pr .specs").forEach(box => {
      const specs = $(box, ":scope > .spec");
      if (!specs.length) return;
      const title = box.previousElementSibling && box.previousElementSibling.querySelector(".title"), repo = title ? title.textContent : "this repo";
      const people = [], seen = new Map();
      specs.forEach(sp => {
        const g = (sp.querySelector(".k") || {}).textContent === "Can merge" ? "can merge" : "opens pull requests";
        $(sp, ".nx-chip, .tag").forEach(c => {
          const login = c.textContent.trim(), rm = c.nextElementSibling && c.nextElementSibling.tagName === "BUTTON" ? c.nextElementSibling : null;
          if (seen.has(login)) { seen.get(login).why.push(g); return; }
          const p = { login, name: nameOf(login), why: [g], rm, rmTip: `Remove ${login}'s merge right on ${repo}` };
          seen.set(login, p); people.push(p);
        });
      });
      const pick = box.querySelector(":scope > select");
      specs.forEach(sp => sp.remove());
      box.prepend(whoLine(people, ["can merge", "opens pull requests"], pick, `Who can merge on ${repo}`));
    });
  }

  // the one tooltip: on hover after 120 ms, at once on keyboard focus, gone on Escape, on leaving, or when its element goes
  const tipBox = document.createElement("div");
  tipBox.id = "nx-tip"; tipBox.setAttribute("role", "tooltip"); tipBox.hidden = true; document.body.append(tipBox);
  let tipFor = null, tipTimer = 0;
  function showTip(el) {
    clearTimeout(tipTimer);
    if (el.dataset.tipNarrow && innerWidth > 700) return;   // the sidebar's words are on screen there
    tipFor = el;
    tipBox.textContent = el.dataset.tip; tipBox.hidden = false;
    if (el.getAttribute("aria-label") !== el.dataset.tip) el.setAttribute("aria-describedby", "nx-tip");
    const r = el.getBoundingClientRect(), w = tipBox.offsetWidth, h = tipBox.offsetHeight;
    const below = r.bottom + 6 + h <= innerHeight - 8;
    tipBox.style.top = `${below ? r.bottom + 6 : r.top - h - 6}px`;
    tipBox.style.left = `${Math.max(8, Math.min(r.left + r.width / 2 - w / 2, innerWidth - w - 8))}px`;
  }
  function hideTip() {
    clearTimeout(tipTimer);
    if (tipFor) tipFor.removeAttribute("aria-describedby");
    tipFor = null; tipBox.hidden = true;
  }
  const tipOf = e => e.target instanceof Element ? e.target.closest("[data-tip]") : null;
  document.addEventListener("pointerover", e => {
    const t = tipOf(e);
    if (t === tipFor) return;
    clearTimeout(tipTimer);
    if (!t) return hideTip();
    tipTimer = setTimeout(() => showTip(t), 120);
  });
  document.addEventListener("pointerleave", hideTip);
  document.addEventListener("focusin", e => { const t = tipOf(e); t ? showTip(t) : hideTip(); });
  document.addEventListener("focusout", hideTip);
  document.addEventListener("keydown", e => { if (e.key === "Escape") hideTip(); });
  // focusing an element scrolls it into view: follow it rather than drop the tooltip
  document.addEventListener("scroll", () => { if (tipFor) showTip(tipFor); }, true);

  // (4) one primary button per page: the action the page is for, else its first filled button
  const NO_PRIMARY = new Set(["vm", "android"]);
  const PRIMARY = { team: /^(Open in Obsidian|Sign in with GitHub|Approve on GitHub|Finish signing in|I agree)$/, people: /^(Send invitation|Invite someone)$/, terms: /^I agree$/ };
  function primary(page) {
    const buttons = $(page, "button").filter(b => !b.closest("dialog, .lang, .segmented") && !b.classList.contains("tile") && !b.classList.contains("nx-info"));
    const live = buttons.filter(b => !b.disabled || b.classList.contains("working"));
    // with the invite drawer open, sending it is the page's action, not the Close that replaced Invite someone
    const want = PRIMARY[page.id] && (live.find(b => b.textContent.trim() === "Send invitation" && b.closest(".drawer.open")) || live.find(b => PRIMARY[page.id].test(b.textContent.trim())));
    // the Windows VM and phone pages act through their rows; no button there is the page's one action
    const pick = NO_PRIMARY.has(page.id) ? null : want || live.find(b => !b.classList.contains("ghost"));
    buttons.forEach(b => b.classList.toggle("nx-primary", b === pick));
  }

  // (1) the page's opening state, as a strip of icons and figures: [tone, sentence, detail, items] or null while the
  // page is still reading. The old sentence and its detail stay, visually hidden, as what a screen reader hears; each
  // item is focusable and named by its fragment of that sentence. Words show only for deviations and things to do.
  // item: {icon, num, unit, chips, text, tone: warn | bad | act | calm, tip}
  function teamSentence(page) {
    const s = tl(), t = typeof termsNow === "undefined" ? null : termsNow;
    // the terms come before anything else is read, so this one stands on termsNow alone
    if (page.querySelector(".terms") && t) {
      const said = t.accepted ? `The team changed its terms. Please agree to version ${t.version}.` : "The team asks you to agree to its terms.";
      return ["warn", said, [`Version ${t.version}. Read it to the end, then press I agree. You are asked once per version.`],
        [{ icon: "scroll-text", text: `Agree to version ${t.version}`, tone: "warn", tip: `${said} Read it to the end, then press I agree.` }]];
    }
    if (page.querySelector(".stage") || !s) return null;
    if (!s.user) return ["warn", "You are not signed in.", ["Nothing is connected yet: no vaults, no machines."],
      [{ icon: "github", text: "Not signed in", tone: "warn", tip: "You are not signed in: no vaults, no machines yet" }]];
    const org = orgOf(s), machines = s.vaults.flatMap(v => (v.access && v.access.machines) || []);
    const reach = [...new Set(machines.map(m => m.host))].filter(h => !org || connectRule(h, machines, org).may(s.user));
    const here = s.vaults.filter(v => local.copies[v.repo]).length;
    // what still needs doing, read from the badge's own state chips
    const todo = $(page, ".claude-line").map(l => [l.querySelector(".label"), l.querySelector(".label + .method")])
      .filter(([k, v]) => k && v && k.textContent !== "GitHub" && (v.classList.contains("m-temporary") || /not done/.test(v.textContent)))
      .map(([k]) => k.textContent);
    if ($(page, ".item .title").some(t => t.textContent === "Owner tools are locked")) todo.push("Owner tools");
    const detail = [`Signed in as ${s.user}. ${n(here, "vault")} on this computer, ${n(reach.length, "machine")} within reach.`];
    const items = [{ icon: "check", chips: [s.user], tip: `Signed in as ${s.user}` },
      { icon: "monitor-check", num: here, unit: here === 1 ? "vault here" : "vaults here", tip: `${n(here, "vault")} on this computer` },
      { icon: "server", num: reach.length, unit: "in reach", tip: `${n(reach.length, "machine")} within reach` }];
    if (todo.length) return ["warn", "Almost everything is connected.", [...detail, ` Still to do: ${todo.join(", ")}.`],
      [...items, { icon: "circle-alert", text: `To do: ${todo.join(", ")}`, tone: "warn", tip: `Still to do: ${todo.join(", ")}` }]];
    return ["ok", "Everything is connected.", detail, items];
  }
  function machinesSentence(page) {
    const pills = $(page, ".list .item .perm");
    if (!pills.length) return null;
    const host = p => p.closest(".item").querySelector(".title").textContent;
    const up = pills.filter(p => p.classList.contains("nx-ok")), bad = pills.filter(p => p.classList.contains("nx-bad"));
    const off = pills.filter(p => p.textContent.trim() === "No access"), checking = pills.filter(p => p.classList.contains("checking"));
    if (checking.length === pills.length) return ["wait", "Checking the machines.", [], [{ icon: "circle-dashed", text: "Checking", tone: "calm", tip: "Checking the machines" }]];
    const warns = $(page, ".specs .warn").map(w => {
      const row = w.closest(".specs").previousElementSibling;
      return [row ? row.querySelector(".title").textContent : "A machine", w.textContent.trim().replace(/^\w/, c => c.toLowerCase()).replace(": ", ", ").replace(/\.$/, "")];
    });
    const items = [], detail = [];
    if (up.length) { items.push({ icon: "server", num: up.length, chips: up.map(host), tip: `${up.length === 1 ? "1 machine is" : `${up.length} machines are`} up: ${up.map(host).join(", ")}` });
      detail.push(`Within reach: ${up.map(host).join(", ")}.`); }
    if (off.length) { const t = `${n(off.length, "other needs", "others need")} access from an owner`; items.push({ icon: "lock", num: off.length, unit: "no access", tone: "calm", tip: t }); detail.push(` ${t}.`); }
    warns.forEach(([h, w]) => { items.push({ icon: "circle-alert", chips: [h], text: w, tone: "warn", tip: `${h}: ${w}` }); detail.push(` ${h}: ${w}.`); });
    if (bad.length) {
      items.push({ icon: "circle-x", chips: bad.map(host), text: "down", tone: "bad", tip: `Cannot be reached: ${bad.map(host).join(", ")}` });
      return ["bad", `${bad.length} of ${n(pills.length, "machine")} cannot be reached.`, [...detail, ` Down: ${bad.map(host).join(", ")}.`], items];
    }
    const lead = up.length === 1 ? "1 machine is up." : `${up.length} machines are up.`;
    return [warns.length ? "warn" : "ok", warns.length ? lead.replace(".", `, ${warns.length} needs a look.`) : lead, detail, items];
  }
  function peopleSentence(page) {
    const org = orgOf(tl());
    if (!org || !page.querySelector(".section") || (org.people[tl().user] || {}).grants !== null) return null;
    const people = Object.keys(org.people).length, waiting = org.requests.length;
    const invited = $(page, ".tag").filter(t => t.textContent === "Invited").length, interns = $(page, ".tag").filter(t => t.textContent === "Intern").length;
    const items = [{ icon: "users-round", num: people, unit: people === 1 ? "person" : "people", tip: n(people, "person", "people") },
      waiting ? { icon: "inbox", num: waiting, text: "waiting", tone: "act", tip: `${waiting} waiting for you to approve or decline` }
        : { icon: "inbox", num: 0, unit: "waiting", tone: "calm", tip: "Nobody waiting for you" }];
    if (invited) items.push({ icon: "mail", num: invited, unit: "invited", tone: "calm", tip: `${n(invited, "invitation")} not accepted yet` });
    if (interns) items.push({ icon: "graduation-cap", num: interns, unit: interns === 1 ? "intern" : "interns", tone: "calm", tip: `${n(interns, "intern", "interns")} outside ${org.org}` });
    const detail = [[invited, `${n(invited, "invitation")} not accepted yet.`], [interns, `${n(interns, "intern", "interns")} outside ${org.org}.`]]
      .filter(([k]) => k).map(([, t]) => t).join(" ");
    return ["ok", `${n(people, "person", "people")}, ${waiting ? `${waiting} waiting for you` : "nobody waiting"}.`, detail ? [detail] : [], items];
  }
  function termsSentence(page) {
    const t = typeof termsNow === "undefined" ? null : termsNow;
    if (!t || !page.querySelector(".terms")) return null;
    if (!t.accepted || t.accepted.version < t.version) return ["warn", `Version ${t.version} of the terms is waiting for you.`, [],
      [{ icon: "scroll-text", text: `Agree to version ${t.version}`, tone: "warn", tip: `Version ${t.version} of the terms is waiting for you` }]];
    const marks = $(page, ".list .perm"), behind = marks.filter(p => !p.classList.contains("nx-ok"));
    const detail = [`Version ${t.version}, agreed on ${t.accepted.date}.`];
    const items = [{ icon: "scroll-text", num: `v${t.version}`, tip: `You agreed to version ${t.version} on ${t.accepted.date}` }];
    if (marks.length) {
      items.push({ icon: "users-round", num: `${marks.length - behind.length}/${marks.length}`, unit: "agreed", tip: `${marks.length - behind.length} of ${marks.length} have agreed to version ${t.version}` });
      if (behind.length) items.push({ icon: "circle-alert", num: behind.length, text: "not yet", tone: "warn", tip: `${behind.length} of ${marks.length} have not agreed to version ${t.version} yet` });
      detail.push(behind.length ? ` ${behind.length} of ${marks.length} have not agreed to it yet.` : ` All ${marks.length} have agreed to it.`);
    }
    return [behind.length ? "warn" : "ok", "You agreed to the current terms.", detail, items];
  }
  function vmSentence(page) {
    const vm = typeof lastVm === "undefined" ? null : lastVm;
    if (!vm || !page.querySelector(".list")) return null;
    const gb = v => String(v).replace(/^(\d+)G$/, "$1 GB"), num = v => (/^(\d+)/.exec(String(v)) || [, v])[1];
    const used = /takes (\d+) GB/.exec((page.querySelector("#clean-sub") || {}).textContent || "");
    const detail = [`${gb(vm.ram)} memory, ${vm.cpus} CPU cores, ${gb(vm.disk)} disk` + (used ? `, ${used[1]} GB of it used.` : ".")];
    const items = [{ icon: "memory-stick", num: num(vm.ram), unit: "GB", tip: `${gb(vm.ram)} memory` },
      { icon: "cpu", num: vm.cpus, unit: "cores", tip: `${vm.cpus} CPU cores` },
      { icon: "hard-drive", num: num(vm.disk), unit: "GB", tip: `${gb(vm.disk)} disk` + (used ? `, ${used[1]} GB of it used` : "") }];
    // on or off is what this page is asked first, so it keeps its word
    return vm.running ? ["ok", "Windows is running.", detail, [{ icon: "power", text: "Running", tip: "Windows is running" }, ...items]]
      : ["off", "Windows is off.", [...detail, " Open Windows starts it."], [{ icon: "power", text: "Off", tone: "calm", tip: "Windows is off: Open Windows starts it" }, ...items]];
  }
  function phonesSentence(page) {
    if (typeof phonesSig === "undefined" || !phonesSig) return null;
    const [phones, waiting] = JSON.parse(phonesSig);
    const how = p => `${p.model} over ${p.wireless ? "Wi-Fi" : "USB"}`;
    const ask = "A phone is waiting: unlock it and tap Allow on the USB debugging prompt.";
    const askItem = { icon: "circle-alert", text: "Allow debugging", tone: "warn", tip: ask };
    if (!phones.length) return waiting ? ["warn", "A phone is waiting for you to allow debugging.", [], [askItem]] : null;
    const lead = phones.length === 1 ? `${phones[0].model} is connected.` : `${phones.length} phones are connected.`;
    const detail = [phones.map(how).join(", ") + "."];
    const items = [{ icon: "smartphone", num: phones.length, tip: `Connected: ${phones.map(how).join(", ")}` }];
    return waiting ? ["warn", lead, [...detail, " " + ask], [...items, askItem]] : ["ok", lead, detail, items];
  }
  const SENTENCE = { team: teamSentence, machines: machinesSentence, people: peopleSentence, terms: termsSentence, vm: vmSentence, android: phonesSentence };

  function stripItem({ icon, num, unit, chips: logins, text, tone, tip }) {
    const it = span("nx-it" + (tone ? " " + tone : ""), "");
    it.tabIndex = 0; it.dataset.tip = tip; it.setAttribute("role", "img"); it.setAttribute("aria-label", tip);
    if (icon) it.append(svg(icon));
    if (num !== undefined) it.append(span("nx-num", String(num)));
    if (unit) it.append(span("nx-unit", unit));
    (logins || []).forEach(l => it.append(span("nx-chip", l)));
    if (text) it.append(span("nx-it-t", text));
    return it;
  }
  function summary(page) {
    const make = SENTENCE[page.id], got = make && make(page);
    const old = page.querySelector(".nx-summary");
    if (!got) { if (old) old.remove(); return; }
    const [tone, sentence, detail, items] = got;
    const key = JSON.stringify([tone, sentence, detail, items]);
    if (old && old.dataset.key === key) return;
    const box = document.createElement("div"); box.className = "nx-summary"; box.dataset.key = key;
    box.setAttribute("role", "status");
    const strip = document.createElement("div"); strip.className = "nx-strip"; strip.dataset.tone = tone;
    // the state mark hangs in the margin; hovering it says the whole old sentence
    const m = span("nx-mark", ""); m.dataset.tip = sentence; m.setAttribute("aria-hidden", "true");
    m.append(svg({ ok: "circle-check", warn: "circle-alert", bad: "circle-x", wait: "circle-dashed", off: "power" }[tone]));
    const said = span("nx-sr", [sentence, ...detail].join(" "));
    strip.append(m, said, ...items.map(stripItem));
    box.append(strip);
    // after the page name; on the phone page the name shares a row with Look again, so after that row
    const h1 = page.querySelector("h1"), at = h1 && h1.parentNode.classList.contains("row") ? h1.parentNode : h1;
    if (old) old.replaceWith(box); else if (at) at.after(box); else page.prepend(box);
  }

  // after every redraw, once per frame; our own edits are skipped by the key checks above, so this settles
  let queued = false;
  const run = () => {
    queued = false;
    observer.disconnect();
    try {
      $(document, ".page").forEach(page => { if (page.hidden) return; rows(page); states(page); chips(page); whoLists(page); summary(page); icons(page); compact(page); primary(page); });
      if (tipFor && !tipFor.isConnected) hideTip();
    } finally { observer.observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["class", "hidden"] }); }
  };
  const observer = new MutationObserver(ms => { if (ms.every(m => tipBox.contains(m.target))) return; if (!queued) { queued = true; requestAnimationFrame(run); } });
  run();
})();
