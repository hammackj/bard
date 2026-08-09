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
  /** Last id dealt, so a rebuilt deck does not repeat it across the seam. */
  last?: string
}

/**
 * Extra copies a loved track gets in each pass.
 *
 * Weighting inside the deck rather than picking loved tracks more often at
 * random is what keeps the guarantee intact: every track still appears in
 * every pass, loved ones simply appear more than once.
 */
export type WeightOf = (id: string) => number

function expand(ids: readonly string[], weightOf?: WeightOf): string[] {
  if (!weightOf) return [...ids]
  const out: string[] = []
  for (const id of ids) {
    const copies = Math.max(1, Math.floor(weightOf(id)))
    for (let i = 0; i < copies; i++) out.push(id)
  }
  return out
}

/**
 * Arranges a weighted deck so no two identical cards sit next to each other.
 *
 * Built rather than patched: an earlier version swapped randomly a few times
 * and gave up, which left adjacent duplicates whenever a loved track was a
 * large share of a small library. Placing the most frequent card into every
 * other slot first is correct by construction whenever it is possible at all
 * (that is, when no card exceeds half the deck, rounded up).
 */
function spread(order: readonly string[], rng: Rng): string[] {
  const counts = new Map<string, number>()
  for (const id of order) counts.set(id, (counts.get(id) ?? 0) + 1)

  // Shuffle the distinct ids so equal-frequency cards are not always in the
  // same order, then place the most frequent first.
  const distinct = shuffled([...counts.keys()], rng).sort(
    (a, b) => (counts.get(b) ?? 0) - (counts.get(a) ?? 0)
  )

  const out = new Array<string>(order.length)
  let slot = 0
  for (const id of distinct) {
    for (let n = counts.get(id) ?? 0; n > 0; n--) {
      if (slot >= out.length) slot = 1 // wrap to the odd slots
      out[slot] = id
      slot += 2
    }
  }
  return out
}

export function newDeck(
  ids: readonly string[],
  rng: Rng = Math.random,
  avoidFirst?: string,
  weightOf?: WeightOf
): Deck {
  const remaining = weightOf
    ? spread(shuffled(expand(ids, weightOf), rng), rng)
    : shuffled(ids, rng)
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
  avoidFirst?: string,
  weightOf?: WeightOf
): { id: string | null; deck: Deck } {
  if (available.length === 0) return { id: null, deck: { remaining: [], dealt: 0 } }

  const valid = new Set(available)
  // The seam is guarded by whatever was dealt last, whether the caller told us
  // or not -- relying on the caller meant a rebuild could repeat a card.
  const guard = avoidFirst ?? deck?.last

  let current: Deck =
    deck && deck.remaining.length > 0
      ? { ...deck, remaining: deck.remaining.filter((id) => valid.has(id)) }
      : newDeck(available, rng, guard, weightOf)

  if (current.remaining.length === 0) current = newDeck(available, rng, guard, weightOf)

  const [id, ...rest] = current.remaining
  return {
    id: id ?? null,
    deck: { remaining: rest, dealt: current.dealt + 1, last: id ?? deck?.last }
  }
}

/** How far through the current deck we are, 0..1. */
export function deckProgress(deck: Deck | null, total: number): number {
  if (!deck || total === 0) return 0
  return Math.min(1, deck.dealt / total)
}
