import WebSocket from 'ws'
import { anonNick, backoffDelay, displayName, isPrivileged, parseFrame } from '../../shared/irc'
import { truncateReply } from '../../shared/commands'
import type { ChatPlatform, Requester } from '../../shared/types'
import { getConfig } from '../config'
import { getValidToken } from '../auth/twitchDevice'
import type { ChatSource, ChatSourceHandlers, ChatStatus } from './source'

const IRC_URL = 'wss://irc-ws.chat.twitch.tv:443'

/** A connection alive this long is considered healthy; backoff resets. */
const HEALTHY_AFTER_MS = 60_000

/**
 * Twitch chat.
 *
 * Connects anonymously when there is no token -- read-only, no OAuth, nothing
 * for a viewer to capture off a screen share. With a bot token it authenticates
 * instead, which is the only way to post replies.
 */
export class TwitchSource implements ChatSource {
  readonly platform: ChatPlatform = 'twitch'

  private ws: WebSocket | null = null
  private handlers: ChatSourceHandlers | null = null
  private channel = ''
  private attempt = 0
  private reconnectTimer: NodeJS.Timeout | null = null
  private healthyTimer: NodeJS.Timeout | null = null
  private deliberateClose = false
  private state: ChatStatus = 'disconnected'
  private login: string | null = null

  get canSend(): boolean {
    return this.login !== null && this.state === 'connected'
  }

  status(): ChatStatus {
    return this.state
  }

  async connect(handlers: ChatSourceHandlers): Promise<void> {
    const cfg = getConfig().twitch
    const name = cfg.channel.trim().replace(/^#/, '').toLowerCase()
    if (!name) return

    this.handlers = handlers
    this.channel = name
    this.deliberateClose = false
    this.attempt = 0
    await this.open()
  }

  disconnect(): void {
    this.deliberateClose = true
    this.clearTimers()
    if (this.ws) {
      this.ws.removeAllListeners()
      try {
        this.ws.close()
      } catch {
        /* already closing */
      }
      this.ws = null
    }
    this.setStatus('disconnected')
  }

  async say(text: string): Promise<boolean> {
    if (!this.canSend || !this.ws) return false
    try {
      this.ws.send(`PRIVMSG #${this.channel} :${truncateReply(text)}`)
      return true
    } catch {
      return false
    }
  }

  private setStatus(next: ChatStatus): void {
    if (this.state === next) return
    this.state = next
    this.handlers?.onStatus(next)
  }

  private clearTimers(): void {
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    if (this.healthyTimer) clearTimeout(this.healthyTimer)
    this.reconnectTimer = null
    this.healthyTimer = null
  }

  private async open(): Promise<void> {
    this.clearTimers()
    this.setStatus('connecting')

    // Resolved per connection: the streamer may authorise between attempts.
    const cfg = getConfig().twitch
    const token = cfg.clientId ? await getValidToken(cfg.clientId) : null
    this.login = token?.login ?? null

    const socket = new WebSocket(IRC_URL)
    this.ws = socket

    socket.on('open', () => {
      socket.send('CAP REQ :twitch.tv/tags twitch.tv/commands')
      if (token && this.login) {
        socket.send(`PASS oauth:${token.accessToken}`)
        socket.send(`NICK ${this.login}`)
      } else {
        // justinfan is Twitch's documented anonymous convention.
        socket.send('PASS SCHMOOPIIE')
        socket.send(`NICK ${anonNick()}`)
      }
      socket.send(`JOIN #${this.channel}`)
      this.healthyTimer = setTimeout(() => {
        this.attempt = 0
      }, HEALTHY_AFTER_MS)
    })

    socket.on('message', (data) => this.handleFrame(socket, data.toString()))
    socket.on('error', () => this.setStatus('error'))
    socket.on('close', () => {
      if (socket === this.ws) this.ws = null
      this.clearTimers()
      if (this.deliberateClose) {
        this.setStatus('disconnected')
        return
      }
      this.scheduleReconnect()
    })
  }

  private scheduleReconnect(): void {
    this.setStatus('connecting')
    const delay = backoffDelay(this.attempt)
    this.attempt += 1
    this.reconnectTimer = setTimeout(() => void this.open(), delay)
  }

  private handleFrame(socket: WebSocket, raw: string): void {
    for (const msg of parseFrame(raw)) {
      switch (msg.command) {
        case 'PING':
          // Miss this and Twitch drops the connection after ~5 minutes with no
          // error event of any kind.
          socket.send(`PONG :${msg.trailing ?? 'tmi.twitch.tv'}`)
          break

        case '376': // end of MOTD -- usable from here
        case 'JOIN':
          if (this.state !== 'connected') {
            this.setStatus('connected')
            this.handlers?.onNotice(
              `Connected to #${this.channel}` + (this.login ? ` as ${this.login}` : ' (read-only)')
            )
          }
          break

        case 'NOTICE':
          // Login failures arrive as a NOTICE, not an error frame.
          if (/login authentication failed|improperly formatted auth/i.test(msg.trailing ?? '')) {
            this.login = null
            this.handlers?.onNotice('Twitch rejected the bot token — reauthorise in settings')
          }
          break

        case 'PRIVMSG': {
          const tags = msg.tags
          const requester: Requester = {
            platform: 'twitch',
            // user-id is stable across renames; display-name is only a label.
            id: tags['user-id'] || undefined,
            name: displayName(tags, msg.nick)
          }
          this.handlers?.onMessage({
            requester,
            text: msg.trailing ?? '',
            privileged: isPrivileged(tags)
          })
          break
        }

        case 'RECONNECT':
          try {
            socket.close()
          } catch {
            /* ignore */
          }
          break

        default:
          break
      }
    }
  }
}
