/**
 * Play history arithmetic.
 *
 * Small, but the off-by-one here is the kind that only shows up as "back went
 * to the wrong song" three tracks later, so it lives apart from the queue's
 * mutable state where it can actually be tested.
 *
 * The invariant everything below assumes: the last entry of `played` is the
 * track playing right now. It is pushed when playback starts, not when it
 * finishes.
 */

/** Anything with a stable identity; the queue passes its QueueItems. */
type Item = { id: string }

/** How far into a track "previous" restarts it instead of stepping back. */
export const RESTART_THRESHOLD_SEC = 3

/** Most history to keep. Bounded so a long session cannot grow without limit. */
export const PLAYED_LIMIT = 200

export function pushPlayed<T extends Item>(played: readonly T[], item: T): T[] {
  return [...played, item].slice(-PLAYED_LIMIT)
}

/**
 * What pressing "previous" should do.
 *
 * `restart` covers both the common case (a few seconds in, you meant "play
 * this again") and the boundary (nothing behind us) -- in both, replaying the
 * current track is better than doing nothing.
 */
export type BackStep<T> = { kind: 'restart' } | { kind: 'back'; prior: T; played: T[] }

export function stepBack<T extends Item>(played: readonly T[], positionSec: number): BackStep<T> {
  if (positionSec > RESTART_THRESHOLD_SEC) return { kind: 'restart' }
  // -1 is what is playing now, so the one before it is -2.
  const prior = played.at(-2)
  if (!prior) return { kind: 'restart' }
  return { kind: 'back', prior, played: played.slice(0, -1) }
}

/**
 * Queue and history after repeat-all wraps around.
 *
 * The subtlety: `played` ends with the track that just finished, so re-queuing
 * it wholesale plays that track twice across the seam. It is dropped here and
 * kept as the sole history entry, which both starts the next lap's history
 * cleanly and leaves one step of "back" still working across the wrap.
 */
export function wrapForRepeatAll<T extends Item>(
  played: readonly T[],
  current: T | null
): { queue: T[]; played: T[] } {
  const currentId = current?.id ?? played.at(-1)?.id
  return {
    queue: played.filter((i) => i.id !== currentId),
    played: current ? [current] : []
  }
}
