import { randomUUID } from 'node:crypto'
import { EVENT } from '../shared/ipc'
import type { QueueItem, Track } from '../shared/types'
import { getConfig } from './config'
import { sendToMain } from './windows'
import { allTracks, getTrackById, randomTrack } from './library'
import { claimedAlbums } from './albums'
import { isLoved } from './loved'
import { nextInAlbumRolling, shuffled } from '../shared/playlist'
import { draw, type Deck } from '../shared/deck'
import { pushPlayed, stepBack, wrapForRepeatAll } from '../shared/history'
import { setNowPlaying, setQueueLength } from './nowplaying'
import { PLAY_THRESHOLD_RATIO, PLAY_THRESHOLD_SEC, getStats, recordPlay, recordSkip } from './stats'

/**
 * The queue lives here and nowhere else.
 *
 * The renderer sends intents and re-renders whatever comes back, so there is
 * one source of truth for what plays next.
 */

let queue: QueueItem[] = []
let nowPlaying: QueueItem | null = null
/** Whether the current track has already been counted, and how far it got. */
let counted = false
let position = 0
/** Recently played ids, newest last. Keeps shuffle from repeating itself. */
let history: string[] = []
/**
 * Tracks actually played, newest last, for stepping backwards.
 *
 * Separate from `history`, which exists to stop shuffle repeating itself and
 * is capped by a different setting. Conflating them would make the back button
 * depend on a shuffle preference.
 */
let played: QueueItem[] = []
/** Set while stepping back, so the item is not re-pushed onto the stack. */

/** Shuffle deck: dealt through completely before any track repeats. */
let deck: Deck | null = null

/**
 * Tracks eligible for automatic play.
 *
 * Albums flagged as Content ID claimed are withheld from shuffle, fill and
 * endless -- the whole point of testing them is to keep them from turning up
 * unattended mid-stream. Playing one deliberately is still allowed: that is an
 * explicit choice, not the player deciding for you.
 */
function playableTracks(): Track[] {
  const all = allTracks()
  if (!getConfig().playback.excludeClaimed) return all
  const blocked = claimedAlbums()
  if (blocked.size === 0) return all
  const left = all.filter((t) => !blocked.has(t.album))
  // If every album is flagged, fall back to the whole library rather than
  // falling silent -- silence would look like a bug, not a policy.
  return left.length > 0 ? left : all
}

/** Draws the next track from the deck, rebuilding it when exhausted. */
function drawFromDeck(): Track | null {
  const ids = playableTracks().map((t) => t.id)
  const weight = Math.max(1, Math.floor(getConfig().playback.lovedWeight))
  const out = draw(deck, ids, Math.random, nowPlaying?.track.id, (id) =>
    weight > 1 && isLoved(id) ? weight : 1
  )
  deck = out.deck
  return out.id ? (getTrackById(out.id) ?? null) : null
}

/**
 * Tops the queue up so playback never runs out.
 *
 * Deliberately fills the visible queue rather than picking a track at the last
 * moment: you can see what is coming, and reorder or drop it, instead of being
 * handed whatever the player decided a second ago.
 */
function topUpEndless(): void {
  const cfg = getConfig().playback
  if (process.env.BARD_ENDLESS_DEBUG) {
    console.log(
      `[endless] on=${cfg.endless} look=${cfg.endlessLookahead} queue=${queue.length} ` +
        `library=${allTracks().length}`
    )
  }
  if (!cfg.endless) return
  const want = Math.max(1, cfg.endlessLookahead)
  if (queue.length >= want) return

  const taken = new Set(queue.map((q) => q.track.id))
  const added: QueueItem[] = []
  // The anchor advances with every pick. Album mode walks forward from the last
  // track that will be queued, not from what is playing now.
  let anchor: Track | null = queue.at(-1)?.track ?? nowPlaying?.track ?? null

  let guard = 0
  while (queue.length + added.length < want && guard++ < want * 6) {
    const next =
      cfg.autoFill === 'album' ? nextInAlbumRolling(playableTracks(), anchor) : drawFromDeck()
    if (!next) break
    anchor = next
    // A duplicate is skipped, not treated as failure: the deck will deal
    // something else next time round.
    if (taken.has(next.id)) continue
    taken.add(next.id)
    added.push({ id: randomUUID(), track: next, addedAt: Date.now() })
  }
  if (!added.length) return
  queue = [...queue, ...added]
  emitQueue()
}

function remember(track: Track): void {
  const limit = Math.max(0, getConfig().playback.avoidRepeat)
  history = [...history.filter((id) => id !== track.id), track.id].slice(-limit)
}

function broadcast(channel: string, payload: unknown): void {
  sendToMain(channel, payload)
}

/**
 * Counts a play once the track has actually been listened to.
 *
 * Counting on start would make skipping through the library inflate every
 * number; 30 seconds, or half of a short track, is the point at which it was
 * genuinely played rather than sampled.
 */
export function reportPosition(seconds: number, duration: number): void {
  position = seconds
  if (counted || !nowPlaying) return
  const enough =
    seconds >= PLAY_THRESHOLD_SEC || (duration > 0 && seconds >= duration * PLAY_THRESHOLD_RATIO)
  if (!enough) return
  counted = true
  recordPlay(nowPlaying.track.id)
  broadcast(EVENT.statsChanged, getStats())
}

/** Called when playback leaves a track before it counted as played. */
function creditSkipIfUnplayed(): void {
  if (!nowPlaying || counted || position <= 1) return
  recordSkip(nowPlaying.track.id)
  broadcast(EVENT.statsChanged, getStats())
}

function emitQueue(): void {
  broadcast(EVENT.queueChanged, queue)
  setQueueLength(queue.length)
}

export function getQueue(): QueueItem[] {
  return queue
}

export function getNowPlaying(): QueueItem | null {
  return nowPlaying
}

/**
 * Picks the next track when the queue has run dry.
 *
 * Album mode carries on through the current album and rolls to another when it
 * ends, so playback does not stop dead between records.
 */
function autoFillTrack(): Track | null {
  const mode = getConfig().playback.autoFill
  if (mode === 'off') return null

  if (mode === 'album') {
    return nextInAlbumRolling(playableTracks(), nowPlaying?.track ?? null)
  }
  return drawFromDeck() ?? randomTrack(new Set(history))
}

export function enqueueTrack(trackId: string): QueueItem | null {
  const track = getTrackById(trackId)
  if (!track) return null
  const item: QueueItem = { id: randomUUID(), track, addedAt: Date.now() }
  queue = [...queue, item]
  emitQueue()
  if (!nowPlaying) playNext()
  return item
}

/** Ids withheld from automatic selection because their album was claimed. */
function excludedIds(): string[] {
  const playable = new Set(playableTracks().map((t) => t.id))
  return allTracks()
    .filter((t) => !playable.has(t.id))
    .map((t) => t.id)
}

/** Appends `count` tracks chosen the same way auto-fill would. */
export function fillQueue(count?: number): number {
  const cfg = getConfig()
  const want = Math.max(1, count ?? cfg.playback.fillCount)
  const room = Math.max(0, cfg.playback.maxQueueLength - queue.length)
  const n = Math.min(want, room)
  if (n === 0) return 0

  // Exclude what is already queued as well as recent history, so a fill does
  // not stack duplicates.
  const taken = new Set([...history, ...queue.map((q) => q.track.id)])
  const added: QueueItem[] = []
  for (let i = 0; i < n; i++) {
    const t = randomTrack(new Set([...taken, ...excludedIds()]))
    if (!t) break
    taken.add(t.id)
    added.push({ id: randomUUID(), track: t, addedAt: Date.now() })
  }
  if (!added.length) return 0
  queue = [...queue, ...added]
  emitQueue()
  if (!nowPlaying) playNext()
  return added.length
}

/** Fisher-Yates over the pending queue; what is already playing is untouched. */
export function shuffleQueue(): void {
  if (queue.length < 2) return
  queue = shuffled(queue)
  emitQueue()
}

export function removeFromQueue(queueItemId: string): void {
  const before = queue.length
  queue = queue.filter((i) => i.id !== queueItemId)
  if (queue.length !== before) {
    emitQueue()
    topUpEndless()
  }
}

/**
 * Removes newly-claimed albums from the pending queue.
 *
 * Excluding them from future picks is not enough: anything already queued
 * would still play, which on a stream is exactly the moment it matters.
 */
export function dropClaimedFromQueue(): void {
  if (!getConfig().playback.excludeClaimed) return
  const blocked = claimedAlbums()
  if (blocked.size === 0) return
  const before = queue.length
  queue = queue.filter((i) => !blocked.has(i.track.album))
  if (queue.length !== before) {
    emitQueue()
    topUpEndless()
  }
}

/** Called when the endless setting changes, so it takes effect immediately. */
export function refreshEndless(): void {
  topUpEndless()
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
  creditSkipIfUnplayed()
  counted = false
  position = 0

  // Stock the queue before drawing from it.
  //
  // Order matters: auto-fill returns a track whenever the library is non-empty,
  // so doing it first meant the "queue is empty" branch always won and played
  // directly, leaving the queue permanently empty. Endless would then be
  // invisible -- music kept playing but nothing was ever queued to look at.
  topUpEndless()

  const [next, ...rest] = queue
  if (!next) {
    // Not endless: fall back to a single auto-filled track, or stop.
    const filled = autoFillTrack()
    if (filled) {
      const item: QueueItem = { id: randomUUID(), track: filled, addedAt: Date.now() }
      nowPlaying = item
      remember(filled)
      played = pushPlayed(played, item)
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
  played = pushPlayed(played, next)
  emitQueue()
  topUpEndless()
  setNowPlaying(next)
  broadcast(EVENT.nowPlaying, next)
  broadcast(EVENT.playTrack, next.track)
  return next
}

export function skip(): void {
  playNext()
}

/**
 * Back one track, or restart the current one.
 *
 * Restarting when a few seconds in matches every other player: pressing back
 * mid-track almost always means "start this again", and only a quick
 * double-press means "the one before".
 */
export function previous(): void {
  const step = stepBack(played, position)
  if (step.kind === 'restart') {
    if (!nowPlaying) return
    counted = false
    position = 0
    broadcast(EVENT.playTrack, nowPlaying.track)
    return
  }

  creditSkipIfUnplayed()
  played = step.played
  // Put the current track back at the head so forward still works.
  if (nowPlaying) queue = [nowPlaying, ...queue]
  nowPlaying = step.prior
  counted = false
  position = 0
  emitQueue()
  setNowPlaying(step.prior)
  broadcast(EVENT.nowPlaying, step.prior)
  broadcast(EVENT.playTrack, step.prior.track)
}

/** Called when the renderer reports the current track finished. */
export function onTrackEnded(): void {
  const mode = getConfig().playback.repeat
  if (mode === 'one' && nowPlaying) {
    counted = false
    position = 0
    broadcast(EVENT.playTrack, nowPlaying.track)
    return
  }
  if (mode === 'all' && queue.length === 0 && played.length > 0) {
    // Re-queue what has been played, in order, and carry on.
    const wrapped = wrapForRepeatAll(played, nowPlaying)
    queue = wrapped.queue.map((i) => ({ ...i, id: randomUUID(), addedAt: Date.now() }))
    played = wrapped.played
    emitQueue()
  }
  playNext()
}

/**
 * Plays a specific track immediately, leaving the pending queue intact.
 *
 * The renderer used to express this as enqueue-then-skip, which cannot work:
 * enqueueTrack appends to the tail and skip() shifts the head, so the user got
 * whatever was already queued and their choice was buried at the back. With
 * endless on and an empty queue it was worse -- the enqueue started the right
 * track, the top-up stuffed the queue in front of it, and the skip immediately
 * abandoned it for filler.
 *
 * Deliberately does not consume the queue: "play this" is an interruption, and
 * what was lined up should still be lined up afterwards.
 */
export function playNowById(trackId: string): QueueItem | null {
  const track = getTrackById(trackId)
  if (!track) return null

  creditSkipIfUnplayed()
  counted = false
  position = 0

  const item: QueueItem = { id: randomUUID(), track, addedAt: Date.now() }
  nowPlaying = item
  remember(track)
  played = pushPlayed(played, item)
  setNowPlaying(item)
  broadcast(EVENT.nowPlaying, item)
  broadcast(EVENT.playTrack, track)
  topUpEndless()
  return item
}
