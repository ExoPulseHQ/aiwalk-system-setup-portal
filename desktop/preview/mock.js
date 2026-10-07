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
  // os=android: only what the phone build registers (app.rs, the Android handlers()); anything else is refused, as there
  const PHONE = new Set(("platform start_page cf_state cf_connect cf_forget cf_share cf_key cf_join cf_renew update_state terms_state terms_accept " +
    "terms_everyone team_access reachable machine_status forwards lab_identity lab_sign_out access_login find_machine my_machines publish_machines " +
    "org_people invite cancel_invite invite_intern remove_intern set_role remove_member set_access machine_extra machine_blocks machine_block machine_guests machine_guest " +
    "public_email pr_permissions merge_right sign_in sign_out switch_account set_team request_access approve_request decline_request vault_local vault_download vault_update work_file vault_changes vault_send vault_incoming vault_trash vault_left vault_fetch").split(" "));
  async function invoke(cmd, args) {
    await wait(150);
    if (os === "android" && !PHONE.has(cmd)) { console.error("preview: the phone build has no command", cmd); throw `Command ${cmd} not found`; }
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
