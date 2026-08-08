import { describe, expect, it } from 'vitest'
import {
  addBlock,
  cooldownRemaining,
  countUserItems,
  describeVerdict,
  evaluateRequest,
  isUserBlocked,
  normalizeName,
  removeBlock,
  requesterKey,
  type EvaluateArgs
} from '../src/shared/requests'
import type { QueueItem, Requester, RequestConfig, Track } from '../src/shared/types'

const dave: Requester = { platform: 'twitch', id: '123', name: 'dave' }
const ytDave: Requester = { platform: 'youtube', id: 'UC999', name: 'dave' }
const named = (name: string): Requester => ({ platform: 'twitch', name })

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

const item = (t: Track, who?: Requester): QueueItem => ({
  id: `q-${t.id}-${who?.name ?? 'streamer'}`,
  track: t,
  requester: who,
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
  requester: dave,
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
      args({
        query: 'nonsense',
        config: config({ blockedUsers: [{ platform: 'twitch', id: '123', name: 'dave', at: 0 }] })
      })
    )
    expect(reason(v)).toBe('blocked_user')
  })

  it('blocks by id even after a rename', () => {
    const v = evaluateRequest(
      args({
        requester: { platform: 'twitch', id: '123', name: 'NewHandle' },
        config: config({ blockedUsers: [{ platform: 'twitch', id: '123', name: 'dave', at: 0 }] })
      })
    )
    expect(reason(v)).toBe('blocked_user')
  })

  it('does not block the same handle on another platform', () => {
    const v = evaluateRequest(
      args({
        requester: ytDave,
        config: config({ blockedUsers: [{ platform: 'twitch', id: '123', name: 'dave', at: 0 }] })
      })
    )
    expect(reason(v)).toBe('ok')
  })

  it('matches a name-only block case-insensitively', () => {
    const v = evaluateRequest(
      args({
        requester: named('DaVe'),
        config: config({ blockedUsers: [{ platform: 'twitch', name: 'dave', at: 0 }] })
      })
    )
    expect(reason(v)).toBe('blocked_user')
  })

  it('rejects when the queue is full', () => {
    const v = evaluateRequest(
      args({ config: config({ maxQueueLength: 1 }), queue: [item(TWO, named('someone'))] })
    )
    expect(reason(v)).toBe('queue_full')
  })

  it('counts only the requester towards the per-user limit', () => {
    const others = [item(TWO, named('someone')), item(track('t3', 'Three'), named('other'))]
    expect(reason(evaluateRequest(args({ queue: others })))).toBe('ok')
  })

  it('rejects at the per-user limit, regardless of username case', () => {
    // Same id, different display name -- still the same person.
    const mine = [
      item(TWO, dave),
      item(track('t3', 'Three'), { platform: 'twitch', id: '123', name: 'DAVE' })
    ]
    expect(reason(evaluateRequest(args({ queue: mine })))).toBe('user_limit')
  })

  describe('cooldown', () => {
    it('passes at exactly the cooldown boundary', () => {
      const lastRequestAt = { [requesterKey(dave)]: NOW - 120_000 }
      expect(reason(evaluateRequest(args({ lastRequestAt })))).toBe('ok')
    })

    it('rejects one millisecond short of the boundary', () => {
      const lastRequestAt = { [requesterKey(dave)]: NOW - 120_000 }
      expect(reason(evaluateRequest(args({ now: NOW - 1, lastRequestAt })))).toBe('cooldown')
    })

    it('reports whole seconds remaining, rounded up', () => {
      expect(
        cooldownRemaining(dave, NOW - 1500, { [requesterKey(dave)]: NOW - 120_000 }, 120)
      ).toBe(2)
    })

    it('is zero for a user who has not requested before', () => {
      expect(cooldownRemaining(named('nobody'), NOW, {}, 120)).toBe(0)
    })
  })

  describe('privileged users', () => {
    it('bypasses cooldown', () => {
      const v = evaluateRequest(
        args({ lastRequestAt: { [requesterKey(dave)]: NOW }, privileged: true })
      )
      expect(reason(v)).toBe('ok')
    })

    it('bypasses the per-user limit', () => {
      const mine = [item(TWO, dave), item(track('t3', 'Three'), dave)]
      expect(reason(evaluateRequest(args({ queue: mine, privileged: true })))).toBe('ok')
    })

    it('does not bypass a full queue', () => {
      const v = evaluateRequest(
        args({
          config: config({ maxQueueLength: 1 }),
          queue: [item(TWO, named('x'))],
          privileged: true
        })
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
    expect(reason(evaluateRequest(args({ queue: [item(ONE, named('someone'))] })))).toBe(
      'already_queued'
    )
  })

  describe('purity', () => {
    it('does not mutate lastRequestAt, even on a rejected request', () => {
      const lastRequestAt = { [requesterKey(dave)]: 5 }
      evaluateRequest(args({ query: 'zzz', lastRequestAt }))
      expect(lastRequestAt).toEqual({ [requesterKey(dave)]: 5 })
    })

    it('does not mutate the queue', () => {
      const queue = [item(TWO, named('someone'))]
      evaluateRequest(args({ queue }))
      expect(queue).toHaveLength(1)
    })
  })
})

describe('helpers', () => {
  it('normalises names', () => {
    expect(normalizeName('  DaVe  ')).toBe('dave')
  })

  it('scopes the identity key by platform', () => {
    expect(requesterKey(dave)).not.toBe(requesterKey(ytDave))
  })

  it('keys on id rather than name, so a rename keeps the same identity', () => {
    expect(requesterKey({ platform: 'twitch', id: '123', name: 'renamed' })).toBe(
      requesterKey(dave)
    )
  })

  it('counts a user’s queued items across renames', () => {
    const queue = [
      item(ONE, dave),
      item(TWO, { platform: 'twitch', id: '123', name: 'DAVE' }),
      item(track('t3', 'T'), named('other'))
    ]
    expect(countUserItems(queue, dave)).toBe(2)
  })

  it('does not count streamer-added items towards any user', () => {
    expect(countUserItems([item(ONE)], dave)).toBe(0)
  })

  it('describes every verdict', () => {
    expect(describeVerdict({ ok: true, track: ONE })).toContain('One')
    expect(describeVerdict({ ok: false, reason: 'not_found' })).toBe('Not in library')
    expect(describeVerdict({ ok: false, reason: 'cooldown', detail: '74s' })).toContain('74s')
  })
})

describe('block list', () => {
  it('adds and removes a block', () => {
    const added = addBlock([], dave, 1)
    expect(isUserBlocked(added, dave)).toBe(true)
    expect(isUserBlocked(removeBlock(added, dave), dave)).toBe(false)
  })

  it('does not duplicate an existing block', () => {
    const twice = addBlock(addBlock([], dave, 1), dave, 2)
    expect(twice).toHaveLength(1)
    expect(twice[0].at).toBe(2)
  })

  it('keeps blocks on other platforms when removing one', () => {
    const both = addBlock(addBlock([], dave, 1), ytDave, 1)
    const left = removeBlock(both, dave)
    expect(isUserBlocked(left, ytDave)).toBe(true)
    expect(isUserBlocked(left, dave)).toBe(false)
  })

  it('an empty list blocks nobody', () => {
    expect(isUserBlocked([], dave)).toBe(false)
  })
})
