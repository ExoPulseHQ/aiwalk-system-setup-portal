// Module Windows VM: the dockur/windows VM for Office and Windows-only tools.

let vmBusy = false, vmSig = null, cleaning = null;   // cleaning = { text, since } while Clean up runs
// rough share of the total time each cleanup step starts at; step 8 (DISM) is the long one
const CLEAN_STEPS = { 1: .02, 2: .04, 3: .06, 4: .08, 5: .10, 6: .13, 7: .16, 8: .20, 9: .85 };

async function vmAction(action, args = {}) {
  vmBusy = true;
  try { const msg = await invoke("vm_action", { action, ...args }); toast(msg); return msg; }
  catch (e) {
    if (e === "need-password" || e === "wrong-password") {
      const pw = await askPassword(e === "wrong-password" ? "Windows rejected this password. Enter the password of the Windows account." : "");
      if (pw) return vmAction(action, { ...args, password: pw });
    } else if (e === "needs-setup") {
      setUpHelper();
    } else toast(e);
  } finally { vmBusy = false; vmSig = null; refreshVm(); }
}

async function askPassword(reason) {
  const user = (lastVm && lastVm.user) || "Windows";
  const input = el("input"); input.type = "password"; input.style.width = "100%";
  const choice = await ask(`Password for ${user}`, (reason ? reason + "\n\n" : "") +
    "Saved in this computer's keyring, used to sign in to Windows.", [["cancel", "Cancel"], ["save", "Save", true]], input);
  return choice === "save" && input.value ? input.value : null;
}

// Shut down first when Windows runs, since these settings apply while the VM is off.
async function whileOff(running, what) {
  if (!running) return true;
  return await ask("Shut down Windows first?", `${what} apply while the VM is off. Windows will shut down, then the change is applied.`,
    [["cancel", "Cancel"], ["apply", "Shut down and apply", true]]) === "apply";
}

async function setUpHelper() {
  const command = "Start-Process powershell -Verb RunAs -ArgumentList '-ExecutionPolicy Bypass -File \\\\host.lan\\Data\\.aiwalk\\install-agent.ps1'";
  const code = el("pre", null, command); code.style.whiteSpace = "pre-wrap";
  const go = await ask("One-time setup", "Windows needs the small aIwalk helper to clean up by itself.\n\n" +
    "1. Windows opens and the setup command below is copied.\n2. In Windows, right-click Start, choose Terminal, press Ctrl+V and Enter.\n" +
    "3. Answer Yes when Windows asks for permission.\n\nThe cleanup then starts on its own. Next time, Clean up needs no steps.",
    [["cancel", "Cancel"], ["go", "Open Windows and copy command", true]], code);
  if (go !== "go") return;
  try { await navigator.clipboard.writeText(command); } catch { toast("Copy the command shown in the dialog"); }
  cleaning = { text: "Waiting for the one-time setup in Windows", since: Date.now() };
  vmAction("open");
  await vmAction("clean-after-setup");
  cleaning = null;
}

listen("vm-clean", e => {
  if (!cleaning || cleaning.text !== e.payload) cleaning = { text: e.payload, since: Date.now() };
  showProgress();
});

function showProgress() {
  const bar = document.getElementById("clean-bar"), sub = document.getElementById("clean-sub");
  if (!bar || !cleaning) return;
  const step = parseInt(cleaning.text) || 0;
  const start = CLEAN_STEPS[step] || 0, end = CLEAN_STEPS[step + 1] || 1;
  // creep towards the next step so the bar keeps moving during long steps
  const f = start + (end - start) * (1 - Math.exp(-(Date.now() - cleaning.since) / 240000));
  bar.hidden = false; bar.value = f;
  sub.textContent = `${cleaning.text}… (${Math.round(f * 100)}%)`;
}
setInterval(showProgress, 2000);

let lastVm = null;
async function refreshVm() {
  if (current !== "vm" || vmBusy) return;
  const s = await invoke("vm_state");
  const sig = JSON.stringify(s);
  if (sig === vmSig) return;
  vmSig = sig; lastVm = s.vm;
  const page = document.getElementById("vm"), vm = s.vm;
  page.replaceChildren(el("h1", null, "Windows VM"));
  if (!vm) {
    const e = el("div", "empty");
    e.append(el("h2", null, "No Windows VM on this computer yet"), el("p", null, "Creating one from here is the next step of aIwalk System Setup."));
    return page.append(e);
  }
  page.append(el("h2", null, vm.running ? "Windows is running" : "Windows is off"),
    el("div", "dim", `${vm.ram} memory, ${vm.cpus} CPU cores, ${vm.disk} disk`));

  const actions = el("div", "list");
  actions.append(item("Open Windows", "Opens the Windows desktop in a window" + (vm.running ? "" : ", starting it first (1–2 minutes)"),
    () => { toast(vm.running ? "Opening Windows…" : "Starting Windows, this takes 1–2 minutes…"); vmAction("open"); }));
  if (vm.running) actions.append(item("Shut down Windows", "Frees its memory; takes up to 2 minutes", () => vmAction("stop")));
  page.append(actions);

  page.append(el("h3", null, "Settings"));
  const settings = el("div", "list");
  const pw = el("button", "small ghost", "Change password");
  pw.onclick = async () => { const p = await askPassword(""); if (p) vmAction("password", { password: p }); };
  settings.append(
    item("Card reader", "Pass the USB card reader (health-insurance / citizen certificate) to Windows. Windows will not start while it is on and the reader is unplugged.", null,
      switchBox(vm.card_reader, async (on, box) => {
        if (!await whileOff(vm.running, "Card reader changes")) { box.checked = !on; return; }
        vmAction("card", { on });
      })),
    item("Windows account", vm.user, null, pw),
    item("VM folder", vm.compose.replace(/\/[^/]*$/, ""), null));
  page.append(settings);

  page.append(el("h3", null, "Shortcuts"), el("div", "sub", "Added to the desktop and the app menu"));
  const shortcuts = el("div", "list");
  s.shortcuts.forEach(([key, name, comment, on]) =>
    shortcuts.append(item(name, comment, null, switchBox(on, v => vmAction("shortcut", { key, on: v })))));
  page.append(shortcuts);

  page.append(el("h3", null, "Resources"), el("div", "sub", "Applied while Windows is off"));
  const res = el("div", "list"), num = t => parseInt(String(t).replace(/\D/g, "")) || 0;
  const now = { RAM_SIZE: num(vm.ram), CPU_CORES: num(vm.cpus), DISK_SIZE: num(vm.disk) };
  const apply = el("button", "small", "Apply"); apply.disabled = true;
  const inputs = {};
  // leave the host at least 4 GB; the disk can only grow (Windows cannot shrink it)
  [["RAM_SIZE", "Memory (GB)", `This computer has ${s.host_gb} GB`, 2, Math.max(s.host_gb - 4, 2)],
   ["CPU_CORES", "CPU cores", `This computer has ${s.host_cpus}`, 1, s.host_cpus],
   ["DISK_SIZE", "Disk size (GB)", "Can only grow", now.DISK_SIZE || 32, 1024]].forEach(([k, title, sub, lo, hi]) => {
    const n = el("input"); n.type = "number"; n.min = lo; n.max = hi; n.value = now[k] || lo;
    n.oninput = () => apply.disabled = !Object.entries(inputs).some(([key, i]) => +i.value !== now[key]);
    inputs[k] = n;
    res.append(item(title, sub, null, n));
  });
  apply.onclick = async () => {
    const values = {};
    for (const [k, i] of Object.entries(inputs)) {
      const v = Math.min(Math.max(+i.value, +i.min), +i.max);
      if (v !== now[k]) values[k] = k === "CPU_CORES" ? String(v) : `${v}G`;
    }
    if (await whileOff(vm.running, "Memory, CPU and disk changes")) vmAction("resources", { values });
  };
  res.append(item("Apply changes", null, null, apply));

  const bar = el("progress"); bar.id = "clean-bar"; bar.max = 1; bar.hidden = !cleaning;
  const clean = el("button", "small", "Clean up"); clean.hidden = !!cleaning;
  clean.onclick = async () => {
    clean.hidden = true;
    cleaning = { text: vm.running ? "Contacting Windows" : "Starting Windows", since: Date.now() };
    await vmAction("clean");
    cleaning = null;
  };
  const cleanRow = item("Clean up space", (s.disk_used_gb ? `The disk takes ${Math.round(s.disk_used_gb)} GB on this computer. ` : "") +
    "Removes Office and PowerPoint caches, crash dumps, temp files and the recycle bin inside Windows, then gives the space back.", null, bar, clean);
  cleanRow.querySelector(".sub").id = "clean-sub";
  res.append(cleanRow);
  page.append(res);
  showProgress();
}
setInterval(refreshVm, 5000);
