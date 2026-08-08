import type { Track } from './types'

/** Injectable so shuffling and picking are deterministic under test. */
export type Rng = () => number

/** Album order: paths carry the track number, so path order is release order. */
export function byPathOrder(a: Track, b: Track): number {
  return a.path.localeCompare(b.path, undefined, { numeric: true, sensitivity: 'base' })
}

export function tracksInAlbum(tracks: Track[], album: string): Track[] {
  return tracks.filter((t) => t.album === album).sort(byPathOrder)
}

export function albumNames(tracks: Track[]): string[] {
  return [...new Set(tracks.map((t) => t.album))].sort()
}

/**
 * A random track, avoiding `exclude`.
 *
 * Pure random repeats itself uncomfortably often -- on a few hundred tracks
 * there is a better-than-even chance of a repeat within ~30 picks. Excluding a
 * recent window is what makes shuffle feel shuffled. If the window has grown to
 * cover everything, it is ignored rather than returning nothing.
 */
export function pickRandom(
  tracks: Track[],
  exclude: ReadonlySet<string> = new Set(),
  rng: Rng = Math.random
): Track | null {
  if (tracks.length === 0) return null
  const pool = tracks.filter((t) => !exclude.has(t.id))
  const from = pool.length > 0 ? pool : tracks
  return from[Math.floor(rng() * from.length)] ?? null
}

/** The next track on the same album, or null at the end of it. */
export function nextInAlbum(tracks: Track[], track: Track): Track | null {
  const album = tracksInAlbum(tracks, track.album)
  const i = album.findIndex((t) => t.id === track.id)
  if (i === -1 || i + 1 >= album.length) return null
  return album[i + 1]
}

/** First track of a random album other than `notThis`. */
export function randomAlbumStart(
  tracks: Track[],
  notThis?: string,
  rng: Rng = Math.random
): Track | null {
  const names = albumNames(tracks).filter((a) => a !== notThis)
  const pick = names.length ? names : albumNames(tracks)
  if (!pick.length) return null
  const album = tracksInAlbum(tracks, pick[Math.floor(rng() * pick.length)])
  return album[0] ?? null
}

/** Fisher-Yates. Returns a new array; the input is untouched. */
export function shuffled<T>(items: readonly T[], rng: Rng = Math.random): T[] {
  const next = [...items]
  for (let i = next.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1))
    ;[next[i], next[j]] = [next[j], next[i]]
  }
  return next
}
