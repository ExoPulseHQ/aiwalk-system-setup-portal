#!/bin/bash
# Removes what install.sh added. Leaves phones untouched.
APP_ID=com.aiwalk.SystemSetup
DESKTOP_DIR=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
rm -rf "$HOME/.local/share/aiwalk-setup"
rm -f "$HOME/.local/share/applications/$APP_ID".*desktop "$DESKTOP_DIR/$APP_ID".*desktop \
      "$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID".*.svg \
      "$HOME/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg"
update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
echo "Removed aIwalk System Setup."
