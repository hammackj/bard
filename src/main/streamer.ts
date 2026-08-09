import { EVENT, type StreamerSource } from '../shared/ipc'
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
/**
 * What armed the mode, for the banner to name.
 *
 * Runtime state rather than config: it describes what OBS is doing now, and
 * persisting it would have the banner claim a recording that stopped hours ago.
 */
let source: StreamerSource = null

export function getStreamerSource(): StreamerSource {
  return source
}

/** Clears the source when the mode is turned off, or armed by hand. */
export function setStreamerSource(next: StreamerSource): void {
  if (next === source) return
  source = next
  sendToMain(EVENT.streamerSource, source)
}

function armFromObs(obsSource: StreamerSource): void {
  // Re-broadcast even when already armed: switching from recording to
  // streaming should update what the banner says.
  setStreamerSource(obsSource)
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
