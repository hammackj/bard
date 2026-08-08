import { describe, expect, it } from 'vitest'
import { migrateConfig } from '../src/shared/migrate'
import { DEFAULT_CONFIG, type Config } from '../src/shared/types'

const withBlocked = (blocked: unknown): Config =>
  ({
    ...DEFAULT_CONFIG,
    requests: { ...DEFAULT_CONFIG.requests, blockedUsers: blocked }
  }) as Config

describe('migrateConfig', () => {
  it('adopts a legacy string[] block list as twitch blocks', () => {
    // Dropping these would silently un-block someone the streamer had blocked.
    const out = migrateConfig(withBlocked(['OldTroll', 'another_user']))
    expect(out.requests.blockedUsers).toEqual([
      { platform: 'twitch', name: 'OldTroll', at: 0 },
      { platform: 'twitch', name: 'another_user', at: 0 }
    ])
  })

  it('leaves an already-migrated list untouched', () => {
    const current = [{ platform: 'youtube' as const, id: 'UC1', name: 'dave', at: 42 }]
    expect(migrateConfig(withBlocked(current)).requests.blockedUsers).toEqual(current)
  })

  it('is idempotent', () => {
    const once = migrateConfig(withBlocked(['dave']))
    const twice = migrateConfig(once)
    expect(twice.requests.blockedUsers).toEqual(once.requests.blockedUsers)
  })

  it('defaults a missing platform to twitch', () => {
    const out = migrateConfig(withBlocked([{ name: 'dave', at: 1 }]))
    expect(out.requests.blockedUsers[0].platform).toBe('twitch')
  })

  it('drops empty and malformed entries', () => {
    const out = migrateConfig(withBlocked(['', '   ', null, 7, { at: 1 }]))
    expect(out.requests.blockedUsers).toEqual([])
  })

  it('survives a missing block list entirely', () => {
    expect(migrateConfig(withBlocked(undefined)).requests.blockedUsers).toEqual([])
  })

  it('does not disturb the rest of the config', () => {
    const out = migrateConfig(withBlocked(['dave']))
    expect(out.audio).toEqual(DEFAULT_CONFIG.audio)
    expect(out.requests.cooldownSeconds).toBe(DEFAULT_CONFIG.requests.cooldownSeconds)
  })
})
