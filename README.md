# aIwalk System Setup

The app every team member installs first: sign in with GitHub, see what you can open, download the team's documents,
and reach the lab machines through Cloudflare. It needs no gh and no cloudflared: it signs in, keeps the sign-in and carries SSH itself. On Windows git comes with it.

## Install

[Releases](../../releases/latest), then the file for your computer:

| Computer | File |
|---|---|
| Windows | `..._x64-setup.exe` (or the `.msi`) |
| Mac with Apple silicon (M1 and later) | `..._aarch64.dmg` |
| Mac with Intel | `..._x64.dmg` |
| Ubuntu / Debian | `..._amd64.deb` (`sudo apt install ./<file>.deb`) |
| Other Linux | `..._amd64.AppImage` |

The `.app.tar.gz` files are for updates; you do not need them. The app is not signed with a paid certificate yet:
Windows SmartScreen asks once (*More info*, *Run anyway*); on macOS open it once, then System Settings, Privacy &
Security, *Open Anyway*.

Who can download: members of ExoPulseHQ (the `members` team writes here). Interns are outside collaborators, so an
owner gives each one **read** on this repository along with `exo-book` and `exo-papers`.

## Contribute

Members open a branch and a pull request; an owner merges. Releases ship to everyone's computer, so only a tag pushed
by an owner builds one (`.github/workflows/release.yml`): `git tag -a v0.2.0 -m ... && git push origin v0.2.0`.

`hosts/` holds scripts owners run **with sudo** on lab machines (`install-host.sh`, `lockdown.sh`). Before running
one, check who changed it last: `git log -p hosts/`.

## Layout

- `desktop/`: the Tauri app (Rust in `src/`, the page in `ui/`); `fetch-tools.sh` puts MinGit into `desktop/tools/` for Windows
  before bundling, checksum verified.
- `core/`: logic with tests, no UI (`cargo test -p exo-core`).
- `hosts/`: what runs on the lab machines: status page, VNC desktops, first setup, lockdown.
- `aiwalk-setup`, `android/`, `windows/`, `lib/`: the earlier Python app and its phone and VM parts.
