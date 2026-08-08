/**
 * The single source of truth for IPC.
 *
 * Channel names live here as consts and nowhere else -- no string literals in
 * main, preload or renderer. Both sides import this file, so a renamed channel
 * is a type error rather than a silent no-op at runtime.
 */
import type {
  ChatEvent,
  Requester,
  Config,
  DeepPartial,
  PlaybackProgress,
  ScanProgress,
  QueueItem,
  Track,
  TwitchStatus
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
  connectTwitch: 'twitch:connect',
  disconnectTwitch: 'twitch:disconnect',
  getConfig: 'config:get',
  setConfig: 'config:set',
  getOverlayUrl: 'overlay:url',
  getPaths: 'app:paths',
  revealPath: 'app:revealPath',
  blockUser: 'requests:block',
  unblockUser: 'requests:unblock'
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
  chatEvent: 'chat:event',
  twitchStatus: 'twitch:status',
  scanProgress: 'library:scanProgress',
  nowPlaying: 'player:nowPlaying',
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

  // twitch
  connectTwitch: (channel: string) => Promise<void>
  disconnectTwitch: () => Promise<void>

  // config
  getConfig: () => Promise<Config>
  setConfig: (patch: DeepPartial<Config>) => Promise<Config>

  /** URL of the running overlay server, or null when it failed to bind. */
  getOverlayUrl: () => Promise<string | null>

  /** Where config, artwork cache and nowplaying.json live on disk. */
  getPaths: () => Promise<AppPaths>
  /** Opens a folder in the system file manager. */
  revealPath: (path: string) => Promise<void>

  /** Blocks a requester from using commands. Returns the updated config. */
  blockUser: (r: Requester) => Promise<Config>
  unblockUser: (r: Requester) => Promise<Config>

  // subscriptions -- each returns its unsubscribe fn
  onQueueChanged: (cb: (q: QueueItem[]) => void) => Unsubscribe
  onPlayTrack: (cb: (t: Track) => void) => Unsubscribe
  onChatEvent: (cb: (e: ChatEvent) => void) => Unsubscribe
  onTwitchStatus: (cb: (s: TwitchStatus) => void) => Unsubscribe
  onScanProgress: (cb: (p: ScanProgress) => void) => Unsubscribe
  /** The queue item now playing, carrying requester info. */
  onNowPlaying: (cb: (i: QueueItem | null) => void) => Unsubscribe
  /** Transport commands originating outside the renderer (tray, media keys). */
  onTransport: (cb: (a: TransportAction) => void) => Unsubscribe
}
