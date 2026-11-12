#!/usr/bin/env bash
#
# Renders resources/screenshot.webp, the README's screenshot, headlessly.
#
# The library is made up -- invented albums and artists with generated
# gradient covers -- so the image carries nobody's music or artwork. It goes
# through the snapshot tool (the real views, no window on screen) with HOME
# pointed at a temp dir, so the real ~/.config/bard is never read.
#
#   ./scripts/screenshot.sh [out.webp]
#
# Needs ImageMagick 7 (`magick`) and python3.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/resources/screenshot.webp}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cargo build --release -p bard --features snapshot --bin bard-snapshot --manifest-path "$root/Cargo.toml"

python3 -I - "$work" <<'EOF'
import hashlib, json, os, subprocess, sys

work = sys.argv[1]
art = os.path.join(work, "home/.config/bard/artwork")
os.makedirs(art)

# (album, artist, tracks, cover gradient)
albums = [
    ("Night Transit", "Marlow Fields", ["Last Train Home", "Platform Nine", "Sodium Lights", "Window Seat", "Terminus"], ("#0e7490", "#1e1b4b")),
    ("Paper Lanterns", "Kite & Comet", ["Festival Smoke", "Folded Wishes", "River Glow", "Small Hours"], ("#b45309", "#3b0764")),
    ("Slow Tide", "Harbour Static", ["Low Water", "Saltglass", "Breakwater", "Drift"], ("#0f766e", "#082f49")),
    ("Copper Hours", "June Atlas", ["Kettle Steam", "Warm Static", "Afternoon Dust", "Brass", "Shortwave"], ("#c2410c", "#422006")),
    ("Greenhouse", "Fern Theory", ["Condensation", "Terracotta", "Grow Lights", "Morning Mist"], ("#15803d", "#052e16")),
    ("Low Orbit", "Satellite Choir", ["Apogee", "Ground Control", "Weightless", "Re-entry"], ("#6d28d9", "#020617")),
]
durations = [154, 187, 203, 171, 226, 198, 142, 215, 181, 163]

tracks, stats, k = [], {}, 0
for i, (album, artist, titles, (top, bottom)) in enumerate(albums):
    cover = os.path.join(work, f"cover{i}.jpg")
    subprocess.run([
        "magick", "-size", "600x600", f"gradient:{top}-{bottom}",
        "(", "-size", "600x600", "radial-gradient:rgba(255,255,255,0.35)-rgba(255,255,255,0)", ")",
        "-geometry", f"+{(i * 97) % 300 - 150}+{(i * 53) % 300 - 150}", "-compose", "screen", "-composite",
        "-depth", "16", "-blur", "0x18", "+noise", "Gaussian", "-attenuate", "0.05", "-quality", "92", cover,
    ], check=True)
    for n, title in enumerate(titles, 1):
        path = f"/home/user/Music/{artist}/{album}/{n:02} - {title}.flac"
        tid = hashlib.sha1(path.encode()).hexdigest()[:16]
        tracks.append({
            "id": tid, "path": path, "title": title, "artist": artist, "album": album,
            "durationSec": float(durations[k % len(durations)]), "hasArtwork": True,
            "searchText": f"{title} {artist} {album}".lower(),
        })
        stats[tid] = {"plays": (k * 7) % 23 + 1, "skips": k % 3}
        with open(cover, "rb") as src, open(os.path.join(art, f"{tid}.jpg"), "wb") as dst:
            dst.write(src.read())
        k += 1

fixture = {
    "tracks": tracks,
    "config": {"libraryPaths": ["/home/user/Music"]},
    "stats": stats,
    "loved": [tracks[0]["id"], tracks[7]["id"], tracks[12]["id"]],
    "albumFlags": {a: {"status": "safe"} for a in ["Night Transit", "Slow Tide", "Greenhouse", "Low Orbit"]},
    "now": "Terminus",
    "queue": ["Platform Nine", "Saltglass", "Kettle Steam", "Grow Lights", "Weightless", "Window Seat", "Breakwater"],
    "position": 83.0,
    "playing": True,
}
with open(os.path.join(work, "fixture.json"), "w") as f:
    json.dump(fixture, f)
EOF

HOME="$work/home" "$root/target/release/bard-snapshot" "$work/fixture.json" "$work/shot.png" 1280x800
magick "$work/shot.png" -resize 1600x -strip -quality 90 "$out"
echo "wrote $out"
