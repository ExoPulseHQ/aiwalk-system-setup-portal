// Module Android Phone: one section per connected phone, its actions under it.

let phonesBusy = false, phonesSig = null;

// The phone, drawn in its current state: upright = phone mode, sideways with a taskbar = desktop mode;
// a cable = USB, Wi-Fi arcs = wireless. The phone in the connected colour, the link in the muted one.
function silhouette(p) {
  const c = "var(--purple)";
  const [bw, bh] = p.desk_on ? [78, 38] : [40, 72];
  const x = 48 - bw / 2, y = 8 + (72 - bh) / 2;
  const desk = p.desk_on ? `<rect x="${x + 3}" y="${y + bh - 9}" width="${bw - 6}" height="6" style="fill:${c}"/>
    <rect x="${x + 10}" y="${y + 7}" width="26" height="16" fill="none" style="stroke:${c}" stroke-width="2"/>
    <rect x="${x + 30}" y="${y + 12}" width="30" height="17" style="fill:${c}"/>` : "";
  const link = p.wireless
    ? [7, 13, 19].map(r => `<path d="M ${48 - r * .707} ${106 - r * .707} A ${r} ${r} 0 0 1 ${48 + r * .707} ${106 - r * .707}" fill="none" stroke="currentColor" stroke-opacity=".7" stroke-width="2"/>`).join("") + `<circle cx="48" cy="106" r="2" fill="currentColor"/>`
    : `<line x1="48" y1="${y + bh + 2}" x2="48" y2="110" stroke="currentColor" stroke-opacity=".7" stroke-width="2"/><rect x="44" y="${y + bh + 2}" width="8" height="6" fill="currentColor" fill-opacity=".7"/>`;
  const box = el("div", "phone-art");
  box.innerHTML = `<svg viewBox="0 0 96 112" width="72" height="84" role="img" aria-label="${p.model}, ${p.desk_on ? "desktop" : "phone"} mode, ${p.wireless ? "Wi-Fi" : "USB"}">
    <rect x="${x}" y="${y}" width="${bw}" height="${bh}" rx="7" fill-opacity=".18" stroke-width="2" style="fill:${c};stroke:${c}"/>${desk}${link}</svg>`;
  return box;
}

// While an action runs, a line under the title says what the phone is doing.
const PHONE_STEP = { keyboard: "Opening the keyboard window", mirror: "Opening the phone screen", install: "Installing on the phone",
  desk: "Switching the desktop", wireless: "Switching the phone to Wi-Fi", disconnect: "Disconnecting Wi-Fi", reconnect: "Looking for phones on this Wi-Fi" };
let phoneActivity = null;
listen("phone-step", e => phoneActivity && phoneActivity.set(null, e.payload));

async function phoneAction(action, phone, extra = {}, trigger) {
  phonesBusy = true;
  phoneActivity = stage(PHONE_STEP[action] || "Working");
  document.getElementById("android").querySelector(".row").after(phoneActivity);
  if (trigger instanceof HTMLButtonElement) { trigger.disabled = true; trigger.classList.add("working"); trigger.prepend(el("span", "spinner")); }
  else busyRow(trigger);
  try { toast(await invoke("phone_action", { action, phone, ...extra })); }
  finally { phoneActivity = null; phonesBusy = false; phonesSig = null; refreshPhones(true); }
}

function deskRow(p) {
  if (!p.root && !p.phonedesk) {
    const b = el("button", "small", "Install");
    b.onclick = () => phoneAction("install", p, {}, b);
    return kind(item("Desktop", "Not rooted: install the Desktop Mode app first (keeps all data)", null, b), "monitor");
  }
  let sub = p.root ? "Built-in setting, applies after a restart"
    : !p.shizuku ? "Shizuku is not running; it will be started first" : "Shown on the phone screen by the Desktop Mode app";
  const right = [];
  if (!p.root && p.phonedesk_outdated) {
    sub = "The Desktop Mode app on this phone is outdated, update it first";
    const b = el("button", "small", "Update");
    b.onclick = () => phoneAction("install", p, {}, b);
    right.push(b);
  }
  right.push(switchBox(p.desk_on, async (on, box) => {
    if (!p.root) return phoneAction("desk", p, { on });
    const choice = await ask(`${on ? "Turn on" : "Turn off"} the desktop on ${p.model}?`,
      "This applies the whole setup (display size, font size, landscape). It takes effect after a restart.",
      [["cancel", "Cancel"], ["later", "Restart later"], ["reboot", "Apply and restart", true]]);
    if (!choice || choice === "cancel") { box.checked = !on; return; }
    phoneAction("desk", p, { on, reboot: choice === "reboot" });
  }));
  return kind(item("Desktop", sub, null, ...right), "monitor");
}

function phoneSection(p) {
  const sec = el("section", "card");
  const head = el("div", "row");
  const text = el("div");
  // connected is the healthy state: the state chip, in purple
  text.append(el("h2", null, p.model), stateText(el("span", "perm s-ok"), "check", p.wireless ? `Connected over Wi-Fi, ${p.ip}` : "Connected over USB"),
    el("div", "dim", (p.desk_on ? "Desktop is on" + (p.root ? "" : " (Desktop Mode app)") : "Phone mode") + (p.root ? ", rooted" : "")));
  head.append(silhouette(p), text);
  const list = el("div", "list");
  list.append(
    kind(item("Type with this keyboard", "Use this computer's keyboard and touchpad on the phone", e => phoneAction("keyboard", p, {}, e.currentTarget)), "keyboard"),
    kind(item("Show the phone screen here", "Mirror it and control it with the mouse", e => phoneAction("mirror", p, {}, e.currentTarget)), "screen-share"),
    deskRow(p),
    p.wireless ? kind(item("Disconnect Wi-Fi", null, e => phoneAction("disconnect", p, {}, e.currentTarget)), "wifi-off")
               : kind(item("Switch to Wi-Fi", "Then you can unplug the USB cable", e => phoneAction("wireless", p, {}, e.currentTarget)), "wifi"));
  sec.append(head, list);
  return sec;
}

async function refreshPhones(force) {
  if (current !== "android" || (phonesBusy && !force)) return;
  const { phones, waiting } = await invoke("phones");
  const sig = JSON.stringify([phones, waiting]);
  if (sig === phonesSig) return;   // redraw only when something changed, so open switches do not jump
  phonesSig = sig;
  const page = document.getElementById("android");
  const top = el("div", "row");
  const again = iconButton("refresh-cw", "Look again", "small ghost", true);
  again.onclick = () => { phonesSig = null; refreshPhones(true); };
  top.append(el("h1", null, "Android phone"), el("span", "detail", ""), again);
  page.replaceChildren(top);
  if (waiting) page.append(el("div", "banner", "A phone is connected but has not allowed debugging. Unlock it and tap Allow on the USB debugging prompt."));
  if (!phones.length) {
    const empty = el("div", "empty");
    empty.append(el("h2", null, "No phone found"),
      el("p", null, "On the phone, turn on USB debugging (Settings › System › Developer options) and connect it with a USB cable. For Wi-Fi, turn on Wireless debugging there instead and keep the phone on the same Wi-Fi as this computer."));
    const b = el("button", null, "Reconnect over Wi-Fi");
    b.onclick = () => phoneAction("reconnect", null, {}, b);
    empty.append(b);
    page.append(empty);
  }
  phones.forEach(p => page.append(phoneSection(p)));
  // the page opens with how many phones are connected, and a phone still waiting for you
  const how = p => `${p.model} over ${p.wireless ? "Wi-Fi" : "USB"}`;
  const asking = "A phone is waiting: unlock it and tap Allow on the USB debugging prompt.";
  const askItem = { icon: "circle-alert", text: "Allow debugging", tone: "warn", tip: asking };
  const lead = phones.length === 1 ? `${phones[0].model} is connected.` : `${phones.length} phones are connected.`;
  const items = [{ icon: "smartphone", num: phones.length, tip: `Connected: ${phones.map(how).join(", ")}` }];
  setOpening(page, !phones.length ? (waiting ? ["warn", "A phone is waiting for you to allow debugging.", [], [askItem]] : null)
    : waiting ? ["warn", lead, [phones.map(how).join(", ") + ".", " " + asking], [...items, askItem]]
    : ["ok", lead, [phones.map(how).join(", ") + "."], items]);
}
setInterval(() => refreshPhones(false), 4000);
