/**
 * Smoothing for the playback position.
 *
 * The renderer reports progress from the audio element's `timeupdate`, which
 * fires about four times a second. Drawing that straight into a 60fps bar
 * means the playhead sits still for fifteen frames and then jumps -- the
 * motion reads as stuttering even though the timing is perfectly accurate.
 *
 * Two steps fix it. `predict` extrapolates from the last report using the wall
 * clock, so between reports the position keeps advancing. `ease` then moves
 * the drawn value toward that prediction a fraction at a time, so the small
 * corrections when a real report lands are absorbed instead of snapping.
 */

export type Sample = {
  position: number
  duration: number
  playing: boolean
  /** When this sample was taken, on the same clock later passed to predict. */
  at: number
}

/**
 * A jump larger than this is a seek or a track change, not drift.
 *
 * Easing across one of those would slide the playhead over the whole bar,
 * which looks like a glitch rather than a seek, so they snap instead.
 */
export const SNAP_SEC = 1.5

/** How quickly the drawn value chases the prediction. Lower is tighter. */
export const TAU_MS = 110

/** Where the track should be right now, given the last report. */
export function predict(s: Sample, now: number): number {
  if (!s.playing) return s.position
  const elapsed = Math.max(0, now - s.at) / 1000
  const p = s.position + elapsed
  // Never run past the end: the element stops there, and a playhead that
  // overshoots the bar before the next track loads is worse than one that
  // waits.
  return s.duration > 0 ? Math.min(p, s.duration) : p
}

/**
 * Moves `displayed` toward `target` by a time-corrected fraction.
 *
 * The step depends on elapsed time rather than being a fixed per-frame
 * fraction, so the same easing looks identical at 60Hz and 144Hz and does not
 * lurch after a dropped frame.
 */
export function ease(displayed: number, target: number, dtMs: number, tauMs = TAU_MS): number {
  if (!Number.isFinite(displayed) || Math.abs(target - displayed) > SNAP_SEC) return target
  if (!(dtMs > 0)) return displayed
  return displayed + (target - displayed) * (1 - Math.exp(-dtMs / tauMs))
}
