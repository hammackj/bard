import { app, safeStorage } from 'electron'
import { readFile, writeFile, rm, mkdir } from 'node:fs/promises'
import { join } from 'node:path'

/**
 * OAuth tokens, encrypted at rest.
 *
 * These are kept out of config.json deliberately. config.json is plain text a
 * streamer may well open, screen-share or paste into a bug report, and a
 * chat:edit token in there could post as the bot from anywhere.
 *
 * Encryption uses Electron's safeStorage, which is backed by the OS keychain
 * (kwallet/gnome-keyring on Linux, Keychain on macOS, DPAPI on Windows). Where
 * no keyring is available the token is NOT written in the clear -- it is held
 * in memory for the session only, and the user is told they will have to
 * re-authorise next launch. Silently downgrading to plaintext would be the
 * wrong trade for a credential.
 */

export type StoredToken = {
  accessToken: string
  refreshToken?: string
  /** Epoch millis. */
  expiresAt?: number
  /** Login name the token belongs to, resolved at authorisation time. */
  login?: string
  scopes?: string[]
}

/** Session-only fallback when the OS has no usable keyring. */
const memory = new Map<string, StoredToken>()

function tokenPath(name: string): string {
  return join(app.getPath('userData'), 'tokens', `${name}.enc`)
}

export function encryptionAvailable(): boolean {
  try {
    return safeStorage.isEncryptionAvailable()
  } catch {
    return false
  }
}

export async function saveToken(name: string, token: StoredToken): Promise<void> {
  memory.set(name, token)
  if (!encryptionAvailable()) return
  const dir = join(app.getPath('userData'), 'tokens')
  await mkdir(dir, { recursive: true })
  const blob = safeStorage.encryptString(JSON.stringify(token))
  await writeFile(tokenPath(name), blob)
}

export async function loadToken(name: string): Promise<StoredToken | null> {
  const cached = memory.get(name)
  if (cached) return cached
  if (!encryptionAvailable()) return null
  try {
    const blob = await readFile(tokenPath(name))
    const token = JSON.parse(safeStorage.decryptString(blob)) as StoredToken
    memory.set(name, token)
    return token
  } catch {
    // Missing, or written under a keyring we can no longer read. Either way the
    // only recovery is to authorise again.
    return null
  }
}

export async function clearToken(name: string): Promise<void> {
  memory.delete(name)
  try {
    await rm(tokenPath(name), { force: true })
  } catch {
    /* already gone */
  }
}

/** True when the token is absent or within `skewMs` of expiring. */
export function isExpired(token: StoredToken | null, skewMs = 60_000): boolean {
  if (!token) return true
  if (!token.expiresAt) return false
  return Date.now() + skewMs >= token.expiresAt
}
