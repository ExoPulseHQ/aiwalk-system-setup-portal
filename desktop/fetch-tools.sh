#!/bin/bash
# Puts gh (and on Windows, MinGit) into desktop/tools/ for one target, checksums verified, so a new member's
# computer needs neither. Run before bundling: ./fetch-tools.sh linux-amd64 | macos-arm64 | macos-amd64 | windows-amd64
# macOS gets no git here: Apple ships none portable; the app asks macOS to install its own (xcode-select --install).
# Linux gets no git here: distros have it, and the app shows the install command when it is missing.
set -euo pipefail
cd "$(dirname "$0")"
GH=2.102.0
MINGIT=2.56.0
MINGIT_SHA=064b440ff870ed5198527e8f3a92cdf5bd2fd0fedf5e718af95e3fdaddeff718  # from the git-for-windows v2.56.0.windows.1 release notes
target=${1:?usage: fetch-tools.sh linux-amd64|macos-arm64|macos-amd64|windows-amd64}
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
rm -rf tools/gh tools/git; mkdir -p tools/gh

case $target in
  linux-amd64)   asset=gh_${GH}_linux_amd64.tar.gz ;;
  macos-arm64)   asset=gh_${GH}_macOS_arm64.zip ;;
  macos-amd64)   asset=gh_${GH}_macOS_amd64.zip ;;
  windows-amd64) asset=gh_${GH}_windows_amd64.zip ;;
  *) echo "unknown target $target" >&2; exit 1 ;;
esac
base=https://github.com/cli/cli/releases/download/v$GH
curl -fsSL -o "$tmp/$asset" "$base/$asset"
curl -fsSL -o "$tmp/sums" "$base/gh_${GH}_checksums.txt"
(cd "$tmp" && grep " $asset\$" sums | sha256sum -c -)
case $asset in
  *.tar.gz) tar -xzf "$tmp/$asset" -C "$tmp" ;;
  *.zip)    unzip -q "$tmp/$asset" -d "$tmp/gh-x" ;;
esac
bin=$(find "$tmp" -type f \( -name gh -o -name gh.exe \) -path '*bin*' | head -1)
cp "$bin" tools/gh/ && chmod +x tools/gh/*

if [ "$target" = windows-amd64 ]; then
  zip=MinGit-$MINGIT-64-bit.zip
  curl -fsSL -o "$tmp/$zip" "https://github.com/git-for-windows/git/releases/download/v$MINGIT.windows.1/$zip"
  echo "$MINGIT_SHA  $tmp/$zip" | sha256sum -c -
  mkdir -p tools/git && unzip -q "$tmp/$zip" -d tools/git
fi
du -sh tools/*
