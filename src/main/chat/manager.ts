import { EVENT } from '../../shared/ipc'
import {
  QUEUE_COMMAND,
  SONG_COMMAND,
  formatQueueReply,
  parseCommand
} from '../../shared/commands'
import type { ChatPlatform, TwitchStatus } from '../../shared/types'
import { getConfig } from '../config'
import { sendToMain } from '../windows'
import { emitChat, getNowPlaying, getQueue, requestFromChat } from '../queue'
import { TwitchSource } from './twitchSource'
import type { ChatMessage, ChatSource } from './source'

/**
 * Owns the chat sources and routes their messages into the request pipeline.
 *
 * Every platform lands here, so the rules, the block list and the command
 * grammar are applied identically no matter where a message came from.
 */

const sources = new Map<ChatPlatform, ChatSource>()

function source(platform: ChatPlatform): ChatSource {
  let s = sources.get(platform)
  if (!s) {
    // YouTube slots in here once credentials exist; see docs/youtube.md.
    s = new TwitchSource()
    sources.set(platform, s)
  }
  return s
}

export function getChatStatus(platform: ChatPlatform = 'twitch'): TwitchStatus {
  return sources.get(platform)?.status() ?? 'disconnected'
}

export function canSend(platform: ChatPlatform = 'twitch'): boolean {
  return sources.get(platform)?.canSend ?? false
}

export async function connectChat(platform: ChatPlatform = 'twitch'): Promise<void> {
  // Returning quietly on a missing channel made Connect look like a dead
  // button. Say so instead.
  if (platform === 'twitch' && !getConfig().twitch.channel.trim()) {
    emitChat('no', 'No channel set — enter one above first')
    throw new Error('No Twitch channel configured')
  }
  await source(platform).connect({
    onMessage: (m) => handleMessage(platform, m),
    onStatus: (s) => sendToMain(EVENT.twitchStatus, s),
    onNotice: (message) => emitChat('sys', message)
  })
}

export function disconnectChat(platform: ChatPlatform = 'twitch'): void {
  sources.get(platform)?.disconnect()
}

export function disconnectAll(): void {
  for (const s of sources.values()) s.disconnect()
}

/**
 * Replies in chat when the platform allows it, and always logs.
 *
 * The log entry is not a consolation prize -- it is what the streamer reads.
 * A read-only connection still shows them what a viewer asked and what the
 * answer was; the viewer just does not see it.
 */
async function reply(platform: ChatPlatform, text: string): Promise<void> {
  emitChat('sys', text)
  const s = sources.get(platform)
  if (s?.canSend) await s.say(text)
}

async function handleMessage(platform: ChatPlatform, m: ChatMessage): Promise<void> {
  const parsed = parseCommand(m.text)
  if (!parsed) return

  const cfg = getConfig()
  const prefix = cfg.twitch.commandPrefix.toLowerCase()

  if (parsed.command === prefix) {
    emitChat('req', `${parsed.command} ${parsed.arg}`.trim(), m.requester)
    if (!parsed.arg) {
      await reply(platform, 'Give me a song name')
      return
    }
    const verdict = requestFromChat(parsed.arg, m.requester, m.privileged)
    // requestFromChat already logs the verdict; only echo it to chat.
    const s = sources.get(platform)
    if (s?.canSend) {
      await s.say(
        verdict.ok
          ? `@${m.requester.name} queued ${verdict.track.title}`
          : `@${m.requester.name} ${describeForChat(verdict.reason)}`
      )
    }
    return
  }

  if (parsed.command === QUEUE_COMMAND) {
    emitChat('req', QUEUE_COMMAND, m.requester)
    await reply(platform, formatQueueReply(getQueue().map((q) => q.track.title)))
    return
  }

  if (parsed.command === SONG_COMMAND) {
    emitChat('req', SONG_COMMAND, m.requester)
    const np = getNowPlaying()
    await reply(
      platform,
      np ? `Now playing: ${np.track.title} — ${np.track.artist}` : 'Nothing playing'
    )
  }
}

/** Shorter phrasing for chat than the streamer-facing log gets. */
function describeForChat(reason: string): string {
  switch (reason) {
    case 'blocked_user':
      return 'you cannot request right now'
    case 'queue_full':
      return 'the queue is full'
    case 'user_limit':
      return 'you already have enough queued'
    case 'cooldown':
      return 'you are on cooldown'
    case 'not_found':
      return 'that is not in the library'
    case 'blocked_track':
      return 'that track is blocked'
    case 'already_queued':
      return 'that is already queued'
    default:
      return 'no'
  }
}
