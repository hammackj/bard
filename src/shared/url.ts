/**
 * Converting between filesystem paths and bard:// URLs.
 *
 * Deliberately free of node:path. The renderer builds these URLs and runs
 * sandboxed, where node modules cannot be resolved -- so the round trip has to
 * be pure string work that both sides can import.
 */

/**
 * The path component of a bard:// URL for a local file.
 *
 * Backslashes are normalised first and the leading slash is always present, so
 * a Windows path becomes /C:/Music/a.flac. Without that leading slash the
 * drive letter was encoded whole (C%3A) and came back out of URL.pathname as a
 * relative-looking segment; path.win32.resolve then read it against the
 * current working drive and produced <cwd-drive>:\C:\Music\a.flac, which is
 * inside no configured root, so the handler answered 403 for every track and
 * nothing played at all on Windows.
 *
 * Each segment is encoded individually rather than running encodeURI over the
 * whole path: encodeURI leaves '#' and '?' intact, and a track called
 * "What's Next? .flac" would have its path truncated at the query separator.
 */
export function toUrlPath(filePath: string): string {
  const slashed = filePath.replace(/\\/g, '/')
  const absolute = slashed.startsWith('/') ? slashed : `/${slashed}`
  return absolute.split('/').map(encodeURIComponent).join('/')
}

/**
 * Reverses toUrlPath, given a URL's pathname.
 *
 * The leading slash belongs to the URL grammar rather than to the path, so it
 * is dropped ahead of a drive letter -- /C:/Music/a.flac is the URL spelling
 * of C:/Music/a.flac. POSIX paths keep theirs, where it is the root.
 */
export function fromUrlPath(pathname: string): string {
  const decoded = decodeURIComponent(pathname)
  return /^\/[A-Za-z]:/.test(decoded) ? decoded.slice(1) : decoded
}

/** The bard:// URL that serves a local file. */
export function trackUrl(filePath: string): string {
  return `bard://local${toUrlPath(filePath)}`
}
