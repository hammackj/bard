import { resolve, relative, isAbsolute, sep } from 'node:path'

/**
 * True when `target` resolves to something strictly beneath `root`.
 *
 * Compared segment-wise rather than with startsWith, so /music-secrets does
 * not read as being inside /music. This is the guard on the bard:// handler,
 * so a false positive here means the renderer can read arbitrary files.
 */
export function isInside(root: string, target: string): boolean {
  const rel = relative(resolve(root), resolve(target))
  return rel !== '' && !rel.startsWith('..' + sep) && rel !== '..' && !isAbsolute(rel)
}

/** Builds the guard used by the bard:// protocol handler. */
export function makeIsAllowed(getRoots: () => string[]) {
  return (filePath: string): boolean => {
    if (!filePath) return false
    const roots = getRoots()
    if (roots.length === 0) return false
    return roots.some((root) => isInside(root, filePath))
  }
}
