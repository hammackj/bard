import WebSocket from 'ws'
import { EVENT } from '../shared/ipc'
import {
  anonNick,
  backoffDelay,
  displayName,
  isPrivileged,
  parseCommand,
  parseFrame
} from '../shared/irc'
import type { Requester, TwitchStatus } from '../shared/types'
import { getConfig } from './config'
import { sendToMain } from './windows'
import { emitChat, getNowPlaying, getQueue, requestFromChat } from './queue'

const IRC_URL = 'wss://irc-ws.chat.twitch.tv:443'

/** A connection alive this long is considered healthy; backoff resets. */
const HEALTHY_AFTER_MS = 60_000

let ws: WebSocket | null = null
let channel = ''
let attempt = 0
let reconnectTimer: NodeJS.Timeout | null = null
let healthyTimer: NodeJS.Timeout | null = null
let deliberateClose = false
let status: TwitchStatus = 'disconnected'

function broadcast(channel: string, payload: unknown): void {
  sendToMain(channel, payload)
}

function setStatus(next: TwitchStatus): void {
  if (status === next) return
  status = next
  broadcast(EVENT.twitchStatus, next)
}

export function getStatus(): TwitchStatus {
  return status
}

export function connectTwitch(target: string): void {
  const name = target.trim().replace(/^#/, '').toLowerCase()
  if (!name) return
  channel = name
  deliberateClose = false
  attempt = 0
  open()
}

export function disconnectTwitch(): void {
  deliberateClose = true
  clearTimers()
  if (ws) {
    ws.removeAllListeners()
    try {
      ws.close()
    } catch {
      /* already closing */
    }
    ws = null
  }
  setStatus('disconnected')
}

function clearTimers(): void {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  if (healthyTimer) {
    clearTimeout(healthyTimer)
    healthyTimer = null
  }
}

function open(): void {
  clearTimers()
  setStatus('connecting')

  const socket = new WebSocket(IRC_URL)
  ws = socket

  socket.on('open', () => {
    // Anonymous read-only: no OAuth, no token to leak on a screen share.
    socket.send('CAP REQ :twitch.tv/tags twitch.tv/commands')
    socket.send('PASS SCHMOOPIIE')
    socket.send(`NICK ${anonNick()}`)
    socket.send(`JOIN #${channel}`)

    healthyTimer = setTimeout(() => {
      attempt = 0
    }, HEALTHY_AFTER_MS)
  })

  socket.on('message', (data) => handleFrame(socket, data.toString()))

  socket.on('error', () => {
    setStatus('error')
  })

  socket.on('close', () => {
    if (socket === ws) ws = null
    clearTimers()
    if (deliberateClose) {
      setStatus('disconnected')
      return
    }
    scheduleReconnect()
  })
}

function scheduleReconnect(): void {
  setStatus('connecting')
  const delay = backoffDelay(attempt)
  attempt += 1
  reconnectTimer = setTimeout(open, delay)
}

function handleFrame(socket: WebSocket, raw: string): void {
  for (const msg of parseFrame(raw)) {
    switch (msg.command) {
      case 'PING':
        // Miss this and Twitch drops the connection after ~5 minutes with no
        // error event of any kind.
        socket.send(`PONG :${msg.trailing ?? 'tmi.twitch.tv'}`)
        break

      case '376': // end of MOTD -- the connection is usable from here
      case 'JOIN':
        if (status !== 'connected') {
          setStatus('connected')
          emitChat('sys', `Connected to #${channel}`)
        }
        break

      case 'PRIVMSG':
        handlePrivmsg(msg.tags, msg.nick, msg.trailing ?? '')
        break

      case 'RECONNECT':
        // Twitch asking us to reconnect; treat as a normal drop.
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

function handlePrivmsg(
  tags: Record<string, string>,
  nick: string | null,
  text: string
): void {
  const parsed = parseCommand(text)
  if (!parsed) return

  const cfg = getConfig()
  // user-id is stable across renames; display-name is only a label.
  const requester: Requester = {
    platform: 'twitch',
    id: tags['user-id'] || undefined,
    name: displayName(tags, nick)
  }
  const prefix = cfg.twitch.commandPrefix.toLowerCase()

  if (parsed.command === prefix) {
    emitChat('req', `${parsed.command} ${parsed.arg}`.trim(), requester)
    if (!parsed.arg) {
      emitChat('no', 'No song given')
      return
    }
    requestFromChat(parsed.arg, requester, isPrivileged(tags))
    return
  }

  // The connection is read-only, so these only produce log entries -- there is
  // no way to reply in chat without OAuth and a bot account.
  if (parsed.command === '!queue') {
    emitChat('req', '!queue', requester)
    emitChat('sys', `Queue length: ${getQueue().length}`)
    return
  }
  if (parsed.command === '!song') {
    emitChat('req', '!song', requester)
    const np = getNowPlaying()
    emitChat('sys', np ? `Now playing: ${np.track.title}` : 'Nothing playing')
  }
}
