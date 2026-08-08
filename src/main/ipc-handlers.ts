import { BrowserWindow, app, dialog, ipcMain, shell } from 'electron'
import { EVENT, INVOKE, SEND, type AppPaths } from '../shared/ipc'
import { getConfig, setConfig, configPath } from './config'
import { scanLibrary } from './library'
import { getArtwork } from './artwork'
import {
  clearQueue,
  enqueueTrack,
  onTrackEnded,
  removeFromQueue,
  reorderQueue,
  skip,
  shuffleQueue,
  fillQueue
} from './queue'
import { connectTwitch, disconnectTwitch } from './twitch'
import { setProgress, nowPlayingPath } from './nowplaying'
import { artworkDir } from './artwork'
import { getOverlayUrl } from './overlay'
import { applyGlobalShortcuts } from './shortcuts'
import type { Config, DeepPartial, PlaybackProgress, ScanProgress, Track } from '../shared/types'

/**
 * Registers the handlers that exist so far. Channels in BardApi without a
 * handler yet reject with "No handler registered", which is the behaviour we
 * want mid-build -- a loud failure rather than a silent undefined.
 */
export function registerIpcHandlers(): void {
  ipcMain.handle(INVOKE.getConfig, (): Config => getConfig())
  ipcMain.handle(INVOKE.setConfig, (_e, patch: DeepPartial<Config>): Config => {
    const next = setConfig(patch)
    // Media keys are global; re-register as soon as the toggle changes rather
    // than waiting for a restart.
    if (patch.shortcuts?.globalMediaKeys !== undefined) {
      applyGlobalShortcuts(next.shortcuts.globalMediaKeys)
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
    const result = win
      ? await dialog.showOpenDialog(win, {
          title: 'Add music folder',
          properties: ['openDirectory', 'multiSelections', 'createDirectory']
        })
      : await dialog.showOpenDialog({
          title: 'Add music folder',
          properties: ['openDirectory', 'multiSelections']
        })
    if (result.canceled || result.filePaths.length === 0) return getConfig().libraryPaths

    const existing = getConfig().libraryPaths
    const merged = [...new Set([...existing, ...result.filePaths])]
    return setConfig({ libraryPaths: merged }).libraryPaths
  })

  ipcMain.handle(INVOKE.getArtwork, (_e, trackId: string): Promise<string | null> => {
    return getArtwork(trackId)
  })

  // Queue intents. Main owns the state; these return void and the renderer
  // re-renders from the onQueueChanged broadcast rather than a return value.
  ipcMain.handle(INVOKE.enqueue, (_e, trackId: string, requester?: string): void => {
    enqueueTrack(trackId, requester)
  })
  ipcMain.handle(INVOKE.removeFromQueue, (_e, queueItemId: string): void => {
    removeFromQueue(queueItemId)
  })
  ipcMain.handle(INVOKE.reorderQueue, (_e, from: number, to: number): void => {
    reorderQueue(from, to)
  })
  ipcMain.handle(INVOKE.skip, (): void => skip())
  ipcMain.handle(INVOKE.clearQueue, (): void => clearQueue())
  ipcMain.handle(INVOKE.shuffleQueue, (): void => shuffleQueue())
  ipcMain.handle(INVOKE.fillQueue, (_e, count?: number): void => {
    fillQueue(count)
  })

  ipcMain.handle(INVOKE.connectTwitch, (_e, ch: string): void => {
    connectTwitch(ch || getConfig().twitch.channel)
  })
  ipcMain.handle(INVOKE.disconnectTwitch, (): void => disconnectTwitch())
  ipcMain.handle(INVOKE.getOverlayUrl, (): string | null => getOverlayUrl())

  ipcMain.handle(INVOKE.getPaths, (): AppPaths => ({
    config: configPath(),
    configDir: app.getPath('userData'),
    artwork: artworkDir(),
    nowPlaying: nowPlayingPath(),
    logs: app.getPath('logs')
  }))
  ipcMain.handle(INVOKE.revealPath, async (_e, target: string): Promise<void> => {
    // Guard against being handed something outside our own directories.
    const roots = [app.getPath('userData'), ...getConfig().libraryPaths]
    if (!roots.some((r) => target === r || target.startsWith(r))) return
    await shell.openPath(target)
  })

  // Playback reporting from the renderer.
  ipcMain.on(SEND.reportEnded, (): void => onTrackEnded())
  ipcMain.on(SEND.reportProgress, (_e, p: PlaybackProgress): void => {
    setProgress(p)
  })
}
