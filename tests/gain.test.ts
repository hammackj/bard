import { describe, expect, it } from 'vitest'
import { MAX_BOOST_DB, gainDbFor, gainMultiplier } from '../src/shared/gain'
import type { Track } from '../src/shared/types'

const track = (over: Partial<Track> = {}): Track => ({
  id: 't',
  path: '/m/t.flac',
  title: 'T',
  artist: 'A',
  album: 'B',
  durationSec: 100,
  hasArtwork: false,
  searchText: 't',
  ...over
})

describe('gainDbFor', () => {
  it('is zero when normalisation is off', () => {
    expect(gainDbFor(track({ trackGainDb: -6 }), 'off')).toBe(0)
  })

  it('is zero for an untagged track rather than attenuating it', () => {
    expect(gainDbFor(track(), 'track')).toBe(0)
    expect(gainDbFor(track(), 'album')).toBe(0)
  })

  it('uses the track gain in track mode', () => {
    expect(gainDbFor(track({ trackGainDb: -10.1, albumGainDb: -6.2 }), 'track')).toBeCloseTo(-10.1)
  })

  it('uses the album gain in album mode', () => {
    expect(gainDbFor(track({ trackGainDb: -10.1, albumGainDb: -6.2 }), 'album')).toBeCloseTo(-6.2)
  })

  it('falls back to track gain when album gain is missing', () => {
    expect(gainDbFor(track({ trackGainDb: -4 }), 'album')).toBeCloseTo(-4)
  })

  it('never boosts past the cap', () => {
    expect(gainDbFor(track({ trackGainDb: 30, trackPeak: 0.01 }), 'track')).toBe(MAX_BOOST_DB)
  })

  describe('clipping protection', () => {
    it('holds back a boost that would push the peak past full scale', () => {
      // Peak is already at full scale, so no positive gain is safe.
      expect(gainDbFor(track({ trackGainDb: 6, trackPeak: 1.0 }), 'track')).toBeCloseTo(0)
    })

    it('allows the headroom a quiet peak leaves', () => {
      // peak 0.5 -> about 6 dB of headroom.
      expect(gainDbFor(track({ trackGainDb: 9, trackPeak: 0.5 }), 'track')).toBeCloseTo(6.02, 1)
    })

    it('still attenuates a track whose peak exceeds full scale', () => {
      // The real library has album peaks slightly over 1.0.
      expect(gainDbFor(track({ trackGainDb: -6, trackPeak: 1.012 }), 'track')).toBeCloseTo(-6)
    })
  })
})

describe('gainMultiplier', () => {
  it('is 1 with no track', () => {
    expect(gainMultiplier(null, 'track')).toBe(1)
  })

  it('converts dB to a linear multiplier', () => {
    expect(gainMultiplier(track({ trackGainDb: -6.02 }), 'track')).toBeCloseTo(0.5, 2)
  })

  it('is 1 for an untagged track', () => {
    expect(gainMultiplier(track(), 'track')).toBe(1)
  })
})
