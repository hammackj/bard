import { createHash } from 'node:crypto'
import WebSocket from 'ws'
import { getConfig } from './config'
import { getObsPassword } from './secrets'

/**
 * obs-websocket v5 client, used for one thing: knowing when OBS goes live.
 *
 * The server ships inside OBS 28+ on port 4455 but is off by default, so this
 * is entirely opt-in and every failure path has to be survivable -- a user who
 * never enables it should notice nothing at all.
 *
 * Only ever *arms* streamer mode. Disarming is always deliberate, so a
 * websocket that drops mid-stream cannot silently remove protection, and a
 * reconnect cannot cut audio. That asymmetry is the whole safety argument: the
 * failure modes of this file are "did not arm when it could have", never "went
 * unprotected without saying so".
 */

/** Reconnect backoff, milliseconds. Caps so a closed OBS is cheap to wait on. */
const RETRY_MS = [2_000, 5_000, 10_000, 30_000, 60_000]

/**
 * Outputs that count as "going out".
 *
 * Stream and record are the obvious two. Replay buffer and virtual camera are
 * included because they are equally capable of capturing audio that ends up
 * somewhere public -- a buffer clip saved to a highlight, a virtual camera fed
 * into a call being recorded at the other end.
 *
 * The cost is that anyone leaving the replay buffer running permanently is
 * effectively always armed. That is the safe direction to be wrong in, and it
 * is still one click to turn off.
 */
const LIVE_EVENTS = [
  'StreamStateChanged',
  'RecordStateChanged',
  'ReplayBufferStateChanged',
  'VirtualcamStateChanged'
]

/** Status requests asked on connect, so an already-live OBS arms immediately. */
const LIVE_REQUESTS = [
  'GetStreamStatus',
  'GetRecordStatus',
  'GetReplayBufferStatus',
  'GetVirtualCamStatus'
]

/** obs-websocket close code for a rejected password. */
const AUTH_FAILED = 4009

function debug(...args: unknown[]): void {
  if (process.env.BARD_OBS_DEBUG) console.log('[obs]', ...args)
}

type Opcode = 0 | 1 | 2 | 5 | 6 | 7
type Message = { op: Opcode; d: Record<string, unknown> }

let socket: WebSocket | null = null
let retry = 0
let timer: NodeJS.Timeout | null = null
let stopped = true
/** Set once per connection so a flapping OBS cannot spam the notice strip. */
let toldUser = false

let onLive: (() => void) | null = null
let onStatus: ((message: string) => void) | null = null

/**
 * obs-websocket v5 auth: base64(sha256(base64(sha256(password + salt)) + challenge)).
 *
 * Only sent when the server offers a challenge; an unauthenticated server
 * omits it, and sending one anyway is rejected.
 */
function authString(password: string, salt: string, challenge: string): string {
  const secret = createHash('sha256')
    .update(password + salt)
    .digest('base64')
  return createHash('sha256')
    .update(secret + challenge)
    .digest('base64')
}

function scheduleReconnect(): void {
  if (stopped || timer) return
  const wait = RETRY_MS[Math.min(retry, RETRY_MS.length - 1)]
  retry++
  timer = setTimeout(() => {
    timer = null
    connect()
  }, wait)
}

function connect(): void {
  if (stopped || socket) return

  const { obsUrl } = getConfig().streamer
  debug('connecting to', obsUrl)
  let ws: WebSocket
  try {
    ws = new WebSocket(obsUrl, { handshakeTimeout: 4000 })
  } catch {
    scheduleReconnect()
    return
  }
  socket = ws

  ws.on('message', (raw) => {
    let msg: Message
    try {
      msg = JSON.parse(String(raw)) as Message
    } catch {
      return
    }

    // Hello -> Identify. Subscribing to Outputs (1 << 6) is what delivers
    // StreamStateChanged and RecordStateChanged.
    if (msg.op === 0) {
      const auth = msg.d.authentication as { salt: string; challenge: string } | undefined
      const identify: Record<string, unknown> = { rpcVersion: 1, eventSubscriptions: 1 << 6 }
      if (auth) {
        const password = getObsPassword()
        if (!password) {
          report('OBS needs a password for its WebSocket server — set one in Settings')
          stop()
          return
        }
        // Length only, never the password itself: enough to tell "decrypted to
        // nothing useful" from "wrong password" without putting a credential
        // in a log file.
        debug('password decrypted, length', password.length)
        identify.authentication = authString(password, auth.salt, auth.challenge)
      }
      debug('sending Identify; auth challenge present:', Boolean(auth))
      ws.send(JSON.stringify({ op: 1, d: identify }))
      return
    }

    // Identified: ask once whether we are already live, so arming does not wait
    // for the next state change.
    if (msg.op === 2) {
      retry = 0
      toldUser = false
      debug('identified; asking current output state')
      for (const requestType of LIVE_REQUESTS) {
        ws.send(JSON.stringify({ op: 6, d: { requestType, requestId: requestType } }))
      }
      return
    }

    // Event.
    if (msg.op === 5) {
      const type = String(msg.d.eventType)
      const data = (msg.d.eventData ?? {}) as Record<string, unknown>
      if (LIVE_EVENTS.includes(type)) {
        debug('event', type, 'outputActive=', data.outputActive)
        if (data.outputActive === true) arm(type)
      }
      return
    }

    // Request response.
    if (msg.op === 7) {
      const data = (msg.d.responseData ?? {}) as Record<string, unknown>
      const requestId = String(msg.d.requestId)
      // Each status request answers under its own field name as well as
      // outputActive, depending on the request; outputActive is the common one.
      if (data.outputActive === true) {
        debug('already active at connect:', requestId)
        arm(requestId)
      }
    }
  })

  ws.on('error', () => {
    // Errors here are ordinary: OBS closed, websocket server disabled, wrong
    // port. Reported once per connection, never repeatedly.
    if (!toldUser) {
      toldUser = true
      report(`Could not reach OBS at ${obsUrl} — streamer mode will not arm itself`)
    }
  })

  ws.on('close', (code, reason) => {
    debug('closed', code, String(reason))
    if (socket === ws) socket = null

    // A rejected password is not a transient fault -- retrying forever would
    // hammer OBS and never say why. Stop, and tell the user once.
    if (code === AUTH_FAILED) {
      report('OBS rejected the password for its WebSocket server — check it in Settings')
      stopObsWatch()
      return
    }
    scheduleReconnect()
  })
}

function arm(source: string): void {
  debug('arming from', source)
  onLive?.()
}

function report(message: string): void {
  onStatus?.(message)
}

/** Starts watching OBS. Safe to call repeatedly; a no-op when already running. */
export function startObsWatch(handlers: {
  onLive: () => void
  onStatus: (m: string) => void
}): void {
  onLive = handlers.onLive
  onStatus = handlers.onStatus
  if (!stopped) return
  stopped = false
  retry = 0
  connect()
}

export function stopObsWatch(): void {
  stopped = true
  if (timer) {
    clearTimeout(timer)
    timer = null
  }
  const ws = socket
  socket = null
  try {
    ws?.close()
  } catch {
    /* already gone */
  }
}

/** Restarts the watcher so a changed URL or password takes effect at once. */
export function restartObsWatch(handlers: {
  onLive: () => void
  onStatus: (m: string) => void
}): void {
  stopObsWatch()
  startObsWatch(handlers)
}

function stop(): void {
  stopObsWatch()
}
