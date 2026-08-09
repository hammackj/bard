import { describe, expect, it } from 'vitest'
import { PLAYED_LIMIT, pushPlayed, stepBack, wrapForRepeatAll } from '../src/shared/history'

const item = (id: string): { id: string } => ({ id })
const ids = (xs: readonly { id: string }[]): string[] => xs.map((x) => x.id)

describe('pushPlayed', () => {
  it('appends newest last', () => {
    expect(ids(pushPlayed([item('a')], item('b')))).toEqual(['a', 'b'])
  })

  it('keeps history bounded, dropping the oldest', () => {
    let played: { id: string }[] = []
    for (let i = 0; i < PLAYED_LIMIT + 25; i++) played = pushPlayed(played, item(`t${i}`))
    expect(played).toHaveLength(PLAYED_LIMIT)
    expect(played[0].id).toBe('t25')
    expect(played.at(-1)?.id).toBe(`t${PLAYED_LIMIT + 24}`)
  })
})

describe('stepBack', () => {
  const played = [item('a'), item('b'), item('c')] // c is playing

  it('restarts when a few seconds into the track', () => {
    expect(stepBack(played, 10)).toEqual({ kind: 'restart' })
  })

  it('steps back when barely into the track', () => {
    const step = stepBack(played, 1)
    expect(step.kind).toBe('back')
    if (step.kind !== 'back') throw new Error('unreachable')
    // The one before what is playing, not what is playing.
    expect(step.prior.id).toBe('b')
    // History drops the track we stepped away from, so b is now the tail.
    expect(ids(step.played)).toEqual(['a', 'b'])
  })

  it('walks back one track per press', () => {
    let history: { id: string }[] = played
    const visited: string[] = []
    for (let i = 0; i < 2; i++) {
      const step = stepBack(history, 0)
      if (step.kind !== 'back') break
      visited.push(step.prior.id)
      history = step.played
    }
    expect(visited).toEqual(['b', 'a'])
  })

  it('restarts rather than doing nothing at the start of history', () => {
    expect(stepBack([item('a')], 0)).toEqual({ kind: 'restart' })
    expect(stepBack([], 0)).toEqual({ kind: 'restart' })
  })
})

describe('wrapForRepeatAll', () => {
  it('does not replay the track that just finished', () => {
    const played = [item('a'), item('b'), item('c')]
    const wrapped = wrapForRepeatAll(played, item('c'))
    expect(ids(wrapped.queue)).toEqual(['a', 'b'])
  })

  it('carries the current track into the next lap as history', () => {
    const wrapped = wrapForRepeatAll([item('a'), item('b')], item('b'))
    expect(ids(wrapped.played)).toEqual(['b'])
  })

  it('loops without duplicating across two wraps', () => {
    // a, b, c played; c finishes and wraps.
    let played = [item('a'), item('b'), item('c')]
    const first = wrapForRepeatAll(played, item('c'))
    expect(ids(first.queue)).toEqual(['a', 'b'])

    // Lap two plays a then b, and b finishes.
    played = pushPlayed(pushPlayed(first.played, item('a')), item('b'))
    const second = wrapForRepeatAll(played, item('b'))
    expect(ids(second.queue)).toEqual(['c', 'a'])
    // Every track appears exactly once per lap.
    expect(new Set(ids(second.queue)).size).toBe(second.queue.length)
  })

  it('falls back to the tail of history when nothing is playing', () => {
    const wrapped = wrapForRepeatAll([item('a'), item('b')], null)
    expect(ids(wrapped.queue)).toEqual(['a'])
  })
})
