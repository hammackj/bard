import { app } from 'electron'
import { readFile, writeFile, rename, mkdir } from 'node:fs/promises'
import { join, dirname } from 'node:path'
import type { TrackStats } from '../shared/types'

/**
 * Play counts and history.
 *
 * Deliberately a separate file from config.json. Stats change on every track
 * while settings change on user action; mixing them would rewrite the settings
 * file constantly and make a corrupt write take the settings down with it.
 *
 * Writes are debounced and atomic (temp + rename), so a crash mid-track costs
 * at most the last few seconds of counting rather than the whole history.
 */

const WRITE_DEBOUNCE_MS = 3000

/** A track counts as played once it has been heard this far. */
export const PLAY_THRESHOLD_SEC = 30
/** ...or this proportion of it, whichever comes first, for short tracks. */
export const PLAY_THRESHOLD_RATIO = 0.5

type StatsFile = { version: 1; tracks: Record<string, TrackStats> }

let stats: StatsFile = { version: 1, tracks: {} }
let loaded = false
let dirty = false
let timer: NodeJS.Timeout | null = null

function statsPath(): string {
  return join(app.getPath('userData'), 'stats.json')
}

export async function loadStats(): Promise<void> {
  if (loaded) return
  loaded = true
  try {
    const parsed = JSON.parse(await readFile(statsPath(), 'utf8')) as StatsFile
    if (parsed && typeof parsed === 'object' && parsed.tracks) stats = parsed
  } catch {
    // Absent or unreadable: start fresh rather than refusing to play music.
  }
}

export function getStats(): Record<string, TrackStats> {
  return stats.tracks
}

export function getTrackStats(trackId: string): TrackStats {
  return stats.tracks[trackId] ?? { plays: 0, skips: 0 }
}

function schedule(): void {
  dirty = true
  if (timer) return
  timer = setTimeout(() => {
    timer = null
    void flushStats()
  }, WRITE_DEBOUNCE_MS)
}

export async function flushStats(): Promise<void> {
  if (!dirty) return
  dirty = false
  const target = statsPath()
  const tmp = `${target}.tmp`
  try {
    await mkdir(dirname(target), { recursive: true })
    await writeFile(tmp, JSON.stringify(stats), 'utf8')
    await rename(tmp, target)
  } catch {
    // Losing play counts must never interrupt playback.
    dirty = true
  }
}

export function recordPlay(trackId: string, at = Date.now()): TrackStats {
  const current = getTrackStats(trackId)
  const next: TrackStats = { ...current, plays: current.plays + 1, lastPlayedAt: at }
  stats.tracks[trackId] = next
  schedule()
  return next
}

/**
 * A track abandoned before the threshold.
 *
 * Counted separately rather than not at all: a track you skip every time is a
 * fact worth surfacing, and lumping it in with plays would make the most-played
 * list meaningless.
 */
export function recordSkip(trackId: string, at = Date.now()): TrackStats {
  const current = getTrackStats(trackId)
  const next: TrackStats = { ...current, skips: current.skips + 1, lastSkippedAt: at }
  stats.tracks[trackId] = next
  schedule()
  return next
}

export function clearStats(): void {
  stats = { version: 1, tracks: {} }
  schedule()
}
