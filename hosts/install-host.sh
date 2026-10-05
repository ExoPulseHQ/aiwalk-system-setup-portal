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
UNIT=/etc/systemd/system/cloudflared.service
if [ -f "$UNIT" ]; then
  # not touched on a machine already behind the tunnel: restarting cloudflared cuts the very connection this runs
  # over, before it could put the old unit back. Its token is moved out of the command line by hand, at the screen.
  echo "cloudflared service already installed; left as it is"
  grep -q -- ' --token ' "$UNIT" && echo "NOTE: the tunnel token is in cloudflared's command line here, readable by every account (ps)"
else
  /usr/local/bin/cloudflared service install "$TOKEN"
  # the token out of the command line, where every account on the machine could read it (ps), into a file only root
  # reads. The old unit comes back if cloudflared does not start with the new one: the tunnel is the only way in.
  if grep -q -- ' --token ' "$UNIT"; then
    T=$(sed -n 's/.* --token \([^ ]*\).*/\1/p' "$UNIT" | head -1)
    install -d -m 700 /etc/cloudflared
    ( umask 077; printf 'TUNNEL_TOKEN=%s\n' "$T" > /etc/cloudflared/token.env )
    cp -p "$UNIT" "$UNIT.bak"
    sed -i -e 's/ --token [^ ]*//' -e '/^\[Service\]/a EnvironmentFile=/etc/cloudflared/token.env' "$UNIT"
    systemctl daemon-reload; systemctl restart cloudflared; sleep 5
    if systemctl is-active --quiet cloudflared; then rm -f "$UNIT.bak"; echo "tunnel token moved out of the command line"
    else mv "$UNIT.bak" "$UNIT"; systemctl daemon-reload; systemctl restart cloudflared; echo "WARNING: cloudflared did not start from the token file; the old unit is back and the token is still in its command line"; fi
  fi
fi
chmod 600 "$UNIT"
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
