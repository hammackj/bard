import { EVENT } from '../shared/ipc'
import { getConfig, setConfig } from './config'
import { restartObsWatch, stopObsWatch } from './obs'
import { refreshStreamerMode } from './queue'
import { sendToMain } from './windows'

/**
 * The policy half of streamer mode: when to arm, and what happens then.
 *
 * Kept apart from obs.ts, which only knows how to talk to a websocket, and
 * from queue.ts, which only knows how to enforce. This is the file that
 * decides.
 */

/**
 * Arms from an OBS live signal.
 *
 * One direction only. OBS reporting "not live" is deliberately not wired to
 * anything: a stream ending, a websocket dropping or OBS quitting must never
 * silently remove protection, and the user may well still be recording
 * something this client cannot see. Disarming stays a deliberate act.
 */
function armFromObs(): void {
  if (getConfig().streamer.enabled) return
  const next = setConfig({ streamer: { enabled: true } })
  // Main changed config without being asked, so the renderer has to be told or
  // its toggle keeps reading "off" while the mode is enforcing.
  sendToMain(EVENT.configChanged, next)
  sendToMain(EVENT.notice, 'OBS went live — streamer mode armed: only albums marked safe will play')
  refreshStreamerMode()
}

function relayStatus(message: string): void {
  sendToMain(EVENT.notice, message)
}

/**
 * Brings the OBS watcher in line with config.
 *
 * Called at startup and whenever the relevant settings change, so a new URL or
 * password takes effect without a restart.
 */
export function syncObsWatch(): void {
  if (getConfig().streamer.autoArm) {
    restartObsWatch({ onLive: armFromObs, onStatus: relayStatus })
  } else {
    stopObsWatch()
  }
}
