import { useEffect, useRef, useState } from 'react'
import { useStore } from '../state/store'

type Props = {
  onOpenSettings: () => void
  onOpenShortcuts: () => void
  onRescan: () => void
}

/**
 * Account-style menu pinned to the bottom of the library dock.
 *
 * Everything that is not moment-to-moment playback lives behind it, which is
 * what keeps the rest of the window to the three things a streamer actually
 * glances at: what is playing, what is next, and what chat said.
 */
export function AvatarMenu({
  onOpenSettings,
  onOpenShortcuts,
  onRescan
}: Props): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const twitch = useStore((s) => s.twitch)
  const channel = useStore((s) => s.config?.twitch.channel ?? '')
  const trackCount = useStore((s) => s.tracks.length)
  const wrap = useRef<HTMLDivElement | null>(null)

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

  const dot =
    twitch === 'connected' ? 'bg-green' : twitch === 'error' ? 'bg-rose' : 'bg-muted'

  return (
    <div ref={wrap} className="relative border-t border-edge">
      {open && (
        <div className="absolute bottom-full left-2 z-30 mb-2 w-60 border border-edge bg-panel py-1 shadow-2xl">
          <div className="border-b border-edge px-3 pt-2 pb-2.5">
            <div className="truncate text-s2 text-text">{channel || 'No channel set'}</div>
            <div className="mt-0.5 flex items-center gap-1.5 text-s1 text-muted">
              <i className={`size-1.5 rounded-full ${dot}`} />
              Twitch {twitch}
            </div>
          </div>

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
        <span className="relative flex-none">
          <Avatar />
          <i
            className={`absolute -right-0.5 -bottom-0.5 size-2 rounded-full ring-2 ring-ink ${dot}`}
          />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-s2 text-text">Bard</span>
          <span className="block truncate text-s1 text-muted">
            {channel ? `#${channel}` : 'Not connected'}
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
      <path
        d="M96 336 Q256 112 416 336"
        fill="none"
        stroke="var(--color-cyan)"
        strokeWidth="34"
        strokeLinecap="round"
      />
      <circle cx="96" cy="336" r="26" fill="var(--color-cyan-deep)" />
      <circle cx="416" cy="336" r="26" fill="var(--color-cyan-deep)" />
      <circle cx="256" cy="194" r="60" fill="var(--color-panel)" />
      <circle cx="256" cy="194" r="38" fill="var(--color-cyan)" />
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
