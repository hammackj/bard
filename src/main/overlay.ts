import express from 'express'
import type { Response } from 'express'
import { createServer, type Server } from 'node:http'
import { join } from 'node:path'
import { existsSync } from 'node:fs'
import { app } from 'electron'
import { artworkDir } from './artwork'
import { onNowPlayingChange, snapshot, setOverlayPort } from './nowplaying'

/** If the configured port is taken, walk upward this many times before failing. */
const PORT_ATTEMPTS = 10

let server: Server | null = null
let boundPort: number | null = null
let clients: Response[] = []
let unsubscribe: (() => void) | null = null

export function getOverlayPort(): number | null {
  return boundPort
}

export function getOverlayUrl(): string | null {
  return boundPort ? `http://127.0.0.1:${boundPort}` : null
}

function overlayRoot(): string {
  // Packaged builds keep resources/overlay outside the asar (extraResources);
  // in dev it sits in the project tree.
  const packaged = join(process.resourcesPath, 'overlay')
  return app.isPackaged && existsSync(packaged)
    ? packaged
    : join(__dirname, '../../resources/overlay')
}

export async function startOverlay(preferredPort: number): Promise<number | null> {
  await stopOverlay()

  const api = express()
  api.disable('x-powered-by')

  api.use(express.static(overlayRoot(), { index: 'index.html', maxAge: 0 }))

  // Artwork is served from the on-disk cache written during the library scan.
  api.get('/art/:trackId', (req, res) => {
    const id = String(req.params.trackId)
    // The id is a hex hash; reject anything else rather than touching the path.
    if (!/^[a-f0-9]{6,64}$/.test(id)) {
      res.status(400).end()
      return
    }
    // Root-relative form: express resolves the name inside artworkDir and
    // refuses to escape it, so this is a second containment check on top of the
    // id pattern above.
    res.sendFile(`${id}.jpg`, { root: artworkDir(), maxAge: '1h' }, (err) => {
      if (!err) return
      const code = (err as NodeJS.ErrnoException).code
      if (code !== 'ENOENT') console.error('[overlay] art send failed:', code ?? err)
      if (!res.headersSent) res.status(404).end()
    })
  })

  api.get('/nowplaying.json', (_req, res) => {
    res.json(snapshot())
  })

  /**
   * SSE rather than polling: the overlay is told about a track change the
   * moment it happens instead of up to a second later.
   */
  api.get('/events', (req, res) => {
    res.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache, no-transform',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no'
    })
    res.write(`data: ${JSON.stringify(snapshot())}\n\n`)
    clients.push(res)
    req.on('close', () => {
      clients = clients.filter((c) => c !== res)
    })
  })

  const http = createServer(api)
  server = http

  const port = await listenWithRetry(http, preferredPort)
  boundPort = port
  // Snapshots build their art URL from this, so it has to be the port we got,
  // not the one we asked for.
  setOverlayPort(port)
  if (port === null) return null

  unsubscribe = onNowPlayingChange((snap) => {
    const frame = `data: ${JSON.stringify(snap)}\n\n`
    for (const c of clients) {
      try {
        c.write(frame)
      } catch {
        /* client vanished mid-write; the close handler will prune it */
      }
    }
  })

  return port
}

/**
 * Binds to 127.0.0.1 explicitly.
 *
 * Never 0.0.0.0: on a shared or public network that would expose the overlay,
 * and the /art route, to anyone who can reach the machine.
 */
function listenWithRetry(http: Server, startPort: number): Promise<number | null> {
  return new Promise((resolve) => {
    let attempt = 0
    const tryPort = (p: number): void => {
      const onError = (err: NodeJS.ErrnoException): void => {
        http.removeListener('error', onError)
        if (err.code === 'EADDRINUSE' && attempt < PORT_ATTEMPTS - 1) {
          attempt++
          tryPort(p + 1)
          return
        }
        resolve(null)
      }
      http.once('error', onError)
      http.listen(p, '127.0.0.1', () => {
        http.removeListener('error', onError)
        resolve(p)
      })
    }
    tryPort(startPort)
  })
}

export function stopOverlay(): Promise<void> {
  unsubscribe?.()
  unsubscribe = null
  for (const c of clients) {
    try {
      c.end()
    } catch {
      /* ignore */
    }
  }
  clients = []
  boundPort = null
  setOverlayPort(null)

  const closing = server
  server = null
  if (!closing) return Promise.resolve()
  return new Promise((resolve) => closing.close(() => resolve()))
}

export function overlayClientCount(): number {
  return clients.length
}
