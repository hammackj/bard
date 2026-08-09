import { useEffect, useRef, useState } from 'react'

const PRESETS = [15, 30, 45, 60, 90]

/**
 * Sleep timer.
 *
 * Pauses rather than quitting: waking to a closed app and a lost queue is
 * worse than waking to a paused one.
 *
 * It lives in the transport rather than in Settings because Settings is a
 * modal -- unmounting it would silently cancel a running timer, and a timer
 * you cannot see the remaining minutes of is not one you would trust to fall
 * asleep to.
 */
export function SleepTimer({ onSleep }: { onSleep: () => void }): React.JSX.Element {
  const [endsAt, setEndsAt] = useState<number | null>(null)
  const [open, setOpen] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const wrap = useRef<HTMLDivElement>(null)

  // The deadline fires from its own timeout rather than from the display tick,
  // so a stalled or throttled render cannot delay the pause.
  const sleep = useRef(onSleep)
  useEffect(() => {
    sleep.current = onSleep
  }, [onSleep])
  useEffect(() => {
    if (endsAt === null) return
    const tick = setInterval(() => setNow(Date.now()), 1000)
    const fire = setTimeout(
      () => {
        setEndsAt(null)
        sleep.current()
      },
      Math.max(0, endsAt - Date.now())
    )
    return () => {
      clearInterval(tick)
      clearTimeout(fire)
    }
  }, [endsAt])

  useEffect(() => {
    if (!open) return
    const away = (e: MouseEvent): void => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', away)
    return () => document.removeEventListener('mousedown', away)
  }, [open])

  const remaining = endsAt ? Math.max(0, Math.ceil((endsAt - now) / 60000)) : 0

  return (
    <div ref={wrap} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        title={endsAt ? `Sleeping in ${remaining} min` : 'Sleep timer'}
        aria-label="Sleep timer"
        className={`flex items-center gap-1 rounded-sm p-1 ${
          endsAt ? 'text-cyan' : 'text-muted hover:text-text'
        }`}
      >
        <MoonIcon />
        {endsAt !== null && <span className="mono text-s1">{remaining}</span>}
      </button>

      {open && (
        <div className="absolute bottom-full left-1/2 z-30 mb-2 -translate-x-1/2 border border-edge bg-panel p-2 shadow-2xl">
          <div className="mb-1.5 text-s1 whitespace-nowrap text-muted">Pause playback in…</div>
          <div className="flex gap-1">
            {PRESETS.map((m) => (
              <button
                key={m}
                onClick={() => {
                  setNow(Date.now())
                  setEndsAt(Date.now() + m * 60_000)
                  setOpen(false)
                }}
                className="mono border border-edge px-2 py-1 text-s1 text-muted hover:border-cyan hover:text-text"
              >
                {m}
              </button>
            ))}
          </div>
          {endsAt !== null && (
            <button
              onClick={() => {
                setEndsAt(null)
                setOpen(false)
              }}
              className="mt-1.5 w-full border border-edge px-2 py-1 text-s1 text-rose hover:border-rose"
            >
              Cancel ({remaining} min left)
            </button>
          )}
        </div>
      )}
    </div>
  )
}

const MoonIcon = (): React.JSX.Element => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.7"
  >
    <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" strokeLinejoin="round" />
  </svg>
)
