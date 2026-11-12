#!/usr/bin/env bash
#
# Renders one MP4 per album: the album cover as a still frame over the whole
# album's audio, plus a chapter list mapping timestamps back to tracks.
#
# The point is Content ID testing. Upload an album as Private, and if YouTube
# raises a claim it reports the claimed segment as a timestamp range -- look
# that range up in the .chapters.txt and you know exactly which track tripped
# it, rather than guessing whether the whole library is a problem.
#
#   ./scripts/make-album-videos.sh --library DIR [--out DIR] [--only NAME]
#
# DIR is a folder whose subfolders are albums (e.g. ~/Music/Some Artist);
# each subfolder becomes one video. --out defaults to ~/Videos/bard-album-tests.
set -euo pipefail

library=""
outdir="$HOME/Videos/bard-album-tests"
only=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --library) library="$2"; shift 2 ;;
    --out) outdir="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done

command -v ffmpeg >/dev/null || { echo "ffmpeg not found" >&2; exit 1; }
[[ -n "$library" ]] || { echo "usage: $0 --library DIR [--out DIR] [--only NAME]" >&2; exit 1; }
[[ -d "$library" ]] || { echo "no library at $library" >&2; exit 1; }
mkdir -p "$outdir"

# A still image needs almost no bitrate, but only if the keyframe interval is
# long: at 2fps with a keyframe every 10s the video track came out at 633 kbps,
# three times the audio, purely from re-sending an unchanging frame. 1fps with
# a keyframe every five minutes drops it to a rounding error.
FPS=1
GOP=300
CRF=30
AUDIO_RATE=48000

fmt_ts() {  # seconds -> H:MM:SS or M:SS, the forms YouTube parses as chapters
  local t=${1%.*}
  local h=$((t / 3600)) m=$(((t % 3600) / 60)) s=$((t % 60))
  if ((h > 0)); then printf '%d:%02d:%02d' "$h" "$m" "$s"
  else printf '%d:%02d' "$m" "$s"; fi
}

shopt -s nullglob
made=0

for albumdir in "$library"/*/; do
  album="$(basename "$albumdir")"
  [[ -n "$only" && "$album" != "$only" ]] && continue

  mapfile -t tracks < <(find "$albumdir" -maxdepth 1 -type f \
    \( -iname '*.flac' -o -iname '*.mp3' -o -iname '*.wav' -o -iname '*.m4a' \
       -o -iname '*.ogg' -o -iname '*.opus' \) | sort -V)
  ((${#tracks[@]})) || continue

  target="$outdir/$album.mp4"
  chapters="$outdir/$album.chapters.txt"
  if [[ -f "$target" ]]; then
    echo "skip   $album (already rendered)"
    continue
  fi

  cover="$albumdir/cover.jpg"
  [[ -f "$cover" ]] || cover="$albumdir/folder.jpg"

  echo "build  $album (${#tracks[@]} tracks)"
  tmp="$(mktemp -d -t bard-album-XXXXXX)"
  trap 'rm -rf "$tmp"' EXIT

  # Normalise every track to identical parameters first.
  #
  # Required, not tidiness: several albums mix sample rates internally (one has
  # 44.1k, 48k and 96k together), and the concat demuxer refuses streams whose
  # parameters differ. Decoding each to a common format up front is what makes
  # the join possible at all.
  : > "$tmp/list.txt"
  : > "$chapters"
  offset=0
  index=0
  for track in "${tracks[@]}"; do
    index=$((index + 1))
    part="$tmp/$(printf '%04d' "$index").wav"
    ffmpeg -nostdin -v error -y -i "$track" \
      -ar "$AUDIO_RATE" -ac 2 -c:a pcm_s16le "$part"

    title="$(basename "${track%.*}" | sed -E 's/^[0-9]+[[:space:]]*-[[:space:]]*//')"
    printf '%s %s\n' "$(fmt_ts "$offset")" "$title" >> "$chapters"
    printf "file '%s'\n" "$part" >> "$tmp/list.txt"

    dur="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$part")"
    offset="$(python3 -c "print($offset + $dur)")"
  done

  ffmpeg -nostdin -v error -y -f concat -safe 0 -i "$tmp/list.txt" \
    -c:a pcm_s16le "$tmp/album.wav"

  if [[ -f "$cover" ]]; then
    ffmpeg -nostdin -v error -y \
      -loop 1 -framerate "$FPS" -i "$cover" -i "$tmp/album.wav" \
      -c:v libx264 -tune stillimage -preset veryfast -crf "$CRF" \
      -pix_fmt yuv420p -r "$FPS" -g "$GOP" \
      -vf "scale=1280:1280:force_original_aspect_ratio=decrease,pad=1280:1280:(ow-iw)/2:(oh-ih)/2" \
      -c:a aac -b:a 192k -ar "$AUDIO_RATE" -movflags +faststart -shortest "$target"
  else
    # No cover: a flat panel-coloured frame rather than failing the album.
    ffmpeg -nostdin -v error -y \
      -f lavfi -i "color=c=0x030712:s=1280x1280:r=$FPS" -i "$tmp/album.wav" \
      -c:v libx264 -tune stillimage -preset veryfast -crf "$CRF" \
      -pix_fmt yuv420p -g "$GOP" \
      -c:a aac -b:a 192k -ar "$AUDIO_RATE" -movflags +faststart -shortest "$target"
  fi

  rm -rf "$tmp"
  trap - EXIT
  made=$((made + 1))
  printf '       -> %s (%s)\n' "$(basename "$target")" "$(du -h "$target" | cut -f1)"
done

echo
echo "rendered $made album video(s) into $outdir"
echo "Each .chapters.txt pastes straight into a YouTube description, so a"
echo "Content ID claim's timestamp range names the track that caused it."
