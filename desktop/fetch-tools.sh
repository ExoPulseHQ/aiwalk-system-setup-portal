#!/bin/bash
# Puts MinGit into desktop/tools/ for Windows, checksum verified, so a new member's computer needs no git of its own;
# the other targets get an empty tools/ (the bundle expects the folder). Run before bundling: ./fetch-tools.sh linux-amd64 | macos-arm64 | macos-amd64 | windows-amd64
# macOS gets no git here: Apple ships none portable; the app asks macOS to install its own (xcode-select --install).
# Linux gets no git here: distros have it, and the app shows the install command when it is missing.
set -euo pipefail
cd "$(dirname "$0")"
MINGIT=2.56.0
MINGIT_SHA=064b440ff870ed5198527e8f3a92cdf5bd2fd0fedf5e718af95e3fdaddeff718  # from the git-for-windows v2.56.0.windows.1 release notes
command -v sha256sum >/dev/null || sha256sum() { shasum -a 256 "$@"; }   # macOS has shasum only
target=${1:?usage: fetch-tools.sh linux-amd64|macos-arm64|macos-amd64|windows-amd64}
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
rm -rf tools/gh tools/git tools/cloudflared; mkdir -p tools

case $target in
  linux-amd64|macos-arm64|macos-amd64|windows-amd64) ;;
  *) echo "unknown target $target" >&2; exit 1 ;;
esac

if [ "$target" = windows-amd64 ]; then
  zip=MinGit-$MINGIT-64-bit.zip
  curl -fsSL -o "$tmp/$zip" "https://github.com/git-for-windows/git/releases/download/v$MINGIT.windows.1/$zip"
  echo "$MINGIT_SHA  $tmp/$zip" | sha256sum -c -
  mkdir -p tools/git && unzip -q "$tmp/$zip" -d tools/git
fi
du -sh tools
