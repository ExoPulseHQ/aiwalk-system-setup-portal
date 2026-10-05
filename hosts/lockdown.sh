#!/bin/bash
# Lock a lab machine down so people reach it only through Cloudflare (the portal's GitHub sign-in): VNC desktops and
# other services listen on 127.0.0.1 and are forwarded over SSH through the tunnel. Registered lab machines may still
# SSH to each other directly (the admins' fallback when Cloudflare is down).
#
#   sudo bash lockdown.sh [account]  prints the plan, asks, then applies and checks; account owns the desktops (ntk)
#
# Registered machines are the lab hosts in System/vault_rules.json `machines`; their addresses come from
# Secrets/Compute_Repo_Access_Registry.md §1b. This machine's own address is left out automatically.
set -euo pipefail
MACHINES="120.126.83.20 120.126.83.112 120.126.83.1 120.126.83.67 120.126.83.76 120.126.83.143 120.126.83.28"
VNC_USER=${1:-ntk}
VNC_CONFIG=/home/$VNC_USER/.vnc/config
VNC_SERVICE=gpu-free-vnc.service
VNC_LAUNCHER=/usr/local/bin/gpu-free-vnc   # starts the boot-time desktops; its own "-localhost no" beats ~/.vnc/config

[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
id "$VNC_USER" >/dev/null 2>&1 || { echo "There is no account called $VNC_USER here: sudo bash lockdown.sh <account>. Nothing was changed."; exit 1; }
# a machine without the boot-time VNC desktops (a laptop that shares its screen another way) has only the firewall to close
# desktops that are not running now are not started: a capture PC whose desktops are off stays that way
VNC_RUNNING=no; systemctl is-active --quiet "$VNC_SERVICE" 2>/dev/null && VNC_RUNNING=yes
HAS_VNC=no; { [ -f "/etc/systemd/system/$VNC_SERVICE" ] || [ -f "/lib/systemd/system/$VNC_SERVICE" ] || [ -f "$VNC_LAUNCHER" ]; } && HAS_VNC=yes
systemctl is-active --quiet cloudflared || { echo "cloudflared is not running. After this the tunnel is the way in, so nothing was changed."; exit 1; }
SELF=$(hostname -I)
PEERS=$(for ip in $MACHINES; do case " $SELF " in *" $ip "*) ;; *) echo -n "$ip ";; esac; done)

# every rule that opens a port to everyone: "[ n] 2222/tcp   ALLOW IN   Anywhere" and their v6 twins. A list of
# known ports left the unknown ones open (2222 on goat, 8080 on monkey); behind the tunnel nothing needs to be.
open_rules() {
  ufw status numbered | awk -F'[][]' '/ALLOW IN/ && /Anywhere/ && !/ from / { print $2 + 0 }'
}
open_names() {
  ufw status numbered | awk -F'[][]' '/ALLOW IN/ && /Anywhere/ && !/ from / { split($3, f, " "); print f[1] }' | sort -u | paste -sd' '
}

echo "== Firewall now"; ufw status verbose | sed -n '1,4p'; ufw status numbered | sed '1,4d'
echo
echo "== Plan"
echo " 1. The tunnel token in the cloudflared service file becomes readable by root only."
if [ $HAS_VNC = yes ] && [ $VNC_RUNNING = yes ]; then
echo " 2. The boot-time desktops :1-:5 are started by exo-desktop: Unix socket only, no TCP port, no VNC password"
echo "    (a systemd override for $VNC_SERVICE; the original files stay). They restart once, so anyone on them"
echo "    is disconnected; afterwards they open from the portal's Open desktop."
elif [ $HAS_VNC = yes ]; then
echo " 2. $VNC_SERVICE is not running, and stays stopped. It gets the override all the same, so whenever it is"
echo "    started its desktops are Unix sockets only: no TCP port, no VNC password. Nobody is disconnected."
else
echo " 2. This machine has no boot-time VNC desktops ($VNC_SERVICE): nothing to change there."
fi
echo " 3. SSH (22) is allowed only from: $PEERS"
if ufw status | grep -q '^Status: active'; then
echo "    Every rule that opens a port to everyone is removed: $(open_names)"
else
echo "    The firewall is off now, so its stored rules cannot be listed yet; it is switched on and every rule that"
echo "    opens a port to everyone is then removed (they are printed as they go)."
fi
echo "    New connections from anywhere else are refused; the Cloudflare tunnel is unaffected."
read -rp "Apply? [y/N] " answer
[ "$answer" = y ] || { echo "Nothing changed."; exit 0; }

chmod 600 /etc/systemd/system/cloudflared.service
systemctl daemon-reload

if [ $HAS_VNC = yes ]; then
mkdir -p "$(dirname "$VNC_CONFIG")" && chown "$VNC_USER": "$(dirname "$VNC_CONFIG")"
if grep -q '^localhost=' "$VNC_CONFIG"; then sed -i 's/^localhost=.*/localhost=yes/' "$VNC_CONFIG"; else echo localhost=yes >> "$VNC_CONFIG"; fi
# desktops come from exo-desktop (socket, no password); the old launcher stays for reference
mkdir -p "/etc/systemd/system/$VNC_SERVICE.d"
cat > "/etc/systemd/system/$VNC_SERVICE.d/exo-desktop.conf" <<CONF
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=
ExecStart=/bin/bash -c 'for i in 1 2 3 4 5; do /home/$VNC_USER/.local/bin/exo-desktop start \$i; done'
ExecStop=
ExecStop=/bin/bash -c 'for i in 1 2 3 4 5 6 7 8; do /home/$VNC_USER/.local/bin/exo-desktop stop \$i || true; done'
CONF
[ -x "/home/$VNC_USER/.local/bin/exo-desktop" ] || { echo "exo-desktop is missing in /home/$VNC_USER/.local/bin; desktops will not start"; }
if [ -f "$VNC_LAUNCHER" ] && grep -q -- '-localhost no' "$VNC_LAUNCHER"; then
  cp -p "$VNC_LAUNCHER" "$VNC_LAUNCHER.bak"
  sed -i 's/-localhost no/-localhost yes/g' "$VNC_LAUNCHER"
fi
fi

ufw default deny incoming >/dev/null
for ip in $PEERS; do ufw allow proto tcp from "$ip" to any port 22 comment 'registered lab machine' >/dev/null; done
# switched on first: an inactive firewall lists none of its stored rules, so the open ones could not be found, and
# switching it on afterwards brought them to life (monkey kept a 5901 opened long ago)
ufw --force enable >/dev/null
# --force answers ufw's own question; piping "yes" into it ends with SIGPIPE, which pipefail turned into an abort
echo "Removing rules open to everyone: $(open_names)"
for n in $(open_rules | sort -rn); do ufw --force delete "$n" >/dev/null; done

systemctl daemon-reload
if [ $HAS_VNC = yes ] && [ $VNC_RUNNING = yes ]; then
systemctl stop "$VNC_SERVICE" || true
for i in 1 2 3 4 5 6 7 8; do sudo -u "$VNC_USER" vncserver -kill ":$i" >/dev/null 2>&1 || true; done   # the old TCP ones
systemctl start "$VNC_SERVICE" || echo "WARNING: $VNC_SERVICE did not start; open desktops from the portal instead (New desktop)"
sleep 3
fi

set +e   # the check only reports; a missing socket or port must not hide the rest of it
echo
echo "== Check"
echo "cloudflared service file: $(stat -c %A /etc/systemd/system/cloudflared.service) (want -rw-------)"
echo "Desktops: $(sudo -u "$VNC_USER" /home/$VNC_USER/.local/bin/exo-desktop list | paste -sd' ')"
echo "Sockets:"; ls -l /home/$VNC_USER/.vnc/desk-*.sock 2>/dev/null | awk '{print "  " $1, $NF}'
ss -ltnH | awk '$4 ~ /:3389$/ && $4 !~ /^(127\.0\.0\.1|\[::1\]):/' | grep -q . && echo "  WARNING: RDP (3389) still listens beyond this machine itself; the firewall now refuses it from outside, stop xrdp or bind it to 127.0.0.1"
ss -ltnH | awk '$4 ~ /:59[0-9][0-9]$/' | grep -q . && echo "  WARNING: something still listens on a 59xx TCP port" || echo "  (no VNC TCP port open)"
echo "Firewall: $(ufw status | head -1)"; ufw status numbered | sed '1,4d'
