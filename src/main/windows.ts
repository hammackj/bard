import { BrowserWindow } from 'electron'

/**
 * The main window, tracked explicitly.
 *
 * Reaching for BrowserWindow.getAllWindows()[0] is wrong as soon as a second
 * window exists: the tray preview is also a BrowserWindow, so "show the app"
 * could land on the preview instead -- appearing as a flicker with no way back
 * to the real window.
 */
let mainWindow: BrowserWindow | null = null

export function setMainWindow(win: BrowserWindow): void {
  mainWindow = win
  win.on('closed', () => {
    if (mainWindow === win) mainWindow = null
  })
}

export function getMainWindow(): BrowserWindow | null {
  return mainWindow && !mainWindow.isDestroyed() ? mainWindow : null
}

/** Restores, shows and focuses the main window, in that order. */
export function showMainWindow(): void {
  const win = getMainWindow()
  if (!win) return
  if (win.isMinimized()) win.restore()
  if (!win.isVisible()) win.show()
  win.setSkipTaskbar(false)
  win.focus()
  win.moveTop()
}

/** Broadcasts to the main window only; the preview has no IPC bridge. */
export function sendToMain(channel: string, payload: unknown): void {
  const win = getMainWindow()
  if (win) win.webContents.send(channel, payload)
}
