import { describe, expect, it } from 'vitest'
import { ease, predict, SNAP_SEC, TAU_MS, type Sample } from '../src/shared/progress'

const sample = (over: Partial<Sample> = {}): Sample => ({
  position: 10,
  duration: 180,
  playing: true,
  at: 1000,
  ...over
})

describe('predict', () => {
  it('advances with the wall clock between reports', () => {
    // The whole point: a report every 250ms must not mean a still playhead.
    expect(predict(sample(), 1000)).toBeCloseTo(10)
    expect(predict(sample(), 1125)).toBeCloseTo(10.125)
    expect(predict(sample(), 1250)).toBeCloseTo(10.25)
  })

  it('holds still while paused', () => {
    expect(predict(sample({ playing: false }), 9999)).toBe(10)
  })

  it('never runs past the end of the track', () => {
    const near = sample({ position: 179.5, duration: 180 })
    expect(predict(near, 1000 + 10_000)).toBe(180)
  })

  it('does not extrapolate backwards if the clock goes back', () => {
    expect(predict(sample(), 500)).toBe(10)
  })

  it('tolerates an unknown duration', () => {
    expect(predict(sample({ duration: 0 }), 3000)).toBeCloseTo(12)
  })
})

describe('ease', () => {
  it('moves toward the target without overshooting', () => {
    const next = ease(10, 10.25, 16)
    expect(next).toBeGreaterThan(10)
    expect(next).toBeLessThan(10.25)
  })

  it('converges on the target when given enough time', () => {
    let v = 10
    for (let i = 0; i < 60; i++) v = ease(v, 10.25, 16)
    expect(v).toBeCloseTo(10.25, 4)
  })

  it('is frame-rate independent', () => {
    // One 32ms step should land where two 16ms steps do, or the easing would
    // run at different speeds on different displays.
    const oneBigStep = ease(0, 10, 32)
    const twoSmallSteps = ease(ease(0, 10, 16), 10, 16)
    expect(oneBigStep).toBeCloseTo(twoSmallSteps, 6)
  })

  it('snaps across a seek rather than sliding the playhead', () => {
    expect(ease(10, 10 + SNAP_SEC + 0.1, 16)).toBe(10 + SNAP_SEC + 0.1)
  })

  it('snaps backwards across a track change', () => {
    expect(ease(178, 0, 16)).toBe(0)
  })

  it('eases rather than snaps for ordinary drift', () => {
    expect(ease(10, 10.2, 16)).not.toBe(10.2)
  })

  it('holds still when no time has passed', () => {
    expect(ease(10, 10.5, 0)).toBe(10)
  })

  it('takes the target when it has no previous value to ease from', () => {
    expect(ease(Number.NaN, 42, 16)).toBe(42)
  })

  it('closes roughly 63% of the gap in one time constant', () => {
    expect(ease(0, 1, TAU_MS)).toBeCloseTo(1 - Math.exp(-1), 6)
  })
})
