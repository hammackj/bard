import { Tray, Menu, nativeImage, app } from 'electron'
import { join } from 'node:path'
import { EVENT } from '../shared/ipc'
import type { QueueItem } from '../shared/types'
import { getQueue, skip } from './queue'
import { toggleTrayPopup, destroyTrayPopup } from './trayPopup'
import { showMainWindow, sendToMain } from './windows'

let tray: Tray | null = null
let current: QueueItem | null = null
let playing = false
/** What the menu and tooltip were last built from; see updateTray. */
let lastKey = ''

function iconPath(): string {
  return app.isPackaged
    ? join(process.resourcesPath, 'icon.png')
    : join(__dirname, '../../resources/icon.png')
}

function send(action: 'toggle' | 'restart'): void {
  sendToMain(EVENT.transport, action)
}

function showWindow(): void {
  showMainWindow()
}

export function createTray(onQuit: () => void): void {
  if (tray) return

  // A full-size app icon in a tray slot renders as a blurry smear on most
  // desktops; 22px is the common Linux panel height.
  const image = nativeImage.createFromPath(iconPath()).resize({ width: 22, height: 22 })
  tray = new Tray(image)
  tray.setToolTip('Bard')
  // Left click restores the window -- that is what a tray icon is expected to
  // do, and routing it to the preview instead left no way back to the app.
  tray.on('click', showWindow)
  tray.on('double-click', showWindow)
  tray.on('middle-click', () => toggleTrayPopup(tray?.getBounds()))

  rebuildMenu(onQuit)
}

export function updateTray(item: QueueItem | null, isPlaying: boolean, onQuit: () => void): void {
  current = item
  playing = isPlaying
  if (!tray) return

  // This is driven off the now-playing snapshot, which arrives about four times
  // a second because it carries the playback position. None of that reaches the
  // tray -- rebuilding a native menu at that rate is pure waste, and on some
  // desktops it makes the menu flicker while it is open. Rebuild only when
  // something the menu or the tooltip actually shows has changed.
  const key = `${item?.track.id ?? ''}|${isPlaying}|${getQueue().length > 0}`
  if (key === lastKey) return
  lastKey = key

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
      { label: 'Show Bard', click: showWindow },
      { label: 'Now playing preview', click: () => toggleTrayPopup(tray?.getBounds()) },
      { type: 'separator' },
      { label: playing ? 'Pause' : 'Play', click: () => send('toggle') },
      { label: 'Skip', click: () => skip(), enabled: getQueue().length > 0 },
      { type: 'separator' },
      { label: 'Quit', click: onQuit }
    ])
  )
}

export function destroyTray(): void {
  destroyTrayPopup()
  tray?.destroy()
  tray = null
  lastKey = ''
}
