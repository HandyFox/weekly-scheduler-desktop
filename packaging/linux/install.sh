#!/bin/sh
# Install Weekly Assignment Scheduler for the current user (no sudo).
#   ./install.sh               install or update
#   ./install.sh --uninstall   remove it (your schedule in ~/Documents is kept)
set -e
cd "$(dirname "$0")"
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"

if [ "$1" = "--uninstall" ]; then
    rm -f "$BIN/weekly-scheduler" "$APPS/weekly-scheduler.desktop" "$ICONS/weekly-scheduler.png"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" 2>/dev/null || true
    echo "Removed. Your schedule is still in ~/Documents/Weekly Assignment Scheduler."
    exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS"
install -m 755 weekly-scheduler "$BIN/weekly-scheduler"
install -m 644 weekly-scheduler.png "$ICONS/weekly-scheduler.png"
sed "s|^Exec=.*|Exec=$BIN/weekly-scheduler|" weekly-scheduler.desktop > "$APPS/weekly-scheduler.desktop"
chmod 644 "$APPS/weekly-scheduler.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "${ICONS%/256x256/apps}" 2>/dev/null || true
echo "Installed. Open \"Weekly Assignment Scheduler\" from your applications menu,"
echo "or run: $BIN/weekly-scheduler"
