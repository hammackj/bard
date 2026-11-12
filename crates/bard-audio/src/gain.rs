//! The engine's gain stage: a *level* (volume x normalisation) times an
//! *envelope* (the crossfade), each with its own automation.
//!
//! The Electron engine had one GainNode for both, and so did this engine at
//! first: a fade-out was a ramp of the one gain, and a volume change during
//! it had to be ignored -- applying it would have cancelled the fade -- so
//! turning the volume to zero left the outgoing track playing for the rest of
//! the fade, up to twelve seconds. Two stages, multiplied per frame, keep the
//! two apart: the level answers the volume at once, the envelope finishes its
//! fade, and nothing done to one can undo the other.
//!
//! - `SetLevel` -- a jump, for the start of a track, so the first moments of
//!   a loud track don't play at the previous track's gain;
//! - `Level` -- `setTargetAtTime(v, now, 0.015)`: an exponential approach,
//!   for volume and normalisation changes, so they don't click;
//! - `SetEnvelope` / `Fade` -- the envelope jumped, or ramped linearly
//!   (`linearRampToValueAtTime`): the crossfade's fade out / in.
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
    SetLevel(f32),
    Level(f32),
    /// 0 silent, 1 open.
    SetEnvelope(f32),
    Fade {
        to: f32,
        secs: f32,
    },
}

#[derive(Clone, Copy, Debug)]
enum LevelMode {
    Hold,
    Target { to: f32, coef: f32 },
}

#[derive(Clone, Copy, Debug)]
enum EnvMode {
    Hold,
    Linear { to: f32, step: f32, left: u32 },
}

/// Per-frame gain automation. Runs on the audio thread: no allocation.
#[derive(Clone, Debug)]
pub(crate) struct GainRamp {
    level: f32,
    level_mode: LevelMode,
    env: f32,
    env_mode: EnvMode,
    rate: f32,
}

impl GainRamp {
    /// At `level`, envelope open.
    pub fn new(rate: u32, level: f32) -> GainRamp {
        GainRamp { level, level_mode: LevelMode::Hold, env: 1.0, env_mode: EnvMode::Hold, rate: rate as f32 }
    }

    /// The gain applied: level x envelope.
    pub fn value(&self) -> f32 {
        self.level * self.env
    }

    pub fn apply(&mut self, cmd: GainCmd) {
        match cmd {
            GainCmd::SetLevel(v) => {
                self.level = v;
                self.level_mode = LevelMode::Hold;
            }
            GainCmd::Level(v) => {
                let coef = 1.0 - (-1.0 / (VOLUME_TAU_SEC * self.rate)).exp();
                self.level_mode = LevelMode::Target { to: v, coef };
            }
            GainCmd::SetEnvelope(v) => {
                self.env = v;
                self.env_mode = EnvMode::Hold;
            }
            GainCmd::Fade { to, secs } => {
                let left = (secs * self.rate).round() as u32;
                if left == 0 {
                    self.env = to;
                    self.env_mode = EnvMode::Hold;
                } else {
                    let step = (to - self.env) / left as f32;
                    self.env_mode = EnvMode::Linear { to, step, left };
                }
            }
        }
    }

    /// The gain for the next frame.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if let LevelMode::Target { to, coef } = self.level_mode {
            self.level += (to - self.level) * coef;
            // f32 stalls a hair short of the target; snap once inaudibly close.
            if (to - self.level).abs() < 1e-4 {
                self.level = to;
                self.level_mode = LevelMode::Hold;
            }
        }
        if let EnvMode::Linear { to, step, left } = &mut self.env_mode {
            *left -= 1;
            if *left == 0 {
                self.env = *to;
                self.env_mode = EnvMode::Hold;
            } else {
                self.env += *step;
            }
        }
        self.value()
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
    fn level_is_a_15ms_exponential() {
        let mut g = GainRamp::new(RATE, 0.0);
        g.apply(GainCmd::Level(1.0));
        // One time constant: 1 - 1/e of the way there.
        let v = run(&mut g, (VOLUME_TAU_SEC * RATE as f32) as usize);
        assert!((v - (1.0 - (-1f32).exp())).abs() < 0.01, "{v}");
        // No step anywhere along the way: that is the point.
        let mut g = GainRamp::new(RATE, 0.0);
        g.apply(GainCmd::Level(1.0));
        let mut prev = 0.0;
        for _ in 0..RATE / 10 {
            let v = g.next();
            assert!(v - prev < 0.002);
            prev = v;
        }
        assert_eq!(run(&mut g, RATE as usize / 2), 1.0, "settles exactly");
    }

    #[test]
    fn a_fade_hits_its_end_on_time() {
        let mut g = GainRamp::new(RATE, 0.8);
        g.apply(GainCmd::Fade { to: 0.0, secs: 2.0 });
        let mid = run(&mut g, RATE as usize);
        assert!((mid - 0.4).abs() < 1e-3, "{mid}");
        let end = run(&mut g, RATE as usize);
        assert_eq!(end, 0.0);
    }

    /// The bug the split exists for: volume during a fade-out applied at once.
    #[test]
    fn volume_zero_during_a_fade_out_silences_at_once() {
        let mut g = GainRamp::new(RATE, 1.0);
        g.apply(GainCmd::Fade { to: 0.0, secs: 2.0 });
        run(&mut g, RATE as usize / 10);
        g.apply(GainCmd::Level(0.0));
        // The usual 15 ms approach, so it does not click: -58 dB by 100 ms,
        // exact silence once it snaps, about 140 ms in.
        assert!(run(&mut g, RATE as usize / 10) < 0.0015);
        assert_eq!(run(&mut g, RATE as usize / 10), 0.0, "silent within 200 ms");
    }

    /// And the other way: raising the volume cannot undo the fade.
    #[test]
    fn volume_up_during_a_fade_out_still_ends_silent() {
        let mut g = GainRamp::new(RATE, 0.5);
        g.apply(GainCmd::Fade { to: 0.0, secs: 1.0 });
        run(&mut g, RATE as usize / 2);
        g.apply(GainCmd::Level(1.0));
        let v = run(&mut g, RATE as usize / 10);
        assert!(v < 0.5, "louder, still fading: {v}");
        assert_eq!(run(&mut g, RATE as usize), 0.0);
    }

    #[test]
    fn volume_during_a_fade_in_moves_where_it_ends() {
        let mut g = GainRamp::new(RATE, 1.0);
        g.apply(GainCmd::SetEnvelope(0.0));
        g.apply(GainCmd::Fade { to: 1.0, secs: 1.0 });
        run(&mut g, RATE as usize / 2);
        g.apply(GainCmd::Level(0.5));
        let v = run(&mut g, RATE as usize / 4);
        assert!(v < 0.5 && v > 0.25, "{v}");
        assert_eq!(run(&mut g, RATE as usize), 0.5);
    }

    #[test]
    fn set_envelope_cancels_a_fade() {
        let mut g = GainRamp::new(RATE, 0.3);
        g.apply(GainCmd::Fade { to: 0.0, secs: 1.0 });
        run(&mut g, 100);
        g.apply(GainCmd::SetEnvelope(1.0));
        assert_eq!(run(&mut g, 1000), 0.3);
    }
}
