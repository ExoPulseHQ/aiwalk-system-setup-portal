// Invented answers for every command the UI invokes, shaped like the Rust returns (desktop/src/*.rs, core/src/lib.rs).
// Nothing here comes from a real organisation: names, logins, repos, hosts and addresses are made up.
// window.__fixtures(scenario, os) -> { command: value | (args) => value | Error }. An Error becomes the app's Err(String).
window.__fixtures = (scenario, os) => {
  const ORG = "ExampleLab", BOOK = `${ORG}/lab-book`, COMPANY = "amy-chen/ExampleCorp";
  const owner = scenario === "owner" || scenario === "terms", signedOut = scenario === "signedout";
  const ME = owner ? "amy-chen" : "cy-wu";
  const none = null, ok = text => () => text;
  const day = n => new Date(Date.now() - n * 864e5).toISOString().slice(0, 10);

  // ---- the organisation: repos, teams, people
  const CODE = ["LabCAP", "lab-train", "lab-sim"];
  const REPOS = ["lab-book", "lab-papers", "lab-notes", "lab-mgmt", "lab-requests", "lab-app", "lab-l1", "lab-l2", "lab-l3", "lab-l4", "lab-l6", ...CODE];
  const grants = (more = {}) => ({ ...Object.fromEntries(REPOS.map(r => [r, 0])), "lab-book": 1, "lab-papers": 1, "lab-notes": 1, "lab-requests": 1, "lab-app": 1, ...more });
  const PEOPLE = {
    "amy-chen": { name: "Amy Chen", grants: none },
    "bo-lin": { name: "Bo Lin", grants: none },
    "cy-wu": { name: "Cy Wu", grants: grants({ "lab-l1": 2, LabCAP: 2, "lab-sim": 1 }) },
    "dee-park": { name: "Dee Park", grants: grants({ "lab-l2": 2, "lab-l3": 1, "lab-train": 2, "lab-sim": 2 }) },
    "eli-ross": { name: "eli-ross", grants: grants({ "lab-l4": 2, "lab-l6": 1 }) },   // no profile name: the login shows
    "fay-ng": { name: "Fay Ng", grants: grants({ "lab-l1": 1, "lab-l2": 1, "lab-l3": 1, LabCAP: 1 }) },
    "gus-oh": { name: "Gus Oh", grants: grants({ "lab-l6": 2, "lab-papers": 2 }) },
  };
  const TEAMS = [
    { slug: "core", repos: REPOS.map(r => [r, 4]), members: ["amy-chen", "bo-lin"] },
    { slug: "members", repos: [["lab-book", 1], ["lab-papers", 1], ["lab-notes", 1], ["lab-requests", 1], ["lab-app", 1]], members: Object.keys(PEOPLE) },
    { slug: "lab-l1", repos: [["lab-l1", 2], ["LabCAP", 2]], members: ["cy-wu"] },
    { slug: "lab-l2", repos: [["lab-l2", 2], ["lab-sim", 2]], members: ["dee-park"] },
    { slug: "lab-l3", repos: [["lab-l3", 2], ["lab-train", 2]], members: ["dee-park"] },
    { slug: "lab-l4", repos: [["lab-l4", 2]], members: ["eli-ross"] },
    { slug: "lab-l6", repos: [["lab-l6", 2]], members: ["gus-oh"] },
    { slug: "machine-dragon", repos: [], members: ["fay-ng"] },
    { slug: "merge-LabCAP", repos: [["LabCAP", 3]], members: ["cy-wu"] },
  ];
  const leaf = (title, repo, detail) => ({ title, repo, detail, children: [] });
  const TREE = {
    title: "main", repo: "lab-book", detail: "Primary logs, guides and templates", children: [
      { title: "Layers", repo: null, detail: "Detail repos, one per domain", children: [
        leaf("L1 Sensing", "lab-l1", "L1_Sensing/"), leaf("L2 Simulation", "lab-l2", "L2_Simulation/"), leaf("L3 Control", "lab-l3", "L3_Control/"),
        leaf("L4 Hardware", "lab-l4", "L4_Hardware/"), leaf("L6 Clinical", "lab-l6", "L6_Clinical/")] },
      { title: "Team shared", repo: null, detail: "Everyone on the team", children: [leaf("Notes", "lab-notes", "Notes/"), leaf("Papers", "lab-papers", "Papers/")] },
      { title: "Founders", repo: null, detail: "Core only", children: [leaf("Mgmt", "lab-mgmt", "Project_Management/")] },
    ] };
  const MACHINES = [
    { host: "dragon", repo: "LabCAP", account: "lab", ready: true, note: "Training and web server", sometimes: false, via: none, personal: false, tunnel: "ssh-dragon.example-lab.org", cert: true, teams: ["lab-l1"] },
    { host: "dragon", repo: "lab-train", account: "lab", ready: true, note: "Training and web server", sometimes: false, via: none, personal: false, tunnel: "ssh-dragon.example-lab.org", cert: true, teams: ["lab-l3"] },
    { host: "horse", repo: "lab-sim", account: "lab", ready: true, note: "Forward simulation", sometimes: false, via: none, personal: false, tunnel: "ssh-horse.example-lab.org", cert: true, teams: ["lab-l2"] },
    { host: "horse", repo: "lab-train", account: "lab", ready: true, note: "Forward simulation", sometimes: false, via: none, personal: false, tunnel: "ssh-horse.example-lab.org", cert: true, teams: ["lab-l3"] },
    { host: "tiger", repo: "LabCAP", account: "lab", ready: true, note: "Motion-capture service", sometimes: true, via: none, personal: false, tunnel: "ssh-tiger.example-lab.org", cert: false, teams: ["lab-l1"] },
  ];
  const REQUESTS = [
    { number: 41, author: "dee-park", repo: "lab-l4", level: "read", body: "repo: lab-l4\nlevel: read\n\nI need the exo drawings for the controller tuning.\n\n<!-- sent by aIwalk System Setup -->" },
    { number: 43, author: "fay-ng", repo: "lab-train", level: "write", body: "repo: lab-train\nlevel: write\n\nTraining runs for the gait study.\n\n<!-- sent by aIwalk System Setup -->" },
  ];

  // what the signed-in account sees: an owner everything, a member only themselves and their own teams
  const org = {
    org: ORG, tree: TREE,
    people: owner ? PEOPLE : { [ME]: PEOPLE[ME] },
    teams: owner ? TEAMS : TEAMS.filter(t => t.members.includes(ME)).map(t => ({ ...t, members: [ME] })),
    machines: MACHINES,
    requests: owner ? REQUESTS : [{ number: 44, author: ME, repo: "lab-l2", level: "read", body: "repo: lab-l2\nlevel: read\n\nCurious about the simulation notes.\n\n<!-- sent by aIwalk System Setup -->" }],
  };
  const teamAccess = signedOut
    ? { user: none, name: none, accounts: [], error: "You are not logged in to any GitHub host", vaults: [], scopes: [] }
    : { user: ME, name: owner ? "Amy Chen" : none,
        accounts: [{ login: ME, active: true, method: "account", protocol: "https" }, ...(owner ? [{ login: "amy-chen-lab", active: false, method: "temporary", protocol: "https" }] : [])],
        error: none,
        vaults: [
          { name: "Lab docs", repo: BOOK, about: "The team's technical documents, papers and progress notes", access: org, permission: owner ? 4 : 1 },
          { name: "Company", repo: COMPANY, about: "Company records, founders only", access: none, permission: owner ? 4 : 0 },
        ],
        scopes: owner ? ["repo", "read:org", "admin:org"] : ["repo", "read:org"] };

  // ---- terms
  const TERMS = "<!-- terms version: 2 -->\n# Terms of use\nThese terms apply to everyone who uses the lab's repositories and machines.\n1. Keep your sign-in to yourself.\n2. Do not copy participant data off the lab machines.\n3. Tell an owner when you leave the team.\n4. Machines are shared: close desktops you no longer use.\n5. Papers and notes stay inside the team until an owner says otherwise.\n6. The owners may change these terms and will ask you to read them again.\n7. Questions go to the owners.\n\n# 使用條款\n本條款適用於所有使用實驗室儲存庫與機器的人。\n1. 請勿與他人共用你的登入。\n2. 請勿將受試者資料複製出實驗室機器。\n3. 離開團隊時請告知擁有者。\n4. 機器為共用資源，用完的桌面請關閉。\n5. 論文與筆記在擁有者同意前僅限團隊內部。\n6. 擁有者可能修改條款，屆時會請你重新閱讀。\n7. 有問題請洽擁有者。\n";
  const accepted = scenario === "terms" ? none : { version: 2, date: day(30) };
  const termsState = signedOut ? none : { org: ORG, version: 2, text: TERMS, accepted };

  // ---- machines
  const gpu = (name, util, used, total, temp) => ({ name, util, mem_used_mb: used, mem_total_mb: total, temp_c: temp });
  const base = { time: Math.floor(Date.now() / 1000), os: "Ubuntu 22.04.4 LTS", users: 2 };
  const STATUS = {
    dragon: { ...base, hostname: "dragon", cpu: "Intel(R) Core(TM) i9-13900K", threads: 32, mem_gb: 125.5, mem_used_gb: 48.2, cpu_pct: 37.5, load: [11.2, 9.8, 8.4],
      gpus: [gpu("NVIDIA GeForce RTX 4090", 78, 17200, 24564, 66), gpu("NVIDIA GeForce RTX 4090", 12, 3100, 24564, 48)],
      disk_gb: 3840, disk_free_gb: 1210, uptime_h: 912.4, tools: { "exo-status.py": 2, exo: 1, "exo-desktop": 1 }, host_tools: "Host tools: current",
      desktops: [{ user: "lab", display: ":2", port: 5902, socket: none, geometry: "1920x1080" }, { user: "lab", display: ":6", port: none, socket: "/home/lab/.vnc/desk-6.sock", geometry: "2560x1440" }], cluster: none },
    horse: { ...base, hostname: "horse", cpu: "AMD Ryzen 9 7950X 16-Core Processor", threads: 32, mem_gb: 62.7, mem_used_gb: 55.9, cpu_pct: 91.3, load: [28.4, 26.1, 20.7],
      gpus: [gpu("NVIDIA RTX 6000 Ada Generation", 96, 41200, 49140, 79)],
      disk_gb: 1920, disk_free_gb: 14, uptime_h: 240.8, tools: { "exo-status.py": 1, exo: 1, "exo-desktop": none }, host_tools: "Host tools: exo-status.py older, exo-desktop missing",
      desktops: [], cluster: none },
    tiger: { ...base, hostname: "tiger", cpu: "Intel(R) Core(TM) i7-12700", threads: 20, mem_gb: 31.1, mem_used_gb: 9.4, cpu_pct: 8.1, load: [0.9, 1.1, 0.8],
      gpus: [], disk_gb: 960, disk_free_gb: 402, uptime_h: 48.0, tools: none, host_tools: "Host tools: older than this app (the status page does not report versions yet)",
      desktops: [], cluster: none },
  };
  // the status page answers only for machines this account may connect to
  const reach = h => owner || ["dragon", "tiger"].includes(h);

  // ---- people and Cloudflare (owners)
  const members = Object.entries(PEOPLE).map(([login, p]) => ({ login, name: p.name, owner: p.grants === none }));
  const people = { members, invites: [{ id: 7001, login: "hal-fox", owner: false, created: day(3) }],
    interns: [{ login: "ivy-tam", repos: [{ repo: "lab-papers", level: "read", invite: none }, { repo: "lab-notes", level: "read", invite: none }, { repo: "lab-l1", level: "read", invite: 8102 }] }] };
  const prRows = owner
    ? CODE.map(r => ({ repo: r, mine: 4, listed: true, merge: ["amy-chen", "bo-lin", ...(r === "LabCAP" ? ["cy-wu"] : [])], write: Object.keys(PEOPLE).filter(l => PEOPLE[l].grants && PEOPLE[l].grants[r] === 2), extra: r === "LabCAP" ? ["cy-wu"] : [] }))
    : [{ repo: "LabCAP", mine: 3, listed: true, merge: ["amy-chen", "bo-lin", "cy-wu"], write: ["fay-ng"], extra: ["cy-wu"] }, { repo: "lab-sim", mine: 1, listed: false, merge: [], write: [], extra: [] }];

  // ---- phone and Windows VM (Linux pages)
  const phone = { serial: "PREVIEW0001", wireless: false, model: "Pixel 8", ip: "", root: false, desk_on: false, desk_app_on: false, phonedesk: true, phonedesk_outdated: false, shizuku: true };
  const phones = { phones: [phone, { ...phone, serial: "192.0.2.15:5555", wireless: true, model: "Galaxy Tab S9", ip: "192.0.2.15", desk_on: true, root: true }], waiting: false };
  const vm = { name: "windows", running: true, compose: "/home/demo/vm/compose.yml", ram: "8G", cpus: "4", disk: "128G", user: "Docker", card_reader: false, rdp_port: "3389", storage: "/home/demo/vm/storage", share: none };

  return {
    // app shell
    platform: os, tools: { git: os === "windows" ? "C:\\tools\\git\\cmd\\git.exe" : "/usr/bin/git", os },
    update_state: { current: "1.4.0", latest: "1.5.0", newer: true, can: true }, update_install: ok("Downloaded 1.5.0. The installer is starting."),
    install_git: none, python_state: { version: "3.12.3", command: "python3" }, python_install: ok("Python installed"),
    claude_state: { path: "/usr/local/bin/claude", version: "2.1.0", signed_in: true, method: "claude.ai", plan: "max" },
    claude_install: ok("Claude Code installed"), claude_login: ok("Signed in to Claude"),
    // GitHub sign-in and access
    team_access: teamAccess, sign_in: true, sign_out: none, switch_account: none, set_team: none, request_access: none,
    approve_request: none, decline_request: none,
    terms_state: termsState, terms_accept: none,
    terms_everyone: { "amy-chen": { version: 2, date: day(30) }, "bo-lin": { version: 2, date: day(29) }, "cy-wu": { version: 2, date: day(12) }, "dee-park": { version: 1, date: day(80) }, "gus-oh": { version: 2, date: day(5) } },
    // owner tools
    org_people: people, invite: ok("Invited"), invite_intern: ok("Invited"), remove_intern: ok("Removed"), cancel_invite: ok("Invitation cancelled"),
    set_role: ok("Role changed"), remove_member: ok("Removed"), set_access: ok("Access changed"), machine_extra: ok("Done"),
    pr_permissions: prRows, merge_right: ok("Done"),
    cf_state: { connected: true, expires: "2027-03-01", shared: true, has_key: true, left: [] }, cf_key: "ABCD-EFGH-IJKL-MNOP-QRST",
    cf_connect: ok("Connected"), cf_share: ok("ABCD-EFGH-IJKL-MNOP-QRST"), cf_join: ok("Connected"), cf_renew: ok("Renewed"), cf_forget: none,
    // vault copies
    vault_local: { copies: { [BOOK]: "/home/demo/Documents/aIwalk/lab-book" }, obsidian: true }, default_folder: a => `/home/demo/Documents/aIwalk/${(a.repo || "").split("/").pop()}`,
    pick_folder: none, vault_link: ok("Linked"), vault_download: ok("Downloaded"), vault_update: ok("Up to date"), vault_open: none, obsidian_install: ok("Obsidian installed"),
    // machines
    reachable: a => Object.fromEntries((a.machines || []).map(([h]) => [h, "up"])),
    machine_status: a => Object.fromEntries((a.tunnels || []).filter(([h]) => reach(h) && STATUS[h]).map(([h]) => [h, STATUS[h]])),
    forwards: { "dragon:2": 5902 }, ssh_status: "current", ssh_setup: ok("Connections set up"), access_login: ok("Signed in to 3 machines"),
    lab_identity: { email: `${ME}@example.com`, expires: Math.floor(Date.now() / 1000) + 20 * 3600, matches: true, missing: 0 }, lab_sign_out: none,
    open_forward: 5911, close_forward: none, open_viewer: none, desktop: ":3", update_host_tools: ok("Host tools updated"),
    // Linux pages
    phones, phone_action: ok("Done"), vm_state: { vm, disk_used_gb: 61.4, host_gb: 32, host_cpus: 16, shortcuts: [["open", "Open Windows", "Start the Windows VM and open its desktop", true], ["stop", "Shut down Windows", "Shut down the Windows VM and free its memory", false]] },
    vm_action: ok("Done"),
  };
};
