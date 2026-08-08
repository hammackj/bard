import { describe, expect, it } from 'vitest'
import {
  albumNames,
  nextInAlbum,
  pickRandom,
  randomAlbumStart,
  shuffled,
  tracksInAlbum
} from '../src/shared/playlist'
import type { Track } from '../src/shared/types'

const make = (id: string, album: string, n: number): Track => ({
  id,
  // Track order lives in the filename, which is why album order is path order.
  path: `/music/${album}/${String(n).padStart(2, '0')} - ${id}.flac`,
  title: id,
  artist: 'Harris Heller',
  album,
  durationSec: 100,
  hasArtwork: true,
  searchText: id
})

const TRACKS: Track[] = [
  make('gold3', 'Gold', 3),
  make('gold1', 'Gold', 1),
  make('gold2', 'Gold', 2),
  make('rain1', 'Rain', 1),
  make('rain2', 'Rain', 2)
]

/** Deterministic rng returning each value in turn, then repeating the last. */
const seq = (...values: number[]): (() => number) => {
  let i = 0
  return () => values[Math.min(i++, values.length - 1)]
}

describe('tracksInAlbum', () => {
  it('returns album tracks in release order regardless of input order', () => {
    expect(tracksInAlbum(TRACKS, 'Gold').map((t) => t.id)).toEqual(['gold1', 'gold2', 'gold3'])
  })

  it('returns an empty array for an unknown album', () => {
    expect(tracksInAlbum(TRACKS, 'Nope')).toEqual([])
  })
})

describe('albumNames', () => {
  it('lists each album once, sorted', () => {
    expect(albumNames(TRACKS)).toEqual(['Gold', 'Rain'])
  })
})

describe('nextInAlbum', () => {
  it('advances through the album', () => {
    const first = tracksInAlbum(TRACKS, 'Gold')[0]
    expect(nextInAlbum(TRACKS, first)?.id).toBe('gold2')
  })

  it('returns null at the end of the album', () => {
    const last = tracksInAlbum(TRACKS, 'Gold')[2]
    expect(nextInAlbum(TRACKS, last)).toBeNull()
  })

  it('returns null for a track not in the library', () => {
    expect(nextInAlbum(TRACKS, make('ghost', 'Gold', 9))).toBeNull()
  })
})

describe('randomAlbumStart', () => {
  it('starts at the first track of an album', () => {
    expect(randomAlbumStart(TRACKS, undefined, seq(0))?.id).toBe('gold1')
  })

  it('avoids the album it was given', () => {
    // Only Rain remains once Gold is excluded.
    expect(randomAlbumStart(TRACKS, 'Gold', seq(0))?.id).toBe('rain1')
  })

  it('falls back to any album when excluding leaves nothing', () => {
    const oneAlbum = TRACKS.filter((t) => t.album === 'Gold')
    expect(randomAlbumStart(oneAlbum, 'Gold', seq(0))?.id).toBe('gold1')
  })

  it('returns null for an empty library', () => {
    expect(randomAlbumStart([], undefined, seq(0))).toBeNull()
  })
})

describe('pickRandom', () => {
  it('never returns an excluded track', () => {
    const exclude = new Set(['gold1', 'gold2', 'gold3', 'rain1'])
    // Whatever the rng says, only rain2 is eligible.
    for (const r of [0, 0.4, 0.99]) {
      expect(pickRandom(TRACKS, exclude, () => r)?.id).toBe('rain2')
    }
  })

  it('ignores the exclusion when it covers the whole library', () => {
    // Better to repeat than to fall silent mid-stream.
    const all = new Set(TRACKS.map((t) => t.id))
    expect(pickRandom(TRACKS, all, seq(0))).not.toBeNull()
  })

  it('returns null for an empty library', () => {
    expect(pickRandom([], new Set(), seq(0))).toBeNull()
  })
})

describe('shuffled', () => {
  it('keeps every item exactly once', () => {
    const out = shuffled(TRACKS, seq(0.9, 0.1, 0.7, 0.3))
    expect(out).toHaveLength(TRACKS.length)
    expect(new Set(out.map((t) => t.id))).toEqual(new Set(TRACKS.map((t) => t.id)))
  })

  it('does not mutate the input', () => {
    const input = [...TRACKS]
    shuffled(input, seq(0.5))
    expect(input.map((t) => t.id)).toEqual(TRACKS.map((t) => t.id))
  })

  it('handles empty and single-item lists', () => {
    expect(shuffled([], seq(0))).toEqual([])
    expect(shuffled(['a'], seq(0))).toEqual(['a'])
  })

  it('actually reorders for a given seed', () => {
    const items = ['a', 'b', 'c', 'd', 'e']
    expect(shuffled(items, seq(0.99, 0.01, 0.99, 0.01))).not.toEqual(items)
  })
})
