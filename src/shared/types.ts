/**
 * Types shared across main, preload and renderer.
 *
 * Anything crossing a process boundary is declared here so both sides agree on
 * the shape. Keep it free of imports from electron or node -- the renderer
 * bundles this file too.
 */

export type RequestConfig = {
  cooldownSeconds: number
  maxQueueLength: number
  maxPerUserInQueue: number
  blockedUsers: string[]
  blockedTracks: string[]
}

export type TwitchConfig = {
  channel: string
  enabled: boolean
  commandPrefix: string
}

export type AudioConfig = {
  volume: number
  outputDeviceId: string
}

export type OverlayConfig = {
  port: number
  writeNowPlayingJson: boolean
}

export type Config = {
  libraryPaths: string[]
  twitch: TwitchConfig
  requests: RequestConfig
  audio: AudioConfig
  overlay: OverlayConfig
}

export const DEFAULT_CONFIG: Config = {
  libraryPaths: [],
  twitch: { channel: '', enabled: false, commandPrefix: '!sr' },
  requests: {
    cooldownSeconds: 120,
    maxQueueLength: 20,
    maxPerUserInQueue: 2,
    blockedUsers: [],
    blockedTracks: []
  },
  audio: { volume: 0.8, outputDeviceId: 'default' },
  overlay: { port: 4747, writeNowPlayingJson: true }
}

/** Recursive partial, for config patches coming over IPC. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K]
}
