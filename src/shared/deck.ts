import { shuffled, type Rng } from './playlist'

/**
 * A shuffle deck: every track is dealt once before any is dealt again.
 *
 * This is the thing random-with-a-recent-window cannot do. With 585 tracks and
 * a 50-track window, a "random" pick can still serve the same track twice in an
 * evening while leaving others untouched for weeks. Dealing from a shuffled
 * deck and only reshuffling once it is exhausted guarantees full coverage, and
 * it is the specific complaint people have about shuffle in big players.
 */
export type Deck = {
  /** Undealt ids, in the order they will be dealt. */
  remaining: string[]
  /** Ids dealt since the last reshuffle, for reporting progress through it. */
  dealt: number
}

export function newDeck(ids: readonly string[], rng: Rng = Math.random, avoidFirst?: string): Deck {
  let remaining = shuffled(ids, rng)
  // Avoid dealing the track that just played as the first of a new deck: the
  // seam between decks is the one place a repeat can still feel wrong.
  if (avoidFirst && remaining.length > 1 && remaining[0] === avoidFirst) {
    const swapWith = 1 + Math.floor(rng() * (remaining.length - 1))
    ;[remaining[0], remaining[swapWith]] = [remaining[swapWith], remaining[0]]
  }
  return { remaining, dealt: 0 }
}

/**
 * Deals the next id, rebuilding the deck when it runs out.
 *
 * `available` is passed on every draw so the deck follows a library that has
 * been rescanned: ids that no longer exist are dropped rather than dealt.
 */
export function draw(
  deck: Deck | null,
  available: readonly string[],
  rng: Rng = Math.random,
  avoidFirst?: string
): { id: string | null; deck: Deck } {
  if (available.length === 0) return { id: null, deck: { remaining: [], dealt: 0 } }

  const valid = new Set(available)
  let current: Deck =
    deck && deck.remaining.length > 0
      ? { ...deck, remaining: deck.remaining.filter((id) => valid.has(id)) }
      : newDeck(available, rng, avoidFirst)

  if (current.remaining.length === 0) current = newDeck(available, rng, avoidFirst)

  const [id, ...rest] = current.remaining
  return { id: id ?? null, deck: { remaining: rest, dealt: current.dealt + 1 } }
}

/** How far through the current deck we are, 0..1. */
export function deckProgress(deck: Deck | null, total: number): number {
  if (!deck || total === 0) return 0
  return Math.min(1, deck.dealt / total)
}
