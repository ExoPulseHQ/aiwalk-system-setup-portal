#!/bin/bash
# Lock a lab machine down so people reach it only through Cloudflare (the portal's GitHub sign-in): VNC desktops and
# other services listen on 127.0.0.1 and are forwarded over SSH through the tunnel. Registered lab machines may still
# SSH to each other directly (the admins' fallback when Cloudflare is down).
#
#   sudo bash lockdown.sh            prints the plan, asks, then applies and checks
#
# Registered machines are the lab hosts in System/vault_rules.json `machines`; their addresses come from
# Secrets/Compute_Repo_Access_Registry.md §1b. This machine's own address is left out automatically.
set -euo pipefail
MACHINES="120.126.83.20 120.126.83.112 120.126.83.1 120.126.83.67 120.126.83.76 120.126.83.143 120.126.83.28 120.126.83.209 120.126.83.228"
VNC_USER=ntk
VNC_CONFIG=/home/$VNC_USER/.vnc/config
VNC_SERVICE=gpu-free-vnc.service

[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
systemctl is-active --quiet cloudflared || { echo "cloudflared is not running. After this the tunnel is the way in, so nothing was changed."; exit 1; }
SELF=$(hostname -I)
PEERS=$(for ip in $MACHINES; do case " $SELF " in *" $ip "*) ;; *) echo -n "$ip ";; esac; done)

# rules that open SSH, HTTP or VNC to everyone: "[ n] 22/tcp   ALLOW IN   Anywhere" and their v6 twins
open_rules() {
  ufw status numbered | awk -F'[][]' '/ALLOW IN/ && /Anywhere/ && !/ from / {
    split($3, f, " "); to = f[1]
    if (to ~ /^(22|80|59[0-9][0-9](:59[0-9][0-9])?)(\/tcp|\/udp)?$/ || to == "OpenSSH" || to ~ /^Nginx/) print $2 + 0 }'
}

echo "== Firewall now"; ufw status verbose | sed -n '1,4p'; ufw status numbered | sed '1,4d'
echo
echo "== Plan"
echo " 1. The tunnel token in the cloudflared service file becomes readable by root only."
echo " 2. VNC desktops listen on 127.0.0.1 only; they restart once, so anyone on them is disconnected."
echo " 3. SSH (22) is allowed only from: $PEERS"
echo "    Rules that open 22, 80 or 59xx to everyone are removed (numbers: $(open_rules | sort -n | paste -sd' '))."
echo "    New connections from anywhere else are refused; the Cloudflare tunnel is unaffected."
read -rp "Apply? [y/N] " answer
[ "$answer" = y ] || { echo "Nothing changed."; exit 0; }

chmod 600 /etc/systemd/system/cloudflared.service
systemctl daemon-reload

if grep -q '^localhost=' "$VNC_CONFIG"; then sed -i 's/^localhost=.*/localhost=yes/' "$VNC_CONFIG"; else echo localhost=yes >> "$VNC_CONFIG"; fi

ufw default deny incoming >/dev/null
for ip in $PEERS; do ufw allow proto tcp from "$ip" to any port 22 comment 'registered lab machine' >/dev/null; done
for n in $(open_rules | sort -rn); do yes | ufw delete "$n" >/dev/null; done

systemctl restart "$VNC_SERVICE"
sleep 3

echo
echo "== Check"
echo "cloudflared service file: $(stat -c %A /etc/systemd/system/cloudflared.service) (want -rw-------)"
echo "VNC listening on:"; ss -ltnH | awk '$4 ~ /:59[0-9][0-9]$/ {print "  " $4}' | sort -u
ss -ltnH | awk '$4 ~ /:59[0-9][0-9]$/' | grep -vqE '127\.0\.0\.1|\[::1\]' && echo "  WARNING: a desktop still listens beyond 127.0.0.1" || echo "  (all on 127.0.0.1)"
echo "Firewall:"; ufw status numbered | sed '1,4d'
