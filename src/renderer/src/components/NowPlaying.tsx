import { useEffect, useState } from 'react'
import type { QueueItem, Track } from '@shared/types'
import { StringBar } from './StringBar'
import { fmtTime } from '../lib/format'

type Props = {
  track: Track | null
  current: QueueItem | null
  position: number
  duration: number
  playing: boolean
  volume: number
  onToggle: () => void
  onSkip: () => void
  onSeek: (s: number) => void
  onVolume: (v: number) => void
}

export function NowPlaying(props: Props): React.JSX.Element {
  const { track, current, position, duration, playing, volume } = props
  const [art, setArt] = useState<string | null>(null)

  // Artwork is fetched by id rather than carried on the Track, so a large
  // library never pushes hundreds of megabytes of JPEG over IPC.
  useEffect(() => {
    let cancelled = false
    setArt(null)
    if (!track?.hasArtwork) return
    window.bard.getArtwork(track.id).then((a) => {
      if (!cancelled) setArt(a)
    })
    return () => {
      cancelled = true
    }
  }, [track?.id, track?.hasArtwork])

  return (
    <div className="flex min-h-0 flex-col justify-center gap-[26px] px-10 pt-[34px] pb-[22px]">
      <div className="flex items-end gap-[26px]">
        {art ? (
          <img
            src={art}
            alt=""
            className="block size-[168px] flex-none border border-edge object-cover"
          />
        ) : (
          <PlaceholderArt />
        )}

        <div className="min-w-0 pb-1">
          <div className="mb-[9px] text-s1 font-semibold tracking-[.15em] text-cyan uppercase">
            {playing ? 'Now playing' : track ? 'Paused' : 'Nothing playing'}
          </div>
          <div className="display truncate text-s5 leading-[1.06] font-semibold tracking-[-.015em]">
            {track?.title ?? '—'}
          </div>
          <div className="mt-[7px] truncate text-s4">{track?.artist ?? ''}</div>
          <div className="mt-1 truncate text-s2 text-muted">{track?.album ?? ''}</div>
          {current?.requestedBy && (
            <div className="mt-3 text-s2 text-muted">
              Requested by <b className="font-medium text-cyan">{current.requestedBy}</b>
            </div>
          )}
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

      <div className="flex items-center gap-[22px]">
        <div className="flex items-center gap-[18px]">
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

        <label className="ml-auto flex items-center gap-[9px] text-muted">
          <VolumeIcon />
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={volume}
            aria-label="Volume"
            onChange={(e) => props.onVolume(Number(e.target.value))}
            className="h-px w-24 appearance-none bg-edge accent-cyan"
          />
          <span className="mono text-s1">{Math.round(volume * 100)}</span>
        </label>
      </div>
    </div>
  )
}

function PlaceholderArt(): React.JSX.Element {
  return (
    <svg viewBox="0 0 168 168" className="block size-[168px] flex-none border border-edge">
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
