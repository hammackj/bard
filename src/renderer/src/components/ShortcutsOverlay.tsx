const KEYS: Array<[string, string]> = [
  ['Space', 'Play / pause'],
  ['← / →', 'Seek 5 seconds'],
  ['↑ / ↓', 'Volume'],
  ['/', 'Focus the library filter'],
  ['Enter', 'Queue the selected track'],
  ['Shift + Enter', 'Play the selected track now'],
  ['Ctrl + ,', 'Settings'],
  ['Esc', 'Close / clear filter'],
  ['?', 'This list']
]

export function ShortcutsOverlay({
  open,
  onClose
}: {
  open: boolean
  onClose: () => void
}): React.JSX.Element | null {
  if (!open) return null
  return (
    <div
      className="absolute inset-0 z-30 flex items-center justify-center bg-ink/85 backdrop-blur-sm"
      onClick={onClose}
    >
      <div className="border border-edge bg-panel px-8 py-7" onClick={(e) => e.stopPropagation()}>
        <h2 className="mb-4 text-s1 font-semibold tracking-[.13em] text-cyan uppercase">
          Shortcuts
        </h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-8 gap-y-2">
          {KEYS.map(([key, what]) => (
            <div key={key} className="contents">
              <dt className="mono text-s2 text-text">{key}</dt>
              <dd className="text-s2 text-muted">{what}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  )
}
