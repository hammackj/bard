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

/**
 * True when `target` is one of `roots` or sits beneath one.
 *
 * The guard on revealPath, which is a wider hole than it looks: shell.openPath
 * does not merely highlight a folder, it hands the path to the desktop's
 * default handler -- so a .desktop file under a directory we admit by mistake
 * is executed. Unlike isInside, the roots themselves pass, because "Open
 * config folder" reveals exactly the userData root.
 */
export function isRevealable(roots: string[], target: string): boolean {
  if (!target) return false
  return roots.some((root) => resolve(root) === resolve(target) || isInside(root, target))
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
