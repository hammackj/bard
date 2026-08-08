/**
 * Types shared across main, preload and renderer.
 *
 * Anything crossing a process boundary is declared here so both sides agree on
 * the shape. Keep it free of imports from electron or node -- the renderer
 * bundles this file too.
 */

/** Chat platforms Bard can take requests from. */
export type ChatPlatform = 'twitch' | 'youtube'

/**
 * Who asked for something.
 *
 * `id` is the platform's stable user id where it provides one. Display names
 * can be changed at will, so anything that must survive a rename -- blocks
 * above all -- keys on the id and treats the name as a label.
 */
export type Requester = {
  platform: ChatPlatform
  id?: string
  name: string
}

export type BlockedUser = {
  platform: ChatPlatform
  id?: string
  /** Name at the time of blocking; the fallback match when there is no id. */
  name: string
  at: number
}

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
  /** Absent when the streamer queued it themselves from the UI. */
  requester?: Requester
  addedAt: number
}

/** Prefixes match the chat log's 3-char gutter: sys / req / ok / no. */
export type ChatEventKind = 'sys' | 'req' | 'ok' | 'no'

export type ChatEvent = {
  id: string
  kind: ChatEventKind
  message: string
  /** Carried so the log can offer "block" on the line that provoked it. */
  requester?: Requester
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
  blockedUsers: BlockedUser[]
  blockedTracks: string[]
}

export type TwitchConfig = {
  channel: string
  enabled: boolean
  commandPrefix: string
  /**
   * Client ID of an application registered at dev.twitch.tv.
   * Public clients only -- Device Code Flow forbids a client secret, so there
   * is nothing confidential to ship in the bundle.
   */
  clientId: string
  /** Post replies in chat. Requires an authorised bot token. */
  replyInChat: boolean
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

/**
 * What happens when the queue runs dry.
 *  off     -- silence; the streamer or chat must queue something
 *  shuffle -- a random track, avoiding the recent window
 *  album   -- carry on through the current album, then roll to another
 */
export type AutoFillMode = 'off' | 'shuffle' | 'album'

export type PlaybackConfig = {
  autoFill: AutoFillMode
  /** How many recently played tracks shuffle refuses to repeat. */
  avoidRepeat: number
  /** How many tracks the "fill queue" action adds. */
  fillCount: number
}

export type TrayPopupCorner =
  | 'bottom-right'
  | 'bottom-left'
  | 'top-right'
  | 'top-left'
  | 'cursor'

export type WindowConfig = {
  /** Close hides to the tray instead of quitting. */
  closeToTray: boolean
  /**
   * Where the tray preview appears. Honoured on X11, Windows and macOS;
   * Wayland compositors place windows themselves and ignore it.
   */
  trayPopupCorner: TrayPopupCorner
  /** Seconds before the preview hides itself. 0 keeps it until dismissed. */
  trayPopupSeconds: number
}

export type Config = {
  libraryPaths: string[]
  twitch: TwitchConfig
  requests: RequestConfig
  audio: AudioConfig
  overlay: OverlayConfig
  shortcuts: ShortcutConfig
  window: WindowConfig
  playback: PlaybackConfig
}

export const DEFAULT_CONFIG: Config = {
  libraryPaths: [],
  twitch: { channel: '', enabled: false, commandPrefix: '!sr', clientId: '', replyInChat: false },
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
  window: { closeToTray: true, trayPopupCorner: 'bottom-right', trayPopupSeconds: 8 },
  playback: { autoFill: 'off', avoidRepeat: 50, fillCount: 10 }
}

/** Recursive partial, for config patches coming over IPC. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K]
}
