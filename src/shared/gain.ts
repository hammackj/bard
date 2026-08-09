import type { NormalizeMode, Track } from './types'

/** ReplayGain reference: a gain of 0 dB means the track is already at target. */
export const MAX_BOOST_DB = 12

/**
 * Playback gain for a track, in dB.
 *
 * Returns 0 when normalisation is off or the file carries no tags, so an
 * untagged library simply plays at its own level rather than being silently
 * attenuated.
 *
 * Positive gain is held back so it cannot push the signal past full scale:
 * ReplayGain says how loud a track *is*, and boosting a quiet-but-peaky track
 * to match would clip it. Boost is also capped outright, because a very quiet
 * track can otherwise ask for 20 dB and bring its noise floor up with it.
 */
export function gainDbFor(track: Track, mode: NormalizeMode): number {
  if (mode === 'off') return 0

  const gain = mode === 'album' ? (track.albumGainDb ?? track.trackGainDb) : track.trackGainDb
  if (gain === undefined || !Number.isFinite(gain)) return 0

  const peak = mode === 'album' ? (track.albumPeak ?? track.trackPeak) : track.trackPeak
  let limited = Math.min(gain, MAX_BOOST_DB)

  if (peak !== undefined && peak > 0) {
    // Headroom available before the peak hits full scale.
    const headroomDb = -20 * Math.log10(peak)
    limited = Math.min(limited, headroomDb)
  }
  return limited
}

/** Linear multiplier for the gain node. */
export function gainMultiplier(track: Track | null, mode: NormalizeMode): number {
  if (!track) return 1
  return 10 ** (gainDbFor(track, mode) / 20)
}
