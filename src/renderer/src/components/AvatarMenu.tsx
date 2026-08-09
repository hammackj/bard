import { useEffect, useRef, useState } from 'react'
import type { AppInfo } from '@shared/ipc'
import { useStore } from '../state/store'

type Props = {
  onOpenSettings: () => void
  onOpenShortcuts: () => void
  onRescan: () => void
}

/**
 * Account-style menu pinned to the bottom of the library dock.
 *
 * Everything that is not moment-to-moment playback lives behind it, keeping
 * the window itself to the two things that matter while listening: what is
 * playing and what is next.
 */
export function AvatarMenu({
  onOpenSettings,
  onOpenShortcuts,
  onRescan
}: Props): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const [info, setInfo] = useState<AppInfo | null>(null)
  const trackCount = useStore((s) => s.tracks.length)
  const wrap = useRef<HTMLDivElement | null>(null)

  // Fetched once on mount rather than per open: it cannot change while the app
  // is running, and "which build am I on?" should be answerable immediately.
  useEffect(() => {
    void window.bard.getAppInfo().then(setInfo)
  }, [])

  // Close on an outside click or Escape, like any menu.
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent): void => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  return (
    <div ref={wrap} className="relative border-t border-edge">
      {open && (
        <div className="absolute bottom-full left-2 z-30 mb-2 w-60 border border-edge bg-panel py-1 shadow-2xl">
          <MenuItem
            label="Settings"
            hint="Ctrl ,"
            onClick={() => {
              setOpen(false)
              onOpenSettings()
            }}
          />
          <MenuItem
            label="Rescan library"
            hint={`${trackCount.toLocaleString()} tracks`}
            onClick={() => {
              setOpen(false)
              onRescan()
            }}
          />
          <MenuItem
            label="Keyboard shortcuts"
            hint="?"
            onClick={() => {
              setOpen(false)
              onOpenShortcuts()
            }}
          />

          {info && (
            <div
              className="mt-1 flex items-center justify-between gap-3 border-t border-edge px-3 pt-1.5 pb-0.5"
              title={`commit ${info.commit} · Electron ${info.electron} · Chrome ${info.chrome} · Node ${info.node}`}
            >
              <span className="text-s1 text-muted">Version</span>
              <span className="mono text-s1 text-muted">
                v{info.version}
                <span className="ml-1.5 opacity-70">{info.commit}</span>
              </span>
            </div>
          )}
        </div>
      )}

      <button
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        className={`flex w-full items-center gap-2.5 px-3 py-2.5 text-left hover:bg-panel ${
          open ? 'bg-panel' : ''
        }`}
      >
        <span className="flex-none">
          <Avatar />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-s2 text-text">Bard</span>
          <span className="block truncate text-s1 text-muted">
            {trackCount.toLocaleString()} tracks
          </span>
        </span>
        <span className="flex-none text-s1 text-muted">{open ? '▾' : '▴'}</span>
      </button>
    </div>
  )
}

/** The app mark, reused at avatar size. */
function Avatar(): React.JSX.Element {
  return (
    <svg viewBox="0 0 512 512" className="block size-7 rounded-full border border-edge">
      <rect width="512" height="512" fill="var(--color-panel)" />
      <circle
        cx="256"
        cy="256"
        r="150"
        fill="none"
        stroke="var(--color-cyan-deep)"
        strokeWidth="34"
      />
      <g stroke="var(--color-cyan)" strokeWidth="34" strokeLinecap="round">
        <path d="M256 34v58M256 420v58M34 256h58M420 256h58" />
      </g>
      <circle cx="256" cy="256" r="62" fill="var(--color-cyan)" />
    </svg>
  )
}

function MenuItem({
  label,
  hint,
  onClick
}: {
  label: string
  hint?: string
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      role="menuitem"
      onClick={onClick}
      className="flex w-full items-center justify-between gap-3 px-3 py-1.5 text-left text-s2 hover:bg-edge"
    >
      <span>{label}</span>
      {hint && <span className="mono text-s1 text-muted">{hint}</span>}
    </button>
  )
}
