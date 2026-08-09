import { useStore } from '../state/store'

/**
 * Love toggle.
 *
 * Loved tracks are dealt extra copies in each shuffle pass, so they surface
 * more often without the rest of the library ever being starved -- every track
 * still appears in every pass.
 */
export function LoveButton({
  trackId,
  size = 18
}: {
  trackId: string | null
  size?: number
}): React.JSX.Element | null {
  const loved = useStore((s) => s.loved)
  const setLoved = useStore((s) => s.setLoved)
  if (!trackId) return null
  const on = loved.includes(trackId)

  return (
    <button
      onClick={async (e) => {
        e.stopPropagation()
        setLoved(await window.bard.toggleLoved(trackId))
      }}
      aria-pressed={on}
      aria-label={on ? 'Remove from loved' : 'Love this track'}
      title={on ? 'Loved — played more often' : 'Love this track so it plays more often'}
      className={on ? 'text-rose' : 'text-muted hover:text-rose'}
    >
      <svg
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill={on ? 'currentColor' : 'none'}
        stroke="currentColor"
        strokeWidth="1.7"
      >
        <path d="M12 20.2 4.8 13a4.4 4.4 0 0 1 6.2-6.2l1 1 1-1A4.4 4.4 0 0 1 19.2 13z" />
      </svg>
    </button>
  )
}
