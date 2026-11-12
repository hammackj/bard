//! Smoothing for the playback position. A port of `src/shared/progress.ts`.
//!
//! The audio engine reports progress a few times a second. Drawing that
//! straight into a 60fps bar means the playhead sits still for fifteen frames
//! and then jumps -- the motion reads as stuttering even though the timing is
//! perfectly accurate.
//!
//! Two steps fix it. [`predict`] extrapolates from the last report using the
//! wall clock, so between reports the position keeps advancing. [`ease`] then
//! moves the drawn value toward that prediction a fraction at a time, so the
//! small corrections when a real report lands are absorbed instead of
//! snapping.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    /// When this sample was taken, in ms, on the same clock later passed to predict.
    pub at: f64,
}

/// A jump larger than this is a seek or a track change, not drift.
///
/// Easing across one of those would slide the playhead over the whole bar,
/// which looks like a glitch rather than a seek, so they snap instead.
pub const SNAP_SEC: f64 = 1.5;

/// How quickly the drawn value chases the prediction, in ms. Lower is tighter.
pub const TAU_MS: f64 = 110.0;

/// Where the track should be right now, given the last report.
pub fn predict(s: &Sample, now: f64) -> f64 {
    if !s.playing {
        return s.position;
    }
    let elapsed = (now - s.at).max(0.0) / 1000.0;
    let p = s.position + elapsed;
    // Never run past the end: playback stops there, and a playhead that
    // overshoots the bar before the next track loads is worse than one that
    // waits.
    if s.duration > 0.0 { p.min(s.duration) } else { p }
}

/// Moves `displayed` toward `target` by a time-corrected fraction.
///
/// The step depends on elapsed time rather than being a fixed per-frame
/// fraction, so the same easing looks identical at 60Hz and 144Hz and does not
/// lurch after a dropped frame.
pub fn ease(displayed: f64, target: f64, dt_ms: f64, tau_ms: f64) -> f64 {
    if !displayed.is_finite() || (target - displayed).abs() > SNAP_SEC {
        return target;
    }
    // Also catches NaN.
    if dt_ms.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return displayed;
    }
    displayed + (target - displayed) * (1.0 - (-dt_ms / tau_ms).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sample {
        Sample { position: 10.0, duration: 180.0, playing: true, at: 1000.0 }
    }
    fn close(a: f64, b: f64, digits: i32) {
        assert!((a - b).abs() < 10f64.powi(-digits) / 2.0, "{a} != {b}");
    }
    fn e(d: f64, t: f64, dt: f64) -> f64 {
        ease(d, t, dt, TAU_MS)
    }

    #[test]
    fn predict_advances_with_the_wall_clock() {
        close(predict(&sample(), 1000.0), 10.0, 2);
        close(predict(&sample(), 1125.0), 10.125, 2);
        close(predict(&sample(), 1250.0), 10.25, 2);
    }

    #[test]
    fn predict_holds_still_while_paused() {
        assert_eq!(predict(&Sample { playing: false, ..sample() }, 9999.0), 10.0);
    }

    #[test]
    fn predict_never_runs_past_the_end() {
        let near = Sample { position: 179.5, duration: 180.0, ..sample() };
        assert_eq!(predict(&near, 11_000.0), 180.0);
    }

    #[test]
    fn predict_does_not_extrapolate_backwards() {
        assert_eq!(predict(&sample(), 500.0), 10.0);
    }

    #[test]
    fn predict_tolerates_an_unknown_duration() {
        close(predict(&Sample { duration: 0.0, ..sample() }, 3000.0), 12.0, 2);
    }

    #[test]
    fn ease_moves_toward_the_target_without_overshooting() {
        let next = e(10.0, 10.25, 16.0);
        assert!(next > 10.0 && next < 10.25);
    }

    #[test]
    fn ease_converges() {
        let mut v = 10.0;
        for _ in 0..60 {
            v = e(v, 10.25, 16.0);
        }
        close(v, 10.25, 4);
    }

    #[test]
    fn ease_is_frame_rate_independent() {
        close(e(0.0, 10.0, 32.0), e(e(0.0, 10.0, 16.0), 10.0, 16.0), 6);
    }

    #[test]
    fn ease_snaps_across_a_seek() {
        assert_eq!(e(10.0, 10.0 + SNAP_SEC + 0.1, 16.0), 10.0 + SNAP_SEC + 0.1);
    }

    #[test]
    fn ease_snaps_backwards_across_a_track_change() {
        assert_eq!(e(178.0, 0.0, 16.0), 0.0);
    }

    #[test]
    fn ease_eases_ordinary_drift() {
        assert_ne!(e(10.0, 10.2, 16.0), 10.2);
    }

    #[test]
    fn ease_holds_still_when_no_time_passed() {
        assert_eq!(e(10.0, 10.5, 0.0), 10.0);
    }

    #[test]
    fn ease_takes_the_target_with_no_previous_value() {
        assert_eq!(e(f64::NAN, 42.0, 16.0), 42.0);
    }

    #[test]
    fn ease_closes_63_percent_in_one_time_constant() {
        close(e(0.0, 1.0, TAU_MS), 1.0 - (-1f64).exp(), 6);
    }
}
