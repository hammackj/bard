import { globalShortcut, BrowserWindow } from 'electron'
import { EVENT } from '../shared/ipc'
import { skip } from './queue'

/**
 * System-wide media keys.
 *
 * Registered only when the user opts in. These are global: once taken, the
 * keys stop reaching every other player on the machine, so grabbing them
 * silently at first launch would be hostile.
 */
const BINDINGS: Array<[string, () => void]> = [
  ['MediaPlayPause', () => broadcast('toggle')],
  ['MediaNextTrack', () => skip()],
  // No play history yet, so "previous" restarts the current track rather than
  // doing nothing.
  ['MediaPreviousTrack', () => broadcast('restart')]
]

function broadcast(action: 'toggle' | 'restart'): void {
  for (const win of BrowserWindow.getAllWindows()) {
    if (!win.isDestroyed()) win.webContents.send(EVENT.transport, action)
  }
}

export function applyGlobalShortcuts(enabled: boolean): { registered: string[]; failed: string[] } {
  globalShortcut.unregisterAll()
  const registered: string[] = []
  const failed: string[] = []
  if (!enabled) return { registered, failed }

  for (const [accelerator, handler] of BINDINGS) {
    // Another application may already hold the key; that is not an error worth
    // interrupting the user over, but it should not look like success either.
    const ok = globalShortcut.register(accelerator, handler)
    ;(ok ? registered : failed).push(accelerator)
  }
  return { registered, failed }
}

export function releaseGlobalShortcuts(): void {
  globalShortcut.unregisterAll()
}
