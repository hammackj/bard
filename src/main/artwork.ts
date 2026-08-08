import { app, nativeImage } from 'electron'
import { mkdir, writeFile, readFile, access } from 'node:fs/promises'
import { join } from 'node:path'

/** Longest edge, in px, that a cached cover is stored at. */
const MAX_EDGE = 512

let dir: string | null = null

export function artworkDir(): string {
  if (!dir) dir = join(app.getPath('userData'), 'artwork')
  return dir
}

export async function ensureArtworkDir(): Promise<void> {
  await mkdir(artworkDir(), { recursive: true })
}

function cachePath(trackId: string): string {
  return join(artworkDir(), `${trackId}.jpg`)
}

export async function hasArtwork(trackId: string): Promise<boolean> {
  try {
    await access(cachePath(trackId))
    return true
  } catch {
    return false
  }
}

/**
 * Downscales an embedded cover and writes it to the artwork cache.
 *
 * Cover art deliberately never travels inside a Track -- a 1.2 MB JPEG times
 * several hundred tracks is not something to hold in memory or push over IPC.
 * Only the id crosses the boundary; the renderer asks for bytes when it needs
 * them.
 *
 * Uses Electron's nativeImage rather than pulling in a native image library.
 */
export async function saveArtwork(trackId: string, data: Uint8Array): Promise<boolean> {
  try {
    const img = nativeImage.createFromBuffer(Buffer.from(data))
    if (img.isEmpty()) return false

    const { width, height } = img.getSize()
    const longest = Math.max(width, height)
    const scaled =
      longest > MAX_EDGE
        ? img.resize(
            width >= height
              ? { width: MAX_EDGE, quality: 'good' }
              : { height: MAX_EDGE, quality: 'good' }
          )
        : img

    await writeFile(cachePath(trackId), scaled.toJPEG(88))
    return true
  } catch {
    return false
  }
}

/** Data URL for a cached cover, or null when the track has none. */
export async function getArtwork(trackId: string): Promise<string | null> {
  try {
    const buf = await readFile(cachePath(trackId))
    return `data:image/jpeg;base64,${buf.toString('base64')}`
  } catch {
    return null
  }
}
