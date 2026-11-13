#!/bin/bash
# Removes Bard from this Mac. Double-click it in Finder, or run it in a
# terminal. Your music files are never touched.
#
# Ships in the disk image as "Uninstall Bard.command", and inside the app at
# Bard.app/Contents/Resources/uninstall.command.

set -u

data="$HOME/Library/Application Support/bard"
state="$HOME/Library/Saved Application State/io.github.hammackj.bard.savedState"

echo "Uninstall Bard"
echo

# Quit it first, politely, then not.
if pgrep -x bard >/dev/null; then
  echo "Quitting Bard..."
  osascript -e 'tell application "Bard" to quit' >/dev/null 2>&1
  sleep 2
  pkill -x bard 2>/dev/null
fi

removed=0
for app in "/Applications/Bard.app" "$HOME/Applications/Bard.app"; do
  if [ -d "$app" ]; then
    if rm -rf "$app" 2>/dev/null || sudo rm -rf "$app"; then
      echo "Removed $app"
      removed=1
    else
      echo "Could not remove $app; drag it to the Trash instead."
    fi
  fi
done
[ "$removed" = 1 ] || echo "Bard.app was not found in Applications."

echo
echo "Bard's own data is kept unless you remove it here: settings, play"
echo "counts, loved tracks and the artwork cache, in"
echo "  $data"
echo "and the OBS password in your keychain."
read -r -p "Delete them too? [y/N] " answer
case "$answer" in
  [yY]*)
    rm -rf "$data" "$state"
    security delete-generic-password -s bard -a obs-password >/dev/null 2>&1
    echo "Deleted."
    ;;
  *)
    echo "Kept; a reinstall picks up where you left off."
    ;;
esac

echo
echo "Done. You can close this window."
