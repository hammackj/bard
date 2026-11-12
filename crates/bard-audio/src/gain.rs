//! The engine's single gain stage: volume x normalisation, with ramps.
//!
//! This stands in for the Web Audio GainNode in the Electron engine and keeps
//! its three kinds of automation, because each one exists for a reason:
//!
//! - `Set` -- `setValueAtTime`: a jump, for the start of a track, so the
//!   first moments of a loud track don't play at the previous track's gain;
//! - `Target` -- `setTargetAtTime(v, now, 0.015)`: an exponential approach,
//!   for volume and normalisation changes, so they don't click;
//! - `Linear` -- `linearRampToValueAtTime`: the crossfade's fade out / in.
//!
//! The Web Audio version ramped to 0.0001 rather than 0, because a linear
//! ramp toward an exact zero is degenerate there. Here zero is fine, and
//! exact silence matters: volume 0 must produce zero samples.

/// Time constant of volume ramps, as in engine.ts applyGain.
pub(crate) const VOLUME_TAU_SEC: f32 = 0.015;

/// Volume and normalisation multiplied into one gain.
///
/// They are multiplied into one gain rather than chained through two stages:
/// the user's volume is a preference, the ReplayGain figure is a property of
/// the file, and keeping them separate in code but combined in the signal
/// path means changing one never disturbs the other. Clamped to 0..4 as the
/// Electron engine did; non-finite input is treated as silence.
pub fn combined_gain(volume: f32, norm_gain: f32) -> f32 {
    let g = volume * norm_gain;
    if g.is_finite() { g.clamp(0.0, 4.0) } else { 0.0 }
}

/// User volume as the engine stores it: 0..1, NaN as 0.
pub(crate) fn clamp_volume(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Normalisation multiplier: non-negative, NaN as unity.
pub(crate) fn clamp_norm(m: f32) -> f32 {
    if m.is_finite() { m.max(0.0) } else { 1.0 }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GainCmd {
    Set(f32),
    Target(f32),
    /// `retarget`: a `Target` arriving mid-ramp moves this ramp's end point
    /// (a volume change during a fade-in) instead of being dropped (a volume
    /// change during a fade-out, which the swap will apply anyway).
    Linear { to: f32, secs: f32, retarget: bool },
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Hold,
    Target { to: f32, coef: f32 },
    Linear { to: f32, step: f32, left: u32, retarget: bool },
}

/// Per-frame gain automation. Runs on the audio thread: no allocation.
#[derive(Clone, Debug)]
pub(crate) struct GainRamp {
    value: f32,
    mode: Mode,
    rate: f32,
}

impl GainRamp {
    pub fn new(rate: u32, value: f32) -> GainRamp {
        GainRamp { value, mode: Mode::Hold, rate: rate as f32 }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn apply(&mut self, cmd: GainCmd) {
        match cmd {
            GainCmd::Set(v) => {
                self.value = v;
                self.mode = Mode::Hold;
            }
            GainCmd::Target(v) => match &mut self.mode {
                Mode::Linear { to, step, left, retarget: true } => {
                    *to = v;
                    *step = (v - self.value) / (*left).max(1) as f32;
                }
                Mode::Linear { retarget: false, .. } => {}
                mode => {
                    let coef = 1.0 - (-1.0 / (VOLUME_TAU_SEC * self.rate)).exp();
                    *mode = Mode::Target { to: v, coef };
                }
            },
            GainCmd::Linear { to, secs, retarget } => {
                let left = (secs * self.rate).round() as u32;
                if left == 0 {
                    self.value = to;
                    self.mode = Mode::Hold;
                } else {
                    let step = (to - self.value) / left as f32;
                    self.mode = Mode::Linear { to, step, left, retarget };
                }
            }
        }
    }

    /// The gain for the next frame.
    #[inline]
    pub fn next(&mut self) -> f32 {
        match &mut self.mode {
            Mode::Hold => {}
            Mode::Target { to, coef } => {
                self.value += (*to - self.value) * *coef;
                // f32 stalls a hair short of the target; snap once inaudibly close.
                if (*to - self.value).abs() < 1e-4 {
                    self.value = *to;
                    self.mode = Mode::Hold;
                }
            }
            Mode::Linear { to, step, left, .. } => {
                *left -= 1;
                if *left == 0 {
                    self.value = *to;
                    self.mode = Mode::Hold;
                } else {
                    self.value += *step;
                }
            }
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn run(g: &mut GainRamp, frames: usize) -> f32 {
        for _ in 0..frames {
            g.next();
        }
        g.value()
    }

    #[test]
    fn combines_like_apply_gain() {
        assert_eq!(combined_gain(0.5, 1.0), 0.5);
        assert_eq!(combined_gain(1.0, 2.0), 2.0);
        // Clamped to 4 however hot the normalisation asks for.
        assert_eq!(combined_gain(1.0, 10.0), 4.0);
        assert_eq!(combined_gain(0.0, 3.0), 0.0);
        assert_eq!(combined_gain(f32::NAN, 1.0), 0.0);
        assert_eq!(clamp_volume(1.5), 1.0);
        assert_eq!(clamp_volume(-1.0), 0.0);
        assert_eq!(clamp_norm(f32::NAN), 1.0);
    }

    #[test]
    fn target_is_a_15ms_exponential() {
        let mut g = GainRamp::new(RATE, 0.0);
        g.apply(GainCmd::Target(1.0));
        // One time constant: 1 - 1/e of the way there.
        let v = run(&mut g, (VOLUME_TAU_SEC * RATE as f32) as usize);
        assert!((v - (1.0 - (-1f32).exp())).abs() < 0.01, "{v}");
        // No step anywhere along the way: that is the point.
        let mut g = GainRamp::new(RATE, 0.0);
        g.apply(GainCmd::Target(1.0));
        let mut prev = 0.0;
        for _ in 0..RATE / 10 {
            let v = g.next();
            assert!(v - prev < 0.002);
            prev = v;
        }
        assert_eq!(run(&mut g, RATE as usize / 2), 1.0, "settles exactly");
    }

    #[test]
    fn linear_ramp_hits_target_on_time() {
        let mut g = GainRamp::new(RATE, 0.8);
        g.apply(GainCmd::Linear { to: 0.0, secs: 2.0, retarget: false });
        let mid = run(&mut g, RATE as usize);
        assert!((mid - 0.4).abs() < 1e-3, "{mid}");
        let end = run(&mut g, RATE as usize);
        assert_eq!(end, 0.0);
    }

    #[test]
    fn volume_change_during_fade_out_is_held_back() {
        let mut g = GainRamp::new(RATE, 1.0);
        g.apply(GainCmd::Linear { to: 0.0, secs: 1.0, retarget: false });
        run(&mut g, RATE as usize / 2);
        g.apply(GainCmd::Target(1.0));
        assert_eq!(run(&mut g, RATE as usize), 0.0);
    }

    #[test]
    fn volume_change_during_fade_in_moves_the_end_point() {
        let mut g = GainRamp::new(RATE, 0.0);
        g.apply(GainCmd::Linear { to: 1.0, secs: 1.0, retarget: true });
        run(&mut g, RATE as usize / 2);
        g.apply(GainCmd::Target(0.5));
        let v = run(&mut g, RATE as usize / 4);
        assert!(v < 0.5 && v > 0.25, "{v}");
        assert_eq!(run(&mut g, RATE as usize), 0.5);
    }

    #[test]
    fn set_cancels_a_ramp() {
        // silenceNow(): cancel whatever is in flight and drop to silence.
        let mut g = GainRamp::new(RATE, 0.3);
        g.apply(GainCmd::Linear { to: 1.0, secs: 1.0, retarget: true });
        run(&mut g, 100);
        g.apply(GainCmd::Set(0.0));
        assert_eq!(run(&mut g, 1000), 0.0);
    }
}
