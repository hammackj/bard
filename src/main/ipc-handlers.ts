import { BrowserWindow, app, dialog, ipcMain, shell } from 'electron'
import {
  EVENT,
  INVOKE,
  SEND,
  type AppPaths,
  type AppInfo,
  type StreamerSource
} from '../shared/ipc'
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
  isCurrentTrack,
  refreshEndless,
  refreshStreamerMode,
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
import { hasObsPassword, secureStorageAvailable, setObsPassword } from './secrets'
import { getStreamerSource, setStreamerSource, syncObsWatch } from './streamer'
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
    // Arming has to reach the queue and the track on air, not just future
    // picks -- see refreshStreamerMode.
    if (patch.streamer?.enabled !== undefined) {
      // Turning it off by hand clears what armed it; turning it on by hand
      // means there is no OBS output to name.
      setStreamerSource(null)
      refreshStreamerMode()
    }
    // A changed endpoint or auto-arm toggle should take effect now, not on the
    // next restart.
    if (patch.streamer?.autoArm !== undefined || patch.streamer?.obsUrl !== undefined) {
      syncObsWatch()
    }
    return next
  })

  ipcMain.handle(INVOKE.getStreamerSource, (): StreamerSource => getStreamerSource())

  ipcMain.handle(INVOKE.secureStorageAvailable, (): boolean => secureStorageAvailable())

  ipcMain.handle(INVOKE.setObsPassword, (_e, password: string): boolean => {
    const stored = setObsPassword(password)
    // Keep the config flag in step so Settings can offer "replace" rather than
    // showing an empty field that implies no password is set.
    setConfig({ streamer: { hasObsPassword: hasObsPassword() } })
    syncObsWatch()
    return stored
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
    const found = await scanLibrary(getConfig().libraryPaths, onProgress)

    // Stock the queue now the library is known.
    //
    // Endless only ever reacted to something happening -- a track ending, an
    // item removed, the setting being changed. None of those occur at startup,
    // so the app opened with endless on and an empty queue, and the only way
    // to fill it was to toggle the setting off and on again. The scan is the
    // first moment there is anything to draw from.
    //
    // This deliberately fills the queue without starting playback: seeing what
    // is queued and choosing when it goes out is the whole point on a stream.
    // A no-op when endless is off or the queue is already full, so rescans
    // cost nothing.
    refreshEndless()
    return found
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

  ipcMain.handle(INVOKE.getPaths, (): AppPaths => ({
    config: configPath(),
    configDir: app.getPath('userData'),
    artwork: artworkDir(),
    nowPlaying: nowPlayingPath(),
    logs: app.getPath('logs')
  }))

  // app.getVersion() reads the packaged package.json, so this is correct in
  // both dev and a built app without bundling the version in at compile time.
  // The commit has to be stamped in at build time -- see electron.vite.config.
  ipcMain.handle(INVOKE.getAppInfo, (): AppInfo => ({
    version: app.getVersion(),
    commit: __GIT_SHA__,
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  }))

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
    // Drop reports belonging to a track we have already moved on from. During a
    // crossfade the renderer keeps playing the outgoing track for several more
    // seconds; crediting those to the incoming one counted a full play for a
    // track nobody heard, and left `position` stale enough that Previous read
    // as "restart" and replayed the track that had just started.
    if (!isCurrentTrack(p.trackId)) return
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
