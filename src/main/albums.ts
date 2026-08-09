import { app } from 'electron'
import { readFile, writeFile, rename, mkdir } from 'node:fs/promises'
import { join, dirname } from 'node:path'
import type { AlbumFlag, ContentIdStatus } from '../shared/types'

/**
 * Per-album Content ID results.
 *
 * Kept apart from both config and stats: these are findings the user records
 * about the world, not settings and not derived numbers. Marking an album
 * `claimed` is what keeps it out of auto-play, so losing this file would
 * quietly put flagged music back on stream -- worth its own store.
 */

const WRITE_DEBOUNCE_MS = 1500

type AlbumsFile = { version: 1; albums: Record<string, AlbumFlag> }

let data: AlbumsFile = { version: 1, albums: {} }
let loaded = false
let dirty = false
let timer: NodeJS.Timeout | null = null

function albumsPath(): string {
  return join(app.getPath('userData'), 'albums.json')
}

export async function loadAlbumFlags(): Promise<void> {
  if (loaded) return
  loaded = true
  try {
    const parsed = JSON.parse(await readFile(albumsPath(), 'utf8')) as AlbumsFile
    if (parsed?.albums) data = parsed
  } catch {
    // Absent or unreadable: everything is simply untested.
  }
}

export function getAlbumFlags(): Record<string, AlbumFlag> {
  return data.albums
}

export function getAlbumStatus(album: string): ContentIdStatus {
  return data.albums[album]?.status ?? 'untested'
}

export function setAlbumFlag(album: string, status: ContentIdStatus, note?: string): AlbumFlag {
  const flag: AlbumFlag = {
    status,
    note: note?.trim() || undefined,
    checkedAt: status === 'untested' ? undefined : Date.now()
  }
  if (status === 'untested' && !flag.note) delete data.albums[album]
  else data.albums[album] = flag
  schedule()
  return flag
}

function schedule(): void {
  dirty = true
  if (timer) return
  timer = setTimeout(() => {
    timer = null
    void flushAlbumFlags()
  }, WRITE_DEBOUNCE_MS)
}

export async function flushAlbumFlags(): Promise<void> {
  if (!dirty) return
  dirty = false
  const target = albumsPath()
  const tmp = `${target}.tmp`
  try {
    await mkdir(dirname(target), { recursive: true })
    await writeFile(tmp, JSON.stringify(data, null, 2), 'utf8')
    await rename(tmp, target)
  } catch {
    dirty = true
  }
}

/** Albums the user has marked as claimed. */
export function claimedAlbums(): Set<string> {
  return new Set(
    Object.entries(data.albums)
      .filter(([, f]) => f.status === 'claimed')
      .map(([name]) => name)
  )
}
