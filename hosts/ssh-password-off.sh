#!/bin/bash
# Turn off password sign-in over SSH on this machine, so the only way in is a certificate that Cloudflare Access
# signed for a person (ssh-cert.sh). The shared account's password then opens nothing over the network; it still
# works at the machine's own screen and for sudo, which this does not touch.
#
#   sudo bash ssh-password-off.sh           prints what sshd does now, applies, checks, reloads
#   sudo bash ssh-password-off.sh --undo    removes the file this wrote and reloads
#
# It refuses to start unless sshd already trusts a certificate authority: without that, turning passwords off on a
# machine whose keys are gone would leave no way in but the screen. It checks sshd's configuration before
# reloading and takes its file away again if the check fails or if passwords would still be accepted (another
# file read earlier says yes). Reruns cleanly. Exit: 0 done, 1 nothing changed.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
# first in the folder: sshd keeps the first value it reads, and Ubuntu's sshd_config includes this folder at its top
FILE=/etc/ssh/sshd_config.d/00-exo-no-password.conf
now() { sshd -T 2>/dev/null | grep -iE "^($1) " | tr '\n' ' '; }
reload() { systemctl reload ssh 2>/dev/null || systemctl reload sshd; }

if [ "${1:-}" = --undo ]; then
  [ -f "$FILE" ] || { echo "Nothing to undo: $FILE is not there."; exit 1; }
  rm -f "$FILE"; sshd -t; reload
  echo "Removed. sshd now: $(now 'passwordauthentication|kbdinteractiveauthentication')"; exit 0
fi

echo "sshd now: $(now 'passwordauthentication|kbdinteractiveauthentication|challengeresponseauthentication|trustedusercakeys')"
sshd -T 2>/dev/null | grep -qiE '^trustedusercakeys +/.+' || { echo "This sshd trusts no certificate authority yet (run ssh-cert.sh first). Nothing changed."; exit 1; }
grep -qE '^\s*Include\s+/etc/ssh/sshd_config\.d/' /etc/ssh/sshd_config || { echo "/etc/ssh/sshd_config does not include sshd_config.d, so a file there would do nothing. Nothing changed."; exit 1; }

# the keyboard-interactive switch changed its name in OpenSSH 8.7; an older sshd rejects the new word
write() { printf '# Written by ssh-password-off.sh: sign-in is by certificate only (ssh-cert.sh). Remove with --undo.\nPasswordAuthentication no\n%s no\n' "$1" > "$FILE"; chmod 644 "$FILE"; }
write KbdInteractiveAuthentication
if ! sshd -t 2>/dev/null; then write ChallengeResponseAuthentication; fi
if ! sshd -t; then rm -f "$FILE"; echo "sshd rejected the file; it is removed. Nothing changed."; exit 1; fi
# a Match block or an earlier file can still say yes: ask sshd what it would do for the accounts people use
for u in $(awk -F: '$3 >= 1000 && $7 !~ /(nologin|false)$/ {print $1}' /etc/passwd); do
  if sshd -T -C "user=$u,host=localhost,addr=127.0.0.1" 2>/dev/null | grep -qi '^passwordauthentication yes'; then
    rm -f "$FILE"; echo "Passwords would still be accepted for $u (another setting wins); the file is removed. Nothing changed."; exit 1
  fi
done
reload
echo "Done. sshd now: $(now 'passwordauthentication|kbdinteractiveauthentication|challengeresponseauthentication')"
echo "Open connections stay open. Check from another computer that a password is refused and a certificate gets in."
