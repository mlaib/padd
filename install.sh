#!/usr/bin/env sh
# Install Padd for the current user (Linux): binary, icon, menu entry.
#   ./install.sh [path/to/padd-binary]    (default: target/release/padd, then dist/padd-linux-<arch>)
#   ./install.sh --uninstall
set -eu
cd "$(dirname "$0")"
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN/padd" "$APPS/padd.desktop" "$ICONS/scalable/apps/padd.svg" "$ICONS/256x256/apps/padd.png"
    update-desktop-database "$APPS" 2>/dev/null || true
    echo "Padd uninstalled."
    exit 0
fi

SRC="${1:-}"
if [ -z "$SRC" ]; then
    for c in target/release/padd "dist/padd-linux-$(uname -m | sed 's/aarch64/arm64/')"; do
        [ -x "$c" ] && SRC="$c" && break
    done
fi
[ -n "$SRC" ] && [ -x "$SRC" ] || { echo "No padd binary found: run 'cargo build --release' first." >&2; exit 1; }

mkdir -p "$BIN" "$APPS" "$ICONS/scalable/apps" "$ICONS/256x256/apps"
install -m 755 "$SRC" "$BIN/padd"
install -m 644 assets/padd.svg "$ICONS/scalable/apps/padd.svg"
command -v convert >/dev/null && convert -background none assets/padd-1024.png -resize 256x256 "$ICONS/256x256/apps/padd.png" 2>/dev/null || true
install -m 644 assets/padd.desktop "$APPS/padd.desktop"
update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
echo "Padd installed: $BIN/padd"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: add $BIN to your PATH." ;; esac
