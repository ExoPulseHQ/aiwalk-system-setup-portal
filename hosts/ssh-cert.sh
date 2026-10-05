#!/bin/bash
# People sign in to this machine's shared account with a short-lived SSH certificate that Cloudflare Access signs
# for them after their GitHub sign-in, so the machine's own log names the person and a certificate cannot be lent:
#   Accepted publickey for ntk ... ID alice@example.com (serial ...) CA ...
#
#   sudo bash ssh-cert.sh <account> < the machine's CA public key      (hosts/ca/<name>.pub)
#
# Adds trust only; keys already in the account's authorized_keys keep working, so nobody is locked out. Take those
# keys away afterwards, once everyone connects with a certificate. Checks sshd's configuration before reloading and
# puts the old state back if the check fails. Reruns cleanly.
set -euo pipefail
ACCOUNT=${1:?Which account do people sign in to? e.g. ntk}
[ "$(id -u)" = 0 ] || { echo "Run it with sudo."; exit 1; }
getent passwd "$ACCOUNT" >/dev/null || { echo "There is no account called $ACCOUNT here."; exit 1; }
CA=$(cat)
case "$CA" in ecdsa-sha2-*|ssh-ed25519\ *|ssh-rsa\ *) ;; *) echo "Pass the CA's public key on stdin."; exit 1 ;; esac
CONF=/etc/ssh/sshd_config.d/60-exo-access.conf
grep -qE '^\s*Include\s+/etc/ssh/sshd_config\.d/' /etc/ssh/sshd_config || { echo "sshd_config does not include sshd_config.d here; nothing was changed."; exit 1; }

printf '%s\n' "$CA" > /etc/ssh/exo_access_ca.pub
cat > /usr/local/bin/exo-principal <<'SH'
#!/bin/sh
# A Cloudflare Access certificate names the person: its key ID is their email, its principal the part before "@".
# sshd asks this for the principals allowed into the account; answering with that part lets in whoever Cloudflare
# signed a certificate for, and Cloudflare signs only for people this machine's Access policy lets through.
printf "%s\n" "${1%%@*}"
SH
# no principal at all, for every other account: without this, sshd's own rule lets a certificate in to the account
# its principal names, and the principal is the part of the person's email before "@" (root@..., eddlai@...)
: > /etc/ssh/exo_no_principals
chown root:root /usr/local/bin/exo-principal /etc/ssh/exo_access_ca.pub /etc/ssh/exo_no_principals
chmod 755 /usr/local/bin/exo-principal; chmod 644 /etc/ssh/exo_access_ca.pub /etc/ssh/exo_no_principals

[ -f "$CONF" ] && cp -p "$CONF" "$CONF.bak"
cat > "$CONF" <<CONF
# aIwalk System Setup: people sign in to the shared account with a short-lived certificate from this machine's
# Cloudflare Access application, so the log names the person. Keys in authorized_keys keep working beside it.
TrustedUserCAKeys /etc/ssh/exo_access_ca.pub
AuthorizedPrincipalsFile /etc/ssh/exo_no_principals
Match User $ACCOUNT
    AuthorizedPrincipalsCommand /usr/local/bin/exo-principal %i
    AuthorizedPrincipalsCommandUser nobody
Match all
CONF
if ! sshd -t; then
  if [ -f "$CONF.bak" ]; then mv "$CONF.bak" "$CONF"; else rm -f "$CONF"; fi
  echo "sshd refused the new configuration; the old one is back and nothing was reloaded."; exit 1
fi
rm -f "$CONF.bak"
# sshd keeps the first value it reads: say so when another file's AuthorizedPrincipalsFile wins over the empty one
T=$(sshd -T -C "user=root,host=localhost,addr=127.0.0.1" 2>/dev/null) || true
grep -qi '^authorizedprincipalsfile /etc/ssh/exo_no_principals' <<<"$T" \
  || echo "WARNING: another setting names AuthorizedPrincipalsFile first, so a certificate could still open an account other than $ACCOUNT. Check: sshd -T | grep -i principals"
systemctl reload ssh 2>/dev/null || systemctl reload sshd
echo "Done: certificates signed by this machine's Cloudflare application are accepted for $ACCOUNT."
