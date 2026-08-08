import { randomUUID } from 'node:crypto'
import { BrowserWindow } from 'electron'
import { EVENT } from '../shared/ipc'
import { evaluateRequest, describeVerdict, normalizeUser } from '../shared/requests'
import type { RequestVerdict } from '../shared/requests'
import type { ChatEvent, ChatEventKind, QueueItem, Track } from '../shared/types'
import { getConfig } from './config'
import {
  getTrackById,
  matchRequest,
  nextInAlbum,
  randomAlbumStart,
  randomTrack
} from './library'
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
/** Recently played ids, newest last. Keeps shuffle from repeating itself. */
let history: string[] = []

function remember(track: Track): void {
  const limit = Math.max(0, getConfig().playback.avoidRepeat)
  history = [...history.filter((id) => id !== track.id), track.id].slice(-limit)
}

/**
 * Picks the next track when the queue has run dry.
 *
 * Album mode carries on through the current album and rolls to another when it
 * ends, so a stream does not stop dead between records.
 */
function autoFillTrack(): Track | null {
  const mode = getConfig().playback.autoFill
  if (mode === 'off') return null

  if (mode === 'album') {
    const from = nowPlaying?.track
    if (from) {
      const next = nextInAlbum(from)
      if (next) return next
      return randomAlbumStart(from.album)
    }
    return randomAlbumStart()
  }
  return randomTrack(new Set(history))
}

/** Appends `count` tracks chosen the same way auto-fill would. */
export function fillQueue(count?: number): number {
  const cfg = getConfig()
  const want = Math.max(1, count ?? cfg.playback.fillCount)
  const room = Math.max(0, cfg.requests.maxQueueLength - queue.length)
  const n = Math.min(want, room)
  if (n === 0) return 0

  // Exclude what is already queued as well as recent history, so a fill does
  // not stack duplicates.
  const taken = new Set([...history, ...queue.map((q) => q.track.id)])
  const added: QueueItem[] = []
  for (let i = 0; i < n; i++) {
    const t = randomTrack(taken)
    if (!t) break
    taken.add(t.id)
    added.push({ id: randomUUID(), track: t, addedAt: Date.now() })
  }
  if (!added.length) return 0
  queue = [...queue, ...added]
  emitQueue()
  emitChat('sys', `Added ${added.length} track${added.length === 1 ? '' : 's'}`)
  if (!nowPlaying) playNext()
  return added.length
}

/** Fisher-Yates over the pending queue; what is already playing is untouched. */
export function shuffleQueue(): void {
  if (queue.length < 2) return
  const next = [...queue]
  for (let i = next.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[next[i], next[j]] = [next[j], next[i]]
  }
  queue = next
  emitQueue()
  emitChat('sys', 'Queue shuffled')
}

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
    const filled = autoFillTrack()
    if (filled) {
      const item: QueueItem = { id: randomUUID(), track: filled, addedAt: Date.now() }
      nowPlaying = item
      remember(filled)
      setNowPlaying(item)
      broadcast(EVENT.nowPlaying, item)
      broadcast(EVENT.playTrack, filled)
      return item
    }
    nowPlaying = null
    setNowPlaying(null)
    broadcast(EVENT.nowPlaying, null)
    return null
  }
  queue = rest
  nowPlaying = next
  remember(next.track)
  emitQueue()
  setNowPlaying(next)
  broadcast(EVENT.nowPlaying, next)
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
