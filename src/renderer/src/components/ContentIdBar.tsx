import { useStore } from '../state/store'
import type { ContentIdStatus } from '@shared/types'

const CHOICES: Array<[ContentIdStatus, string, string]> = [
  ['untested', 'Untested', 'Not checked against Content ID yet'],
  ['safe', 'Safe', 'Uploaded and produced no claim'],
  ['claimed', 'Claimed', 'Produced a claim — kept out of shuffle, fill and endless']
]

/**
 * Records the Content ID result for an album.
 *
 * Sits in the album view rather than settings because this is what you do
 * immediately after checking an upload, with the album already in front of
 * you. Marking one "claimed" both records the finding and takes it out of
 * automatic play, which is the reason for testing in the first place.
 */
export function ContentIdBar({ album }: { album: string }): React.JSX.Element {
  const flag = useStore((s) => s.albumFlags[album])
  const setAlbumFlags = useStore((s) => s.setAlbumFlags)
  const status: ContentIdStatus = flag?.status ?? 'untested'

  const choose = async (next: ContentIdStatus): Promise<void> => {
    setAlbumFlags(await window.bard.setAlbumFlag(album, next, flag?.note))
  }

  return (
    <div className="border-b border-edge px-3 py-2">
      <div className="mb-1.5 text-s1 tracking-[.13em] text-muted uppercase">Content ID</div>
      <div className="flex gap-1">
        {CHOICES.map(([value, label, title]) => (
          <button
            key={value}
            title={title}
            onClick={() => void choose(value)}
            className={`flex-1 border px-2 py-1 text-s1 ${
              status === value
                ? value === 'claimed'
                  ? 'border-rose bg-rose/20 text-rose'
                  : value === 'safe'
                    ? 'border-green bg-green/20 text-green'
                    : 'border-edge bg-panel text-text'
                : 'border-edge text-muted hover:text-text'
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <input
        defaultValue={flag?.note ?? ''}
        placeholder="who claimed it, or a note"
        onBlur={async (e) =>
          setAlbumFlags(await window.bard.setAlbumFlag(album, status, e.target.value))
        }
        className="mt-1.5 w-full border border-edge bg-panel px-2 py-1 text-s1 placeholder:text-muted focus:border-cyan focus:outline-none"
      />

      {flag?.checkedAt && (
        <div className="mono mt-1 text-s1 text-muted">
          checked {new Date(flag.checkedAt).toLocaleDateString()}
        </div>
      )}
    </div>
  )
}
