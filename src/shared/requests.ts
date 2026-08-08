/**
 * Request rules -- pure, no I/O, no clock access.
 *
 * `now` is passed in rather than read, so the cooldown boundary is testable
 * without faking timers. Nothing here mutates its arguments; recording a
 * request is the caller's job, which is what keeps a rejected request from
 * consuming the requester's cooldown.
 */
import type { QueueItem, RequestConfig, Track } from './types'

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
  requester: string
  now: number
  queue: QueueItem[]
  lastRequestAt: Record<string, number>
  config: RequestConfig
  match: (q: string) => Track | null
  /** Broadcaster or moderator: bypasses cooldown and the per-user cap. */
  privileged?: boolean
}

/** Usernames are compared case-insensitively throughout. */
export function normalizeUser(user: string): string {
  return user.trim().toLowerCase()
}

export function cooldownRemaining(
  requester: string,
  now: number,
  lastRequestAt: Record<string, number>,
  cooldownSeconds: number
): number {
  const last = lastRequestAt[normalizeUser(requester)]
  if (last === undefined) return 0
  const elapsed = now - last
  const window = cooldownSeconds * 1000
  // At exactly the cooldown the request passes, so this is a strict compare.
  return elapsed >= window ? 0 : Math.ceil((window - elapsed) / 1000)
}

export function countUserItems(queue: QueueItem[], requester: string): number {
  const user = normalizeUser(requester)
  return queue.filter((i) => i.requestedBy && normalizeUser(i.requestedBy) === user).length
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
  const user = normalizeUser(requester)

  if (config.blockedUsers.some((u) => normalizeUser(u) === user)) {
    return { ok: false, reason: 'blocked_user' }
  }

  if (queue.length >= config.maxQueueLength) {
    return { ok: false, reason: 'queue_full', detail: `${queue.length}/${config.maxQueueLength}` }
  }

  if (!privileged) {
    const mine = countUserItems(queue, requester)
    if (mine >= config.maxPerUserInQueue) {
      return {
        ok: false,
        reason: 'user_limit',
        detail: `${mine}/${config.maxPerUserInQueue}`
      }
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
