import { app } from 'electron'
import { writeFile, rename } from 'node:fs/promises'
import { writeFileSync, renameSync } from 'node:fs'
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
let progress: PlaybackProgress = { position: 0, duration: 0, playing: false, trackId: null }
let queueLength = 0
/**
 * The port the overlay server actually bound, or null when it is not running.
 *
 * Pushed in by overlay.ts rather than read back out of it: overlay.ts already
 * imports this module for the snapshot, and reaching the other way would make
 * the pair circular.
 */
let overlayPort: number | null = null
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

/** Called by overlay.ts when the server binds or stops. */
export function setOverlayPort(port: number | null): void {
  if (port === overlayPort) return
  overlayPort = port
  emit()
}

export function snapshot(): NowPlaying {
  return {
    title: current?.track.title ?? '',
    artist: current?.track.artist ?? '',
    album: current?.track.album ?? '',
    durationSec: progress.duration || current?.track.durationSec || 0,
    positionSec: progress.position,
    isPlaying: progress.playing,
    // The bound port, not the configured one. When the configured port is
    // taken the server walks upward, and building this from config pointed the
    // overlay's cover art at a port some other process holds -- so every track
    // showed a blank cover for the whole session, and nowplaying.json handed
    // the same dead URL to anything else reading it. Null when nothing is
    // bound: there is no server to serve the art.
    artUrl:
      current?.track.hasArtwork && overlayPort !== null
        ? `http://127.0.0.1:${overlayPort}/art/${current.track.id}`
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
  if (!item) progress = { position: 0, duration: 0, playing: false, trackId: null }
  emit()
  void writeFileIfInteresting(true)
}

/**
 * Writes the stopped state, synchronously, on the way out.
 *
 * Nothing reset this file on quit, so it kept whatever was last written --
 * after a normal exit it sat at `"isPlaying": true` with the last track's
 * title indefinitely. Anything reading it, an OBS text or browser source or
 * another tool, went on showing a now-playing card for a track that stopped
 * when the app did, with no running process left to correct it.
 *
 * Synchronous on purpose. The normal path is serialised behind a promise chain
 * and before-quit does not await it, so an async write here loses the race with
 * process exit -- which is precisely the case that has to work. Same temp-file
 * and rename as the async path, so a reader never catches it half-written.
 */
export function writeStoppedNowPlaying(): void {
  if (!getConfig().overlay.writeNowPlayingJson) return
  current = null
  progress = { position: 0, duration: 0, playing: false, trackId: null }
  queueLength = 0

  const target = nowPlayingPath()
  const tmp = `${target}.tmp`
  try {
    writeFileSync(tmp, JSON.stringify(snapshot(), null, 2), 'utf8')
    renameSync(tmp, target)
  } catch {
    /* a failed overlay write must never hold up quitting */
  }
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
