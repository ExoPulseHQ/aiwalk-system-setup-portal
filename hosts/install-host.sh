#!/bin/bash
# First-time setup of a lab machine for aIwalk System Setup: the Cloudflare tunnel as a system service, the status
# page (127.0.0.1:9101) and exo-desktop for the account that owns the desktops. Run lockdown.sh afterwards.
#
#   sudo bash install-host.sh <account> < tunnel-token
#
# Expects cloudflared, exo-status.py, exo-desktop and exo next to this script. Reruns cleanly.
set -euo pipefail
ACCOUNT=${1:?Which account owns the desktops? e.g. ntk}
DIR=$(cd "$(dirname "$0")" && pwd)
[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
TOKEN=$(cat)
[ -n "$TOKEN" ] || { echo "Pass the tunnel token on stdin."; exit 1; }
HOME_DIR=$(getent passwd "$ACCOUNT" | cut -d: -f6)

install -m 755 "$DIR/cloudflared" /usr/local/bin/cloudflared
if [ -f /etc/systemd/system/cloudflared.service ]; then
  echo "cloudflared service already installed; left as it is"
else
  /usr/local/bin/cloudflared service install "$TOKEN"
fi
chmod 600 /etc/systemd/system/cloudflared.service   # the token is inside; 644 by default

install -o "$ACCOUNT" -g "$ACCOUNT" -m 755 -D "$DIR/exo-desktop" "$HOME_DIR/.local/bin/exo-desktop"
install -o "$ACCOUNT" -g "$ACCOUNT" -m 755 -D "$DIR/exo-status.py" "$HOME_DIR/.local/bin/exo-status.py"
# the machine's side of the vault plugin's Sync code page; python3 >= 3.8 and git are all it needs
install -o "$ACCOUNT" -g "$ACCOUNT" -m 755 -D "$DIR/exo" "$HOME_DIR/.local/bin/exo"
STATUS="/usr/bin/python3 $HOME_DIR/.local/bin/exo-status.py"
sudo -u "$ACCOUNT" bash -c "{ crontab -l 2>/dev/null | grep -v exo-status.py; echo '@reboot $STATUS >/dev/null 2>&1'; } | crontab -"
ss -ltnH | grep -q '127.0.0.1:9101 ' || sudo -u "$ACCOUNT" bash -c "nohup $STATUS >/dev/null 2>&1 &"
sleep 3

echo "cloudflared: $(systemctl is-active cloudflared), service file $(stat -c %A /etc/systemd/system/cloudflared.service)"
echo "status page: $(ss -ltnH | grep -q '127.0.0.1:9101 ' && echo listening || echo NOT listening)"
