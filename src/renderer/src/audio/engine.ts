import type { Track } from '@shared/types'

/**
 * The one and only audio graph.
 *
 *   HTMLAudioElement -> MediaElementAudioSourceNode -> Gain -> Analyser -> out
 *
 * Both the element and the source node are created exactly once at module load.
 * Chromium caps live AudioContexts, and a MediaElementAudioSourceNode can only
 * ever be built once per element -- so playing a new track swaps `src`, it
 * never rebuilds the graph.
 */

export type EngineEvents = {
  onProgress?: (position: number, duration: number, playing: boolean) => void
  onEnded?: () => void
  onError?: (message: string) => void
}

let el: HTMLAudioElement | null = null
let ctx: AudioContext | null = null
let source: MediaElementAudioSourceNode | null = null
let gain: GainNode | null = null
let analyser: AnalyserNode | null = null
let events: EngineEvents = {}
let currentTrack: Track | null = null

/** Chromium exposes setSinkId on AudioContext; TS lib does not declare it yet. */
type SinkCapableContext = AudioContext & { setSinkId?: (id: string) => Promise<void> }

function element(): HTMLAudioElement {
  if (!el) {
    el = new Audio()
    el.preload = 'auto'
    // Required before src is ever set. Without it the bard:// source counts as
    // opaque cross-origin, and MediaElementAudioSourceNode emits zeroes rather
    // than the decoded audio -- silent playback with no error anywhere. The
    // bard:// handler returns the matching Access-Control-Allow-Origin.
    el.crossOrigin = 'anonymous'
    // Volume is handled by the GainNode; leave the element wide open so the
    // gain ramp has full headroom to work with.
    el.volume = 1
    el.addEventListener('timeupdate', () => {
      events.onProgress?.(el!.currentTime, el!.duration || 0, !el!.paused)
    })
    el.addEventListener('ended', () => events.onEnded?.())
    el.addEventListener('play', () =>
      events.onProgress?.(el!.currentTime, el!.duration || 0, true)
    )
    el.addEventListener('pause', () =>
      events.onProgress?.(el!.currentTime, el!.duration || 0, false)
    )
    el.addEventListener('error', () => {
      const code = el?.error?.code
      events.onError?.(`audio element error${code ? ` (code ${code})` : ''}`)
    })
  }
  return el
}

function graph(): { ctx: AudioContext; gain: GainNode; analyser: AnalyserNode } {
  if (!ctx) {
    ctx = new AudioContext()
    source = ctx.createMediaElementSource(element())
    gain = ctx.createGain()
    analyser = ctx.createAnalyser()
    analyser.fftSize = 2048
    source.connect(gain).connect(analyser).connect(ctx.destination)
  }
  return { ctx, gain: gain!, analyser: analyser! }
}

export function setEvents(next: EngineEvents): void {
  events = next
}

/** An AudioContext starts suspended until a gesture; call this from one. */
export async function resume(): Promise<void> {
  const { ctx } = graph()
  if (ctx.state === 'suspended') await ctx.resume()
}

export function getAnalyser(): AnalyserNode {
  return graph().analyser
}

export function getCurrentTrack(): Track | null {
  return currentTrack
}

/**
 * file:// is unreachable from the renderer, so local audio is fetched over the
 * bard:// scheme registered in main.
 *
 * Each segment is encoded individually rather than running encodeURI over the
 * whole path: encodeURI leaves '#' and '?' intact, and a track called
 * "What's Next? .flac" would have its path truncated at the query separator.
 */
export function trackUrl(path: string): string {
  const encoded = path.split('/').map(encodeURIComponent).join('/')
  return `bard://local${encoded.startsWith('/') ? '' : '/'}${encoded}`
}

export async function play(track: Track): Promise<void> {
  const a = element()
  graph()
  currentTrack = track
  a.src = trackUrl(track.path)
  await resume()
  await a.play()
}

export async function togglePlay(): Promise<boolean> {
  const a = element()
  await resume()
  if (a.paused) {
    await a.play()
    return true
  }
  a.pause()
  return false
}

export function pause(): void {
  element().pause()
}

export function seek(seconds: number): void {
  const a = element()
  if (Number.isFinite(a.duration)) {
    a.currentTime = Math.min(Math.max(0, seconds), a.duration)
  }
}

/** 0..1. Ramped rather than set, so volume changes don't click. */
export function setVolume(v: number): void {
  const { ctx, gain } = graph()
  const clamped = Math.min(Math.max(0, v), 1)
  gain.gain.setTargetAtTime(clamped, ctx.currentTime, 0.015)
}

export function getPosition(): { position: number; duration: number; playing: boolean } {
  const a = element()
  return { position: a.currentTime, duration: a.duration || 0, playing: !a.paused }
}

/**
 * Routes output to a specific device.
 *
 * NOTE: this sets the sink on the AudioContext, not on the media element.
 * Once an element is wired into a MediaElementAudioSourceNode its audio is
 * emitted by the context, so HTMLMediaElement.setSinkId no longer has any
 * effect on what you hear. Chromium's AudioContext.setSinkId is the one that
 * works for a Web Audio graph; the element call is kept only as a fallback for
 * runtimes without it.
 */
export async function setOutputDevice(deviceId: string): Promise<void> {
  const { ctx } = graph()
  const sink = deviceId === 'default' ? '' : deviceId
  const c = ctx as SinkCapableContext
  if (typeof c.setSinkId === 'function') {
    try {
      await c.setSinkId(sink)
    } catch (err) {
      // The saved device is gone -- an interface unplugged between runs, say.
      // Falling back to the system default keeps audio playing; throwing here
      // used to leave the app silent with only an unhandled rejection to show
      // for it.
      if (sink === '') throw err
      await c.setSinkId('')
      throw new Error(`saved output device unavailable, using system default`)
    }
    return
  }
  const a = element() as HTMLAudioElement & { setSinkId?: (id: string) => Promise<void> }
  if (typeof a.setSinkId === 'function') await a.setSinkId(deviceId)
}

/**
 * enumerateDevices() returns blank labels until media permission is granted,
 * which would leave the device picker full of unnamed rows. One getUserMedia
 * call unlocks them; the stream is stopped immediately and nothing is recorded.
 */
let labelsUnlocked = false

export async function unlockDeviceLabels(): Promise<void> {
  // Opening a capture stream makes the audio pipeline renegotiate, which is
  // audible as a short blip in playback. Doing it once per session -- and only
  // when the device list is actually wanted -- keeps that from happening every
  // time the settings panel opens.
  if (labelsUnlocked) return
  if (navigator.mediaDevices.enumerateDevices) {
    const existing = await navigator.mediaDevices.enumerateDevices()
    // Labels are already populated (permission persisted from a prior run).
    if (existing.some((d) => d.kind === 'audiooutput' && d.label)) {
      labelsUnlocked = true
      return
    }
  }
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    for (const t of stream.getTracks()) t.stop()
    labelsUnlocked = true
  } catch {
    // Denied -- the picker still works, the labels just stay generic.
  }
}

export async function listOutputDevices(): Promise<MediaDeviceInfo[]> {
  const devices = await navigator.mediaDevices.enumerateDevices()
  return devices.filter((d) => d.kind === 'audiooutput')
}
