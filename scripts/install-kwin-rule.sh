#!/usr/bin/env bash
#
# Pins Bard's now-playing preview to a screen corner under KWin (Plasma).
#
# Wayland compositors own window placement -- Electron's setPosition is a
# no-op there, verified. A KWin window rule is the native way to place a
# window, and unlike forcing XWayland it does not compromise the rest of the
# app.
#
#   ./scripts/install-kwin-rule.sh [--corner bottom-right] [--screen DP-3]
#                                  [--margin 16] [--remove]
#
# Screen geometry comes from kscreen-doctor. If a panel overlaps the chosen
# corner, raise --margin past its thickness.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rules_file="${XDG_CONFIG_HOME:-$HOME/.config}/kwinrulesrc"

corner="bottom-right"
screen_name=""
margin=16
remove=0
# Substring of the BrowserWindow title in src/main/trayPopup.ts. A substring is
# used rather than the full caption because exact matching is brittle -- the
# caption carries an em dash and any drift breaks the rule silently.
window_title="now playing"
# Stable id so re-running updates the rule instead of stacking duplicates.
rule_id="bard-nowplaying-preview"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --corner) corner="$2"; shift 2 ;;
    --screen) screen_name="$2"; shift 2 ;;
    --margin) margin="$2"; shift 2 ;;
    --remove) remove=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

reload_kwin() {
  qdbus6 org.kde.KWin /KWin reconfigure 2>/dev/null ||
    qdbus org.kde.KWin /KWin reconfigure 2>/dev/null ||
    echo "note: could not signal KWin; log out and back in to apply" >&2
}

python_edit() {
  python3 - "$@" <<'PYEOF'
import configparser, os, shutil, sys

path, rule_id, action = sys.argv[1], sys.argv[2], sys.argv[3]

# KWin's file is INI-ish but keys are case-sensitive, which configparser
# lowercases by default.
cp = configparser.RawConfigParser()
cp.optionxform = str
if os.path.exists(path):
    cp.read(path)
    shutil.copy2(path, path + '.bard-backup')

if not cp.has_section('General'):
    cp.add_section('General')

existing = [r for r in cp.get('General', 'rules', fallback='').split(',') if r]

if action == 'remove':
    if cp.has_section(rule_id):
        cp.remove_section(rule_id)
    existing = [r for r in existing if r != rule_id]
else:
    title, x, y = sys.argv[4], sys.argv[5], sys.argv[6]
    if not cp.has_section(rule_id):
        cp.add_section(rule_id)
    cp.set(rule_id, 'Description', 'Bard now-playing preview')
    cp.set(rule_id, 'title', title)
    cp.set(rule_id, 'titlematch', '2')          # 2 = substring match
    cp.set(rule_id, 'position', f'{x},{y}')
    cp.set(rule_id, 'positionrule', '2')        # 2 = Force
    if rule_id not in existing:
        existing.append(rule_id)

cp.set('General', 'rules', ','.join(existing))
cp.set('General', 'count', str(len(existing)))

with open(path, 'w') as fh:
    cp.write(fh, space_around_delimiters=False)
print(f'wrote {path} ({len(existing)} rule(s))')
PYEOF
}

if [[ "$remove" == 1 ]]; then
  python_edit "$rules_file" "$rule_id" remove
  reload_kwin
  echo "removed the Bard preview rule"
  exit 0
fi

# --- work out where the corner actually is -------------------------------
# kscreen-doctor is the source of truth here. Electron's screen API only
# reports a single output under Wayland (verified: one of four), so asking the
# app for geometry would silently place the window on the wrong monitor.
if ! command -v kscreen-doctor >/dev/null 2>&1; then
  echo "error: kscreen-doctor not found; this script targets KDE Plasma" >&2
  exit 1
fi

screens="$(kscreen-doctor -o 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | awk '
  /^Output:/  { name=$3 }
  /priority/  { pri=$2 }
  /^\tGeometry:/ { print name, $2, $3, pri }
')"

if [[ -z "$screens" ]]; then
  echo "error: could not read screen geometry from kscreen-doctor" >&2
  exit 1
fi

read -r x y chosen <<<"$(
  CORNER="$corner" SCREEN="$screen_name" MARGIN="$margin" \
  python3 -c '
import os, sys
rows = [l.split() for l in sys.argv[1].strip().splitlines()]
want, corner, margin = os.environ["SCREEN"], os.environ["CORNER"], int(os.environ["MARGIN"])
W, H = 470, 208

screens = []
for name, pos, size, pri in rows:
    px, py = (int(v) for v in pos.split(","))
    sw, sh = (int(v) for v in size.split("x"))
    screens.append({"name": name, "x": px, "y": py, "w": sw, "h": sh, "pri": int(pri)})

if want:
    pick = next((s for s in screens if s["name"] == want), None)
    if pick is None:
        sys.exit("no screen named %r; saw: %s" % (want, ", ".join(s["name"] for s in screens)))
else:
    # kscreen priority 1 is the primary output.
    pick = min(screens, key=lambda s: s["pri"])

right, bottom = corner.endswith("right"), corner.startswith("bottom")
x = pick["x"] + pick["w"] - W - margin if right else pick["x"] + margin
y = pick["y"] + pick["h"] - H - margin if bottom else pick["y"] + margin
print(x, y, pick["name"])
' "$screens"
)"

python_edit "$rules_file" "$rule_id" set "$window_title" "$x" "$y"
reload_kwin

echo
echo "screen   : $chosen"
echo "corner   : $corner (margin ${margin}px)"
echo "position : ${x},${y}"
echo "matching : title = \"$window_title\""
echo
echo "A backup of the previous rules is at ${rules_file}.bard-backup"
echo "Undo with: $0 --remove"
