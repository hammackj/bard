import { describe, expect, it } from 'vitest'
import { createMatcher, searchTextFor, MATCH_THRESHOLD } from '../src/shared/match'
import type { Track } from '../src/shared/types'

const make = (id: string, title: string, artist: string, album: string): Track => ({
  id,
  path: `/music/${album}/${title}.flac`,
  title,
  artist,
  album,
  durationSec: 120,
  hasArtwork: true,
  searchText: searchTextFor(title, artist, album)
})

// Real titles from the StreamBeats lofi catalogue this was developed against.
const TRACKS: Track[] = [
  make('a1', 'Ambient Gold', 'Harris Heller', 'Gold'),
  make('a2', 'No One’s Hero', 'Harris Heller', 'Gold'),
  make('b1', 'Twinkle Tunes', 'Harris Heller', 'Electric'),
  make('b2', 'Neon in the Metropolis', 'Harris Heller', 'Electric'),
  make('c1', 'Coffee Nook', 'Harris Heller', 'Prime'),
  make('c2', 'Cherry Blossoms', 'Harris Heller', 'Moonwalk'),
  make('d1', 'Butterfly Effect', 'Harris Heller', 'Rain'),
  make('d2', 'Doin’ Fine', 'Harris Heller', 'Rain')
]

const match = createMatcher(TRACKS)
const titleOf = (q: string): string | null => match(q)?.title ?? null

describe('createMatcher', () => {
  it('matches an exact title', () => {
    expect(titleOf('Ambient Gold')).toBe('Ambient Gold')
  })

  it('is case-insensitive', () => {
    expect(titleOf('ambient gold')).toBe('Ambient Gold')
  })

  it('matches a partial title', () => {
    expect(titleOf('Twinkle')).toBe('Twinkle Tunes')
  })

  it('matches a single-character typo', () => {
    expect(titleOf('Ambent Gold')).toBe('Ambient Gold')
    expect(titleOf('Cofee Nook')).toBe('Coffee Nook')
  })

  it('matches a trailing truncation', () => {
    expect(titleOf('Butterfly Effec')).toBe('Butterfly Effect')
  })

  it('matches on artist name', () => {
    expect(match('Harris Heller')).not.toBeNull()
  })

  it('matches on album name', () => {
    expect(match('Metropolis')?.album).toBe('Electric')
  })

  describe('refusing weak matches', () => {
    // A wrong match plays on stream; a miss just replies "not found". The
    // matcher is tuned to prefer the miss.
    it.each([
      ['zzzzqqqqxxxx', 'nonsense'],
      ['Never Gonna Give You Up', 'not in the library'],
      ['Bohemian Rhapsody', 'not in the library'],
      ['shape of you', 'not in the library']
    ])('returns null for %s (%s)', (query) => {
      expect(match(query)).toBeNull()
    })

    it('returns null for an empty or whitespace query', () => {
      expect(match('')).toBeNull()
      expect(match('   ')).toBeNull()
    })

    it('does not let a generic genre word hijack the queue', () => {
      // The exact regression the threshold is set for: at 0.5 this matched an
      // unrelated track.
      expect(match('lofi')).toBeNull()
    })
  })

  it('handles an empty library without throwing', () => {
    expect(createMatcher([])('anything')).toBeNull()
  })

  it('keeps the threshold conservative', () => {
    expect(MATCH_THRESHOLD).toBeLessThanOrEqual(0.4)
  })
})

describe('searchTextFor', () => {
  it('lowercases and joins the indexed fields', () => {
    expect(searchTextFor('Title', 'Artist', 'Album')).toBe('title artist album')
  })
})
