import { protocol, net } from 'electron'
import { pathToFileURL } from 'node:url'
import { resolve, relative, isAbsolute, sep } from 'node:path'

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
        bypassCSP: true
      }
    }
  ])
}

export function handleScheme(isAllowed: (path: string) => boolean): void {
  protocol.handle('bard', (req) => {
    let filePath: string
    try {
      filePath = decodeURIComponent(new URL(req.url).pathname)
    } catch {
      return new Response('Bad Request', { status: 400 })
    }
    if (!isAllowed(filePath)) return new Response('Forbidden', { status: 403 })
    return net.fetch(pathToFileURL(filePath).toString())
  })
}

/**
 * True when `target` resolves to something at or beneath `root`.
 *
 * Compares resolved paths segment-wise rather than with startsWith, so
 * /music-secrets does not read as being inside /music.
 */
export function isInside(root: string, target: string): boolean {
  const rel = relative(resolve(root), resolve(target))
  return rel !== '' && !rel.startsWith('..' + sep) && rel !== '..' && !isAbsolute(rel)
}

/** Builds the guard passed to handleScheme from the configured library roots. */
export function makeIsAllowed(getRoots: () => string[]) {
  return (filePath: string): boolean => {
    if (!filePath) return false
    const roots = getRoots()
    if (roots.length === 0) return false
    return roots.some((root) => isInside(root, filePath))
  }
}
