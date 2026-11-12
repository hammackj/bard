import type { Track } from '@shared/types'

/**
 * What the OS shows on the lock screen, in the media flyout and over MPRIS.
 *
 * Chromium publishes a media session for any page with a playing <audio>
 * element whether or not the page asks it to. With no metadata set, it fills
 * the gaps from the window title and the app name -- which is how the lock
 * screen came to read "Bard / Bard" for every track. Setting the metadata here
 * is the whole fix; the action handlers make the lock-screen buttons drive
 * Bard's own queue instead of only the bare element.
 */

export type MediaSessionActions = {
  play: () => void
  pause: () => void
  next: () => void
  previous: () => void
  seek: (seconds: number) => void
}

function session(): MediaSession | null {
  return typeof navigator !== 'undefined' && 'mediaSession' in navigator
    ? navigator.mediaSession
    : null
}

/** Cover art arrives a beat after the track, so it is applied separately. */
export function setTrackMetadata(track: Track | null, artwork: string | null = null): void {
  const ms = session()
  if (!ms) return
  if (!track) {
    ms.metadata = null
    ms.playbackState = 'none'
    return
  }
  ms.metadata = new MediaMetadata({
    title: track.title,
    artist: track.artist,
    album: track.album,
    artwork: artwork ? [{ src: artwork, sizes: '512x512', type: 'image/jpeg' }] : []
  })
}

export function setPlaybackState(playing: boolean, position: number, duration: number): void {
  const ms = session()
  if (!ms || !ms.metadata) return
  ms.playbackState = playing ? 'playing' : 'paused'
  // setPositionState throws on a position past the duration, which a
  // timeupdate racing a track change can briefly report.
  if (Number.isFinite(duration) && duration > 0) {
    try {
      ms.setPositionState({
        duration,
        position: Math.min(Math.max(0, position), duration),
        playbackRate: 1
      })
    } catch {
      // Out-of-range report mid-swap; the next tick corrects it.
    }
  }
}

export function setActions(actions: MediaSessionActions): () => void {
  const ms = session()
  if (!ms) return () => {}
  const handlers: [MediaSessionAction, MediaSessionActionHandler][] = [
    ['play', () => actions.play()],
    ['pause', () => actions.pause()],
    ['nexttrack', () => actions.next()],
    ['previoustrack', () => actions.previous()],
    [
      'seekto',
      (d) => {
        if (d.seekTime !== undefined) actions.seek(d.seekTime)
      }
    ]
  ]
  for (const [action, handler] of handlers) {
    try {
      ms.setActionHandler(action, handler)
    } catch {
      // Unsupported action on this platform -- the rest still apply.
    }
  }
  return () => {
    for (const [action] of handlers) {
      try {
        ms.setActionHandler(action, null)
      } catch {
        // As above.
      }
    }
  }
}
