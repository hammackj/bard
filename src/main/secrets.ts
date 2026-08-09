import { app, safeStorage } from 'electron'
import { readFileSync, writeFileSync, renameSync, existsSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'

/**
 * The obs-websocket password, kept out of config.json.
 *
 * config.json is plain text the user is invited to open -- Settings shows its
 * path and offers a button to reveal it -- so a credential does not belong
 * there, even a localhost one. This goes through Electron safeStorage, which
 * uses the OS keyring where there is one.
 *
 * Where there is not, safeStorage.isEncryptionAvailable() is false and we
 * refuse to store rather than quietly writing plaintext under a name that
 * implies otherwise. Losing the convenience of auto-arm is a smaller harm than
 * a file that looks encrypted and is not.
 */

const FILE = 'obs-password.bin'

function secretPath(): string {
  return join(app.getPath('userData'), FILE)
}

export function secureStorageAvailable(): boolean {
  try {
    return safeStorage.isEncryptionAvailable()
  } catch {
    return false
  }
}

/** True when a password is stored. Never returns the password itself. */
export function hasObsPassword(): boolean {
  return existsSync(secretPath())
}

/**
 * Stores, or clears when given an empty string.
 *
 * Returns false when no OS encryption is available, so the caller can say why
 * rather than reporting a success that did not happen.
 */
export function setObsPassword(password: string): boolean {
  const target = secretPath()
  if (!password) {
    try {
      if (existsSync(target)) unlinkSync(target)
    } catch {
      /* nothing useful to do; treated as cleared */
    }
    return true
  }
  if (!secureStorageAvailable()) return false
  try {
    const tmp = `${target}.tmp`
    writeFileSync(tmp, safeStorage.encryptString(password))
    renameSync(tmp, target)
    return true
  } catch {
    return false
  }
}

/** The stored password, or '' when absent or undecryptable. */
export function getObsPassword(): string {
  const target = secretPath()
  if (!existsSync(target)) return ''
  try {
    return safeStorage.decryptString(readFileSync(target))
  } catch {
    // A keyring that changed under us, or a file from another machine.
    return ''
  }
}
