import { useEffect, useState } from 'react'
import type { Track } from '@shared/types'
import { StringBar } from './StringBar'
import { fmtTime } from '../lib/format'
import { useStore } from '../state/store'
import { LoveButton } from './LoveButton'
import { SleepTimer } from './SleepTimer'
import type { AutoFillMode, RepeatMode } from '@shared/types'

type Props = {
  track: Track | null
  position: number
  duration: number
  playing: boolean
  volume: number
  onToggle: () => void
  onSkip: () => void
  onSeek: (s: number) => void
  onVolume: (v: number) => void
  /** Called by the sleep timer; expected to pause, not to quit. */
  onSleep: () => void
}

export function NowPlaying(props: Props): React.JSX.Element {
  const { track, position, duration, playing, volume } = props
  // Kept with the id it belongs to, rather than being cleared on every track
  // change. Clearing meant a setArt(null) in the effect body, which is a
  // set-state-in-effect violation and cost a second render per track; tagging
  // it instead lets a stale cover simply not match, which needs no reset.
  const [art, setArt] = useState<{ trackId: string; url: string | null } | null>(null)

  // Artwork is fetched by id rather than carried on the Track, so a large
  // library never pushes hundreds of megabytes of JPEG over IPC.
  useEffect(() => {
    if (!track?.hasArtwork) return
    const trackId = track.id
    let cancelled = false
    window.bard.getArtwork(trackId).then((url) => {
      if (!cancelled) setArt({ trackId, url })
    })
    return () => {
      cancelled = true
    }
  }, [track?.id, track?.hasArtwork])

  const cover = track && art?.trackId === track.id ? art.url : null

  return (
    <div className="flex min-h-0 flex-col justify-center gap-[26px] px-5 pt-[34px] pb-[22px] @2xl:px-10">
      <div className="flex min-w-0 items-end gap-[26px]">
        {cover ? (
          <img
            src={cover}
            alt=""
            className="block aspect-square w-[min(336px,58%,45vh)] shrink-0 border border-edge object-cover"
          />
        ) : (
          <PlaceholderArt />
        )}

        <div className="min-w-0 pb-1">
          <div className="mb-[9px] text-s1 font-semibold tracking-[.15em] text-cyan uppercase">
            {playing ? 'Now playing' : track ? 'Paused' : 'Nothing playing'}
          </div>
          <div className="flex items-center gap-3">
            <div className="display truncate text-s5 leading-[1.06] font-semibold tracking-[-.015em]">
              {track?.title ?? '—'}
            </div>
            <LoveButton trackId={track?.id ?? null} size={22} />
          </div>
          <div className="mt-[7px] truncate text-s4">{track?.artist ?? ''}</div>
          <div className="mt-1 truncate text-s2 text-muted">{track?.album ?? ''}</div>
        </div>
      </div>

      <div>
        <StringBar
          position={position}
          duration={duration}
          playing={playing}
          onSeek={props.onSeek}
        />
        <div className="-mt-0.5 flex justify-between text-s1 text-muted">
          <span className="mono">{fmtTime(position)}</span>
          <span className="mono">{fmtTime(duration)}</span>
        </div>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-x-[22px] gap-y-3">
        <div className="flex items-center gap-[18px]">
          <button
            className="rounded-sm p-1 text-muted hover:text-text"
            aria-label="Previous"
            title="Previous track (restarts if a few seconds in)"
            onClick={() => void window.bard.previous()}
          >
            <PrevIcon />
          </button>
          <button
            className="rounded-sm p-1 text-muted hover:text-text disabled:opacity-40"
            aria-label="Play/Pause"
            onClick={props.onToggle}
            disabled={!track}
          >
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button
            className="rounded-sm p-1 text-muted hover:text-text"
            aria-label="Skip"
            onClick={props.onSkip}
          >
            <SkipIcon />
          </button>
        </div>

        <RepeatButton />
        <PlaybackMode />

        <div className="ml-auto">
          <SleepTimer onSleep={props.onSleep} />
        </div>

        <label className="flex min-w-0 items-center gap-[9px] text-muted">
          <VolumeIcon />
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={volume}
            aria-label="Volume"
            onChange={(e) => props.onVolume(Number(e.target.value))}
            className="h-px w-16 appearance-none bg-edge accent-cyan @2xl:w-24"
          />
          <span className="mono text-s1">{Math.round(volume * 100)}</span>
        </label>
      </div>
    </div>
  )
}

/**
 * Auto-fill mode, sitting in the transport rather than buried in settings --
 * "what plays when the queue empties" is a playback control, and nobody goes
 * hunting through a settings dialog mid-stream to change it.
 */
function PlaybackMode(): React.JSX.Element {
  const mode = useStore((s) => s.config?.playback.autoFill ?? 'off')
  const setConfig = useStore((s) => s.setConfig)

  const choose = async (next: AutoFillMode): Promise<void> => {
    setConfig(await window.bard.setConfig({ playback: { autoFill: next } }))
  }

  const options: Array<[AutoFillMode, string, string]> = [
    ['off', 'Off', 'Stop when the queue runs out'],
    ['shuffle', 'Shuffle', 'Keep playing — random, avoiding recent tracks'],
    ['album', 'Album', 'Keep playing — finish the album, then another']
  ]

  return (
    <div className="ml-6 flex items-center gap-3">
      <StreamerToggle />
      <EndlessToggle />
      <div className="flex items-center gap-1 border border-edge p-0.5" role="group">
        {options.map(([value, label, title]) => (
          <button
            key={value}
            title={title}
            onClick={() => void choose(value)}
            className={`px-2 py-0.5 text-s1 ${
              mode === value ? 'bg-cyan-deep text-text' : 'text-muted hover:text-text'
            }`}
          >
            {label}
          </button>
        ))}
      </div>
    </div>
  )
}

/**
 * Streamer mode.
 *
 * Sits beside the transport rather than in Settings because being armed
 * changes what the player will do, and a mode that silently refuses tracks has
 * to be visible while it is on. It is styled loudly when armed for the same
 * reason -- this is the one control where "I forgot it was on" and "I forgot
 * it was off" are both expensive.
 */
function StreamerToggle(): React.JSX.Element {
  const on = useStore((s) => s.config?.streamer.enabled ?? false)
  const setConfig = useStore((s) => s.setConfig)
  return (
    <button
      onClick={async () => setConfig(await window.bard.setConfig({ streamer: { enabled: !on } }))}
      title={
        on
          ? 'Streamer mode is ON — only albums checked and marked safe will play, however they are chosen. Click to turn off.'
          : 'Streamer mode: play only albums marked safe for Content ID. Blocks untested albums too, including ones you pick deliberately.'
      }
      className={`flex items-center gap-1.5 border px-2 py-0.5 text-s1 ${
        on ? 'border-green bg-green/20 text-green' : 'border-edge text-muted hover:text-text'
      }`}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor">
        <path
          d="M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z"
          strokeWidth="1.7"
          strokeLinejoin="round"
        />
      </svg>
      {on ? 'Live-safe' : 'Streamer'}
    </button>
  )
}

/**
 * Endless play.
 *
 * Distinct from auto-fill: auto-fill decides what to play when the queue is
 * already empty, endless keeps it from ever getting there, and shows you the
 * upcoming tracks so you can still steer.
 *
 * The wording matters more than it looks. Shuffle deals from a deck, and the
 * deck reshuffles itself when it runs out -- so this plays forever. Describing
 * it as "never repeat until the whole library has played" was accurate about
 * the guarantee and misleading about the duration: it read as though playback
 * ended with the library, which is the opposite of what the setting does.
 */
function EndlessToggle(): React.JSX.Element {
  const on = useStore((s) => s.config?.playback.endless ?? false)
  const setConfig = useStore((s) => s.setConfig)
  return (
    <button
      onClick={async () => setConfig(await window.bard.setConfig({ playback: { endless: !on } }))}
      title={
        on
          ? 'Endless play is on — music keeps going indefinitely. Every track plays once before any repeats; when the library is exhausted it reshuffles and carries on.'
          : 'Endless play: keep music going indefinitely. Every track plays once before any repeats, then the library reshuffles and continues — it does not stop at the end of the library.'
      }
      className={`flex items-center gap-1.5 border px-2 py-0.5 text-s1 ${
        on ? 'border-cyan-deep bg-cyan-deep text-text' : 'border-edge text-muted hover:text-text'
      }`}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor">
        <path
          d="M17 7a5 5 0 0 1 0 10c-3 0-4-3-7-3a4 4 0 1 0 0-8c3 0 4 3 7 3z"
          strokeWidth="1.7"
          strokeLinecap="round"
        />
      </svg>
      Endless
    </button>
  )
}

/** Repeat off / one / all, cycled by clicking. */
function RepeatButton(): React.JSX.Element {
  const mode = useStore((s) => s.config?.playback.repeat ?? 'off')
  const setConfig = useStore((s) => s.setConfig)
  const next: Record<RepeatMode, RepeatMode> = { off: 'all', all: 'one', one: 'off' }
  const label: Record<RepeatMode, string> = {
    off: 'Repeat off',
    all: 'Repeat all — replay what has played once the queue empties',
    one: 'Repeat one — keep replaying this track'
  }
  return (
    <button
      title={label[mode]}
      onClick={async () =>
        setConfig(await window.bard.setConfig({ playback: { repeat: next[mode] } }))
      }
      className={`relative ml-4 rounded-sm p-1 ${mode === 'off' ? 'text-muted hover:text-text' : 'text-cyan'}`}
    >
      <svg
        width="17"
        height="17"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
      >
        <path d="M17 2l4 4-4 4" strokeLinecap="round" strokeLinejoin="round" />
        <path d="M3 11V9a4 4 0 0 1 4-4h14" />
        <path d="M7 22l-4-4 4-4" strokeLinecap="round" strokeLinejoin="round" />
        <path d="M21 13v2a4 4 0 0 1-4 4H3" />
      </svg>
      {mode === 'one' && (
        <span className="mono absolute -right-0.5 -bottom-0.5 text-[9px] leading-none">1</span>
      )}
    </button>
  )
}

function PlaceholderArt(): React.JSX.Element {
  return (
    <svg
      viewBox="0 0 168 168"
      className="block aspect-square w-[min(336px,58%,45vh)] shrink-0 border border-edge"
    >
      <rect width="168" height="168" fill="var(--color-panel)" />
      <g fill="none" stroke="var(--color-cyan-deep)" strokeOpacity=".75">
        <circle cx="84" cy="84" r="46" />
        <circle cx="84" cy="84" r="34" />
        <circle cx="84" cy="84" r="22" />
        <circle cx="84" cy="84" r="10" />
      </g>
      <g stroke="var(--color-cyan-deep)" strokeOpacity=".45">
        <path d="M84 26v116M26 84h116M43 43l82 82M125 43l-82 82" />
      </g>
      <circle cx="84" cy="84" r="3" fill="var(--color-cyan)" />
    </svg>
  )
}

const PrevIcon = (): React.JSX.Element => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
    <path d="M7 6h2v12H7zm11 0l-8 6 8 6z" />
  </svg>
)
const PlayIcon = (): React.JSX.Element => (
  <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor">
    <path d="M8 5l11 7-11 7z" />
  </svg>
)
const PauseIcon = (): React.JSX.Element => (
  <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor">
    <path d="M7 5h3.2v14H7zm6.8 0H17v14h-3.2z" />
  </svg>
)
const SkipIcon = (): React.JSX.Element => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
    <path d="M15 6h2v12h-2zM6 6l8 6-8 6z" />
  </svg>
)
const VolumeIcon = (): React.JSX.Element => (
  <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor">
    <path d="M4 9h4l5-4v14l-5-4H4z" />
    <path d="M16 8.5a5 5 0 0 1 0 7" fill="none" stroke="currentColor" strokeWidth="1.6" />
  </svg>
)
