import { useCallback, useEffect, useRef, useState } from 'react'
import type { Track } from '@shared/types'
import * as engine from './engine'
import { useStore } from '../state/store'

export type AudioState = {
  track: Track | null
  position: number
  duration: number
  playing: boolean
  volume: number
  error: string | null
}

/**
 * Binds the module-level engine to React and reports playback up to main.
 *
 * Progress rides the element's own timeupdate (~4/sec), which is the rate the
 * overlay and nowplaying.json want. Driving this from requestAnimationFrame
 * would be 60 IPC messages a second for no visible benefit -- the string
 * visualiser reads the analyser directly instead.
 */
export function useAudio(initialVolume = 0.8): {
  state: AudioState
  play: (t: Track) => Promise<void>
  toggle: () => Promise<void>
  seek: (s: number) => void
  setVolume: (v: number) => void
  setOutputDevice: (id: string) => Promise<void>
} {
  const [state, setState] = useState<AudioState>({
    track: null,
    position: 0,
    duration: 0,
    playing: false,
    volume: initialVolume,
    error: null
  })

  useEffect(() => {
    engine.setEvents({
      onProgress: (position, duration, playing, trackId) => {
        setState((s) => ({ ...s, position, duration, playing }))
        window.bard.reportProgress({ position, duration, playing, trackId })
      },
      onEnded: () => {
        setState((s) => ({ ...s, playing: false }))
        window.bard.reportEnded()
      },
      onError: (message) => setState((s) => ({ ...s, error: message }))
    })
    engine.setVolume(initialVolume)
    return () => engine.setEvents({})
    // Mount only. initialVolume is a seed for the first render, before config
    // has resolved -- the saved level arrives later through setVolume, which is
    // the one path that moves the engine and React state together.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Normalisation is a property of playback, not of a track, so it follows the
  // setting rather than being applied at play time.
  const normalize = useStore((s) => s.config?.audio.normalize ?? 'off')
  useEffect(() => {
    engine.setNormalize(normalize)
  }, [normalize])

  const crossfade = useStore((s) => s.config?.audio.crossfadeSec ?? 0)
  const crossfadeRef = useRef(crossfade)
  crossfadeRef.current = crossfade

  // Main drives what plays: it owns the queue and pushes the next track.
  useEffect(() => {
    return window.bard.onPlayTrack((t) => {
      setState((s) => ({ ...s, track: t, error: null }))
      void engine
        .play(t, crossfadeRef.current)
        .catch((e: Error) => setState((s) => ({ ...s, error: e.message })))
    })
  }, [])

  /**
   * Main clears nowPlaying when the queue runs dry with auto-fill off.
   *
   * Nothing in the renderer listened for it, so the event reached the overlay
   * and nowplaying.json -- which correctly went blank -- but not the app. The
   * window went on showing the finished track's title and cover, header
   * reading "Paused", progress stuck at 100%, and the player and the on-stream
   * overlay disagreed about what was playing with no way to clear it short of
   * starting something else.
   *
   * Only the null case is handled here: a non-null nowPlaying is always
   * followed by playTrack, which is what actually loads the audio.
   */
  useEffect(() => {
    return window.bard.onNowPlaying((item) => {
      if (item) return
      engine.pause()
      setState((s) => ({ ...s, track: null, position: 0, duration: 0, playing: false }))
    })
  }, [])

  const play = useCallback(async (t: Track) => {
    setState((s) => ({ ...s, track: t, error: null }))
    try {
      await engine.play(t)
    } catch (e) {
      setState((s) => ({ ...s, error: (e as Error).message }))
    }
  }, [])

  const toggle = useCallback(async () => {
    const playing = await engine.togglePlay()
    setState((s) => ({ ...s, playing }))
  }, [])

  const seek = useCallback((s: number) => engine.seek(s), [])

  const setVolume = useCallback((v: number) => {
    engine.setVolume(v)
    setState((s) => ({ ...s, volume: v }))
  }, [])

  const setOutputDevice = useCallback(async (id: string) => {
    try {
      await engine.setOutputDevice(id)
    } catch (e) {
      setState((s) => ({ ...s, error: `output device: ${(e as Error).message}` }))
    }
  }, [])

  return { state, play, toggle, seek, setVolume, setOutputDevice }
}
