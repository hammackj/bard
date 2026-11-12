//! ReplayGain to a playback multiplier. A port of `src/shared/gain.ts`.

use crate::types::{NormalizeMode, Track};

/// Most boost, in dB, normalisation may apply. ReplayGain's reference is
/// 0 dB: a gain of 0 means the track is already at target.
pub const MAX_BOOST_DB: f64 = 12.0;

/// Playback gain for a track, in dB.
///
/// Returns 0 when normalisation is off or the file carries no tags, so an
/// untagged library simply plays at its own level rather than being silently
/// attenuated.
///
/// Positive gain is held back so it cannot push the signal past full scale:
/// ReplayGain says how loud a track *is*, and boosting a quiet-but-peaky track
/// to match would clip it. Boost is also capped outright, because a very quiet
/// track can otherwise ask for 20 dB and bring its noise floor up with it.
pub fn gain_db_for(track: &Track, mode: NormalizeMode) -> f64 {
    let (gain, peak) = match mode {
        NormalizeMode::Off => return 0.0,
        NormalizeMode::Track => (track.track_gain_db, track.track_peak),
        NormalizeMode::Album => (track.album_gain_db.or(track.track_gain_db), track.album_peak.or(track.track_peak)),
    };
    let Some(gain) = gain.filter(|g| g.is_finite()) else { return 0.0 };

    let mut limited = gain.min(MAX_BOOST_DB);
    if let Some(peak) = peak.filter(|p| *p > 0.0) {
        // Headroom available before the peak hits full scale.
        let headroom_db = -20.0 * peak.log10();
        limited = limited.min(headroom_db);
    }
    limited
}

/// Linear multiplier for the audio engine's gain stage.
///
/// The TypeScript took `Track | null` and returned 1 for null; callers with
/// no track simply use 1.0.
pub fn gain_multiplier(track: &Track, mode: NormalizeMode) -> f64 {
    10f64.powf(gain_db_for(track, mode) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        Track {
            id: "t".into(),
            path: "/m/t.flac".into(),
            title: "T".into(),
            artist: "A".into(),
            album: "B".into(),
            duration_sec: 100.0,
            has_artwork: false,
            search_text: "t".into(),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        }
    }
    fn with(tg: Option<f64>, ag: Option<f64>, tp: Option<f64>) -> Track {
        Track { track_gain_db: tg, album_gain_db: ag, track_peak: tp, ..track() }
    }
    fn close(a: f64, b: f64, digits: i32) {
        assert!((a - b).abs() < 10f64.powi(-digits) / 2.0, "{a} != {b}");
    }
    use NormalizeMode::{Album, Off};
    const TRACK_MODE: NormalizeMode = NormalizeMode::Track;

    #[test]
    fn zero_when_normalisation_is_off() {
        assert_eq!(gain_db_for(&with(Some(-6.0), None, None), Off), 0.0);
    }

    #[test]
    fn zero_for_an_untagged_track_rather_than_attenuating_it() {
        assert_eq!(gain_db_for(&track(), TRACK_MODE), 0.0);
        assert_eq!(gain_db_for(&track(), Album), 0.0);
    }

    #[test]
    fn uses_track_gain_in_track_mode() {
        close(gain_db_for(&with(Some(-10.1), Some(-6.2), None), TRACK_MODE), -10.1, 2);
    }

    #[test]
    fn uses_album_gain_in_album_mode() {
        close(gain_db_for(&with(Some(-10.1), Some(-6.2), None), Album), -6.2, 2);
    }

    #[test]
    fn falls_back_to_track_gain_when_album_gain_is_missing() {
        close(gain_db_for(&with(Some(-4.0), None, None), Album), -4.0, 2);
    }

    #[test]
    fn never_boosts_past_the_cap() {
        assert_eq!(gain_db_for(&with(Some(30.0), None, Some(0.01)), TRACK_MODE), MAX_BOOST_DB);
    }

    #[test]
    fn holds_back_a_boost_that_would_push_the_peak_past_full_scale() {
        // Peak is already at full scale, so no positive gain is safe.
        close(gain_db_for(&with(Some(6.0), None, Some(1.0)), TRACK_MODE), 0.0, 2);
    }

    #[test]
    fn allows_the_headroom_a_quiet_peak_leaves() {
        // peak 0.5 -> about 6 dB of headroom.
        close(gain_db_for(&with(Some(9.0), None, Some(0.5)), TRACK_MODE), 6.02, 1);
    }

    #[test]
    fn still_attenuates_a_track_whose_peak_exceeds_full_scale() {
        // The real library has album peaks slightly over 1.0.
        close(gain_db_for(&with(Some(-6.0), None, Some(1.012)), TRACK_MODE), -6.0, 2);
    }

    #[test]
    fn converts_db_to_a_linear_multiplier() {
        close(gain_multiplier(&with(Some(-6.02), None, None), TRACK_MODE), 0.5, 2);
    }

    #[test]
    fn multiplier_is_1_for_an_untagged_track() {
        assert_eq!(gain_multiplier(&track(), TRACK_MODE), 1.0);
    }
}
