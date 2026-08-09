import { app } from 'electron'
import { readFile, writeFile, rename, mkdir } from 'node:fs/promises'
import { join, dirname } from 'node:path'

/**
 * Tracks the user has loved.
 *
 * Its own file, alongside albums.json, for the same reason: this is something
 * the user declared, not a number the app derived. Play counts can be rebuilt
 * by listening again; a lost list of favourites cannot.
 */

const WRITE_DEBOUNCE_MS = 1500

type LovedFile = { version: 1; loved: string[] }

let loved = new Set<string>()
let ready = false
let dirty = false
let timer: NodeJS.Timeout | null = null

function lovedPath(): string {
  return join(app.getPath('userData'), 'loved.json')
}

export async function loadLoved(): Promise<void> {
  if (ready) return
  ready = true
  try {
    const parsed = JSON.parse(await readFile(lovedPath(), 'utf8')) as LovedFile
    if (Array.isArray(parsed?.loved)) loved = new Set(parsed.loved)
  } catch {
    // No file yet: nothing is loved.
  }
}

export function getLoved(): string[] {
  return [...loved]
}

export function isLoved(trackId: string): boolean {
  return loved.has(trackId)
}

export function toggleLoved(trackId: string): boolean {
  if (loved.has(trackId)) loved.delete(trackId)
  else loved.add(trackId)
  schedule()
  return loved.has(trackId)
}

function schedule(): void {
  dirty = true
  if (timer) return
  timer = setTimeout(() => {
    timer = null
    void flushLoved()
  }, WRITE_DEBOUNCE_MS)
}

export async function flushLoved(): Promise<void> {
  if (!dirty) return
  dirty = false
  const target = lovedPath()
  const tmp = `${target}.tmp`
  try {
    await mkdir(dirname(target), { recursive: true })
    await writeFile(tmp, JSON.stringify({ version: 1, loved: [...loved] }, null, 2), 'utf8')
    await rename(tmp, target)
  } catch {
    dirty = true
  }
}
