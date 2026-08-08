/**
 * Shown when no library folder is configured.
 *
 * An invitation, not an error -- nothing has gone wrong, the app just has
 * nothing to play yet. One sentence of explanation and one button.
 */
export function FirstRun({ onChoose }: { onChoose: () => void }): React.JSX.Element {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-6 px-10 text-center">
      <div className="display text-s5 leading-tight font-semibold">Nothing to play yet</div>
      <p className="max-w-md text-s2 text-muted">
        Bard only ever plays files you add from your own machine. Nothing is downloaded, and chat
        can only request tracks already in your library.
      </p>
      <button onClick={onChoose} className="bg-cyan-deep px-4 py-2 text-s2">
        Choose a music folder
      </button>
    </div>
  )
}
