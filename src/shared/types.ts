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
  /** ReplayGain, in dB, when the file carries the tags. */
  trackGainDb?: number
  albumGainDb?: number
  /** Linear sample peak, used to hold back gain that would clip. */
  trackPeak?: number
  albumPeak?: number
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
  /**
   * The track the audio element is actually playing, which during a crossfade
   * is still the outgoing one -- main has already selected the next track by
   * then, so an untagged report gets credited to the wrong track. Null before
   * anything has been loaded.
   */
  trackId: string | null
}

export type ScanProgress = {
  scanned: number
  total: number
  currentPath: string
}

/** Which ReplayGain value to apply, if any. */
export type NormalizeMode = 'off' | 'track' | 'album'

export type AudioConfig = {
  volume: number
  outputDeviceId: string
  /**
   * Loudness normalisation. 'album' preserves the intended dynamics between
   * tracks on a record; 'track' evens everything out, which is what shuffle
   * wants.
   */
  normalize: NormalizeMode
  /** Crossfade between tracks, in seconds. 0 is a hard cut. */
  crossfadeSec: number
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
 *  off     -- silence; nothing plays until something is queued
 *  shuffle -- a random track, avoiding the recent window
 *  album   -- carry on through the current album, then roll to another
 */
export type AutoFillMode = 'off' | 'shuffle' | 'album'

/**
 * off  -- advance normally
 * one  -- replay the current track until changed
 * all  -- when the queue empties, replay what was just played
 */
export type RepeatMode = 'off' | 'one' | 'all'

export type PlaybackConfig = {
  autoFill: AutoFillMode
  /** How many recently played tracks shuffle refuses to repeat. */
  avoidRepeat: number
  /** How many tracks the "fill queue" action adds. */
  fillCount: number
  repeat: RepeatMode
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

export type TrayPopupCorner = 'bottom-right' | 'bottom-left' | 'top-right' | 'top-left' | 'cursor'

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

/**
 * Streamer mode: while armed, only albums flagged `safe` may play.
 *
 * Stricter than `playback.excludeClaimed`, in two ways. It is an allow-list
 * rather than a block-list, so `untested` is withheld too -- an album nobody
 * has checked is not evidence of safety. And it applies to deliberate play as
 * well as auto-selection, because the risk it guards against does not care
 * whether a human picked the track.
 */
export type StreamerConfig = {
  /** Armed. Only ever set true by detection; clearing it is always deliberate. */
  enabled: boolean
  /** Arm automatically when OBS reports streaming or recording. */
  autoArm: boolean
  /** obs-websocket endpoint. Ships inside OBS 28+, off by default, port 4455. */
  obsUrl: string
  /**
   * Whether an obs-websocket password has been stored.
   *
   * The password itself is not kept here -- config.json is plain text on disk.
   * It goes through Electron safeStorage; this flag only tells the UI whether
   * one is set, so it can offer "replace" rather than showing a blank field.
   */
  hasObsPassword: boolean
}

export type Config = {
  libraryPaths: string[]
  audio: AudioConfig
  overlay: OverlayConfig
  shortcuts: ShortcutConfig
  window: WindowConfig
  playback: PlaybackConfig
  streamer: StreamerConfig
}

export const DEFAULT_CONFIG: Config = {
  libraryPaths: [],
  audio: { volume: 0.8, outputDeviceId: 'default', normalize: 'track', crossfadeSec: 0 },
  overlay: { port: 4747, writeNowPlayingJson: true },
  shortcuts: { globalMediaKeys: false },
  window: { closeToTray: true, trayPopupCorner: 'bottom-right', trayPopupSeconds: 8 },
  playback: {
    autoFill: 'shuffle',
    repeat: 'off',
    avoidRepeat: 50,
    fillCount: 10,
    maxQueueLength: 200,
    endless: true,
    endlessLookahead: 10,
    excludeClaimed: true,
    lovedWeight: 3
  },
  streamer: {
    enabled: false,
    autoArm: false,
    obsUrl: 'ws://127.0.0.1:4455',
    hasObsPassword: false
  }
}

/** Recursive partial, for config patches coming over IPC. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K]
}
