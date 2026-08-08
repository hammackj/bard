/**
 * Types shared across main, preload and renderer.
 *
 * Anything crossing a process boundary is declared here so both sides agree on
 * the shape. Keep it free of imports from electron or node -- the renderer
 * bundles this file too.
 */

export type Track = {
  /** Stable hash of the absolute path. */
  id: string
  path: string
  /** Tag title, else the filename without extension. */
  title: string
  artist: string
  album: string
  durationSec: number
  hasArtwork: boolean
  /** Lowercased "title artist album" -- what Fuse indexes. */
  searchText: string
}

export type QueueItem = {
  /** Identifies this queue entry, not the track -- the same track may be queued twice. */
  id: string
  track: Track
  /** Twitch username, or undefined when the streamer queued it from the UI. */
  requestedBy?: string
  addedAt: number
}

/** Prefixes match the chat log's 3-char gutter: sys / req / ok / no. */
export type ChatEventKind = 'sys' | 'req' | 'ok' | 'no'

export type ChatEvent = {
  id: string
  kind: ChatEventKind
  message: string
  user?: string
  at: number
}

export type TwitchStatus = 'connected' | 'connecting' | 'disconnected' | 'error'

export type NowPlaying = {
  title: string
  artist: string
  album: string
  durationSec: number
  positionSec: number
  requestedBy?: string
  isPlaying: boolean
  artUrl: string | null
  queueLength: number
  updatedAt: string
}

export type PlaybackProgress = {
  position: number
  duration: number
  playing: boolean
}

export type ScanProgress = {
  scanned: number
  total: number
  currentPath: string
}

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

export type ShortcutConfig = {
  /**
   * System-wide media keys. Off by default -- grabbing the media keys for the
   * whole machine without being asked is hostile, and would hijack them from
   * whatever else the streamer has open.
   */
  globalMediaKeys: boolean
}

export type WindowConfig = {
  /** Close hides to the tray instead of quitting. */
  closeToTray: boolean
}

export type Config = {
  libraryPaths: string[]
  twitch: TwitchConfig
  requests: RequestConfig
  audio: AudioConfig
  overlay: OverlayConfig
  shortcuts: ShortcutConfig
  window: WindowConfig
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
  overlay: { port: 4747, writeNowPlayingJson: true },
  shortcuts: { globalMediaKeys: false },
  window: { closeToTray: true }
}

/** Recursive partial, for config patches coming over IPC. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K]
}
