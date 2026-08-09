import { describe, expect, it } from 'vitest'
import { draw, newDeck, deckProgress, type Deck } from '../src/shared/deck'

const ids = ['a', 'b', 'c', 'd', 'e']
/** Deterministic rng cycling through the given values. */
const seq = (...v: number[]): (() => number) => {
  let i = 0
  return () => v[i++ % v.length]
}

describe('shuffle deck', () => {
  it('deals every track before repeating any', () => {
    // The whole point: a rolling random window cannot guarantee this.
    let deck: Deck | null = null
    const dealt: string[] = []
    for (let i = 0; i < ids.length; i++) {
      const out = draw(deck, ids, seq(0.1, 0.7, 0.3, 0.9))
      deck = out.deck
      dealt.push(out.id!)
    }
    expect(new Set(dealt).size).toBe(ids.length)
    expect([...dealt].sort()).toEqual([...ids].sort())
  })

  it('reshuffles once exhausted and keeps dealing', () => {
    let deck: Deck | null = null
    const dealt: string[] = []
    for (let i = 0; i < ids.length * 3; i++) {
      const out = draw(deck, ids, seq(0.4, 0.8, 0.2))
      deck = out.deck
      dealt.push(out.id!)
    }
    expect(dealt).toHaveLength(15)
    expect(dealt.every((d) => ids.includes(d))).toBe(true)
    // Each full pass should still contain every track exactly once.
    for (let pass = 0; pass < 3; pass++) {
      const slice = dealt.slice(pass * 5, pass * 5 + 5)
      expect(new Set(slice).size).toBe(5)
    }
  })

  it('avoids repeating the previous track across the seam', () => {
    // rng returns 0, so an unguarded shuffle would deal the same first card.
    const deck = newDeck(ids, () => 0, ids[0])
    expect(deck.remaining[0]).not.toBe(ids[0])
  })

  it('drops ids that no longer exist after a rescan', () => {
    const stale: Deck = { remaining: ['a', 'gone', 'b'], dealt: 0 }
    const out = draw(stale, ['a', 'b'], seq(0.5))
    expect(['a', 'b']).toContain(out.id)
    expect(out.deck.remaining).not.toContain('gone')
  })

  it('returns null for an empty library rather than throwing', () => {
    expect(draw(null, [], seq(0.5)).id).toBeNull()
  })

  it('handles a single-track library', () => {
    const out = draw(null, ['only'], seq(0.5))
    expect(out.id).toBe('only')
    expect(draw(out.deck, ['only'], seq(0.5)).id).toBe('only')
  })

  it('never stalls: 585 tracks dealt for many passes', () => {
    // The "endless" promise. Deals well past the library size and checks that
    // it keeps producing tracks and that every pass is a full permutation.
    const big = Array.from({ length: 585 }, (_, i) => `t${i}`)
    let deck: Deck | null = null
    const dealt: string[] = []
    for (let i = 0; i < 585 * 4; i++) {
      const out = draw(deck, big)
      expect(out.id).not.toBeNull()
      deck = out.deck
      dealt.push(out.id!)
    }
    expect(dealt).toHaveLength(2340)
    for (let pass = 0; pass < 4; pass++) {
      const slice = dealt.slice(pass * 585, (pass + 1) * 585)
      expect(new Set(slice).size).toBe(585)
    }
  })

  it('keeps dealing when the library shrinks mid-deck', () => {
    let deck: Deck | null = draw(null, ['a', 'b', 'c', 'd']).deck
    for (let i = 0; i < 20; i++) {
      const out = draw(deck, ['a', 'b'])
      expect(['a', 'b']).toContain(out.id)
      deck = out.deck
    }
  })

  it('reports progress through the deck', () => {
    expect(deckProgress(null, 10)).toBe(0)
    expect(deckProgress({ remaining: [], dealt: 5 }, 10)).toBe(0.5)
    expect(deckProgress({ remaining: [], dealt: 99 }, 10)).toBe(1)
  })
})
