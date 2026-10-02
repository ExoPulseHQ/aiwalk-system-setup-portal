#!/bin/bash
# Puts gh, cloudflared (and on Windows, MinGit) into desktop/tools/ for one target, checksums verified, so a new member's
# computer needs neither. Run before bundling: ./fetch-tools.sh linux-amd64 | macos-arm64 | macos-amd64 | windows-amd64
# macOS gets no git here: Apple ships none portable; the app asks macOS to install its own (xcode-select --install).
# Linux gets no git here: distros have it, and the app shows the install command when it is missing.
set -euo pipefail
cd "$(dirname "$0")"
GH=2.102.0
MINGIT=2.56.0
MINGIT_SHA=064b440ff870ed5198527e8f3a92cdf5bd2fd0fedf5e718af95e3fdaddeff718  # from the git-for-windows v2.56.0.windows.1 release notes
CF=2026.9.3   # cloudflared: members reach lab machines through Cloudflare Access (Machine_Login_Identity_Summary §二)
# from the cloudflare/cloudflared 2026.9.3 release notes; a case, not an associative array, for macOS's bash 3.2
cf_sha() { case $1 in
  cloudflared-linux-amd64)       echo 77e26d8d900e0b8469f416239d14b5f296525fdf79fee6f511ef55609e3fbac2 ;;
  cloudflared-darwin-arm64.tgz)  echo 5472c1a01c84bc31b3021056a73b4e5774ddddefc572124ea8fdf6c340639f32 ;;
  cloudflared-darwin-amd64.tgz)  echo ab588b3b4db9cdb4476c30a3db2a72635b1d8327d44741fee6799a0f37b0ec07 ;;
  cloudflared-windows-amd64.exe) echo f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2 ;;
esac; }
command -v sha256sum >/dev/null || sha256sum() { shasum -a 256 "$@"; }   # macOS has shasum only
target=${1:?usage: fetch-tools.sh linux-amd64|macos-arm64|macos-amd64|windows-amd64}
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
rm -rf tools/gh tools/git tools/cloudflared; mkdir -p tools/gh tools/cloudflared

case $target in
  linux-amd64)   asset=gh_${GH}_linux_amd64.tar.gz;  cf=cloudflared-linux-amd64 ;;
  macos-arm64)   asset=gh_${GH}_macOS_arm64.zip;     cf=cloudflared-darwin-arm64.tgz ;;
  macos-amd64)   asset=gh_${GH}_macOS_amd64.zip;     cf=cloudflared-darwin-amd64.tgz ;;
  windows-amd64) asset=gh_${GH}_windows_amd64.zip;   cf=cloudflared-windows-amd64.exe ;;
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

curl -fsSL -o "$tmp/$cf" "https://github.com/cloudflare/cloudflared/releases/download/$CF/$cf"
# for the macOS .tgz files the published sum is that of the cloudflared binary inside, so check after unpacking
case $cf in
  *.tgz) tar -xzf "$tmp/$cf" -C tools/cloudflared && echo "$(cf_sha "$cf")  tools/cloudflared/cloudflared" | sha256sum -c - ;;
  *.exe) echo "$(cf_sha "$cf")  $tmp/$cf" | sha256sum -c - && cp "$tmp/$cf" tools/cloudflared/cloudflared.exe ;;
  *)     echo "$(cf_sha "$cf")  $tmp/$cf" | sha256sum -c - && cp "$tmp/$cf" tools/cloudflared/cloudflared ;;
esac
chmod +x tools/cloudflared/*

if [ "$target" = windows-amd64 ]; then
  zip=MinGit-$MINGIT-64-bit.zip
  curl -fsSL -o "$tmp/$zip" "https://github.com/git-for-windows/git/releases/download/v$MINGIT.windows.1/$zip"
  echo "$MINGIT_SHA  $tmp/$zip" | sha256sum -c -
  mkdir -p tools/git && unzip -q "$tmp/$zip" -d tools/git
fi
du -sh tools/*
