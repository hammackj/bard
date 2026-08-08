import fg from 'fast-glob'
import Fuse from 'fuse.js'
import { createHash } from 'node:crypto'
import { basename, extname, resolve } from 'node:path'
import { parseFile } from 'music-metadata'
import type { ScanProgress, Track } from '../shared/types'
import { ensureArtworkDir, hasArtwork, saveArtwork } from './artwork'

export const AUDIO_EXTENSIONS = ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'opus'] as const

/** Parsing many files at once exhausts file handles; a small pool is plenty. */
const PARSE_CONCURRENCY = 8

/** Fuse scores run 0 (perfect) to 1 (nothing). Above this we treat it as no match. */
const MATCH_THRESHOLD = 0.4

let tracks: Track[] = []
let index: Fuse<Track> | null = null
const byId = new Map<string, Track>()

export function trackId(path: string): string {
  return createHash('sha1').update(resolve(path)).digest('hex').slice(0, 16)
}

export function allTracks(): Track[] {
  return tracks
}

export function getTrackById(id: string): Track | undefined {
  return byId.get(id)
}

export async function scanLibrary(
  roots: string[],
  onProgress?: (p: ScanProgress) => void
): Promise<Track[]> {
  await ensureArtworkDir()

  const files = await findAudioFiles(roots)
  const total = files.length
  const found: Track[] = []
  let scanned = 0

  // Fixed-size worker pool pulling from a shared cursor.
  let cursor = 0
  const worker = async (): Promise<void> => {
    for (;;) {
      const i = cursor++
      if (i >= files.length) return
      const path = files[i]
      const track = await readTrack(path)
      scanned++
      if (track) found.push(track)
      onProgress?.({ scanned, total, currentPath: path })
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(PARSE_CONCURRENCY, files.length) }, () => worker())
  )

  found.sort(byPathOrder)
  setTracks(found)
  return found
}

async function findAudioFiles(roots: string[]): Promise<string[]> {
  const seen = new Set<string>()
  for (const root of roots) {
    const matches = await fg(`**/*.{${AUDIO_EXTENSIONS.join(',')}}`, {
      cwd: root,
      absolute: true,
      onlyFiles: true,
      caseSensitiveMatch: false,
      followSymbolicLinks: false,
      suppressErrors: true
    })
    for (const m of matches) seen.add(m)
  }
  return [...seen]
}

async function readTrack(path: string): Promise<Track | null> {
  const id = trackId(path)
  try {
    const meta = await parseFile(path, { duration: true, skipCovers: false })
    const common = meta.common

    // Cache the cover once. A rescan of an unchanged library skips the decode.
    let artwork = await hasArtwork(id)
    const picture = common.picture?.[0]
    if (!artwork && picture?.data) {
      artwork = await saveArtwork(id, picture.data)
    }

    const title = clean(common.title) ?? basename(path, extname(path))
    const artist = clean(common.artist) ?? clean(common.albumartist) ?? 'Unknown Artist'
    const album = clean(common.album) ?? 'Unknown Album'

    return {
      id,
      path,
      title,
      artist,
      album,
      durationSec: meta.format.duration ?? 0,
      hasArtwork: artwork,
      searchText: `${title} ${artist} ${album}`.toLowerCase()
    }
  } catch {
    // An unreadable or malformed file should not abort the whole scan; fall
    // back to what the path alone tells us.
    return {
      id,
      path,
      title: basename(path, extname(path)),
      artist: 'Unknown Artist',
      album: 'Unknown Album',
      durationSec: 0,
      hasArtwork: await hasArtwork(id),
      searchText: basename(path, extname(path)).toLowerCase()
    }
  }
}

function clean(v: string | undefined): string | undefined {
  const t = v?.trim()
  return t ? t : undefined
}

function byPathOrder(a: Track, b: Track): number {
  return a.path.localeCompare(b.path, undefined, { numeric: true, sensitivity: 'base' })
}

function setTracks(next: Track[]): void {
  tracks = next
  byId.clear()
  for (const t of next) byId.set(t.id, t)
  index = new Fuse(next, {
    keys: ['searchText'],
    threshold: MATCH_THRESHOLD,
    includeScore: true,
    ignoreLocation: true
  })
}

/**
 * Best match for a chat request, or null.
 *
 * Returning null on a weak match is deliberate: a viewer typing "!sr lofi"
 * should get a "not found" reply rather than whichever track happened to score
 * least badly.
 */
export function matchRequest(query: string): Track | null {
  const q = query.trim()
  if (!q || !index) return null
  const [best] = index.search(q, { limit: 1 })
  if (!best) return null
  if (best.score !== undefined && best.score > MATCH_THRESHOLD) return null
  return best.item
}
