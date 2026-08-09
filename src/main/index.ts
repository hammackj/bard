// MUST be first: pins userData before any module reads a path from it.
import './userData'
import { app, shell, BrowserWindow, session } from 'electron'
import { join } from 'path'
import { writeFile } from 'node:fs/promises'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { getLibraryPaths, configPath, getConfig } from './config'
import { registerScheme, handleScheme, makeIsAllowed } from './protocol'
import { registerIpcHandlers } from './ipc-handlers'
import { startOverlay, stopOverlay, getOverlayUrl } from './overlay'
import { nowPlayingPath, onNowPlayingChange } from './nowplaying'
import { createTray, updateTray, destroyTray } from './tray'
import { applyGlobalShortcuts, releaseGlobalShortcuts } from './shortcuts'
import { getNowPlaying } from './queue'
import { setMainWindow } from './windows'
import { loadStats, flushStats } from './stats'
import { loadAlbumFlags, flushAlbumFlags } from './albums'
import { loadLoved, flushLoved } from './loved'
import { toggleTrayPopup, warmTrayPopup } from './trayPopup'

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

  mainWindow.webContents.setWindowOpenHandler((details) => {
    shell.openExternal(details.url)
    return { action: 'deny' }
  })

  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

app.whenReady().then(() => {
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

  console.log('[bard] config:', configPath())

  void startOverlay(getConfig().overlay.port).then((port) => {
    console.log(
      port
        ? `[bard] overlay: ${getOverlayUrl()}  (nowplaying: ${nowPlayingPath()})`
        : '[bard] overlay: FAILED to bind a port'
    )
  })

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
})

app.on('before-quit', () => {
  void flushStats()
  void flushAlbumFlags()
  void flushLoved()
  quitting = true
  releaseGlobalShortcuts()
  destroyTray()
  void stopOverlay()
})

app.on('window-all-closed', () => {
  // With close-to-tray on, the window closing is not a request to exit.
  if (process.platform !== 'darwin' && !getConfig().window.closeToTray) {
    app.quit()
  }
})
