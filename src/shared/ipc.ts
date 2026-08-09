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
  TrackStats
} from './types'

export type TransportAction = 'toggle' | 'restart'

export type AppPaths = {
  config: string
  configDir: string
  artwork: string
  nowPlaying: string
  logs: string
}

/** Renderer -> main, expects a reply (ipcRenderer.invoke). */
export const INVOKE = {
  scanLibrary: 'library:scan',
  addLibraryPath: 'library:addPath',
  getArtwork: 'library:artwork',
  enqueue: 'queue:enqueue',
  removeFromQueue: 'queue:remove',
  reorderQueue: 'queue:reorder',
  skip: 'queue:skip',
  clearQueue: 'queue:clear',
  shuffleQueue: 'queue:shuffle',
  fillQueue: 'queue:fill',
  getConfig: 'config:get',
  setConfig: 'config:set',
  getOverlayUrl: 'overlay:url',
  getPaths: 'app:paths',
  revealPath: 'app:revealPath',
  secureStorageAvailable: 'app:secureStorage',
  openExternal: 'app:openExternal',
  getStats: 'stats:get',
  clearStats: 'stats:clear'
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
  transport: 'player:transport'
} as const

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
  removeFromQueue: (queueItemId: string) => Promise<void>
  reorderQueue: (from: number, to: number) => Promise<void>
  skip: () => Promise<void>
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
  /** Opens a folder in the system file manager. */
  revealPath: (path: string) => Promise<void>


  /** False when no OS keyring is available to encrypt tokens at rest. */
  secureStorageAvailable: () => Promise<boolean>
  /** Opens an allowlisted https URL in the system browser. */
  openExternal: (url: string) => Promise<boolean>

  /** Play counts keyed by track id. */
  getStats: () => Promise<Record<string, TrackStats>>
  clearStats: () => Promise<void>

  // subscriptions -- each returns its unsubscribe fn
  onQueueChanged: (cb: (q: QueueItem[]) => void) => Unsubscribe
  onPlayTrack: (cb: (t: Track) => void) => Unsubscribe
  onScanProgress: (cb: (p: ScanProgress) => void) => Unsubscribe
  /** The queue item now playing, carrying requester info. */
  onNowPlaying: (cb: (i: QueueItem | null) => void) => Unsubscribe
  /** Transport commands originating outside the renderer (tray, media keys). */
  onTransport: (cb: (a: TransportAction) => void) => Unsubscribe
  onStatsChanged: (cb: (s: Record<string, TrackStats>) => void) => Unsubscribe
}
