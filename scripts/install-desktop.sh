#!/usr/bin/env bash
#
# Installs a desktop launcher and icon theme entries for Bard, per-user.
#
# Packaged installers (AppImage/deb) ship their own launcher; this exists so a
# locally built or unpacked binary shows up in the application menu too.
#
#   ./scripts/install-desktop.sh [path-to-bard-binary]
#
# With no argument it uses dist/linux-unpacked/bard.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-$repo_root/dist/linux-unpacked/bard}"

if [[ ! -x "$target" ]]; then
  echo "error: no executable at $target" >&2
  echo "build one first:  npm run build && npx electron-builder --linux --dir" >&2
  exit 1
fi
target="$(cd "$(dirname "$target")" && pwd)/$(basename "$target")"

apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons_dir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
mkdir -p "$apps_dir"

installed=0
for size in 16 22 24 32 48 64 128 256 512; do
  src="$repo_root/resources/icons/icon-${size}.png"
  [[ -f "$src" ]] || continue
  mkdir -p "$icons_dir/${size}x${size}/apps"
  install -m 644 "$src" "$icons_dir/${size}x${size}/apps/bard.png"
  installed=$((installed + 1))
done

# Scalable entry too, so the icon stays crisp on HiDPI panels.
if [[ -f "$repo_root/resources/icon.svg" ]]; then
  mkdir -p "$icons_dir/scalable/apps"
  install -m 644 "$repo_root/resources/icon.svg" "$icons_dir/scalable/apps/bard.svg"
fi

desktop_file="$apps_dir/bard.desktop"
cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=Bard
GenericName=Music Player
Comment=Local music player with Twitch song requests
Exec=$target %U
Icon=bard
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=music;player;twitch;stream;obs;
StartupNotify=true
StartupWMClass=Bard
EOF
chmod 644 "$desktop_file"

# Best effort: these are absent on some systems and their failure is harmless.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps_dir" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -qtf "$icons_dir" || true

echo "installed launcher : $desktop_file"
echo "        pointing at: $target"
echo "        icon sizes : $installed (+ scalable svg)"
echo
echo "If it does not appear immediately, log out and back in, or run:"
echo "  update-desktop-database \"$apps_dir\""
