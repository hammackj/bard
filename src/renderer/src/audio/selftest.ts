import * as engine from './engine'
import { useStore } from '../state/store'

/**
 * Opt-in diagnostic, run with VITE_BARD_SELFTEST=1.
 *
 * It exists to catch one specific class of failure: a MediaElementAudioSource
 * that has been tainted cross-origin emits pure zeroes while the element still
 * reports playing and the position still advances. Every other check passes
 * against silence, so the only reliable signal is the analyser itself.
 *
 * AudioContext.setSinkId({type:'none'}) renders the graph into no device at
 * all, so this can run at full gain without making a sound.
 */
export async function maybeRunSelfTest(): Promise<void> {
  if (import.meta.env.VITE_BARD_SELFTEST !== '1') return

  const tracks = useStore.getState().tracks
  if (tracks.length === 0) {
    console.warn('[selftest] no tracks; skipped')
    return
  }

  const ctx = engine.getAnalyser().context as AudioContext & {
    setSinkId?: (v: unknown) => Promise<void>
  }
  let silent = false
  try {
    await ctx.setSinkId?.({ type: 'none' })
    silent = true
  } catch {
    /* older runtime: fall back to muting, which cannot detect the bug */
  }

  const previousVolume = useStore.getState().config?.audio.volume ?? 0.8
  engine.setVolume(silent ? 1 : 0)

  const [a, b] = tracks
  await window.bard.clearQueue()
  await window.bard.enqueue(a.id, 'selftest')
  await window.bard.enqueue(b.id, 'selftest')
  await wait(2000)

  const analyser = engine.getAnalyser()
  const buf = new Uint8Array(analyser.fftSize)
  let peak = 0
  for (let i = 0; i < 40; i++) {
    analyser.getByteTimeDomainData(buf)
    for (const v of buf) peak = Math.max(peak, Math.abs(v - 128))
    await wait(25)
  }

  const pos = engine.getPosition()
  console.log(
    `[selftest] playing="${engine.getCurrentTrack()?.title}" expected="${a.title}" ` +
      `pos=${pos.position.toFixed(2)}s ctx=${ctx.state}`
  )
  console.log(
    `[selftest] analyser peak=${peak} (silentSink=${silent}) -> ` +
      (peak > 0 ? 'AUDIO PRESENT' : 'SILENT - source is emitting zeroes')
  )

  await window.bard.skip()
  await wait(1200)
  console.log(
    `[selftest] after skip playing="${engine.getCurrentTrack()?.title}" expected="${b.title}"`
  )

  engine.pause()
  await window.bard.clearQueue()
  if (silent) {
    await engine.setOutputDevice(useStore.getState().config?.audio.outputDeviceId ?? 'default')
  }
  engine.setVolume(previousVolume)
}

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))
