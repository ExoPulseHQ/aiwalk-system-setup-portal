#!/bin/bash
# Installs this folder for the current user (no sudo): copies it to ~/.local/share/aiwalk-setup
# and adds the app to the application menu and desktop. Run it from the USB disk.
set -e
SRC=$(cd "$(dirname "$0")" && pwd)
DEST="$HOME/.local/share/aiwalk-setup"
APP_ID=com.aiwalk.SystemSetup

missing=$(/usr/bin/python3 - <<'PY' 2>&1
import gi
try:
    gi.require_version("Gtk", "4.0"); gi.require_version("Adw", "1")
    from gi.repository import Gtk, Adw
except Exception as e:
    print(e)
PY
)
if [ -n "$missing" ]; then
  echo "GTK4 / libadwaita for Python is missing. Install it first:"
  echo "  sudo apt install python3-gi gir1.2-gtk-4.0 gir1.2-adw-1"
  exit 1
fi

# earlier versions of this tool were called "Android Phone"
rm -rf "$HOME/.local/share/phone-panel"
rm -f "$HOME/.local/share/applications/com.eddlai.PhonePanel.desktop" \
      "$HOME/.local/share/icons/hicolor/scalable/apps/com.eddlai.PhonePanel.svg"
if [ "$SRC" != "$DEST" ]; then
  rm -rf "$DEST"; mkdir -p "$DEST"; cp -a "$SRC"/. "$DEST"/
fi
chmod +x "$DEST/aiwalk-setup" "$DEST"/tools/platform-tools/adb "$DEST"/tools/scrcpy/scrcpy 2>/dev/null || true

mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps" "$HOME/.local/share/applications"
cp "$DEST/icon.svg" "$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg"
cat > "$HOME/.local/share/applications/$APP_ID.desktop" <<DESK
[Desktop Entry]
Name=aIwalk System Setup
Comment=Set up a new team member: Android phone, Windows VM
Exec=$DEST/aiwalk-setup
Icon=$APP_ID
Terminal=false
Type=Application
Categories=Utility;
StartupWMClass=$APP_ID
DESK
DESKTOP_DIR=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
if [ -d "$DESKTOP_DIR" ]; then
  cp "$HOME/.local/share/applications/$APP_ID.desktop" "$DESKTOP_DIR/$APP_ID.desktop"
  chmod +x "$DESKTOP_DIR/$APP_ID.desktop"
  gio set "$DESKTOP_DIR/$APP_ID.desktop" metadata::trusted true 2>/dev/null || true
fi
gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true

echo "Installed to $DEST. Open \"aIwalk System Setup\" from the app menu or the desktop."
if ! id -nG | grep -qw plugdev; then
  echo "If a USB-connected phone does not show up, run once:"
  echo "  sudo apt install android-sdk-platform-tools-common && sudo usermod -aG plugdev $USER"
  echo "then log out and back in."
fi
