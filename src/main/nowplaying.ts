import { app } from 'electron'
import { writeFile, rename } from 'node:fs/promises'
import { join } from 'node:path'
import type { NowPlaying, PlaybackProgress, QueueItem } from '../shared/types'
import { getConfig } from './config'

/**
 * Assembles the now-playing payload and keeps nowplaying.json current.
 *
 * Progress arrives ~4x/second. The overlay wants all of it, but the file does
 * not -- writing on every tick is four disk writes a second for an eight-hour
 * stream. The file is written only when something a viewer would notice
 * changes: the track, or play/pause.
 */

type Listener = (snapshot: NowPlaying) => void

let listeners: Listener[] = []
let current: QueueItem | null = null
let progress: PlaybackProgress = { position: 0, duration: 0, playing: false }
let queueLength = 0
let lastWritten = ''
let writing: Promise<void> = Promise.resolve()

export function nowPlayingPath(): string {
  return join(app.getPath('userData'), 'nowplaying.json')
}

export function onNowPlayingChange(fn: Listener): () => void {
  listeners.push(fn)
  return () => {
    listeners = listeners.filter((l) => l !== fn)
  }
}

export function snapshot(): NowPlaying {
  const port = getConfig().overlay.port
  return {
    title: current?.track.title ?? '',
    artist: current?.track.artist ?? '',
    album: current?.track.album ?? '',
    durationSec: progress.duration || current?.track.durationSec || 0,
    positionSec: progress.position,
    requestedBy: current?.requestedBy,
    isPlaying: progress.playing,
    artUrl: current?.track.hasArtwork
      ? `http://127.0.0.1:${port}/art/${current.track.id}`
      : null,
    queueLength,
    updatedAt: new Date().toISOString()
  }
}

function emit(): void {
  const snap = snapshot()
  for (const l of listeners) l(snap)
}

/** Track changed, or playback stopped entirely. */
export function setNowPlaying(item: QueueItem | null): void {
  current = item
  if (!item) progress = { position: 0, duration: 0, playing: false }
  emit()
  void writeFileIfInteresting(true)
}

export function setQueueLength(n: number): void {
  if (n === queueLength) return
  queueLength = n
  emit()
}

export function setProgress(p: PlaybackProgress): void {
  const playStateChanged = p.playing !== progress.playing
  progress = p
  emit()
  if (playStateChanged) void writeFileIfInteresting(false)
}

/**
 * Writes atomically -- temp file then rename -- so OBS never reads a
 * half-written file and shows a torn overlay.
 */
async function writeFileIfInteresting(force: boolean): Promise<void> {
  if (!getConfig().overlay.writeNowPlayingJson) return
  const snap = snapshot()
  // positionSec/updatedAt always differ; key off what a viewer would notice.
  const key = `${snap.title}|${snap.artist}|${snap.isPlaying}|${snap.queueLength}`
  if (!force && key === lastWritten) return
  lastWritten = key

  const target = nowPlayingPath()
  const tmp = `${target}.tmp`
  const body = JSON.stringify(snap, null, 2)
  // Serialise writes so two rapid changes cannot interleave on the temp file.
  writing = writing
    .then(async () => {
      await writeFile(tmp, body, 'utf8')
      await rename(tmp, target)
    })
    .catch(() => {
      /* a failed overlay file write must never break playback */
    })
  return writing
}
