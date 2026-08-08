import type { ChatPlatform, Requester, TwitchStatus } from '../../shared/types'

/** Connection state. Shared across platforms; the names came from Twitch. */
export type ChatStatus = TwitchStatus

export type ChatMessage = {
  requester: Requester
  text: string
  /** Broadcaster or moderator: bypasses cooldown and the per-user cap. */
  privileged: boolean
}

export type ChatSourceHandlers = {
  onMessage: (m: ChatMessage) => void
  onStatus: (s: ChatStatus) => void
  /** Connection notices for the chat log, not chat traffic. */
  onNotice: (message: string) => void
}

/**
 * One chat platform.
 *
 * Everything above this interface -- the request rules, the block list, the
 * command router -- is platform-agnostic. Adding a platform means implementing
 * this and nothing else.
 */
export interface ChatSource {
  readonly platform: ChatPlatform

  /** False until the platform has credentials that allow sending. */
  readonly canSend: boolean

  status(): ChatStatus

  connect(handlers: ChatSourceHandlers): Promise<void>
  disconnect(): void

  /**
   * Posts to chat. Resolves false when the source is read-only, so callers can
   * fall back to a log entry instead of pretending the reply went out.
   */
  say(text: string): Promise<boolean>
}
