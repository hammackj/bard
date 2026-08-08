import { ipcMain } from 'electron'
import { INVOKE } from '../shared/ipc'
import { getConfig, setConfig } from './config'
import type { Config, DeepPartial } from '../shared/types'

/**
 * Registers the handlers that exist so far. Later phases add library, queue and
 * twitch channels; anything in the BardApi surface without a handler yet will
 * reject with "No handler registered", which is the behaviour we want while
 * building -- a loud failure rather than a silent undefined.
 */
export function registerIpcHandlers(): void {
  ipcMain.handle(INVOKE.getConfig, (): Config => getConfig())
  ipcMain.handle(INVOKE.setConfig, (_e, patch: DeepPartial<Config>): Config => setConfig(patch))
}
