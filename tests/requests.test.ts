import { describe, expect, it } from 'vitest'
import {
  cooldownRemaining,
  countUserItems,
  describeVerdict,
  evaluateRequest,
  normalizeUser,
  type EvaluateArgs
} from '../src/shared/requests'
import type { QueueItem, RequestConfig, Track } from '../src/shared/types'

const track = (id: string, title: string): Track => ({
  id,
  path: `/music/${id}.flac`,
  title,
  artist: 'Harris Heller',
  album: 'Gold',
  durationSec: 120,
  hasArtwork: false,
  searchText: title.toLowerCase()
})

const ONE = track('t1', 'One')
const TWO = track('t2', 'Two')

const item = (t: Track, who?: string): QueueItem => ({
  id: `q-${t.id}-${who ?? 'streamer'}`,
  track: t,
  requestedBy: who,
  addedAt: 0
})

const config = (over: Partial<RequestConfig> = {}): RequestConfig => ({
  cooldownSeconds: 120,
  maxQueueLength: 20,
  maxPerUserInQueue: 2,
  blockedUsers: [],
  blockedTracks: [],
  ...over
})

const NOW = 1_000_000

const args = (over: Partial<EvaluateArgs> = {}): EvaluateArgs => ({
  query: 'one',
  requester: 'dave',
  now: NOW,
  queue: [],
  lastRequestAt: {},
  config: config(),
  match: (q) => (q === 'one' ? ONE : q === 'two' ? TWO : null),
  ...over
})

const reason = (v: ReturnType<typeof evaluateRequest>): string => (v.ok ? 'ok' : v.reason)

describe('evaluateRequest', () => {
  it('accepts a plain request', () => {
    const v = evaluateRequest(args())
    expect(v.ok).toBe(true)
    if (v.ok) expect(v.track.id).toBe('t1')
  })

  it('rejects a blocked user before anything else runs', () => {
    // The query would not match either; blocked must win so the reason is
    // never misreported as not_found.
    const v = evaluateRequest(
      args({ query: 'nonsense', config: config({ blockedUsers: ['dave'] }) })
    )
    expect(reason(v)).toBe('blocked_user')
  })

  it('matches blocked users case-insensitively', () => {
    expect(
      reason(evaluateRequest(args({ requester: 'DaVe', config: config({ blockedUsers: ['dave'] }) })))
    ).toBe('blocked_user')
  })

  it('rejects when the queue is full', () => {
    const v = evaluateRequest(
      args({ config: config({ maxQueueLength: 1 }), queue: [item(TWO, 'someone')] })
    )
    expect(reason(v)).toBe('queue_full')
  })

  it('counts only the requester towards the per-user limit', () => {
    const others = [item(TWO, 'someone'), item(track('t3', 'Three'), 'other')]
    expect(reason(evaluateRequest(args({ queue: others })))).toBe('ok')
  })

  it('rejects at the per-user limit, regardless of username case', () => {
    const mine = [item(TWO, 'dave'), item(track('t3', 'Three'), 'DAVE')]
    expect(reason(evaluateRequest(args({ queue: mine })))).toBe('user_limit')
  })

  describe('cooldown', () => {
    it('passes at exactly the cooldown boundary', () => {
      const lastRequestAt = { dave: NOW - 120_000 }
      expect(reason(evaluateRequest(args({ lastRequestAt })))).toBe('ok')
    })

    it('rejects one millisecond short of the boundary', () => {
      const lastRequestAt = { dave: NOW - 120_000 }
      expect(reason(evaluateRequest(args({ now: NOW - 1, lastRequestAt })))).toBe('cooldown')
    })

    it('reports whole seconds remaining, rounded up', () => {
      expect(cooldownRemaining('dave', NOW - 1500, { dave: NOW - 120_000 }, 120)).toBe(2)
    })

    it('is zero for a user who has not requested before', () => {
      expect(cooldownRemaining('nobody', NOW, {}, 120)).toBe(0)
    })
  })

  describe('privileged users', () => {
    it('bypasses cooldown', () => {
      const v = evaluateRequest(args({ lastRequestAt: { dave: NOW }, privileged: true }))
      expect(reason(v)).toBe('ok')
    })

    it('bypasses the per-user limit', () => {
      const mine = [item(TWO, 'dave'), item(track('t3', 'Three'), 'dave')]
      expect(reason(evaluateRequest(args({ queue: mine, privileged: true })))).toBe('ok')
    })

    it('does not bypass a full queue', () => {
      const v = evaluateRequest(
        args({ config: config({ maxQueueLength: 1 }), queue: [item(TWO, 'x')], privileged: true })
      )
      expect(reason(v)).toBe('queue_full')
    })
  })

  it('returns not_found for an unmatched query', () => {
    expect(reason(evaluateRequest(args({ query: 'zzz' })))).toBe('not_found')
  })

  it('rejects a blocked track', () => {
    expect(reason(evaluateRequest(args({ config: config({ blockedTracks: ['t1'] }) })))).toBe(
      'blocked_track'
    )
  })

  it('rejects a track already in the queue', () => {
    expect(reason(evaluateRequest(args({ queue: [item(ONE, 'someone')] })))).toBe('already_queued')
  })

  describe('purity', () => {
    it('does not mutate lastRequestAt, even on a rejected request', () => {
      const lastRequestAt = { dave: 5 }
      evaluateRequest(args({ query: 'zzz', lastRequestAt }))
      expect(lastRequestAt).toEqual({ dave: 5 })
    })

    it('does not mutate the queue', () => {
      const queue = [item(TWO, 'someone')]
      evaluateRequest(args({ queue }))
      expect(queue).toHaveLength(1)
    })
  })
})

describe('helpers', () => {
  it('normalises usernames', () => {
    expect(normalizeUser('  DaVe  ')).toBe('dave')
  })

  it('counts a user’s queued items', () => {
    const queue = [item(ONE, 'dave'), item(TWO, 'DAVE'), item(track('t3', 'T'), 'other')]
    expect(countUserItems(queue, 'dave')).toBe(2)
  })

  it('does not count streamer-added items towards any user', () => {
    expect(countUserItems([item(ONE)], 'dave')).toBe(0)
  })

  it('describes every verdict', () => {
    expect(describeVerdict({ ok: true, track: ONE })).toContain('One')
    expect(describeVerdict({ ok: false, reason: 'not_found' })).toBe('Not in library')
    expect(describeVerdict({ ok: false, reason: 'cooldown', detail: '74s' })).toContain('74s')
  })
})
