import type { StreamerSource } from '@shared/ipc'
import { useStore } from '../state/store'

/**
 * Full-width bar shown while streamer mode is armed.
 *
 * A banner rather than a control in the transport row, for the reason Discord
 * uses one: the mode changes what the app will refuse to do, so "is it on?"
 * has to be answerable from across the room without hunting for a lit button.
 * It also names what armed it, because "recording" and "streaming" are
 * different enough that a wrong guess sends you looking in the wrong place.
 *
 * Only rendered when armed. There is deliberately no dormant version -- a
 * permanent bar saying "off" is noise, and the arming control lives in
 * Settings where the rest of the mode's configuration is.
 */
const LABELS: Record<Exclude<StreamerSource, null>, string> = {
  stream: 'Streaming',
  record: 'Recording',
  replay: 'Replay buffer running',
  virtualcam: 'Virtual camera on'
}

export function StreamerBanner(): React.JSX.Element | null {
  const armed = useStore((s) => s.config?.streamer.enabled ?? false)
  const source = useStore((s) => s.streamerSource)
  const setConfig = useStore((s) => s.setConfig)
  if (!armed) return null

  const what = source ? LABELS[source] : 'Streamer mode'

  return (
    <div className="flex items-center gap-3 border-b border-green/40 bg-green/15 px-4 py-1.5">
      <span className="flex items-center gap-2 text-green">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor">
          <path
            d="M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z"
            strokeWidth="1.8"
            strokeLinejoin="round"
          />
        </svg>
        <span className="text-s1 font-semibold tracking-[.13em] uppercase">{what}</span>
      </span>
      <span className="min-w-0 flex-1 truncate text-s1 text-muted">
        Only albums marked safe for Content ID will play — untested and claimed albums are blocked,
        including ones you pick yourself.
      </span>
      <button
        onClick={async () =>
          setConfig(await window.bard.setConfig({ streamer: { enabled: false } }))
        }
        title="Stop restricting playback. Nothing re-arms it until OBS goes live again."
        className="flex-none border border-green/50 px-2 py-0.5 text-s1 text-green hover:bg-green/20"
      >
        Turn off
      </button>
    </div>
  )
}
