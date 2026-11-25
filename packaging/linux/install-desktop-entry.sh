#!/bin/sh
# Add pbrt-ui to the current user's application menu, with its icon:
#   ./install-desktop-entry.sh              install, pointing at this folder
#   ./install-desktop-entry.sh --uninstall  remove the entry and the icons
# The entry stores this folder's path, so run it again after moving the folder.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
entry=$data/applications/pbrt-ui.desktop
png=$data/icons/hicolor/256x256/apps/pbrt-ui.png
svg=$data/icons/hicolor/scalable/apps/pbrt-ui.svg

refresh() {
    update-desktop-database "$data/applications" 2>/dev/null || true
    gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
}

case "${1-}" in
--uninstall)
    rm -f "$entry" "$png" "$svg"
    refresh
    echo "Removed $entry"
    exit 0
    ;;
"") ;;
*)
    echo "usage: $0 [--uninstall]" >&2
    exit 2
    ;;
esac

if [ ! -x "$dir/pbrt-ui" ]; then
    echo "pbrt-ui is not in $dir; run this script from the release folder" >&2
    exit 1
fi

case $dir in
*%*)
    # The spec's %% escape is not honoured by GLib launchers, so the entry would not start.
    echo "the folder path contains %; move the folder to a path without it" >&2
    exit 1
    ;;
esac

# Exec= quoting (Desktop Entry spec): a quoted argument escapes " ` $ \ with a
# backslash, and the string level doubles every backslash.
exec_path=$(printf '%s' "$dir/pbrt-ui" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g')

mkdir -p "$(dirname "$entry")" "$(dirname "$png")" "$(dirname "$svg")"
cp "$dir/pbrt-ui.png" "$png"
cp "$dir/pbrt-ui.svg" "$svg"
cat >"$entry" <<ENTRY
[Desktop Entry]
Type=Application
Name=pbrt-ui
Comment=Scene editor for PBRT v4
Exec="$exec_path"
Icon=pbrt-ui
Terminal=false
Categories=Graphics;3DGraphics;
StartupWMClass=pbrt-ui
ENTRY
refresh
echo "Installed $entry"
