import { BrowserWindow, screen, type Rectangle } from 'electron'
import { getOverlayUrl } from './overlay'
import { getConfig } from './config'

/**
 * A small now-playing preview shown from the tray.
 *
 * It loads the same page the OBS browser source uses, so the preview and the
 * on-stream card cannot drift apart, and it is already fed live by SSE.
 */

const WIDTH = 470
const HEIGHT = 208
/** Gap from the screen edge so the popup never sits flush against it. */
const MARGIN = 16

let popup: BrowserWindow | null = null
let hideTimer: NodeJS.Timeout | null = null
let hadFocus = false

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
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    // Some compositors place a utility surface more sensibly than a plain one,
    // and it keeps the preview out of the window switcher.
    type: process.platform === 'linux' ? 'toolbar' : undefined,
    title: 'Bard — now playing',
    backgroundColor: '#00000000',
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true }
  })

  win.setMenuBarVisibility(false)
  win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true })
  void win.loadURL(url)

  /**
   * Dismiss on blur only when the popup genuinely held focus.
   *
   * The previous version called focus() right after showInactive() so that
   * blur would dismiss it like a menu. Under Wayland the focus request is not
   * granted, blur fires immediately, and the popup appears and vanishes in one
   * frame -- which reads as a flicker. It now never asks for focus and relies
   * on the timer instead.
   */
  win.on('blur', () => {
    if (hadFocus) win.hide()
  })
  win.on('focus', () => {
    hadFocus = true
  })
  win.on('closed', () => {
    popup = null
    hadFocus = false
  })
  return win
}

/**
 * Places the popup in the configured corner of the display under the cursor.
 *
 * NOTE: on Wayland the compositor owns window placement and setPosition is a
 * no-op -- verified, a request for (4700, 2660) lands at (0, 0). The call is
 * kept because it does work on X11, Windows and macOS; on Wayland the popup
 * appears wherever the compositor decides, and a KWin/compositor rule is the
 * only way to pin it.
 */
function position(win: BrowserWindow, trayBounds?: Rectangle): void {
  const cursor = screen.getCursorScreenPoint()
  const area = screen.getDisplayNearestPoint(cursor).workArea
  const corner = getConfig().window.trayPopupCorner

  let x: number
  let y: number

  if (corner === 'cursor') {
    const anchorX =
      trayBounds && trayBounds.width > 0 ? trayBounds.x + trayBounds.width / 2 : cursor.x
    const anchorY = trayBounds && trayBounds.height > 0 ? trayBounds.y : cursor.y
    x = Math.round(anchorX - WIDTH / 2)
    y =
      anchorY < area.y + area.height / 2
        ? anchorY + (trayBounds?.height ?? 0) + MARGIN
        : anchorY - HEIGHT - MARGIN
  } else {
    const right = corner.endsWith('right')
    const bottom = corner.startsWith('bottom')
    x = right ? area.x + area.width - WIDTH - MARGIN : area.x + MARGIN
    y = bottom ? area.y + area.height - HEIGHT - MARGIN : area.y + MARGIN
  }

  x = Math.min(Math.max(area.x + MARGIN, x), area.x + area.width - WIDTH - MARGIN)
  y = Math.min(Math.max(area.y + MARGIN, y), area.y + area.height - HEIGHT - MARGIN)
  win.setPosition(Math.round(x), Math.round(y), false)
}

function armAutoHide(): void {
  if (hideTimer) clearTimeout(hideTimer)
  const seconds = Math.max(0, getConfig().window.trayPopupSeconds)
  if (seconds === 0) return // 0 means stay until dismissed
  hideTimer = setTimeout(() => popup?.hide(), seconds * 1000)
}

export function toggleTrayPopup(trayBounds?: Rectangle): void {
  if (popup && popup.isVisible()) {
    popup.hide()
    return
  }
  if (!popup) popup = build()
  if (!popup) return
  hadFocus = false
  position(popup, trayBounds)
  popup.showInactive()
  // Position again after showing: some window managers only honour a move once
  // the surface exists.
  position(popup, trayBounds)
  popup.setAlwaysOnTop(true, 'screen-saver')
  armAutoHide()
}

export function hideTrayPopup(): void {
  if (hideTimer) clearTimeout(hideTimer)
  hideTimer = null
  popup?.hide()
}

export function destroyTrayPopup(): void {
  if (hideTimer) clearTimeout(hideTimer)
  hideTimer = null
  popup?.destroy()
  popup = null
}
