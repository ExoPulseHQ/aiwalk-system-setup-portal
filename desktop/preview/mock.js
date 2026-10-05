// Stands in for the Tauri backend in a browser. Read by index.html before its own scripts.
// Query: as=owner|member|signedout|terms|guest|invited, page=team|machines|people|terms|android|vm, theme=light|dark, os=linux|windows|macos|android, ssh=fail
(() => {
  const q = new URLSearchParams(location.search);
  const scenario = q.get("as") || "owner", os = q.get("os") || "linux";
  if (/^(light|dark)$/.test(q.get("theme"))) document.documentElement.dataset.theme = q.get("theme");   // app.css honours both
  document.write('<script src="/__preview/fixtures.js"><\/script>');   // parsed before any later script runs
  // a guest's kept machines live in localStorage (team.js keptMachines); every guest page load starts from these two
  if (scenario === "guest") try { localStorage.setItem("guest-machines:jo-vance", JSON.stringify([["otter", "ssh-otter.example-corp.org"], ["heron", "ssh-heron.example-corp.org"]])); } catch {}

  const wait = ms => new Promise(r => setTimeout(r, ms));
  async function invoke(cmd, args) {
    await wait(150);
    const table = window.__fixtures(scenario, os);
    if (!(cmd in table)) { console.warn("preview: no fixture for", cmd); return null; }
    const a = table[cmd];
    const v = typeof a === "function" ? a(args || {}) : a;
    if (v instanceof Error) throw v.message;   // the app's Err(String) arrives as a plain string
    return JSON.parse(JSON.stringify(v ?? null));   // fresh copy: the page mutates what it is given
  }
  const listen = async () => () => {};
  window.__TAURI__ = { core: { invoke }, event: { listen } };

  // ?page= opens a page directly, once the app's own start-up has drawn the first one
  const page = q.get("page");
  // (Terms, People and Machines read what Team access loaded, so wait for it, at most ~3 s)
  if (page) addEventListener("load", () => {
    let tries = 0;
    const open = () => { if (typeof lastTeam !== "undefined" && lastTeam || ++tries > 30) go(page); else setTimeout(open, 100); };
    setTimeout(open, 300);
  });
})();
