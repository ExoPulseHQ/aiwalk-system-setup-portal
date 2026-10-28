#!/bin/sh
# Claude for Obsidian on this phone. Run once in Android's Linux Terminal, and again after updating aIwalk System Setup:
#   sh /mnt/shared/Documents/aIwalk/linux-terminal/setup.sh
# aIwalk System Setup puts this file here (linuxterm.rs), with aiwalk-claude-vm beside it. It installs what Claude needs
# that the Terminal's Debian lacks, then `aiwalk-claude-vm install`: exo-claude, and the service Obsidian's Claude pane
# talks to, started with the VM from then on. It ends with the 6-digit code Obsidian asks for.
set -e
version=@VERSION@
here=$(cd "$(dirname "$0")" && pwd)
export PATH="$HOME/.local/bin:$PATH"
if ! command -v tmux >/dev/null 2>&1; then echo "Installing tmux..."; sudo apt-get update -qq && sudo apt-get install -y -qq tmux; fi
if ! command -v claude >/dev/null 2>&1; then echo "Installing Claude Code..."; curl -fsSL https://claude.ai/install.sh | bash; fi
# shared storage does not run programs: the program is copied into this VM first
tmp=$(mktemp)
if [ -s "$here/aiwalk-claude-vm" ]; then cp "$here/aiwalk-claude-vm" "$tmp"
else
  # an app built without it (a build on a computer): the same version from the release
  echo "Downloading aiwalk-claude-vm $version..."
  curl -fL -o "$tmp" "https://github.com/ExoPulseHQ/aiwalk-system-setup-portal/releases/download/v$version/aiwalk-claude-vm_${version}_linux_arm64"
fi
chmod +x "$tmp"
# the vault: the one named here, or the first folder beside this one that Obsidian has opened
vault=${1:-}
if [ -z "$vault" ]; then for d in "$here"/../*/; do if [ -d "$d.obsidian" ]; then vault=${d%/}; break; fi; done; fi
"$tmp" install ${vault:+"$vault"}
rm -f "$tmp"
