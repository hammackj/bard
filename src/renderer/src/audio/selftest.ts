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

  // The deterministic checks below assume nothing is topping the queue up
  // behind them, so endless is disabled for the duration and restored after.
  const endlessWas = useStore.getState().config?.playback.endless ?? false
  const autoFillWas = useStore.getState().config?.playback.autoFill ?? 'shuffle'
  const crossfadeWas = useStore.getState().config?.audio.crossfadeSec ?? 0
  const repeatWas = useStore.getState().config?.playback.repeat ?? 'off'
  await window.bard.setConfig({ playback: { endless: false } })

  const [a, b] = tracks
  await window.bard.clearQueue()
  await window.bard.enqueue(a.id)
  await window.bard.enqueue(b.id)
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

  // Queue tools: fill, shuffle, and auto-fill when the queue runs dry.
  await window.bard.clearQueue()
  await window.bard.fillQueue(6)
  await wait(600)
  const filled = useStore.getState().queue
  console.log(`[selftest] fillQueue(6) -> queue=${filled.length}`)
  const uniq = new Set(filled.map((q) => q.track.id)).size
  console.log(`[selftest] fill produced ${uniq} distinct of ${filled.length}`)

  const before = useStore.getState().queue.map((q) => q.id)
  await window.bard.shuffleQueue()
  await wait(400)
  const after = useStore.getState().queue.map((q) => q.id)
  const moved = before.filter((id, i) => after[i] !== id).length
  console.log(`[selftest] shuffle moved ${moved}/${before.length} items`)

  // Auto-fill: with a mode set, an empty queue should still produce a track.
  const mode = useStore.getState().config?.playback.autoFill
  await window.bard.clearQueue()
  await wait(300)
  await window.bard.skip()
  await wait(1200)
  const kept = engine.getCurrentTrack()
  console.log(
    `[selftest] autoFill=${mode} after emptying queue -> ` +
      (kept ? `still playing "${kept.title}"` : 'stopped')
  )

  // Endless: the queue must refill itself and never hand out a repeat.
  await window.bard.setConfig({ playback: { endless: true } })
  await window.bard.clearQueue()
  await window.bard.skip()
  await wait(900)
  const q1 = useStore.getState().queue
  console.log(`[selftest] endless topped queue to ${q1.length}`)
  const seen = new Set(q1.map((i) => i.track.id))
  for (let i = 0; i < 12; i++) {
    await window.bard.skip()
    await wait(250)
    for (const item of useStore.getState().queue) seen.add(item.track.id)
  }
  const q2 = useStore.getState().queue
  console.log(
    `[selftest] after 12 skips queue=${q2.length} distinct tracks seen=${seen.size} ` +
      `(repeats=${seen.size < 12 ? 'YES' : 'none'})`
  )

  // ReplayGain: two tracks with different tagged loudness must end up with
  // different applied gain, and an untagged one with none.
  const tagged = useStore.getState().tracks.filter((t) => t.trackGainDb !== undefined)
  if (tagged.length > 1) {
    const loud = tagged.reduce((a, b) => ((a.trackGainDb ?? 0) < (b.trackGainDb ?? 0) ? a : b))
    const quiet = tagged.reduce((a, b) => ((a.trackGainDb ?? 0) > (b.trackGainDb ?? 0) ? a : b))
    engine.setNormalize('track')
    await engine.play(loud)
    await wait(400)
    const gLoud = engine.currentGainDb()
    await engine.play(quiet)
    await wait(400)
    const gQuiet = engine.currentGainDb()
    engine.pause()
    console.log(
      `[selftest] replaygain tagged=${tagged.length}/${useStore.getState().tracks.length} ` +
        `loudest "${loud.title}" ${gLoud.toFixed(1)}dB vs quietest "${quiet.title}" ` +
        `${gQuiet.toFixed(1)}dB (delta ${(gQuiet - gLoud).toFixed(1)}dB)`
    )
  }

  // Previous: back one track, and restart when already a few seconds in.
  //
  // Driven through main rather than the engine, because the whole point of the
  // feature is the history main keeps -- the engine has no idea what came
  // before.
  // Auto-fill is off here so a skip past the end of the queue cannot slip an
  // unrelated track in and make the history ambiguous.
  await window.bard.setConfig({ playback: { endless: false, autoFill: 'off' } })
  await window.bard.clearQueue()
  const [p1, p2, p3] = tracks
  for (const t of [p1, p2, p3]) await window.bard.enqueue(t.id)
  for (let i = 0; i < 3; i++) {
    await window.bard.skip()
    await wait(600)
  }
  const atThird = engine.getCurrentTrack()?.title
  await window.bard.previous()
  await wait(700)
  const backOne = engine.getCurrentTrack()?.title
  await window.bard.previous()
  await wait(700)
  const backTwo = engine.getCurrentTrack()?.title
  console.log(
    `[selftest] previous: at "${atThird}" -> "${backOne}" -> "${backTwo}" ` +
      `(expected "${p3.title}" -> "${p2.title}" -> "${p1.title}") ` +
      (atThird === p3.title && backOne === p2.title && backTwo === p1.title ? 'OK' : 'WRONG')
  )

  // Past the restart threshold, previous should replay rather than step back.
  //
  // Seeking through the engine alone is not enough: main decides this from the
  // position the renderer reports, so the test has to let a report land first.
  // Seek until it sticks: right after a track change the element can still be
  // loading, and a seek issued before metadata arrives is silently dropped.
  let seeked = 0
  for (let i = 0; i < 20 && seeked < 15; i++) {
    engine.seek(20)
    await wait(150)
    seeked = engine.getPosition().position
  }
  // ...then let a progress report carry that position to main, which is what
  // actually decides between restarting and stepping back.
  await wait(1200)
  const beforeRestart = engine.getCurrentTrack()?.title
  await window.bard.previous()
  await wait(900)
  const afterRestart = engine.getCurrentTrack()?.title
  const restartedPos = engine.getPosition().position
  console.log(
    `[selftest] previous ${seeked.toFixed(0)}s in: "${beforeRestart}" -> "${afterRestart}" at ` +
      `${restartedPos.toFixed(1)}s ` +
      (afterRestart === beforeRestart && restartedPos < 5 ? 'OK (restarted)' : 'WRONG')
  )
  engine.pause()
  // Restore auto-fill before the later checks, which assume a skip on an empty
  // queue still produces a track.
  await window.bard.setConfig({ playback: { autoFill: autoFillWas } })

  // Repeat one: the same track must come back after it ends.
  await window.bard.setConfig({ playback: { repeat: 'one' } })
  await window.bard.clearQueue()
  await window.bard.enqueue(p1.id)
  await window.bard.enqueue(p2.id)
  await window.bard.skip()
  await wait(600)
  const beforeEnd = engine.getCurrentTrack()?.title
  await window.bard.reportEnded()
  await wait(800)
  console.log(
    `[selftest] repeat=one: "${beforeEnd}" -> "${engine.getCurrentTrack()?.title}" ` +
      (engine.getCurrentTrack()?.title === beforeEnd ? 'OK' : 'WRONG')
  )

  // Repeat all: an emptied queue refills from history, without replaying the
  // track that just finished across the seam.
  await window.bard.setConfig({ playback: { repeat: 'all' } })
  await window.bard.clearQueue()
  for (const t of [p1, p2, p3]) await window.bard.enqueue(t.id)
  for (let i = 0; i < 3; i++) {
    await window.bard.skip()
    await wait(400)
  }
  const lastOfLap = engine.getCurrentTrack()?.title
  await window.bard.reportEnded()
  await wait(900)
  const firstOfNextLap = engine.getCurrentTrack()?.title
  console.log(
    `[selftest] repeat=all: lap ended on "${lastOfLap}" -> next lap starts "${firstOfNextLap}" ` +
      (firstOfNextLap && firstOfNextLap !== lastOfLap ? 'OK (no seam repeat)' : 'WRONG')
  )
  engine.pause()

  // Crossfade: the gain must actually come down before the swap, not jump.
  await window.bard.setConfig({ audio: { crossfadeSec: 2 } })
  await engine.play(p1)
  await wait(500)
  const gBefore = engine.currentGain()
  const swap = engine.play(p2, 2)
  await wait(900)
  const mid = engine.currentGain()
  await swap
  await wait(2200)
  const settled = engine.currentGain()
  console.log(
    `[selftest] crossfade 2s: gain ${gBefore.toFixed(3)} -> ${mid.toFixed(3)} (mid-fade) -> ` +
      `${settled.toFixed(3)} ` +
      (mid < gBefore * 0.9 && settled > mid ? 'OK (faded down then back)' : 'WRONG')
  )
  engine.pause()

  // Loved: check the flag round-trips through main and back into the store.
  //
  // Deliberately NOT a statistical check: 40 draws from a 585-track deck would
  // expect a weight-3 track about 0.2 times, so any result would be noise. The
  // weighting itself is proven deterministically in tests/deck.test.ts.
  const favourite = useStore.getState().tracks[3]
  if (favourite) {
    const after = await window.bard.toggleLoved(favourite.id)
    const on = after.includes(favourite.id)
    const back = await window.bard.toggleLoved(favourite.id)
    console.log(
      `[selftest] loved "${favourite.title}" toggled on=${on} off=${!back.includes(favourite.id)}`
    )
  }

  // Content ID exclusion: a claimed album must leave the queue and stay out of
  // future picks, not merely get a badge.
  const victim = useStore.getState().tracks[0]?.album
  if (victim) {
    await window.bard.setAlbumFlag(victim, 'claimed', 'selftest')
    await window.bard.clearQueue()
    await window.bard.skip()
    await wait(1200)
    const q = useStore.getState().queue
    const leaked = q.filter((i) => i.track.album === victim).length
    const playingClaimed = engine.getCurrentTrack()?.album === victim
    console.log(
      `[selftest] claimed "${victim}": queued from it=${leaked} playing from it=${playingClaimed}`
    )
    useStore.getState().setAlbumFlags(await window.bard.setAlbumFlag(victim, 'untested'))
  }

  engine.pause()
  await window.bard.clearQueue()
  useStore.getState().setConfig(
    await window.bard.setConfig({
      playback: { endless: endlessWas, autoFill: autoFillWas, repeat: repeatWas },
      audio: { crossfadeSec: crossfadeWas }
    })
  )
  if (silent) {
    try {
      await engine.setOutputDevice(useStore.getState().config?.audio.outputDeviceId ?? 'default')
    } catch (e) {
      console.warn(`[selftest] restoring output device: ${(e as Error).message}`)
    }
  }
  engine.setVolume(previousVolume)
}

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))
