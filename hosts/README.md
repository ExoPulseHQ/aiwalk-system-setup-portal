# The machines' side

What runs on a team machine so people reach it through Cloudflare, and the scripts that set it up. Everything here
is run on the machine itself; the scripts that change the system need sudo and say what they will do first.

## What starts by itself when a machine restarts

| What | How it starts | Set up by |
|---|---|---|
| The Cloudflare tunnel (`cloudflared`) | systemd service `cloudflared.service`, enabled, restarts itself when it fails, waits for the network | `install-host.sh` |
| The firewall (`ufw`) | systemd service `ufw.service`, enabled; where `netfilter-persistent` is also enabled, `ufw` waits for it (a drop-in in `/etc/systemd/system/ufw.service.d/`) | `lockdown.sh` |
| The status page (`exo-status.py` on 127.0.0.1:9101) | the account's crontab, an `@reboot` line | `install-host.sh` |
| SSH (`sshd`) with the certificate rules | the system's own service; the rules are a file in `/etc/ssh/sshd_config.d/` | `ssh-cert.sh`, `ssh-password-off.sh` |
| VNC desktops | not at boot unless the machine has a VNC service; the app starts them on request | `exo-desktop`, `lockdown.sh` |

The tunnel is the only way in. It makes an outgoing connection from the machine to Cloudflare, so no port is open
to the outside; if it is not running, nobody reaches the machine and someone has to sit at it.

## The scripts, in the order a new machine gets them

1. `install-host.sh` (sudo): installs `cloudflared` as a system service with the machine's tunnel token, the status
   page, and the tools below into the account's `~/.local/bin`. Reruns cleanly; on a machine already behind the
   tunnel it leaves the service alone.
2. `ssh-cert.sh` (sudo): the account takes a short-lived SSH certificate signed by Cloudflare Access, and no other
   account takes any certificate. The machine's CA public key is kept in `ca/<machine>.pub`.
3. `lockdown.sh` (sudo): the firewall refuses every new connection from outside except SSH from the other
   registered machines; VNC listens on the machine alone. Prints what it will change and asks first.
4. `ssh-password-off.sh` (sudo): password sign-in over SSH off, once certificates are known to work. Only on
   monkey so far.

## The tools the app and the vault plugin call (no sudo)

- `exo-status.py`: what the machine has and how busy it is, for the app's Machines page.
- `exo-desktop`: start, list and stop VNC desktops.
- `exo-term`: terminal sessions that outlive the connection (`aiwalk-setup ssh --session`).
- `exo`: the vault plugin's Sync code page: the repos on the machine, their worktrees, commit and push.

Each has a `VERSION`; the app's Machines page compares it with its own copy and offers **Update host tools**.
`test_exo.py` checks `exo`.

## Checking a machine after a restart

```
systemctl is-active cloudflared ufw        # both: active
systemctl is-failed ufw                    # active (not: failed)
ss -ltn | grep 127.0.0.1:9101              # the status page
```

`ufw` failed with `Chain 'ufw-logging-deny' does not exist` means something else loaded firewall rules at the
same moment (see `lockdown.sh`); the machine then has no network at all until `ufw` is started again.
