#!/bin/bash
# Installs this folder for the current user (no sudo): copies it to ~/.local/share/phone-panel
# and adds the app to the application menu and desktop. Run it from the USB disk.
set -e
SRC=$(cd "$(dirname "$0")" && pwd)
DEST="$HOME/.local/share/phone-panel"
APP_ID=com.eddlai.PhonePanel

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
  echo "缺少 GTK4 / libadwaita 的 Python 元件，請先執行："
  echo "  sudo apt install python3-gi gir1.2-gtk-4.0 gir1.2-adw-1"
  exit 1
fi

if [ "$SRC" != "$DEST" ]; then
  rm -rf "$DEST"; mkdir -p "$DEST"; cp -a "$SRC"/. "$DEST"/
fi
chmod +x "$DEST/phone-panel" "$DEST"/tools/platform-tools/adb "$DEST"/tools/scrcpy/scrcpy 2>/dev/null || true

mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps" "$HOME/.local/share/applications"
cp "$DEST/icon.svg" "$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg"
cat > "$HOME/.local/share/applications/$APP_ID.desktop" <<DESK
[Desktop Entry]
Name=Android 手機
Comment=公務機：手機電腦桌面、鏡像、鍵盤、有線／無線連線
Exec=$DEST/phone-panel
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

echo "已安裝到 $DEST，可在應用程式選單或桌面找到「Android 手機」。"
if ! id -nG | grep -qw plugdev; then
  echo "提醒：若 USB 接手機時 adb 看不到手機，請執行一次："
  echo "  sudo apt install android-sdk-platform-tools-common && sudo usermod -aG plugdev $USER"
  echo "然後登出再登入。"
fi
