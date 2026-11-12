#!/usr/bin/env bash
#
# Installs Bard per-user: the release binary, a desktop launcher and the icon
# theme entries.
#
#   ./scripts/install-desktop.sh [path-to-bard-binary] [--desktop]
#
# With no argument it builds the release binary and installs it to
# ~/.local/bin/bard, so the launcher keeps working while target/ is rebuilt.
# Run it again to update the installed app. A path skips the build and points
# the launcher at that binary instead.
# --desktop also drops a copy on the desktop itself.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

want_desktop=0
target=""
for arg in "$@"; do
  case "$arg" in
    --desktop) want_desktop=1 ;;
    *) target="$arg" ;;
  esac
done

if [[ -z "$target" ]]; then
  (cd "$repo_root" && cargo build --release -p bard)
  bin_dir="$HOME/.local/bin"
  mkdir -p "$bin_dir"
  # Copied to a temporary name and renamed, so a running Bard keeps its
  # binary and the next launch gets the new one.
  install -m 755 "$repo_root/target/release/bard" "$bin_dir/.bard.new"
  mv -f "$bin_dir/.bard.new" "$bin_dir/bard"
  target="$bin_dir/bard"
fi

if [[ ! -x "$target" ]]; then
  echo "error: no executable at $target" >&2
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

# The Icon Theme Specification wants an index.theme in every theme directory,
# and we were writing icons into one without it. It works on a normal desktop
# only because /usr/share/icons/hicolor/index.theme supplies the directory list
# and Qt/GTK merge a theme across XDG base dirs -- on a system with no
# system-level hicolor index, or under a stricter loader, these icons would not
# resolve.
#
# Built from what is actually on disk rather than from the sizes written above:
# other applications install into the same per-user hicolor tree, and an index
# listing only Bard's directories would hide theirs.
write_icon_index() {
  local dirs=() sections="" path sub ctx
  for path in "$icons_dir"/*/*/; do
    [[ -d "$path" ]] || continue
    ctx="$(basename "$path")"
    sub="$(basename "$(dirname "$path")")"
    if [[ "$sub" =~ ^([0-9]+)x[0-9]+$ ]]; then
      dirs+=("$sub/$ctx")
      sections+="[$sub/$ctx]"$'\n'"Size=${BASH_REMATCH[1]}"$'\n'"Type=Threshold"$'\n\n'
    elif [[ "$sub" == scalable ]]; then
      dirs+=("$sub/$ctx")
      sections+="[$sub/$ctx]"$'\n'"Size=48"$'\n'"MinSize=8"$'\n'"MaxSize=512"$'\n'"Type=Scalable"$'\n\n'
    fi
  done
  (( ${#dirs[@]} )) || return 0

  local joined
  joined="$(IFS=,; echo "${dirs[*]}")"
  # Context is deliberately omitted: it is optional in the spec, and guessing it
  # for directories other applications created would be worse than leaving it out.
  cat > "$icons_dir/index.theme" <<EOF
[Icon Theme]
Name=Hicolor
Comment=Fallback icon theme
Directories=$joined

$sections
EOF
  chmod 644 "$icons_dir/index.theme"
}
write_icon_index

desktop_file="$apps_dir/bard.desktop"
cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=Bard
GenericName=Music Player
Comment=Local music player for a curated library
Exec=$target %U
Icon=bard
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=music;player;stream;obs;
StartupNotify=true
StartupWMClass=bard
EOF
chmod 644 "$desktop_file"

# Best effort: these are absent on some systems and their failure is harmless.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps_dir" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -qtf "$icons_dir" || true

if [[ "$want_desktop" == 1 ]]; then
  # Respect a relocated or localised desktop folder rather than assuming ~/Desktop.
  desktop_dir="$HOME/Desktop"
  if [[ -f "${XDG_CONFIG_HOME:-$HOME/.config}/user-dirs.dirs" ]]; then
    # shellcheck disable=SC1091
    eval "$(grep -E '^XDG_DESKTOP_DIR=' "${XDG_CONFIG_HOME:-$HOME/.config}/user-dirs.dirs" || true)"
    [[ -n "${XDG_DESKTOP_DIR:-}" ]] && desktop_dir="$XDG_DESKTOP_DIR"
  fi

  if [[ -d "$desktop_dir" ]]; then
    install -m 755 "$desktop_file" "$desktop_dir/bard.desktop"
    # KDE and GNOME both refuse to launch a desktop file they do not consider
    # trusted; the executable bit is what marks it so on KDE. GNOME
    # additionally wants this attribute, and setting it is harmless elsewhere.
    gio set "$desktop_dir/bard.desktop" metadata::trusted true 2>/dev/null || true
    echo "desktop shortcut   : $desktop_dir/bard.desktop"
  else
    echo "warning: no desktop directory at $desktop_dir; skipped" >&2
  fi
fi

echo "installed launcher : $desktop_file"
echo "        pointing at: $target"
echo "        icon sizes : $installed (+ scalable svg)"
echo
echo "If it does not appear immediately, log out and back in, or run:"
echo "  update-desktop-database \"$apps_dir\""
