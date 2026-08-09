import { useEffect, useRef } from 'react'
import { getAnalyser } from '../audio/engine'
import { ease, predict, type Sample } from '@shared/progress'

const W = 1000
const H = 44
const MID = H / 2
const SEGMENTS = 90
/** Displacement in user units at full deflection. */
const AMPLITUDE = 8.5

type Props = {
  position: number
  duration: number
  playing: boolean
  onSeek: (seconds: number) => void
}

/**
 * The progress bar is a plucked string.
 *
 * At rest it is a straight cyan hairline; during playback its control points
 * are displaced by the live analyser amplitude, so it vibrates in time with
 * the music. On pause the displacement damps to flat over ~400ms rather than
 * snapping.
 *
 * Everything is driven imperatively inside one rAF loop -- putting a 60fps
 * path through React state would re-render the whole player on every frame.
 */
export function StringBar({ position, duration, playing, onSeek }: Props): React.JSX.Element {
  const wrap = useRef<HTMLDivElement | null>(null)
  const ahead = useRef<SVGPathElement | null>(null)
  const behind = useRef<SVGPathElement | null>(null)
  const head = useRef<SVGCircleElement | null>(null)
  const clip = useRef<SVGRectElement | null>(null)

  // Read progress from a ref so the animation loop never needs re-creating.
  //
  // Stamped in an effect rather than during render: `at` has to record when
  // the report actually arrived, and a ref written during render is both a
  // lint error and wrong under a re-render that carries no new position.
  const progress = useRef<Sample>({ position: 0, duration: 0, playing: false, at: 0 })
  useEffect(() => {
    progress.current = { position, duration, playing, at: performance.now() }
  }, [position, duration, playing])

  useEffect(() => {
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches
    const analyser = getAnalyser()
    const buf = new Uint8Array(analyser.fftSize)

    let raf = 0
    let amp = 0
    let t = 0
    // The drawn position, which trails the reported one smoothly.
    let shown = 0
    let last = performance.now()

    const frame = (): void => {
      raf = requestAnimationFrame(frame)
      const now = performance.now()
      const dt = now - last
      last = now

      const { duration, playing } = progress.current
      shown = ease(shown, predict(progress.current, now), dt)

      let target = 0
      if (playing && !reduce) {
        analyser.getByteTimeDomainData(buf)
        // Peak deviation from the 128 midpoint, normalised to 0..1.
        let peak = 0
        for (let i = 0; i < buf.length; i += 4) {
          const d = Math.abs(buf[i] - 128)
          if (d > peak) peak = d
        }
        target = Math.min(1, peak / 90) * AMPLITUDE
      }
      // Damps toward the target instead of snapping.
      amp += (target - amp) * 0.045
      t += 0.06

      const pts: Array<[number, number]> = []
      for (let i = 0; i <= SEGMENTS; i++) {
        const x = i / SEGMENTS
        // Pinned at both ends, like a real string.
        const env = Math.sin(Math.PI * x)
        const y =
          MID -
          env *
            amp *
            (Math.sin(x * 13 + t * 3.1) * 0.62 +
              Math.sin(x * 29 - t * 4.7) * 0.26 +
              Math.sin(x * 47 + t * 2.3) * 0.12) *
            (1 + 0.35 * Math.sin(t * 1.7))
        pts.push([x * W, y])
      }
      const d = 'M' + pts.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(2)}`).join('L')
      ahead.current?.setAttribute('d', d)
      behind.current?.setAttribute('d', d)

      const p = duration > 0 ? Math.min(1, Math.max(0, shown / duration)) : 0
      clip.current?.setAttribute('width', (p * W).toFixed(1))
      const hp = pts[Math.min(SEGMENTS, Math.round(p * SEGMENTS))]
      head.current?.setAttribute('cx', String(hp[0]))
      head.current?.setAttribute('cy', String(hp[1]))
    }
    frame()
    return () => cancelAnimationFrame(raf)
  }, [])

  const seekFromEvent = (clientX: number): void => {
    const el = wrap.current
    const { duration } = progress.current
    if (!el || duration <= 0) return
    const r = el.getBoundingClientRect()
    const ratio = Math.min(1, Math.max(0, (clientX - r.left) / r.width))
    onSeek(ratio * duration)
  }

  return (
    <div
      ref={wrap}
      className="cursor-pointer py-0.5"
      role="slider"
      tabIndex={0}
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(position)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId)
        seekFromEvent(e.clientX)
      }}
      onPointerMove={(e) => {
        if (e.buttons === 1) seekFromEvent(e.clientX)
      }}
      onKeyDown={(e) => {
        if (e.key === 'ArrowLeft') onSeek(Math.max(0, position - 5))
        if (e.key === 'ArrowRight') onSeek(Math.min(duration, position + 5))
      }}
    >
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        className="block h-11 w-full overflow-visible"
      >
        <defs>
          <clipPath id="played">
            <rect ref={clip} x="0" y="0" width="0" height={H} />
          </clipPath>
        </defs>
        <path
          ref={ahead}
          fill="none"
          stroke="var(--color-edge)"
          strokeWidth="1.25"
          vectorEffect="non-scaling-stroke"
        />
        <path
          ref={behind}
          fill="none"
          stroke="var(--color-cyan)"
          strokeWidth="1.5"
          vectorEffect="non-scaling-stroke"
          clipPath="url(#played)"
          style={{ filter: 'drop-shadow(0 0 4px rgba(34,211,238,.45))' }}
        />
        <circle
          ref={head}
          r="3.5"
          fill="var(--color-cyan)"
          style={{ filter: 'drop-shadow(0 0 5px rgba(34,211,238,.6))' }}
        />
      </svg>
    </div>
  )
}
