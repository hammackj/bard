/**
 * Request rules -- pure, no I/O, no clock access.
 *
 * `now` is passed in rather than read, so the cooldown boundary is testable
 * without faking timers. Nothing here mutates its arguments; recording a
 * request is the caller's job, which is what keeps a rejected request from
 * consuming the requester's cooldown.
 */
import type { BlockedUser, QueueItem, Requester, RequestConfig, Track } from './types'

export type RejectReason =
  | 'blocked_user'
  | 'blocked_track'
  | 'cooldown'
  | 'queue_full'
  | 'user_limit'
  | 'not_found'
  | 'already_queued'

export type RequestVerdict =
  | { ok: true; track: Track }
  | { ok: false; reason: RejectReason; detail?: string }

export type EvaluateArgs = {
  query: string
  requester: Requester
  now: number
  queue: QueueItem[]
  lastRequestAt: Record<string, number>
  config: RequestConfig
  match: (q: string) => Track | null
  /** Broadcaster or moderator: bypasses cooldown and the per-user cap. */
  privileged?: boolean
}

/** Names are compared case-insensitively throughout. */
export function normalizeName(name: string): string {
  return name.trim().toLowerCase()
}

/**
 * Identity key for cooldowns and per-user counting.
 *
 * Scoped by platform so the same handle on Twitch and YouTube is two people,
 * and keyed on the platform id when there is one so a rename does not hand
 * someone a fresh cooldown.
 */
export function requesterKey(r: Requester): string {
  return `${r.platform}:${r.id ?? normalizeName(r.name)}`
}

/** True when the block list covers this requester. */
export function isUserBlocked(blocked: BlockedUser[], r: Requester): boolean {
  const name = normalizeName(r.name)
  return blocked.some((b) => {
    if (b.platform !== r.platform) return false
    // Prefer the stable id; fall back to the name only when neither side has one.
    if (b.id && r.id) return b.id === r.id
    return normalizeName(b.name) === name
  })
}

export function cooldownRemaining(
  requester: Requester,
  now: number,
  lastRequestAt: Record<string, number>,
  cooldownSeconds: number
): number {
  const last = lastRequestAt[requesterKey(requester)]
  if (last === undefined) return 0
  const elapsed = now - last
  const window = cooldownSeconds * 1000
  // At exactly the cooldown the request passes, so this is a strict compare.
  return elapsed >= window ? 0 : Math.ceil((window - elapsed) / 1000)
}

export function countUserItems(queue: QueueItem[], requester: Requester): number {
  const key = requesterKey(requester)
  return queue.filter((i) => i.requester && requesterKey(i.requester) === key).length
}

/**
 * Cheapest and most definitive checks first, so a blocked user never causes a
 * library search and an unmatched query never reports "cooldown".
 *
 * Order: blocked user -> queue full -> user limit -> cooldown -> match ->
 * blocked track -> already queued.
 */
export function evaluateRequest(args: EvaluateArgs): RequestVerdict {
  const { query, requester, now, queue, lastRequestAt, config, match, privileged } = args

  if (isUserBlocked(config.blockedUsers, requester)) {
    return { ok: false, reason: 'blocked_user' }
  }

  if (queue.length >= config.maxQueueLength) {
    return { ok: false, reason: 'queue_full', detail: `${queue.length}/${config.maxQueueLength}` }
  }

  if (!privileged) {
    const mine = countUserItems(queue, requester)
    if (mine >= config.maxPerUserInQueue) {
      return { ok: false, reason: 'user_limit', detail: `${mine}/${config.maxPerUserInQueue}` }
    }

    const remaining = cooldownRemaining(requester, now, lastRequestAt, config.cooldownSeconds)
    if (remaining > 0) {
      return { ok: false, reason: 'cooldown', detail: `${remaining}s` }
    }
  }

  const track = match(query)
  if (!track) return { ok: false, reason: 'not_found', detail: query }

  if (config.blockedTracks.includes(track.id)) {
    return { ok: false, reason: 'blocked_track', detail: track.title }
  }

  if (queue.some((i) => i.track.id === track.id)) {
    return { ok: false, reason: 'already_queued', detail: track.title }
  }

  return { ok: true, track }
}

/** Human-readable line for the chat log. */
export function describeVerdict(v: RequestVerdict): string {
  if (v.ok) return `Queued — ${v.track.title}`
  switch (v.reason) {
    case 'blocked_user':
      return 'Blocked'
    case 'queue_full':
      return `Queue full — ${v.detail ?? ''}`.trim()
    case 'user_limit':
      return `Already have ${v.detail ?? 'too many'} queued`
    case 'cooldown':
      return `Cooldown — ${v.detail ?? ''} left`.trim()
    case 'not_found':
      return 'Not in library'
    case 'blocked_track':
      return `Track blocked — ${v.detail ?? ''}`.trim()
    case 'already_queued':
      return `Already queued — ${v.detail ?? ''}`.trim()
  }
}

/** Adds a block, replacing any existing entry for the same identity. */
export function addBlock(blocked: BlockedUser[], r: Requester, now: number): BlockedUser[] {
  const without = blocked.filter((b) => !isUserBlocked([b], r))
  return [...without, { platform: r.platform, id: r.id, name: r.name, at: now }]
}

export function removeBlock(blocked: BlockedUser[], r: Requester): BlockedUser[] {
  return blocked.filter((b) => !isUserBlocked([b], r))
}
