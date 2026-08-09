import { useEffect, useMemo, useRef } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { Track } from '@shared/types'
import { AlbumGrid, type Album } from './AlbumGrid'
import { ContentIdBar } from './ContentIdBar'
import { useStore } from '../state/store'
import { fmtTime } from '../lib/format'

const ROW_HEIGHT = 44

type Props = {
  onEnqueue: (t: Track) => void
  onPlayNow: (t: Track) => void
}

/**
 * Virtualised because a library of a few thousand rows will jank if every one
 * is in the DOM. Row height is fixed, so the estimate is exact.
 */
export function LibraryPanel({ onEnqueue, onPlayNow }: Props): React.JSX.Element {
  const tracks = useStore((s) => s.tracks)
  const filter = useStore((s) => s.filter)
  const setFilter = useStore((s) => s.setFilter)
  const selected = useStore((s) => s.selectedIndex)
  const setSelected = useStore((s) => s.setSelectedIndex)
  const view = useStore((s) => s.view)
  const setView = useStore((s) => s.setView)
  const openAlbum = useStore((s) => s.openAlbum)
  const setOpenAlbum = useStore((s) => s.setOpenAlbum)
  const stats = useStore((s) => s.stats)
  const scanning = useStore((s) => s.scanning)
  const progress = useStore((s) => s.scanProgress)

  const input = useRef<HTMLInputElement | null>(null)
  const scroller = useRef<HTMLDivElement | null>(null)

  const filtered = useMemo(() => {
    const base = openAlbum ? tracks.filter((t) => t.album === openAlbum) : tracks
    const q = filter.trim().toLowerCase()
    if (!q) return base
    return base.filter((t) => t.searchText.includes(q))
  }, [tracks, filter, openAlbum])

  const virtual = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10
  })

  // Keep the keyboard selection in view as it moves.
  useEffect(() => {
    if (filtered.length) virtual.scrollToIndex(Math.min(selected, filtered.length - 1))
  }, [selected, filtered.length, virtual])

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setSelected(Math.min(selected + 1, filtered.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setSelected(Math.max(selected - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const t = filtered[selected]
      if (t) (e.shiftKey ? onPlayNow : onEnqueue)(t)
    } else if (e.key === 'Escape') {
      setFilter('')
      input.current?.blur()
    }
  }

  return (
    <section className="flex min-h-0 flex-col" onKeyDown={onKeyDown}>
      <header className="flex items-center justify-between gap-2 px-4 pt-[13px] pb-[11px] text-s1 font-semibold tracking-[.13em] text-muted uppercase">
        {openAlbum ? (
          <button
            onClick={() => setOpenAlbum(null)}
            className="min-w-0 truncate normal-case hover:text-text"
            title="Back to albums"
          >
            ‹ {openAlbum}
          </button>
        ) : (
          <span>Library</span>
        )}
        <span className="flex flex-none gap-2 normal-case">
          <button
            onClick={() => setView('albums')}
            className={view === 'albums' && !openAlbum ? 'text-cyan' : 'hover:text-text'}
          >
            Albums
          </button>
          <button
            onClick={() => setView('tracks')}
            className={view === 'tracks' && !openAlbum ? 'text-cyan' : 'hover:text-text'}
          >
            Tracks
          </button>
        </span>
      </header>

      {openAlbum && <ContentIdBar album={openAlbum} />}

      <div className="px-3 pb-2.5">
        <input
          ref={input}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter tracks…"
          aria-label="Filter tracks"
          data-library-filter
          className="w-full rounded-sm border border-edge bg-panel px-2.5 py-[7px] text-s2 placeholder:text-muted focus:border-cyan focus:outline-none"
        />
      </div>

      {view === 'albums' && !openAlbum ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <AlbumGrid
            onOpenAlbum={(a: Album) => setOpenAlbum(a.name)}
            onQueueAlbum={(a: Album) => a.tracks.forEach((t) => onEnqueue(t))}
            onPlayAlbum={(a: Album) => {
              a.tracks.forEach((t) => onEnqueue(t))
              onPlayNow(a.tracks[0])
            }}
          />
        </div>
      ) : (
      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto px-1.5">
        <div style={{ height: virtual.getTotalSize(), position: 'relative' }}>
          {virtual.getVirtualItems().map((row) => {
            const t = filtered[row.index]
            const isSel = row.index === selected
            return (
              <div
                key={t.id}
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  height: row.size,
                  transform: `translateY(${row.start}px)`
                }}
              >
                <button
                  onClick={() => setSelected(row.index)}
                  onDoubleClick={() => onEnqueue(t)}
                  title={`${t.title} — ${t.artist}`}
                  className={`grid h-full w-full grid-cols-[1fr_auto] items-center gap-2.5 rounded-sm px-2.5 text-left hover:bg-panel ${
                    isSel ? 'bg-panel shadow-[inset_2px_0_0_var(--color-cyan)]' : ''
                  }`}
                >
                  <span className="min-w-0">
                    <span className="block truncate text-s2">{t.title}</span>
                    <span className="mt-px block truncate text-s1 text-muted">{t.artist}</span>
                  </span>
                  <span className="mono flex items-baseline gap-2 text-s1 text-muted">
                    {(stats[t.id]?.plays ?? 0) > 0 && (
                      <span className="text-cyan" title={`Played ${stats[t.id].plays} times`}>
                        {stats[t.id].plays}×
                      </span>
                    )}
                    {fmtTime(t.durationSec)}
                  </span>
                </button>
              </div>
            )
          })}
        </div>
      </div>
      )}

      <footer className="flex justify-between border-t border-edge px-4 py-[9px] text-s1 text-muted">
        <span>
          {scanning && progress
            ? `Scanning ${progress.scanned}/${progress.total}`
            : view === 'albums' && !openAlbum
              ? `${new Set(tracks.map((t) => t.album)).size} albums`
              : `${filtered.length.toLocaleString()} tracks`}
        </span>
        <span>Enter to queue</span>
      </footer>
    </section>
  )
}
