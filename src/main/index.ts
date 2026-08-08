import { app, shell, BrowserWindow, session } from 'electron'
import { join } from 'path'
import { writeFile } from 'node:fs/promises'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { getLibraryPaths, configPath, getConfig } from './config'
import { registerScheme, handleScheme, makeIsAllowed } from './protocol'
import { registerIpcHandlers } from './ipc-handlers'
import { startOverlay, stopOverlay, getOverlayUrl } from './overlay'
import { nowPlayingPath } from './nowplaying'

// Must run before app.whenReady().
registerScheme()

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
        mainWindow.webContents
          .capturePage()
          .then((img) => writeFile(target, img.toPNG()))
          .then(() => console.log(`[bard] captured window -> ${target}`))
          .catch((e) => console.error('[bard] capture failed', e))
      }, delay)
    })
  }

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

  app.on('activate', function () {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('before-quit', () => {
  void stopOverlay()
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})
