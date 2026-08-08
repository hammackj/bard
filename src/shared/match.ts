import Fuse from 'fuse.js'
import type { Track } from './types'

/**
 * Fuse scores run 0 (perfect) to 1 (nothing).
 *
 * 0.4 is deliberately conservative. Measured against a real 585-track library
 * it tolerates single-character typos ("Ambent Gold", "Cofee Nook") and
 * rejects every out-of-library probe. At 0.5 it additionally handles
 * transpositions, but "lofi" starts matching an unrelated track -- exactly the
 * failure a request bot must not have, since a wrong match plays on stream
 * while a miss just replies "not found".
 */
export const MATCH_THRESHOLD = 0.4

export type Matcher = (query: string) => Track | null

/** What Fuse indexes. Kept here so scanning and matching cannot disagree. */
export function searchTextFor(title: string, artist: string, album: string): string {
  return `${title} ${artist} ${album}`.toLowerCase()
}

/**
 * Builds a matcher over a track list.
 *
 * Returns null rather than a weak best-effort hit: a viewer typing something
 * vague should get "not found", not whichever track scored least badly.
 */
export function createMatcher(tracks: Track[]): Matcher {
  const index = new Fuse(tracks, {
    keys: ['searchText'],
    threshold: MATCH_THRESHOLD,
    includeScore: true,
    ignoreLocation: true
  })

  return (query: string): Track | null => {
    const q = query.trim()
    if (!q) return null
    const [best] = index.search(q, { limit: 1 })
    if (!best) return null
    if (best.score !== undefined && best.score > MATCH_THRESHOLD) return null
    return best.item
  }
}
