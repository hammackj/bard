import { randomUUID } from 'node:crypto'
import { BrowserWindow } from 'electron'
import { EVENT } from '../shared/ipc'
import { evaluateRequest, describeVerdict, normalizeUser } from '../shared/requests'
import type { RequestVerdict } from '../shared/requests'
import type { ChatEvent, ChatEventKind, QueueItem, Track } from '../shared/types'
import { getConfig } from './config'
import { getTrackById, matchRequest } from './library'
import { setNowPlaying, setQueueLength } from './nowplaying'

/**
 * The queue lives here and nowhere else.
 *
 * Chat requests and UI actions both land in this module, so there is one source
 * of truth for what plays next -- the renderer sends intents and re-renders
 * whatever comes back.
 */

let queue: QueueItem[] = []
let nowPlaying: QueueItem | null = null
const lastRequestAt: Record<string, number> = {}

function broadcast(channel: string, payload: unknown): void {
  for (const win of BrowserWindow.getAllWindows()) {
    if (!win.isDestroyed()) win.webContents.send(channel, payload)
  }
}

function emitQueue(): void {
  broadcast(EVENT.queueChanged, queue)
  setQueueLength(queue.length)
}

export function emitChat(kind: ChatEventKind, message: string, user?: string): ChatEvent {
  const event: ChatEvent = { id: randomUUID(), kind, message, user, at: Date.now() }
  broadcast(EVENT.chatEvent, event)
  return event
}

export function getQueue(): QueueItem[] {
  return queue
}

export function getNowPlaying(): QueueItem | null {
  return nowPlaying
}

/**
 * Streamer-initiated add. Deliberately skips evaluateRequest -- the person
 * running the stream is never rate-limited on their own machine.
 */
export function enqueueTrack(trackId: string, requester?: string): QueueItem | null {
  const track = getTrackById(trackId)
  if (!track) return null
  const item: QueueItem = { id: randomUUID(), track, requestedBy: requester, addedAt: Date.now() }
  queue = [...queue, item]
  emitQueue()
  if (!nowPlaying) playNext()
  return item
}

/**
 * Chat-initiated request. Runs the full rule set and logs a chat event either
 * way. lastRequestAt is only touched when the request is accepted, so a
 * rejected request does not burn the requester's cooldown.
 */
export function requestFromChat(
  query: string,
  requester: string,
  privileged = false
): RequestVerdict {
  const verdict = evaluateRequest({
    query,
    requester,
    now: Date.now(),
    queue,
    lastRequestAt,
    config: getConfig().requests,
    match: matchRequest,
    privileged
  })

  if (verdict.ok) {
    lastRequestAt[normalizeUser(requester)] = Date.now()
    const item: QueueItem = {
      id: randomUUID(),
      track: verdict.track,
      requestedBy: requester,
      addedAt: Date.now()
    }
    queue = [...queue, item]
    emitQueue()
    emitChat('ok', describeVerdict(verdict))
    if (!nowPlaying) playNext()
  } else {
    emitChat('no', describeVerdict(verdict))
  }
  return verdict
}

export function removeFromQueue(queueItemId: string): void {
  const before = queue.length
  queue = queue.filter((i) => i.id !== queueItemId)
  if (queue.length !== before) emitQueue()
}

export function reorderQueue(from: number, to: number): void {
  if (from === to) return
  if (from < 0 || from >= queue.length) return
  const clampedTo = Math.min(Math.max(0, to), queue.length - 1)
  const next = [...queue]
  const [moved] = next.splice(from, 1)
  next.splice(clampedTo, 0, moved)
  queue = next
  emitQueue()
}

export function clearQueue(): void {
  if (queue.length === 0) return
  queue = []
  emitQueue()
}

/** Pulls the head of the queue and tells the renderer to play it. */
export function playNext(): QueueItem | null {
  const [next, ...rest] = queue
  if (!next) {
    nowPlaying = null
    setNowPlaying(null)
    return null
  }
  queue = rest
  nowPlaying = next
  emitQueue()
  setNowPlaying(next)
  broadcast(EVENT.playTrack, next.track)
  return next
}

export function skip(): void {
  playNext()
}

/** Called when the renderer reports the current track finished. */
export function onTrackEnded(): void {
  playNext()
}

/** Test seam: lets a caller inspect cooldown bookkeeping. */
export function getLastRequestAt(): Readonly<Record<string, number>> {
  return lastRequestAt
}

export function playTrackNow(track: Track): void {
  nowPlaying = { id: randomUUID(), track, addedAt: Date.now() }
  setNowPlaying(nowPlaying)
  broadcast(EVENT.playTrack, track)
}
