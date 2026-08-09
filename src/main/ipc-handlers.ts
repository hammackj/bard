import { BrowserWindow, app, dialog, ipcMain, shell } from 'electron'
import { EVENT, INVOKE, SEND, type AppPaths, type AppInfo } from '../shared/ipc'
import { getConfig, setConfig, configPath } from './config'
import { scanLibrary } from './library'
import { getArtwork, artworkDir } from './artwork'
import {
  clearQueue,
  enqueueTrack,
  enqueueTracks,
  playNowById,
  onTrackEnded,
  removeFromQueue,
  reorderQueue,
  skip,
  previous,
  shuffleQueue,
  fillQueue,
  reportPosition,
  refreshEndless,
  dropClaimedFromQueue
} from './queue'
import { setProgress, nowPlayingPath } from './nowplaying'
import { clearStats, getStats } from './stats'
import { getAlbumFlags, setAlbumFlag } from './albums'
import { getLoved, toggleLoved } from './loved'
import { getOverlayUrl } from './overlay'
import { applyGlobalShortcuts } from './shortcuts'
import { sendToMain } from './windows'
import { isRevealable } from '../shared/paths'
import type {
  Config,
  DeepPartial,
  PlaybackProgress,
  ScanProgress,
  Track,
  ContentIdStatus
} from '../shared/types'

export function registerIpcHandlers(): void {
  ipcMain.handle(INVOKE.getConfig, (): Config => getConfig())
  ipcMain.handle(INVOKE.setConfig, (_e, patch: DeepPartial<Config>): Config => {
    const next = setConfig(patch)
    // Media keys are global; re-register as soon as the toggle changes rather
    // than waiting for a restart.
    if (patch.shortcuts?.globalMediaKeys !== undefined) {
      applyGlobalShortcuts(next.shortcuts.globalMediaKeys)
    }
    if (patch.playback?.endless !== undefined || patch.playback?.endlessLookahead !== undefined) {
      refreshEndless()
    }
    return next
  })

  ipcMain.handle(INVOKE.scanLibrary, async (e): Promise<Track[]> => {
    const sender = e.sender
    // Throttle progress pushes -- one IPC message per file would flood the
    // renderer on a large library and make the progress bar the slow part.
    let last = 0
    const onProgress = (p: ScanProgress): void => {
      const now = Date.now()
      if (now - last < 100 && p.scanned !== p.total) return
      last = now
      if (!sender.isDestroyed()) sender.send(EVENT.scanProgress, p)
    }
    return scanLibrary(getConfig().libraryPaths, onProgress)
  })

  ipcMain.handle(INVOKE.addLibraryPath, async (e): Promise<string[]> => {
    const win = BrowserWindow.fromWebContents(e.sender)
    const options = {
      title: 'Add music folder',
      properties: ['openDirectory', 'multiSelections', 'createDirectory'] as const
    }
    const result = win
      ? await dialog.showOpenDialog(win, { ...options, properties: [...options.properties] })
      : await dialog.showOpenDialog({ ...options, properties: [...options.properties] })
    if (result.canceled || result.filePaths.length === 0) return getConfig().libraryPaths

    const merged = [...new Set([...getConfig().libraryPaths, ...result.filePaths])]
    return setConfig({ libraryPaths: merged }).libraryPaths
  })

  ipcMain.handle(INVOKE.getArtwork, (_e, trackId: string): Promise<string | null> => {
    return getArtwork(trackId)
  })

  // Queue intents. Main owns the state; these return void and the renderer
  // re-renders from the onQueueChanged broadcast rather than a return value.
  ipcMain.handle(INVOKE.enqueue, (_e, trackId: string): void => {
    enqueueTrack(trackId)
  })
  ipcMain.handle(INVOKE.enqueueMany, (_e, trackIds: string[], startNow?: boolean): void => {
    enqueueTracks(trackIds, startNow ?? false)
  })
  ipcMain.handle(INVOKE.playNow, (_e, trackId: string): void => {
    playNowById(trackId)
  })
  ipcMain.handle(INVOKE.removeFromQueue, (_e, queueItemId: string): void => {
    removeFromQueue(queueItemId)
  })
  ipcMain.handle(INVOKE.reorderQueue, (_e, from: number, to: number): void => {
    reorderQueue(from, to)
  })
  ipcMain.handle(INVOKE.skip, (): void => skip())
  ipcMain.handle(INVOKE.previous, (): void => previous())
  ipcMain.handle(INVOKE.clearQueue, (): void => clearQueue())
  ipcMain.handle(INVOKE.shuffleQueue, (): void => shuffleQueue())
  ipcMain.handle(INVOKE.fillQueue, (_e, count?: number): void => {
    fillQueue(count)
  })

  ipcMain.handle(INVOKE.getOverlayUrl, (): string | null => getOverlayUrl())

  ipcMain.handle(
    INVOKE.getPaths,
    (): AppPaths => ({
      config: configPath(),
      configDir: app.getPath('userData'),
      artwork: artworkDir(),
      nowPlaying: nowPlayingPath(),
      logs: app.getPath('logs')
    })
  )

  // app.getVersion() reads the packaged package.json, so this is correct in
  // both dev and a built app without bundling the version in at compile time.
  ipcMain.handle(
    INVOKE.getAppInfo,
    (): AppInfo => ({
      version: app.getVersion(),
      electron: process.versions.electron,
      chrome: process.versions.chrome,
      node: process.versions.node
    })
  )

  ipcMain.handle(INVOKE.revealPath, async (_e, target: string): Promise<void> => {
    // Guard against being handed something outside our own directories. Uses
    // the shared segment-wise check, not a prefix match: startsWith admits any
    // sibling, so a userData of ~/.config/bard also admitted ~/.config/bard-evil.
    const roots = [app.getPath('userData'), ...getConfig().libraryPaths]
    if (!isRevealable(roots, target)) return
    await shell.openPath(target)
  })

  // Playback reporting from the renderer.
  ipcMain.on(SEND.reportEnded, (): void => onTrackEnded())
  ipcMain.on(SEND.reportProgress, (_e, p: PlaybackProgress): void => {
    setProgress(p)
    if (p.playing) reportPosition(p.position, p.duration)
  })

  ipcMain.handle(INVOKE.getAlbumFlags, () => getAlbumFlags())
  ipcMain.handle(
    INVOKE.setAlbumFlag,
    (_e, album: string, status: ContentIdStatus, note?: string) => {
      setAlbumFlag(album, status, note)
      const flags = getAlbumFlags()
      sendToMain(EVENT.albumFlagsChanged, flags)
      // A newly claimed album must leave the pending queue, not just future
      // picks -- otherwise it still plays in a few minutes' time.
      dropClaimedFromQueue()
      return flags
    }
  )

  ipcMain.handle(INVOKE.getLoved, () => getLoved())
  ipcMain.handle(INVOKE.toggleLoved, (_e, trackId: string) => {
    toggleLoved(trackId)
    const ids = getLoved()
    sendToMain(EVENT.lovedChanged, ids)
    return ids
  })

  ipcMain.handle(INVOKE.getStats, () => getStats())
  ipcMain.handle(INVOKE.clearStats, (): void => {
    clearStats()
    sendToMain(EVENT.statsChanged, getStats())
  })
}
