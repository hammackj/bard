import { Tray, Menu, BrowserWindow, nativeImage, app } from 'electron'
import { join } from 'node:path'
import { EVENT } from '../shared/ipc'
import type { QueueItem } from '../shared/types'
import { getQueue, skip } from './queue'

let tray: Tray | null = null
let current: QueueItem | null = null
let playing = false

function iconPath(): string {
  return app.isPackaged
    ? join(process.resourcesPath, 'icon.png')
    : join(__dirname, '../../resources/icon.png')
}

function send(action: 'toggle' | 'restart'): void {
  for (const win of BrowserWindow.getAllWindows()) {
    if (!win.isDestroyed()) win.webContents.send(EVENT.transport, action)
  }
}

function showWindow(): void {
  const win = BrowserWindow.getAllWindows()[0]
  if (!win) return
  if (!win.isVisible()) win.show()
  if (win.isMinimized()) win.restore()
  win.focus()
}

export function createTray(onQuit: () => void): void {
  if (tray) return

  // A full-size app icon in a tray slot renders as a blurry smear on most
  // desktops; 22px is the common Linux panel height.
  const image = nativeImage.createFromPath(iconPath()).resize({ width: 22, height: 22 })
  tray = new Tray(image)
  tray.setToolTip('Bard')
  tray.on('click', showWindow)

  rebuildMenu(onQuit)
}

export function updateTray(item: QueueItem | null, isPlaying: boolean, onQuit: () => void): void {
  current = item
  playing = isPlaying
  if (!tray) return
  tray.setToolTip(
    current ? `Bard — ${current.track.title} · ${current.track.artist}` : 'Bard — nothing playing'
  )
  rebuildMenu(onQuit)
}

function rebuildMenu(onQuit: () => void): void {
  if (!tray) return
  tray.setContextMenu(
    Menu.buildFromTemplate([
      {
        label: current ? `${current.track.title} — ${current.track.artist}` : 'Nothing playing',
        enabled: false
      },
      { type: 'separator' },
      { label: playing ? 'Pause' : 'Play', click: () => send('toggle') },
      { label: 'Skip', click: () => skip(), enabled: getQueue().length > 0 },
      { type: 'separator' },
      { label: 'Show Bard', click: showWindow },
      { label: 'Quit', click: onQuit }
    ])
  )
}

export function destroyTray(): void {
  tray?.destroy()
  tray = null
}
