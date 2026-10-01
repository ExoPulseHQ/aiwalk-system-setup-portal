#!/bin/bash
# Builds dist/aIwalkSetup: this app plus adb, scrcpy and the phone APKs, ready to copy to a USB disk.
set -e
cd "$(dirname "$0")"
OUT=dist/aIwalkSetup
rm -rf "$OUT"; mkdir -p "$OUT/tools" "$OUT/apk"
cp aiwalk-setup icon.svg icon.png install.sh uninstall.sh "$OUT"/
cp -r icons windows lib "$OUT"/
cp -a "$HOME/.local/platform-tools" "$OUT/tools/platform-tools"
cp -aL "$HOME/.local/scrcpy" "$OUT/tools/scrcpy"
cp icon.png "$OUT/tools/scrcpy/scrcpy.png"  # scrcpy shows this in its window and title bar
# PhoneDesk is built from android/ (cd android && ./gradlew assembleDebug); Shizuku is third-party and kept out of git
PD=android/app/build/outputs/apk/debug/app-debug.apk; SZ=android/deps/shizuku-v13.6.0.apk
[ -f "$PD" ] || { echo "Missing $PD: run (cd android && ./gradlew assembleDebug) first" >&2; exit 1; }
[ -f "$SZ" ] || { echo "Missing $SZ: download Shizuku v13.6.0 from https://github.com/RikkaApps/Shizuku/releases and save it there" >&2; exit 1; }
cp "$PD" "$OUT/apk/phonedesk.apk"
cp "$SZ" "$OUT/apk/shizuku.apk"
chmod +x "$OUT"/aiwalk-setup "$OUT"/install.sh "$OUT"/uninstall.sh
du -sh "$OUT"
