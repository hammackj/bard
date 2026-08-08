/**
 * The single source of truth for IPC.
 *
 * Channel names live here as consts and nowhere else -- no string literals in
 * main, preload or renderer. Both sides import this file, so a renamed channel
 * is a type error rather than a silent no-op at runtime.
 */
import type {
  ChatEvent,
  Config,
  DeepPartial,
  PlaybackProgress,
  ScanProgress,
  QueueItem,
  Track,
  TwitchStatus
} from './types'

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
  connectTwitch: 'twitch:connect',
  disconnectTwitch: 'twitch:disconnect',
  getConfig: 'config:get',
  setConfig: 'config:set'
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
  scanProgress: 'library:scanProgress'
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
  enqueue: (trackId: string, requester?: string) => Promise<void>
  removeFromQueue: (queueItemId: string) => Promise<void>
  reorderQueue: (from: number, to: number) => Promise<void>
  skip: () => Promise<void>
  clearQueue: () => Promise<void>

  // playback reporting (renderer -> main)
  reportProgress: (p: PlaybackProgress) => void
  reportEnded: () => void

  // twitch
  connectTwitch: (channel: string) => Promise<void>
  disconnectTwitch: () => Promise<void>

  // config
  getConfig: () => Promise<Config>
  setConfig: (patch: DeepPartial<Config>) => Promise<Config>

  // subscriptions -- each returns its unsubscribe fn
  onQueueChanged: (cb: (q: QueueItem[]) => void) => Unsubscribe
  onPlayTrack: (cb: (t: Track) => void) => Unsubscribe
  onChatEvent: (cb: (e: ChatEvent) => void) => Unsubscribe
  onTwitchStatus: (cb: (s: TwitchStatus) => void) => Unsubscribe
  onScanProgress: (cb: (p: ScanProgress) => void) => Unsubscribe
}
