// MUST be first: pins userData before any module reads a path from it.
import './userData'
import { app, BrowserWindow, session } from 'electron'
import { join } from 'path'
import { writeFile } from 'node:fs/promises'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { getLibraryPaths, configPath, getConfig } from './config'
import { registerScheme, handleScheme, makeIsAllowed } from './protocol'
import { registerIpcHandlers } from './ipc-handlers'
import { startOverlay, stopOverlay, getOverlayUrl } from './overlay'
import { nowPlayingPath, onNowPlayingChange, writeStoppedNowPlaying } from './nowplaying'
import { createTray, updateTray, destroyTray } from './tray'
import { applyGlobalShortcuts, releaseGlobalShortcuts } from './shortcuts'
import { getNowPlaying } from './queue'
import { setMainWindow, getMainWindow, showMainWindow } from './windows'
import { syncObsWatch } from './streamer'
import { loadStats, flushStats } from './stats'
import { loadAlbumFlags, flushAlbumFlags } from './albums'
import { loadLoved, flushLoved } from './loved'
import { toggleTrayPopup, warmTrayPopup } from './trayPopup'
import { stopObsWatch } from './obs'

// Must run before app.whenReady().
registerScheme()

/** Set once the user really means to exit, so the close handler stops intercepting. */
let quitting = false
function quitApp(): void {
  quitting = true
  app.quit()
}

function createWindow(): void {
  const mainWindow = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 1000,
    minHeight: 640,
    show: false,
    backgroundColor: '#020617',
    autoHideMenuBar: true,
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true
    }
  })

  // Show only once painted, or the dark UI is preceded by a white flash.
  setMainWindow(mainWindow)

  mainWindow.on('ready-to-show', () => {
    mainWindow.show()
  })

  // Renderer console output otherwise only exists inside devtools. Mirroring it
  // to the terminal in dev makes the renderer debuggable from the same place as
  // the main process.
  if (is.dev) {
    mainWindow.webContents.on('console-message', (e) => {
      console.log(`[renderer:${e.level}] ${e.message}`)
    })
  }

  /**
   * Dev-only window capture: BARD_SHOT=/path/to.png writes a PNG of this
   * window and nothing else. Useful for checking layout without taking a
   * screenshot of whatever else is on the developer's screen.
   */
  if (is.dev && process.env.BARD_SHOT) {
    const target = process.env.BARD_SHOT
    const delay = Number(process.env.BARD_SHOT_DELAY ?? 6000)
    mainWindow.webContents.once('did-finish-load', () => {
      setTimeout(() => {
        /**
         * Raise before capturing.
         *
         * capturePage returns the last frame the compositor asked for, and an
         * occluded window is never asked to repaint -- so a capture of a
         * buried window silently returns whatever it looked like when it first
         * appeared, which is a convincing screenshot of the wrong thing.
         */
        mainWindow.show()
        mainWindow.moveTop()
        mainWindow.focus()
        setTimeout(() => {
          mainWindow.webContents
            .capturePage()
            .then((img) => writeFile(target, img.toPNG()))
            .then(() => console.log(`[bard] captured window -> ${target}`))
            .catch((e) => console.error('[bard] capture failed', e))
        }, 900)
      }, delay)
    })
  }

  // Closing hides to the tray by default: a streamer will hit the X by
  // accident mid-stream, and quitting the music is not a recoverable mistake.
  mainWindow.on('close', (e) => {
    if (!quitting && getConfig().window.closeToTray) {
      e.preventDefault()
      mainWindow.hide()
    }
  })

  // Bard's UI has no outbound links, so a window.open from the renderer is a
  // bug or an injection -- not a user following a link. Refuse it rather than
  // handing the URL to the desktop.
  mainWindow.webContents.setWindowOpenHandler(() => {
    return { action: 'deny' }
  })

  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

function startBard(): void {
  electronApp.setAppUserModelId('io.github.hammackj.bard')

  // enumerateDevices() returns blank labels until media permission is granted,
  // which would leave the output-device picker full of empty rows.
  session.defaultSession.setPermissionRequestHandler((_wc, permission, callback) => {
    callback(permission === 'media')
  })

  handleScheme(makeIsAllowed(getLibraryPaths))
  void loadStats()
  void loadAlbumFlags()
  void loadLoved()
  registerIpcHandlers()

  app.on('browser-window-created', (_, window) => {
    optimizer.watchWindowShortcuts(window)
  })

  createWindow()

  console.log(
    `[bard] version ${app.getVersion()} (${__GIT_SHA__}, electron ${process.versions.electron})`
  )
  console.log('[bard] config:', configPath())

  void startOverlay(getConfig().overlay.port).then((port) => {
    console.log(
      port
        ? `[bard] overlay: ${getOverlayUrl()}  (nowplaying: ${nowPlayingPath()})`
        : '[bard] overlay: FAILED to bind a port'
    )
  })

  // Only connects when auto-arm is on; a no-op otherwise.
  syncObsWatch()

  createTray(quitApp)
  // Give the preview a caption before it is ever shown, so a compositor rule
  // matching on title applies to the first show as well as later ones.
  warmTrayPopup()

  // Dev aid: BARD_POPUP_TEST=1 shows the preview shortly after start, so its
  // placement can be checked without clicking the tray.
  if (is.dev && process.env.BARD_POPUP_TEST) {
    setTimeout(() => toggleTrayPopup(), 4000)
  }
  const shortcuts = applyGlobalShortcuts(getConfig().shortcuts.globalMediaKeys)
  if (shortcuts.failed.length) {
    console.warn('[bard] media keys already taken:', shortcuts.failed.join(', '))
  }

  // Keep the tray tooltip and menu in step with playback.
  onNowPlayingChange((snap) => updateTray(getNowPlaying(), snap.isPlaying, quitApp))

  app.on('activate', function () {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
}

/**
 * One Bard at a time.
 *
 * Two instances do not merely duplicate the window: they race for the overlay
 * port and the global media keys, and both flush stats/loved/album flags on
 * quit -- so whichever exits last silently overwrites the other's play counts.
 */
if (!app.requestSingleInstanceLock()) {
  /**
   * quit, not exit.
   *
   * requestSingleInstanceLock has already handed our argv to the primary, and
   * quit lets that delivery finish before we go; exit terminates hard enough to
   * risk cutting it off, which would leave the running window un-raised -- the
   * one thing the second launch is for.
   *
   * Safe specifically because the before-quit handler that flushes stats and
   * loved tracks is registered in the primary branch below. This process has
   * loaded none of that state, so a quit that ran it would write empty data
   * over the running instance's; a quit that cannot reach it will not.
   */
  app.quit()
} else {
  // Fired in this (primary) process when a second launch is turned away.
  app.on('second-instance', () => {
    if (getMainWindow()) {
      // May be hidden rather than destroyed -- closeToTray defaults on.
      showMainWindow()
    } else {
      createWindow()
    }
  })

  void app.whenReady().then(startBard)

  app.on('before-quit', () => {
    void flushStats()
    void flushAlbumFlags()
    void flushLoved()
    // Synchronous, unlike the flushes above: this one has to land before the
    // process goes, or the overlay keeps showing a track that stopped with us.
    writeStoppedNowPlaying()
    quitting = true
    releaseGlobalShortcuts()
    destroyTray()
    stopObsWatch()
    void stopOverlay()
  })

  app.on('window-all-closed', () => {
    // With close-to-tray on, the window closing is not a request to exit.
    if (process.platform !== 'darwin' && !getConfig().window.closeToTray) {
      app.quit()
    }
  })
}
