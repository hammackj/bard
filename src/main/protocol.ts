import { protocol, net } from 'electron'
import { pathToFileURL } from 'node:url'
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

export function handleScheme(isAllowed: (path: string) => boolean): void {
  protocol.handle('bard', async (req) => {
    let filePath: string
    try {
      filePath = decodeURIComponent(new URL(req.url).pathname)
    } catch {
      return new Response('Bad Request', { status: 400 })
    }
    if (!isAllowed(filePath)) return new Response('Forbidden', { status: 403 })

    const res = await net.fetch(pathToFileURL(filePath).toString(), {
      headers: req.headers // forward Range so seeking still works
    })

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
    const headers = new Headers(res.headers)
    headers.set('Access-Control-Allow-Origin', '*')
    headers.set('Cross-Origin-Resource-Policy', 'cross-origin')
    if (!headers.has('Accept-Ranges')) headers.set('Accept-Ranges', 'bytes')

    return new Response(res.body, {
      status: res.status,
      statusText: res.statusText,
      headers
    })
  })
}

