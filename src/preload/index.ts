import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { EVENT, INVOKE, SEND, type BardApi, type Unsubscribe } from '../shared/ipc'

/**
 * `electron` is the only module this file may import.
 *
 * The window runs sandboxed, so a preload cannot resolve anything from
 * node_modules at runtime. It also deliberately does not re-export a generic
 * ipcRenderer: every channel the renderer can reach is one named in BardApi.
 */

/**
 * Wraps ipcRenderer.on so callers get an unsubscribe function back.
 *
 * Every subscription must be torn down in its effect cleanup -- under HMR the
 * renderer re-runs effects without reloading the preload, so listeners
 * accumulate and each chat message renders once per stale listener.
 */
function subscribe<T>(channel: string, cb: (payload: T) => void): Unsubscribe {
  const listener = (_e: IpcRendererEvent, payload: T): void => cb(payload)
  ipcRenderer.on(channel, listener)
  return () => {
    ipcRenderer.removeListener(channel, listener)
  }
}

const api: BardApi = {
  scanLibrary: () => ipcRenderer.invoke(INVOKE.scanLibrary),
  addLibraryPath: () => ipcRenderer.invoke(INVOKE.addLibraryPath),
  getArtwork: (trackId) => ipcRenderer.invoke(INVOKE.getArtwork, trackId),

  enqueue: (trackId) => ipcRenderer.invoke(INVOKE.enqueue, trackId),
  removeFromQueue: (queueItemId) => ipcRenderer.invoke(INVOKE.removeFromQueue, queueItemId),
  reorderQueue: (from, to) => ipcRenderer.invoke(INVOKE.reorderQueue, from, to),
  skip: () => ipcRenderer.invoke(INVOKE.skip),
  previous: () => ipcRenderer.invoke(INVOKE.previous),
  clearQueue: () => ipcRenderer.invoke(INVOKE.clearQueue),
  shuffleQueue: () => ipcRenderer.invoke(INVOKE.shuffleQueue),
  fillQueue: (count) => ipcRenderer.invoke(INVOKE.fillQueue, count),

  reportProgress: (p) => ipcRenderer.send(SEND.reportProgress, p),
  reportEnded: () => ipcRenderer.send(SEND.reportEnded),


  getConfig: () => ipcRenderer.invoke(INVOKE.getConfig),
  setConfig: (patch) => ipcRenderer.invoke(INVOKE.setConfig, patch),
  getOverlayUrl: () => ipcRenderer.invoke(INVOKE.getOverlayUrl),
  getPaths: () => ipcRenderer.invoke(INVOKE.getPaths),
  getAppInfo: () => ipcRenderer.invoke(INVOKE.getAppInfo),
  revealPath: (path) => ipcRenderer.invoke(INVOKE.revealPath, path),
  secureStorageAvailable: () => ipcRenderer.invoke(INVOKE.secureStorageAvailable),
  openExternal: (url) => ipcRenderer.invoke(INVOKE.openExternal, url),
  getStats: () => ipcRenderer.invoke(INVOKE.getStats),
  clearStats: () => ipcRenderer.invoke(INVOKE.clearStats),
  getAlbumFlags: () => ipcRenderer.invoke(INVOKE.getAlbumFlags),
  setAlbumFlag: (album, status, note) =>
    ipcRenderer.invoke(INVOKE.setAlbumFlag, album, status, note),
  getLoved: () => ipcRenderer.invoke(INVOKE.getLoved),
  toggleLoved: (trackId) => ipcRenderer.invoke(INVOKE.toggleLoved, trackId),

  onQueueChanged: (cb) => subscribe(EVENT.queueChanged, cb),
  onPlayTrack: (cb) => subscribe(EVENT.playTrack, cb),
  onScanProgress: (cb) => subscribe(EVENT.scanProgress, cb),
  onNowPlaying: (cb) => subscribe(EVENT.nowPlaying, cb),
  onTransport: (cb) => subscribe(EVENT.transport, cb),
  onStatsChanged: (cb) => subscribe(EVENT.statsChanged, cb),
  onAlbumFlagsChanged: (cb) => subscribe(EVENT.albumFlagsChanged, cb),
  onLovedChanged: (cb) => subscribe(EVENT.lovedChanged, cb)
}

if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld('bard', api)
  } catch (error) {
    console.error('[preload] failed to expose bridge', error)
  }
} else {
  // @ts-ignore (declared in index.d.ts)
  window.bard = api
}
