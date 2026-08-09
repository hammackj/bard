/**
 * The single source of truth for IPC.
 *
 * Channel names live here as consts and nowhere else -- no string literals in
 * main, preload or renderer. Both sides import this file, so a renamed channel
 * is a type error rather than a silent no-op at runtime.
 */
import type {
  Config,
  DeepPartial,
  PlaybackProgress,
  ScanProgress,
  QueueItem,
  Track,
  TrackStats,
  AlbumFlag,
  ContentIdStatus
} from './types'

export type TransportAction = 'toggle' | 'restart'

export type AppPaths = {
  config: string
  configDir: string
  artwork: string
  nowPlaying: string
  logs: string
}

/**
 * Build identity, for the About row and for bug reports.
 *
 * The runtime versions matter as much as ours: an audio or rendering fault is
 * usually a Chromium version's fault, and "which Bard?" is unanswerable from a
 * screenshot without them.
 */
export type AppInfo = {
  version: string
  /**
   * Short commit this build came from, `-dirty` when it carried uncommitted
   * changes, or 'unknown' when built without git.
   *
   * The version only moves at release, so it cannot distinguish two builds
   * between releases -- which is how a packaged build ran for a day without
   * the ReplayGain code while the source had it.
   */
  commit: string
  electron: string
  chrome: string
  node: string
}

/** Renderer -> main, expects a reply (ipcRenderer.invoke). */
export const INVOKE = {
  scanLibrary: 'library:scan',
  addLibraryPath: 'library:addPath',
  getArtwork: 'library:artwork',
  enqueue: 'queue:enqueue',
  enqueueMany: 'queue:enqueueMany',
  playNow: 'queue:playNow',
  removeFromQueue: 'queue:remove',
  reorderQueue: 'queue:reorder',
  skip: 'queue:skip',
  previous: 'queue:previous',
  clearQueue: 'queue:clear',
  shuffleQueue: 'queue:shuffle',
  fillQueue: 'queue:fill',
  getConfig: 'config:get',
  setConfig: 'config:set',
  getOverlayUrl: 'overlay:url',
  getPaths: 'app:paths',
  getAppInfo: 'app:info',
  getStreamerSource: 'streamer:getSource',
  revealPath: 'app:revealPath',
  secureStorageAvailable: 'app:secureStorage',
  setObsPassword: 'streamer:setObsPassword',
  getStats: 'stats:get',
  clearStats: 'stats:clear',
  getAlbumFlags: 'albums:get',
  setAlbumFlag: 'albums:set',
  getLoved: 'loved:get',
  toggleLoved: 'loved:toggle'
} as const

/** Renderer -> main, fire and forget (ipcRenderer.send). */
export const SEND = {
  reportProgress: 'playback:progress',
  reportEnded: 'playback:ended'
} as const

/** Main -> renderer pushes (webContents.send). */
export const EVENT = {
  queueChanged: 'queue:changed',
  playTrack: 'player:play',
  scanProgress: 'library:scanProgress',
  nowPlaying: 'player:nowPlaying',
  statsChanged: 'stats:changed',
  albumFlagsChanged: 'albums:changed',
  lovedChanged: 'loved:changed',
  transport: 'player:transport',
  /** Human-readable message from main, for things the user must be told. */
  notice: 'app:notice',
  /** Config changed in main rather than through the renderer -- see onConfigChanged. */
  configChanged: 'config:changed',
  /** What armed streamer mode, so the banner can name it. */
  streamerSource: 'streamer:source'
} as const

/**
 * Which OBS output armed streamer mode, or null when it was armed by hand.
 *
 * Runtime state, deliberately not config: it describes what OBS is doing right
 * now, and persisting it across a restart would make the banner claim a
 * recording that stopped hours ago.
 */
export type StreamerSource = 'stream' | 'record' | 'replay' | 'virtualcam' | null

/** Unsubscribe handle returned by every onX subscription. */
export type Unsubscribe = () => void

export type BardApi = {
  // library
  scanLibrary: () => Promise<Track[]>
  /** Opens a directory dialog; resolves to the full list of library paths. */
  addLibraryPath: () => Promise<string[]>
  /** Data URL, or null when the track has no embedded art. */
  getArtwork: (trackId: string) => Promise<string | null>

  // queue -- the renderer sends intents, main owns the state
  enqueue: (trackId: string) => Promise<void>
  /**
   * Appends a batch in one call. Must be used for anything multi-track: a loop
   * of single enqueues lets auto-fill interleave after the first one.
   * `startNow` begins playback on the batch's first track.
   */
  enqueueMany: (trackIds: string[], startNow?: boolean) => Promise<void>
  /** Plays this track at once, leaving the pending queue as it was. */
  playNow: (trackId: string) => Promise<void>
  removeFromQueue: (queueItemId: string) => Promise<void>
  reorderQueue: (from: number, to: number) => Promise<void>
  skip: () => Promise<void>
  /** Steps back through play history; restarts the track if far enough in. */
  previous: () => Promise<void>
  clearQueue: () => Promise<void>
  /** Reorders the pending queue at random. */
  shuffleQueue: () => Promise<void>
  /** Appends `count` tracks chosen by the current auto-fill mode. */
  fillQueue: (count?: number) => Promise<void>

  // playback reporting (renderer -> main)
  reportProgress: (p: PlaybackProgress) => void
  reportEnded: () => void

  // config
  getConfig: () => Promise<Config>
  setConfig: (patch: DeepPartial<Config>) => Promise<Config>

  /** URL of the running overlay server, or null when it failed to bind. */
  getOverlayUrl: () => Promise<string | null>

  /** Where config, artwork cache and nowplaying.json live on disk. */
  getPaths: () => Promise<AppPaths>
  /** App version plus the Electron/Chrome/Node it is running on. */
  getAppInfo: () => Promise<AppInfo>
  /** Read once on mount; thereafter it arrives via onStreamerSource. */
  getStreamerSource: () => Promise<StreamerSource>
  /** Opens a folder in the system file manager. */
  revealPath: (path: string) => Promise<void>

  /** False when no OS keyring is available to encrypt tokens at rest. */
  secureStorageAvailable: () => Promise<boolean>
  /**
   * Stores the obs-websocket password, or clears it when given ''.
   *
   * Resolves false when no OS encryption is available -- the password is not
   * stored in that case rather than being written as plaintext.
   */
  setObsPassword: (password: string) => Promise<boolean>

  /** Play counts keyed by track id. */
  getStats: () => Promise<Record<string, TrackStats>>
  clearStats: () => Promise<void>

  /** Content ID results, keyed by album name. */
  getAlbumFlags: () => Promise<Record<string, AlbumFlag>>
  setAlbumFlag: (
    album: string,
    status: ContentIdStatus,
    note?: string
  ) => Promise<Record<string, AlbumFlag>>

  /** Track ids the user has loved. */
  getLoved: () => Promise<string[]>
  toggleLoved: (trackId: string) => Promise<string[]>

  // subscriptions -- each returns its unsubscribe fn
  onQueueChanged: (cb: (q: QueueItem[]) => void) => Unsubscribe
  onPlayTrack: (cb: (t: Track) => void) => Unsubscribe
  onScanProgress: (cb: (p: ScanProgress) => void) => Unsubscribe
  /** The queue item now playing, carrying requester info. */
  onNowPlaying: (cb: (i: QueueItem | null) => void) => Unsubscribe
  /** Transport commands originating outside the renderer (tray, media keys). */
  onTransport: (cb: (a: TransportAction) => void) => Unsubscribe
  /**
   * Messages main needs the user to see -- a refusal and its reason, chiefly.
   * Streamer mode blocks tracks, and a block the user cannot see is
   * indistinguishable from a broken player.
   */
  onNotice: (cb: (message: string) => void) => Unsubscribe
  /**
   * Config that main changed on its own.
   *
   * Normally the renderer owns config edits and holds the result, but OBS
   * auto-arming flips streamer mode from main -- without this the toggle would
   * keep showing "off" while the mode was enforcing.
   */
  onConfigChanged: (cb: (c: Config) => void) => Unsubscribe
  /** What armed streamer mode; null when armed by hand. */
  onStreamerSource: (cb: (s: StreamerSource) => void) => Unsubscribe
  onStatsChanged: (cb: (s: Record<string, TrackStats>) => void) => Unsubscribe
  onAlbumFlagsChanged: (cb: (f: Record<string, AlbumFlag>) => void) => Unsubscribe
  onLovedChanged: (cb: (ids: string[]) => void) => Unsubscribe
}
