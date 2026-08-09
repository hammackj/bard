import { useCallback, useEffect, useState } from 'react'
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
      onProgress: (position, duration, playing) => {
        setState((s) => ({ ...s, position, duration, playing }))
        window.bard.reportProgress({ position, duration, playing })
      },
      onEnded: () => {
        setState((s) => ({ ...s, playing: false }))
        window.bard.reportEnded()
      },
      onError: (message) => setState((s) => ({ ...s, error: message }))
    })
    engine.setVolume(initialVolume)
    return () => engine.setEvents({})
  }, [initialVolume])

  // Normalisation is a property of playback, not of a track, so it follows the
  // setting rather than being applied at play time.
  const normalize = useStore((s) => s.config?.audio.normalize ?? 'off')
  useEffect(() => {
    engine.setNormalize(normalize)
  }, [normalize])

  // Main drives what plays: it owns the queue and pushes the next track.
  useEffect(() => {
    return window.bard.onPlayTrack((t) => {
      setState((s) => ({ ...s, track: t, error: null }))
      void engine.play(t).catch((e: Error) => setState((s) => ({ ...s, error: e.message })))
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
