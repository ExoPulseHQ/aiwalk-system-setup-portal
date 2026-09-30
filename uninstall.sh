#!/bin/bash
# Removes what install.sh added. Leaves phones untouched.
APP_ID=com.eddlai.PhonePanel
DESKTOP_DIR=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
rm -rf "$HOME/.local/share/phone-panel"
rm -f "$HOME/.local/share/applications/$APP_ID.desktop" "$DESKTOP_DIR/$APP_ID.desktop" \
      "$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg"
update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
echo "已移除「Android 手機」。"
