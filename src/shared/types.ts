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

/**
 * Result of checking an album against YouTube Content ID.
 *
 * 'claimed' does not mean the music is unlicensed -- a rights-holder matching
 * their own catalogue is common and benign. It means this album produced a
 * claim, which is a reason to keep it off a stream until you know why.
 */
export type ContentIdStatus = 'untested' | 'safe' | 'claimed'

export type AlbumFlag = {
  status: ContentIdStatus
  /** Who claimed it, or anything else worth remembering. */
  note?: string
  checkedAt?: number
}

export type TrackStats = {
  plays: number
  skips: number
  lastPlayedAt?: number
  lastSkippedAt?: number
}

export type QueueItem = {
  /** Identifies this queue entry, not the track -- the same track may be queued twice. */
  id: string
  track: Track
  addedAt: number
}

export type NowPlaying = {
  title: string
  artist: string
  album: string
  durationSec: number
  positionSec: number
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
  /** Upper bound on the queue, so Fill cannot run away with the library. */
  maxQueueLength: number
  /**
   * Never stop. The queue is kept topped up ahead of the current track, so
   * playback continues indefinitely and you can still see and reorder what is
   * coming.
   */
  endless: boolean
  /** How many tracks endless mode keeps queued ahead. */
  endlessLookahead: number
  /**
   * Keep albums marked as Content ID claimed out of shuffle, fill and endless.
   * Playing one deliberately still works; this only stops them turning up on
   * their own while a stream is running.
   */
  excludeClaimed: boolean
  /**
   * How many times a loved track appears in each shuffle pass. 1 disables the
   * preference; higher values surface favourites more often without ever
   * starving the rest of the library, since every track still appears.
   */
  lovedWeight: number
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
  audio: AudioConfig
  overlay: OverlayConfig
  shortcuts: ShortcutConfig
  window: WindowConfig
  playback: PlaybackConfig
}

export const DEFAULT_CONFIG: Config = {
  libraryPaths: [],
  audio: { volume: 0.8, outputDeviceId: 'default' },
  overlay: { port: 4747, writeNowPlayingJson: true },
  shortcuts: { globalMediaKeys: false },
  window: { closeToTray: true, trayPopupCorner: 'bottom-right', trayPopupSeconds: 8 },
  playback: {
    autoFill: 'shuffle',
    avoidRepeat: 50,
    fillCount: 10,
    maxQueueLength: 200,
    endless: true,
    endlessLookahead: 10,
    excludeClaimed: true,
    lovedWeight: 3
  }
}

/** Recursive partial, for config patches coming over IPC. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K]
}
