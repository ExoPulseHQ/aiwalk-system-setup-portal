#!/bin/bash
# Builds dist/PhonePanel: this app plus adb, scrcpy and the phone APKs, ready to copy to a USB disk.
set -e
cd "$(dirname "$0")"
OUT=dist/PhonePanel
rm -rf "$OUT"; mkdir -p "$OUT/tools" "$OUT/apk"
cp phone-panel icon.svg install.sh uninstall.sh "$OUT"/
cp -a "$HOME/.local/platform-tools" "$OUT/tools/platform-tools"
cp -aL "$HOME/.local/scrcpy" "$OUT/tools/scrcpy"
cp ../phone-desk/app/build/outputs/apk/debug/app-debug.apk "$OUT/apk/phonedesk.apk"
cp ../phone-desk/deps/shizuku-v13.6.0.apk "$OUT/apk/shizuku.apk"
chmod +x "$OUT"/phone-panel "$OUT"/install.sh "$OUT"/uninstall.sh
du -sh "$OUT"
