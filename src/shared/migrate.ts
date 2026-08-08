import type { BlockedUser, Config } from './types'

/**
 * Brings older config shapes forward on read.
 *
 * Kept pure and separate from the electron-store wrapper so it can be tested
 * without Electron. Migrations run on every read and must be idempotent -- the
 * result is only written back when something else saves the config.
 */
export function migrateConfig(cfg: Config): Config {
  return { ...cfg, requests: { ...cfg.requests, blockedUsers: migrateBlocked(cfg.requests.blockedUsers) } }
}

/**
 * blockedUsers used to be a flat string[] of Twitch names.
 *
 * Those entries are adopted as twitch blocks rather than dropped. Silently
 * un-blocking somebody a streamer had deliberately blocked would be the worst
 * possible regression of the multi-platform change.
 */
function migrateBlocked(value: BlockedUser[] | string[] | undefined): BlockedUser[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((entry): BlockedUser[] => {
    if (typeof entry === 'string') {
      const name = entry.trim()
      return name ? [{ platform: 'twitch', name, at: 0 }] : []
    }
    if (entry && typeof entry === 'object' && typeof entry.name === 'string') {
      return [{ ...entry, platform: entry.platform ?? 'twitch' }]
    }
    return []
  })
}
