// Module Android Phone: one section per connected phone, its actions under it.

const DESK_BLUE = "#1A73E8", PHONE_GREEN = "#188038";
let phonesBusy = false, phonesSig = null;

// The phone, drawn in its current state: upright = phone mode, sideways with a taskbar = desktop mode;
// a cable = USB, Wi-Fi arcs = wireless.
function silhouette(p) {
  const c = p.desk_on ? DESK_BLUE : PHONE_GREEN;
  const [bw, bh] = p.desk_on ? [78, 38] : [40, 72];
  const x = 48 - bw / 2, y = 8 + (72 - bh) / 2;
  const desk = p.desk_on ? `<rect x="${x + 3}" y="${y + bh - 9}" width="${bw - 6}" height="6" fill="${c}"/>
    <rect x="${x + 10}" y="${y + 7}" width="26" height="16" fill="none" stroke="${c}" stroke-width="2"/>
    <rect x="${x + 30}" y="${y + 12}" width="30" height="17" fill="${c}"/>` : "";
  const link = p.wireless
    ? [7, 13, 19].map(r => `<path d="M ${48 - r * .707} ${106 - r * .707} A ${r} ${r} 0 0 1 ${48 + r * .707} ${106 - r * .707}" fill="none" stroke="currentColor" stroke-opacity=".7" stroke-width="2"/>`).join("") + `<circle cx="48" cy="106" r="2" fill="currentColor"/>`
    : `<line x1="48" y1="${y + bh + 2}" x2="48" y2="110" stroke="currentColor" stroke-opacity=".7" stroke-width="2"/><rect x="44" y="${y + bh + 2}" width="8" height="6" fill="currentColor" fill-opacity=".7"/>`;
  const box = el("div");
  box.innerHTML = `<svg width="96" height="112" role="img" aria-label="${p.model}, ${p.desk_on ? "desktop" : "phone"} mode, ${p.wireless ? "Wi-Fi" : "USB"}">
    <rect x="${x}" y="${y}" width="${bw}" height="${bh}" rx="7" fill="${c}" fill-opacity=".18" stroke="${c}" stroke-width="2"/>${desk}${link}</svg>`;
  return box;
}

async function phoneAction(action, phone, extra = {}) {
  phonesBusy = true;
  try { toast(await invoke("phone_action", { action, phone, ...extra })); }
  finally { phonesBusy = false; phonesSig = null; refreshPhones(true); }
}

function deskRow(p) {
  if (!p.root && !p.phonedesk) {
    const b = el("button", "small", "Install");
    b.onclick = () => { b.disabled = true; phoneAction("install", p); };
    return item("Desktop", "Not rooted: install the Desktop Mode app first (keeps all data)", null, b);
  }
  let sub = p.root ? "Built-in setting, applies after a restart"
    : !p.shizuku ? "Shizuku is not running; it will be started first" : "Shown on the phone screen by the Desktop Mode app";
  const right = [];
  if (!p.root && p.phonedesk_outdated) {
    sub = "The Desktop Mode app on this phone is outdated, update it first";
    const b = el("button", "small", "Update");
    b.onclick = () => { b.disabled = true; phoneAction("install", p); };
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
  return item("Desktop", sub, null, ...right);
}

function phoneSection(p) {
  const sec = el("section", "card");
  const head = el("div", "row");
  const text = el("div");
  text.append(el("h2", null, p.model), el("div", "dim", p.wireless ? `Connected over Wi-Fi, ${p.ip}` : "Connected over USB"),
    el("div", "dim", (p.desk_on ? "Desktop is on" + (p.root ? "" : " (Desktop Mode app)") : "Phone mode") + (p.root ? ", rooted" : "")));
  head.append(silhouette(p), text);
  const list = el("div", "list");
  list.append(
    item("Type with this keyboard", "Use this computer's keyboard and touchpad on the phone", () => phoneAction("keyboard", p)),
    item("Show the phone screen here", "Mirror it and control it with the mouse", () => phoneAction("mirror", p)),
    deskRow(p),
    p.wireless ? item("Disconnect Wi-Fi", null, () => phoneAction("disconnect", p))
               : item("Switch to Wi-Fi", "Then you can unplug the USB cable", () => phoneAction("wireless", p)));
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
  const again = el("button", "small ghost", "Look again");
  again.onclick = () => { phonesSig = null; refreshPhones(true); };
  top.append(el("h1", null, "Android phone"), el("span", "detail", ""), again);
  page.replaceChildren(top);
  if (waiting) page.append(el("div", "banner", "A phone is connected but has not allowed debugging. Unlock it and tap Allow on the USB debugging prompt."));
  if (!phones.length) {
    const empty = el("div", "empty");
    empty.append(el("h2", null, "No phone found"),
      el("p", null, "On the phone, turn on USB debugging (Settings › System › Developer options) and connect it with a USB cable. For Wi-Fi, turn on Wireless debugging there instead and keep the phone on the same Wi-Fi as this computer."));
    const b = el("button", null, "Reconnect over Wi-Fi");
    b.onclick = () => { b.disabled = true; phoneAction("reconnect", null); };
    empty.append(b);
    page.append(empty);
  }
  phones.forEach(p => page.append(phoneSection(p)));
}
setInterval(() => refreshPhones(false), 4000);
