import { useEffect, useMemo, useState } from 'react'
import type { Track } from '@shared/types'
import { useStore } from '../state/store'
import { fmtTime } from '../lib/format'

export type Album = {
  name: string
  artist: string
  tracks: Track[]
  /** Total runtime, seconds. */
  duration: number
  /** First track with artwork -- the cover is the same for the whole album. */
  artTrackId: string | null
}

/** Groups the flat track list into albums, in library (path) order. */
export function groupAlbums(tracks: Track[]): Album[] {
  const byName = new Map<string, Album>()
  for (const t of tracks) {
    let album = byName.get(t.album)
    if (!album) {
      album = { name: t.album, artist: t.artist, tracks: [], duration: 0, artTrackId: null }
      byName.set(t.album, album)
    }
    album.tracks.push(t)
    album.duration += t.durationSec
    if (!album.artTrackId && t.hasArtwork) album.artTrackId = t.id
  }
  return [...byName.values()]
}

/**
 * Album browser.
 *
 * A flat list of several hundred tracks is not navigable -- you can only find
 * something you can already name. Twenty covers is.
 */
export function AlbumGrid({
  onPlayAlbum,
  onQueueAlbum,
  onOpenAlbum
}: {
  onPlayAlbum: (a: Album) => void
  onQueueAlbum: (a: Album) => void
  onOpenAlbum: (a: Album) => void
}): React.JSX.Element {
  const tracks = useStore((s) => s.tracks)
  const filter = useStore((s) => s.filter)

  const albums = useMemo(() => {
    const all = groupAlbums(tracks)
    const q = filter.trim().toLowerCase()
    if (!q) return all
    return all.filter(
      (a) =>
        a.name.toLowerCase().includes(q) ||
        a.artist.toLowerCase().includes(q) ||
        a.tracks.some((t) => t.title.toLowerCase().includes(q))
    )
  }, [tracks, filter])

  return (
    <div className="grid grid-cols-2 gap-3 px-3 pb-3">
      {albums.map((album) => (
        <AlbumCard
          key={album.name}
          album={album}
          onPlay={() => onPlayAlbum(album)}
          onQueue={() => onQueueAlbum(album)}
          onOpen={() => onOpenAlbum(album)}
        />
      ))}
      {albums.length === 0 && (
        <p className="col-span-2 px-1 py-3 text-s2 text-muted">No albums match that.</p>
      )}
    </div>
  )
}

function AlbumCard({
  album,
  onPlay,
  onQueue,
  onOpen
}: {
  album: Album
  onPlay: () => void
  onQueue: () => void
  onOpen: () => void
}): React.JSX.Element {
  const art = useAlbumArt(album.artTrackId)

  return (
    <div className="group">
      <button
        onClick={onOpen}
        onDoubleClick={onPlay}
        className="relative block w-full"
        title={`${album.name} — ${album.tracks.length} tracks`}
      >
        {art ? (
          <img src={art} alt="" className="aspect-square w-full border border-edge object-cover" />
        ) : (
          <span className="block aspect-square w-full border border-edge bg-panel" />
        )}
        <span className="absolute inset-0 flex items-end justify-end gap-1 p-1.5 opacity-0 transition-opacity group-hover:opacity-100">
          <span
            role="button"
            tabIndex={0}
            onClick={(e) => {
              e.stopPropagation()
              onQueue()
            }}
            className="bg-ink/85 px-1.5 py-0.5 text-s1 text-text hover:text-cyan"
          >
            + queue
          </span>
          <span
            role="button"
            tabIndex={0}
            onClick={(e) => {
              e.stopPropagation()
              onPlay()
            }}
            className="bg-cyan-deep px-1.5 py-0.5 text-s1 text-text"
          >
            play
          </span>
        </span>
      </button>
      <div className="mt-1.5 truncate text-s2">{album.name}</div>
      <div className="mono truncate text-s1 text-muted">
        {album.tracks.length} · {fmtTime(album.duration)}
      </div>
    </div>
  )
}

/** Album covers are fetched once per album, not once per track. */
function useAlbumArt(trackId: string | null): string | null {
  const [art, setArt] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    setArt(null)
    if (!trackId) return
    window.bard.getArtwork(trackId).then((a) => {
      if (!cancelled) setArt(a)
    })
    return () => {
      cancelled = true
    }
  }, [trackId])
  return art
}
