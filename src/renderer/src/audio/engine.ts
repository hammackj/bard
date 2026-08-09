import type { NormalizeMode, Track } from '@shared/types'
import { gainMultiplier } from '@shared/gain'
import { trackUrl } from '@shared/url'

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
  /**
   * `trackId` is whatever is loaded in the element right now. It lags the
   * track main has selected for the length of a crossfade, which is the point:
   * it lets main tell "this is my current track" from "this is the tail of the
   * one before".
   */
  onProgress?: (
    position: number,
    duration: number,
    playing: boolean,
    trackId: string | null
  ) => void
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
/** User volume, 0..1, kept separate from the per-track normalisation gain. */
let userVolume = 1
let normalize: NormalizeMode = 'off'

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
    const report = (playing: boolean): void => {
      events.onProgress?.(el!.currentTime, el!.duration || 0, playing, currentTrack?.id ?? null)
    }
    el.addEventListener('timeupdate', () => report(!el!.paused))
    el.addEventListener('ended', () => events.onEnded?.())
    el.addEventListener('play', () => report(true))
    el.addEventListener('pause', () => report(false))
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
 * bard:// scheme registered in main. See shared/url.ts for the encoding.
 */
export { trackUrl }

export async function play(track: Track, crossfadeSec = 0): Promise<void> {
  const a = element()
  graph()
  if (crossfadeSec > 0 && !a.paused) await fadeOut(crossfadeSec)

  currentTrack = track
  // Set the level before the audio starts, or the first moments of a loud
  // track play at the previous track's gain.
  //
  // When crossfading, that level is silence. This used to call fadeIn(0),
  // which routes to applyGain and snaps the gain straight to full -- undoing
  // the fade-out that had just finished. The ramp is only scheduled after the
  // awaits on resume() and play() below, so the incoming track could begin at
  // full volume in the gap, which is the hard edge the crossfade exists to
  // remove.
  if (crossfadeSec > 0) silenceNow()
  else applyGain(true)
  a.src = trackUrl(track.path)
  await resume()
  await a.play()
  if (crossfadeSec > 0) fadeIn(crossfadeSec)
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

/**
 * Fades the output down and back up around a track change.
 *
 * A true crossfade needs two decoders playing at once, which this graph
 * deliberately does not have -- one element, one source node. This is a fade
 * out, swap, fade in: it removes the hard edge between tracks, which is what
 * the seam actually sounds like, without a second audio path to keep in sync.
 */
export async function fadeOut(seconds: number): Promise<void> {
  if (seconds <= 0) return
  const { ctx, gain } = graph()
  const now = ctx.currentTime
  gain.gain.cancelScheduledValues(now)
  gain.gain.setValueAtTime(gain.gain.value, now)
  gain.gain.linearRampToValueAtTime(0.0001, now + seconds)
  await new Promise((r) => setTimeout(r, seconds * 1000))
}

/**
 * Drops the gain to silence at once, cancelling any ramp still in flight.
 *
 * 0.0001 rather than 0 throughout: linearRampToValueAtTime toward an exact
 * zero is a degenerate ramp, and the value is already inaudible.
 */
function silenceNow(): void {
  const { ctx, gain } = graph()
  gain.gain.cancelScheduledValues(ctx.currentTime)
  gain.gain.setValueAtTime(0.0001, ctx.currentTime)
}

export function fadeIn(seconds: number): void {
  if (seconds <= 0) {
    applyGain(true)
    return
  }
  const { ctx, gain } = graph()
  const now = ctx.currentTime
  const target = userVolume * gainMultiplier(currentTrack, normalize)
  gain.gain.cancelScheduledValues(now)
  gain.gain.setValueAtTime(0.0001, now)
  gain.gain.linearRampToValueAtTime(Math.max(0.0001, target), now + seconds)
}

export function seek(seconds: number): void {
  const a = element()
  if (Number.isFinite(a.duration)) {
    a.currentTime = Math.min(Math.max(0, seconds), a.duration)
  }
}

/**
 * Applies volume and normalisation together.
 *
 * They are multiplied into one gain rather than chained through two nodes: the
 * user's volume is a preference, the ReplayGain figure is a property of the
 * file, and keeping them separate in code but combined in the graph means
 * changing one never disturbs the other.
 */
function applyGain(immediate = false): void {
  const { ctx, gain } = graph()
  const target = userVolume * gainMultiplier(currentTrack, normalize)
  const safe = Math.min(Math.max(0, target), 4)
  if (immediate) gain.gain.setValueAtTime(safe, ctx.currentTime)
  else gain.gain.setTargetAtTime(safe, ctx.currentTime, 0.015)
}

/** 0..1. Ramped rather than set, so volume changes don't click. */
export function setVolume(v: number): void {
  userVolume = Math.min(Math.max(0, v), 1)
  applyGain()
}

export function setNormalize(mode: NormalizeMode): void {
  normalize = mode
  applyGain()
}

/**
 * The gain node's live value.
 *
 * Distinct from currentGainDb, which reports what the level *should* be from
 * volume and ReplayGain. This reads the node itself, so it also sees fades in
 * progress -- the only way to tell a crossfade actually happened.
 */
export function currentGain(): number {
  return graph().gain.gain.value
}

/** The gain currently being applied for the playing track, in dB. */
export function currentGainDb(): number {
  const m = gainMultiplier(currentTrack, normalize)
  return m > 0 ? 20 * Math.log10(m) : 0
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
