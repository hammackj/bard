import { protocol } from 'electron'
import { createReadStream } from 'node:fs'
import { stat } from 'node:fs/promises'
import { extname } from 'node:path'
import { Readable } from 'node:stream'
import { fromUrlPath } from '../shared/url'
export { isInside, makeIsAllowed } from '../shared/paths'

/**
 * The renderer is served from http://localhost:5173 in dev, so it cannot load
 * file:// URLs. Everything local goes through bard:// instead.
 *
 * `stream: true` is required -- without it the renderer cannot issue range
 * requests and seeking within a track silently fails.
 */
export function registerScheme(): void {
  protocol.registerSchemesAsPrivileged([
    {
      scheme: 'bard',
      privileges: {
        standard: true,
        secure: true,
        stream: true,
        supportFetchAPI: true,
        bypassCSP: true,
        /**
         * Required, and easy to miss.
         *
         * The renderer is served from http://localhost in dev, so bard:// is a
         * different origin. Without corsEnabled, Chromium refuses CORS for
         * custom schemes outright ("Cross origin requests are only supported
         * for protocol schemes: chrome, ... http, https"), and the audio
         * element's crossOrigin request is blocked before it starts.
         *
         * With it off but crossOrigin unset, the request succeeds yet the
         * MediaElementAudioSourceNode is tainted and emits pure zeroes -- the
         * failure is inaudible and nothing throws. Both halves are needed.
         */
        corsEnabled: true
      }
    }
  ])
}

const MIME: Record<string, string> = {
  '.flac': 'audio/flac',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.ogg': 'audio/ogg',
  '.opus': 'audio/ogg',
  '.m4a': 'audio/mp4',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.png': 'image/png',
  '.webp': 'image/webp'
}

/**
 * Parses a single-range `bytes=` header against a known file size.
 *
 * Only the one-range form is handled, which is all a media element ever sends.
 * Anything malformed or out of bounds returns null so the caller answers 416
 * rather than guesses.
 */
export function parseRange(
  header: string | null,
  size: number
): { start: number; end: number } | null {
  if (!header) return null
  const m = /^bytes=(\d*)-(\d*)$/.exec(header.trim())
  if (!m) return null
  const [, rawStart, rawEnd] = m
  if (rawStart === '' && rawEnd === '') return null

  // "bytes=-500" means the last 500 bytes, not "from 0 to 500".
  if (rawStart === '') {
    const length = Number(rawEnd)
    if (length <= 0) return null
    return { start: Math.max(0, size - length), end: size - 1 }
  }

  const start = Number(rawStart)
  if (start >= size) return null
  const end = rawEnd === '' ? size - 1 : Math.min(Number(rawEnd), size - 1)
  if (end < start) return null
  return { start, end }
}

/**
 * Serves local files, with real range support.
 *
 * This used to hand the request to net.fetch on a file:// URL and forward the
 * Range header, which looks right and is not: net.fetch ignores Range for
 * file://, answers 200, and reports no Content-Length. A media element that
 * cannot learn the resource size treats the stream as unseekable, so every
 * seek was silently dropped -- the progress bar moved and playback carried on
 * from where it was. Serving the bytes here is what makes seeking work.
 */
export function handleScheme(isAllowed: (path: string) => boolean): void {
  protocol.handle('bard', async (req) => {
    let filePath: string
    try {
      filePath = fromUrlPath(new URL(req.url).pathname)
    } catch {
      return new Response('Bad Request', { status: 400 })
    }
    if (!isAllowed(filePath)) return new Response('Forbidden', { status: 403 })

    let size: number
    try {
      const info = await stat(filePath)
      if (!info.isFile()) return new Response('Not Found', { status: 404 })
      size = info.size
    } catch {
      return new Response('Not Found', { status: 404 })
    }

    /**
     * CORS headers are load-bearing, not boilerplate.
     *
     * The renderer runs on http://localhost:5173 in dev while audio comes from
     * the bard:// origin. Feeding a cross-origin media element into a
     * MediaElementAudioSourceNode taints it, and Chromium silently substitutes
     * ZEROES for the audio -- playback position still advances, the element
     * still reports playing, and nothing throws. It is inaudible failure.
     *
     * The element sets crossOrigin='anonymous'; this is the matching half.
     */
    const headers = new Headers({
      'Access-Control-Allow-Origin': '*',
      'Cross-Origin-Resource-Policy': 'cross-origin',
      'Accept-Ranges': 'bytes',
      'Content-Type': MIME[extname(filePath).toLowerCase()] ?? 'application/octet-stream'
    })

    const range = parseRange(req.headers.get('range'), size)
    if (req.headers.get('range')) {
      if (!range) {
        headers.set('Content-Range', `bytes */${size}`)
        return new Response(null, { status: 416, headers })
      }
      headers.set('Content-Range', `bytes ${range.start}-${range.end}/${size}`)
      headers.set('Content-Length', String(range.end - range.start + 1))
      return new Response(toWeb(filePath, range), { status: 206, headers })
    }

    headers.set('Content-Length', String(size))
    return new Response(toWeb(filePath), { status: 200, headers })
  })
}

function toWeb(path: string, range?: { start: number; end: number }): ReadableStream {
  return Readable.toWeb(createReadStream(path, range)) as ReadableStream
}
