#!/usr/bin/env python3
"""
Compute and write ReplayGain 2.0 tags across a FLAC library.

Why not `metaflac --add-replay-gain`: it refuses a batch whose files differ in
sample rate, and 8 of the 20 albums here mix 44.1k with 48k (one also has
96k). It would simply error out on exactly the albums that need doing.

So loudness is measured with ffmpeg's ebur128 filter, which does not care, and
the tags are written with metaflac afterwards.

    ./scripts/replaygain.py [--library DIR] [--target -18] [--jobs N] [--force]
"""
import argparse
import concurrent.futures as cf
import math
import os
import re
import subprocess
import sys

# ReplayGain 2.0 reference level.
DEFAULT_TARGET_LUFS = -18.0
AUDIO_EXT = {'.flac'}

INTEGRATED = re.compile(r'^\s*I:\s*(-?\d+(?:\.\d+)?)\s*LUFS', re.M)
TRUE_PEAK = re.compile(r'^\s*Peak:\s*(-?\d+(?:\.\d+)?)\s*dBFS', re.M)


def measure(path):
    """Integrated loudness (LUFS), true peak (dBFS) and duration for one file."""
    proc = subprocess.run(
        ['ffmpeg', '-nostdin', '-v', 'info', '-i', path,
         '-filter:a', 'ebur128=peak=true', '-f', 'null', '-'],
        capture_output=True, text=True)
    summary = proc.stderr.split('Summary:')[-1]
    lufs = INTEGRATED.search(summary)
    peak = TRUE_PEAK.search(summary)
    dur = subprocess.run(
        ['ffprobe', '-v', 'error', '-show_entries', 'format=duration',
         '-of', 'csv=p=0', path], capture_output=True, text=True).stdout.strip()
    if not lufs:
        return None
    return {
        'path': path,
        'lufs': float(lufs.group(1)),
        # dBFS -> linear, which is the form ReplayGain peak tags use.
        'peak': 10 ** (float(peak.group(1)) / 20) if peak else 1.0,
        'duration': float(dur) if dur else 0.0,
    }


def album_loudness(tracks):
    """
    Energy-weighted mean loudness for an album.

    Averaging the dB values directly would be wrong -- loudness is logarithmic,
    so a quiet track would drag the mean down far more than it should. This
    converts back to energy, weights by duration, and returns to dB.
    """
    total_energy = sum(t['duration'] * 10 ** (t['lufs'] / 10) for t in tracks)
    total_time = sum(t['duration'] for t in tracks)
    if total_time <= 0 or total_energy <= 0:
        return None
    return 10 * math.log10(total_energy / total_time)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--library', default=os.path.expanduser('~/Music/Lofi/Harris Heller'))
    ap.add_argument('--target', type=float, default=DEFAULT_TARGET_LUFS)
    ap.add_argument('--jobs', type=int, default=max(2, (os.cpu_count() or 4) - 2))
    ap.add_argument('--force', action='store_true', help='retag files that already have tags')
    args = ap.parse_args()

    albums = {}
    for root, _, files in os.walk(args.library):
        for name in sorted(files):
            if os.path.splitext(name)[1].lower() in AUDIO_EXT:
                albums.setdefault(os.path.basename(root), []).append(os.path.join(root, name))
    if not albums:
        sys.exit(f'no FLAC files under {args.library}')

    total = sum(len(v) for v in albums.values())
    print(f'{total} tracks across {len(albums)} albums, target {args.target} LUFS')

    done = 0
    for album, paths in sorted(albums.items()):
        if not args.force:
            tagged = subprocess.run(['metaflac', '--export-tags-to=-', paths[0]],
                                    capture_output=True, text=True).stdout
            if 'REPLAYGAIN_TRACK_GAIN' in tagged.upper():
                print(f'skip  {album} (already tagged)')
                done += len(paths)
                continue

        with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            measured = [m for m in ex.map(measure, paths) if m]
        if not measured:
            print(f'WARN  {album}: nothing measurable', file=sys.stderr)
            continue

        alb_lufs = album_loudness(measured)
        alb_gain = args.target - alb_lufs
        alb_peak = max(m['peak'] for m in measured)

        for m in measured:
            gain = args.target - m['lufs']
            subprocess.run([
                'metaflac', '--preserve-modtime',
                '--remove-tag=REPLAYGAIN_TRACK_GAIN',
                '--remove-tag=REPLAYGAIN_TRACK_PEAK',
                '--remove-tag=REPLAYGAIN_ALBUM_GAIN',
                '--remove-tag=REPLAYGAIN_ALBUM_PEAK',
                '--remove-tag=REPLAYGAIN_REFERENCE_LOUDNESS',
                f'--set-tag=REPLAYGAIN_TRACK_GAIN={gain:+.2f} dB',
                f'--set-tag=REPLAYGAIN_TRACK_PEAK={m["peak"]:.6f}',
                f'--set-tag=REPLAYGAIN_ALBUM_GAIN={alb_gain:+.2f} dB',
                f'--set-tag=REPLAYGAIN_ALBUM_PEAK={alb_peak:.6f}',
                f'--set-tag=REPLAYGAIN_REFERENCE_LOUDNESS={abs(args.target):.2f} LUFS',
                m['path'],
            ], check=False)

        done += len(measured)
        spread = max(m['lufs'] for m in measured) - min(m['lufs'] for m in measured)
        print(f'ok    {album:24} album {alb_lufs:6.1f} LUFS  gain {alb_gain:+5.2f} dB  '
              f'spread {spread:4.1f} dB  ({done}/{total})')

    print('\ndone. Tracks now carry REPLAYGAIN_* tags; Bard applies them on playback.')


if __name__ == '__main__':
    main()
