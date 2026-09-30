#!/bin/bash
# Builds dist/aIwalkSetup: this app plus adb, scrcpy and the phone APKs, ready to copy to a USB disk.
set -e
cd "$(dirname "$0")"
OUT=dist/aIwalkSetup
rm -rf "$OUT"; mkdir -p "$OUT/tools" "$OUT/apk"
cp aiwalk-setup icon.svg icon.png install.sh uninstall.sh "$OUT"/
cp -r icons windows "$OUT"/
cp -a "$HOME/.local/platform-tools" "$OUT/tools/platform-tools"
cp -aL "$HOME/.local/scrcpy" "$OUT/tools/scrcpy"
cp icon.png "$OUT/tools/scrcpy/scrcpy.png"  # scrcpy shows this in its window and title bar
cp ../phone-desk/app/build/outputs/apk/debug/app-debug.apk "$OUT/apk/phonedesk.apk"
cp ../phone-desk/deps/shizuku-v13.6.0.apk "$OUT/apk/shizuku.apk"
chmod +x "$OUT"/aiwalk-setup "$OUT"/install.sh "$OUT"/uninstall.sh
du -sh "$OUT"
