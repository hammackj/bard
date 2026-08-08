import { BrowserWindow, screen, type Rectangle } from 'electron'
import { getOverlayUrl } from './overlay'

/**
 * A small now-playing preview shown from the tray.
 *
 * It loads the same page the OBS browser source uses, so the preview and the
 * on-stream card are the same thing and cannot drift apart -- and it is already
 * fed live by SSE, so no extra plumbing is needed to keep it current.
 */

const WIDTH = 470
const HEIGHT = 208
/** Gap from the screen edge / tray so the popup never sits flush against it. */
const MARGIN = 10

let popup: BrowserWindow | null = null

function build(): BrowserWindow | null {
  const url = getOverlayUrl()
  if (!url) return null

  const win = new BrowserWindow({
    width: WIDTH,
    height: HEIGHT,
    show: false,
    frame: false,
    transparent: true,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    focusable: true,
    backgroundColor: '#00000000',
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true }
  })

  win.setMenuBarVisibility(false)
  void win.loadURL(url)

  // Dismiss like a menu: clicking elsewhere closes it.
  win.on('blur', () => win.hide())
  win.on('closed', () => {
    popup = null
  })
  return win
}

/**
 * Places the popup near the tray icon when the platform reports usable tray
 * bounds, and near the cursor otherwise -- most Linux desktops return an empty
 * rectangle for tray bounds, which would otherwise pin it to the top-left.
 */
function position(win: BrowserWindow, trayBounds?: Rectangle): void {
  const cursor = screen.getCursorScreenPoint()
  const display = screen.getDisplayNearestPoint(cursor)
  const area = display.workArea

  const anchorX =
    trayBounds && trayBounds.width > 0 ? trayBounds.x + trayBounds.width / 2 : cursor.x
  const anchorY = trayBounds && trayBounds.height > 0 ? trayBounds.y : cursor.y

  let x = Math.round(anchorX - WIDTH / 2)
  // Below the anchor if it sits in the top half of the screen, above otherwise.
  const inTopHalf = anchorY < area.y + area.height / 2
  let y = inTopHalf ? anchorY + (trayBounds?.height ?? 0) + MARGIN : anchorY - HEIGHT - MARGIN

  x = Math.min(Math.max(area.x + MARGIN, x), area.x + area.width - WIDTH - MARGIN)
  y = Math.min(Math.max(area.y + MARGIN, y), area.y + area.height - HEIGHT - MARGIN)
  win.setPosition(x, y, false)
}

export function toggleTrayPopup(trayBounds?: Rectangle): void {
  if (popup && popup.isVisible()) {
    popup.hide()
    return
  }
  if (!popup) popup = build()
  if (!popup) return
  position(popup, trayBounds)
  popup.showInactive()
  // showInactive keeps focus where it was, but then blur never fires and the
  // popup would stick around; focusing it makes it dismiss like a menu.
  popup.focus()
}

export function hideTrayPopup(): void {
  popup?.hide()
}

export function destroyTrayPopup(): void {
  popup?.destroy()
  popup = null
}
