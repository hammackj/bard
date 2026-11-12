//! The window's model, updated from events: what the core, the audio engine,
//! the services and the app's own workers report.
//!
//! Everything it needs from outside goes through [`Effects`], which the app
//! implements against the core and the engine and the tests implement as a
//! recording, so the state transitions can be checked without either.

use super::{AppEvent, Control};
use crate::model::Ui;
use bard_audio::PlayerEvent;
use bard_core::types::{AlbumFlags, Config, PlaybackProgress, QueueItem, StatsMap, Track, TransportAction};
use bard_core::{CoreEvent, Transition, gain_multiplier};
use std::time::Instant;

/// The fallback notice, worded as Electron's, which also said the choice stays.
pub(super) const DEVICE_FALLBACK: &str =
    "saved output device unavailable, using system default — your setting is kept.";

/// What the reducer asks of the rest of the app.
pub(super) trait Effects {
    // the engine
    /// Returns the request's generation (see `Player::play`).
    fn play(&self, track: &Track, norm_gain: f32, crossfade_sec: f32) -> u64;
    /// Returns the request's generation.
    fn stop(&self) -> u64;
    /// Ends a crossfade's fade-out at once (see `Player::cut_fade`).
    fn cut_fade(&self);
    /// Loads a track paused (see `Player::load_paused`); its generation.
    fn load_paused(&self, track: &Track, norm_gain: f32) -> u64;
    /// The engine is playing, or fading towards a track that will play.
    fn playing(&self) -> bool;
    /// The request the user's pause holds (see `Player::held`).
    fn held(&self) -> Option<u64>;
    fn pause(&self);
    fn seek(&self, seconds: f64);
    /// For the request `generation`.
    fn set_norm_gain(&self, generation: u64, gain: f32);
    fn set_output(&self, id: &str) -> Result<(), String>;
    // intents
    fn toggle_play(&self);
    fn skip(&self);
    fn previous(&self);
    fn set_volume(&self, volume: f64);
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>);
    fn scan(&self);
    // the core
    fn report_progress(&self, p: PlaybackProgress);
    fn report_ended(&self);
    fn report_failed(&self, give_up: bool);
    fn stats(&self) -> StatsMap;
    fn album_flags(&self) -> AlbumFlags;
    fn loved(&self) -> Vec<String>;
    fn tracks(&self) -> Vec<Track>;
    fn queue(&self) -> Vec<QueueItem>;
}

pub(super) fn reduce(ui: &mut Ui, fx: &dyn Effects, ev: AppEvent) {
    match ev {
        AppEvent::Control(c) => match c {
            // Asked of the engine: `ui.playing` lags a pause just made, and is
            // false from a track's end until the next one first reports.
            Control::Play => {
                if !fx.playing() {
                    fx.toggle_play();
                }
            }
            Control::Pause => fx.pause(),
            Control::Toggle => fx.toggle_play(),
            Control::Next => fx.skip(),
            Control::Previous => fx.previous(),
            Control::Seek(s) => fx.seek(s),
            Control::SetVolume(v) => {
                ui.volume = v.clamp(0., 1.);
                fx.set_volume(ui.volume);
            }
            // Handled by dispatch.
            Control::ShowWindow(_) | Control::ShowPopup | Control::Quit => {}
        },
        // Handled by dispatch: the picker needs the App.
        AppEvent::ChooseFolder => {}
        // Only while that is still the saved choice.
        AppEvent::OutputAppeared { saved, id } if ui.config.audio.output_device_id == saved => {
            if id != saved {
                eprintln!("[bard] output device: carried over the Electron setting as {id}");
                let new = id.clone();
                fx.update_config(Box::new(move |c| c.audio.output_device_id = new));
            }
            // Only the fallback notice is cleared; an unrelated one stays.
            if fx.set_output(&id).is_ok() && ui.notice.as_deref() == Some(DEVICE_FALLBACK) {
                ui.notice = None;
            }
        }
        AppEvent::OutputAppeared { .. } => {}
        AppEvent::SleepDue(at) => {
            // Pauses rather than quitting: waking to a closed app and a lost
            // queue is worse than waking to a paused one.
            if ui.sleep_ends_at == Some(at) {
                ui.sleep_ends_at = None;
                // Whatever the last report said: between one track's end
                // and the next one's first report the window shows nothing
                // playing, and the music would play on all night.
                fx.pause();
            }
        }
        AppEvent::ObsSaved { stored, chars } => {
            // A clear is not news; a save says so either way, since a
            // credential field that changes nothing visible on save is
            // indistinguishable from one that failed.
            ui.obs_message = if chars == 0 {
                None
            } else if stored {
                Some(format!("Password saved ({chars} characters) — reconnecting to OBS"))
            } else {
                Some("No OS keyring available — password not stored".into())
            };
        }
        AppEvent::Rescan => {
            ui.scanning = true;
            fx.scan();
        }
        AppEvent::Scanned => {
            ui.queue = fx.queue();
            ui.scanning = false;
            ui.scan_progress = None;
            // Dev aid: start playback once the library is in, so a change can
            // be checked end to end without clicking.
            if std::env::var_os("BARD_AUTOPLAY").is_some() {
                fx.toggle_play();
            }
        }
        AppEvent::Core(ev) => core(ui, fx, ev),
        AppEvent::Player(ev) => player(ui, fx, ev),
    }
}

fn core(ui: &mut Ui, fx: &dyn Effects, ev: CoreEvent) {
    match ev {
        CoreEvent::QueueChanged(q) => ui.queue = q,
        CoreEvent::PlayTrack(t, transition) => {
            let gain = gain_multiplier(&t, ui.config.audio.normalize) as f32;
            // Streamer mode replacing a track while the user has paused: the
            // replacement waits, paused. Asked of the pause itself (held by
            // the engine handle, set the moment it is made), not of the last
            // report, which lags -- nor of whether the engine is playing: a
            // track that has just run out is not playing either, and its
            // replacement must play.
            let held = fx.held() == Some(ui.generation);
            let hold = std::mem::take(&mut ui.next_paused) || (transition == Transition::Cut && held);
            ui.generation = match transition {
                _ if hold => fx.load_paused(&t, gain),
                Transition::Normal => fx.play(&t, gain, ui.config.audio.crossfade_sec as f32),
                Transition::Cut => fx.play(&t, gain, 0.),
            };
            ui.track = Some(t);
            ui.position = 0.;
            ui.duration = 0.;
            ui.position_at = Instant::now();
        }
        // Only the null case matters here: a non-null now-playing is
        // always followed by PlayTrack, which is what loads the audio.
        // Without this the window kept the finished track's title and
        // cover while the overlay had already gone blank.
        CoreEvent::NowPlaying(None) => {
            ui.next_paused = false;
            ui.generation = fx.stop();
            ui.track = None;
            ui.position = 0.;
            ui.duration = 0.;
            ui.playing = false;
        }
        // A rescan's refresh of the track on screen: its title, album, art
        // as the scan found them. Nothing about playback changes. (A new
        // track's NowPlaying is followed by its PlayTrack, which sets it.)
        CoreEvent::NowPlaying(Some(item)) => {
            if ui.track.as_ref().is_some_and(|t| t.id == item.track.id) {
                ui.track = Some(item.track);
            }
        }
        CoreEvent::CutFade => fx.cut_fade(),
        CoreEvent::ScanProgress(p) => {
            // Progress and the end of a scan arrive on different channels,
            // so a last report can land after `Scanned`; one that says the
            // scan is done must not turn the indicator back on.
            ui.scanning = p.scanned < p.total;
            ui.scan_progress = ui.scanning.then_some(p);
        }
        CoreEvent::Overlay(_) => {}
        CoreEvent::StatsChanged => ui.stats = fx.stats(),
        CoreEvent::AlbumFlagsChanged => ui.album_flags = fx.album_flags(),
        CoreEvent::LovedChanged => ui.loved = fx.loved(),
        CoreEvent::Transport(TransportAction::Toggle) => fx.toggle_play(),
        CoreEvent::Transport(TransportAction::Restart) => fx.seek(0.),
        CoreEvent::Notice(msg) => ui.notice = Some(msg),
        CoreEvent::LibraryChanged => ui.set_tracks(fx.tracks()),
        CoreEvent::ConfigChanged(c) => {
            // The core has already dropped a removed folder's tracks; a
            // scan picks up an added folder's, and a removed folder's
            // scan in flight is discarded.
            if c.library_paths != ui.config.library_paths {
                ui.scanning = true;
                fx.scan();
            }
            // Normalisation is a property of playback, not of a track, so
            // it follows the setting rather than being applied at play time.
            if c.audio.normalize != ui.config.audio.normalize
                && let Some(t) = &ui.track
            {
                fx.set_norm_gain(ui.generation, gain_multiplier(t, c.audio.normalize) as f32);
            }
            ui.config = c;
        }
        CoreEvent::StreamerSource(s) => ui.streamer_source = s,
    }
}

fn player(ui: &mut Ui, fx: &dyn Effects, ev: PlayerEvent) {
    match ev {
        // Reports from a request since replaced -- the outgoing track's
        // fade-out, a track restarted, a stop overtaken -- are dropped
        // before they touch anything.
        PlayerEvent::Progress { generation, .. } | PlayerEvent::Ended { generation } if !ui.is_current(generation) => {}
        PlayerEvent::Failed { generation, .. } if !ui.is_current(generation) => {}
        PlayerEvent::Progress { position, duration, playing, track_id, .. } => {
            ui.position = position;
            ui.duration = duration;
            ui.playing = playing;
            ui.position_at = Instant::now();
            if playing {
                ui.open_failures = 0;
            }
            fx.report_progress(PlaybackProgress { position, duration, playing, track_id });
        }
        PlayerEvent::Ended { generation } => {
            ui.playing = false;
            // Restarted and paused before this end arrived: the user's pause
            // stands, so the next track waits.
            ui.next_paused = fx.held() == Some(generation);
            fx.report_ended();
        }
        PlayerEvent::Failed { message, generation, .. } => {
            let give_up = ui.open_failed(&message);
            // What follows a paused load stays paused too.
            ui.next_paused = fx.held() == Some(generation);
            fx.report_failed(give_up);
        }
        PlayerEvent::Error(msg) if msg == "output device: saved output device unavailable, using system default" => {
            ui.notice = Some(DEVICE_FALLBACK.into())
        }
        PlayerEvent::Error(msg) => ui.notice = Some(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NullBackend;
    use bard_core::types::ScanProgress;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    /// Records what the reducer asked for; `play` and `stop` hand out
    /// generations as the engine does.
    #[derive(Default)]
    struct Fx {
        calls: RefCell<Vec<String>>,
        generation: RefCell<u64>,
        paused: RefCell<bool>,
        /// As `Player::held`: set by pause and a paused load, cleared by play.
        held: RefCell<u64>,
        tracks: Vec<Track>,
    }

    impl Fx {
        fn log(&self, s: impl Into<String>) {
            self.calls.borrow_mut().push(s.into());
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.calls.borrow_mut())
        }
        fn next(&self) -> u64 {
            *self.generation.borrow_mut() += 1;
            *self.generation.borrow()
        }
    }

    impl Effects for Fx {
        fn play(&self, t: &Track, _: f32, crossfade: f32) -> u64 {
            self.log(if crossfade > 0. { format!("play {} fade {crossfade}", t.id) } else { format!("play {}", t.id) });
            *self.held.borrow_mut() = 0;
            self.next()
        }
        fn stop(&self) -> u64 {
            self.log("stop");
            self.next()
        }
        fn cut_fade(&self) {
            self.log("cut fade")
        }
        fn load_paused(&self, t: &Track, _: f32) -> u64 {
            self.log(format!("load paused {}", t.id));
            let g = self.next();
            *self.held.borrow_mut() = g;
            g
        }
        fn playing(&self) -> bool {
            !*self.paused.borrow()
        }
        fn held(&self) -> Option<u64> {
            Some(*self.held.borrow()).filter(|g| *g != 0)
        }
        fn pause(&self) {
            self.log("pause");
            *self.held.borrow_mut() = *self.generation.borrow();
        }
        fn seek(&self, s: f64) {
            self.log(format!("seek {s}"))
        }
        fn set_norm_gain(&self, _: u64, _: f32) {
            self.log("norm")
        }
        fn set_output(&self, id: &str) -> Result<(), String> {
            self.log(format!("output {id}"));
            Ok(())
        }
        fn toggle_play(&self) {
            self.log("toggle")
        }
        fn skip(&self) {
            self.log("skip")
        }
        fn previous(&self) {
            self.log("previous")
        }
        fn set_volume(&self, v: f64) {
            self.log(format!("volume {v}"))
        }
        fn update_config(&self, _: Box<dyn FnOnce(&mut Config)>) {
            self.log("config")
        }
        fn scan(&self) {
            self.log("scan")
        }
        fn report_progress(&self, p: PlaybackProgress) {
            self.log(format!("progress {} {}", p.position, p.playing))
        }
        fn report_ended(&self) {
            self.log("ended")
        }
        fn report_failed(&self, give_up: bool) {
            self.log(format!("failed give_up={give_up}"))
        }
        fn stats(&self) -> StatsMap {
            StatsMap::default()
        }
        fn album_flags(&self) -> AlbumFlags {
            AlbumFlags::default()
        }
        fn loved(&self) -> Vec<String> {
            Vec::new()
        }
        fn tracks(&self) -> Vec<Track> {
            self.tracks.clone()
        }
        fn queue(&self) -> Vec<QueueItem> {
            Vec::new()
        }
    }

    fn track(id: &str, duration: f64) -> Track {
        Track {
            id: id.into(),
            path: format!("/m/{id}.flac"),
            title: format!("Title {id}"),
            artist: String::new(),
            album: "A".into(),
            duration_sec: duration,
            has_artwork: false,
            search_text: id.into(),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        }
    }

    fn ui() -> Ui {
        Ui::new(Rc::new(NullBackend), PathBuf::new())
    }

    fn progress(generation: u64, position: f64, duration: f64, id: &str) -> AppEvent {
        AppEvent::Player(PlayerEvent::Progress {
            position,
            duration,
            playing: true,
            track_id: Some(id.into()),
            generation,
        })
    }

    /// A cut plays with no crossfade whatever the setting; CutFade reaches
    /// the engine.
    #[test]
    fn a_cut_ignores_the_crossfade_setting() {
        let (mut u, fx) = (ui(), Fx::default());
        u.config.audio.crossfade_sec = 4.;
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 60.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("b", 60.), Transition::Cut)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::CutFade));
        assert_eq!(fx.take(), ["play a fade 4", "play b", "cut fade"]);

        // Paused: the replacement is loaded paused; a user's skip still plays.
        reduce(&mut u, &fx, AppEvent::Control(Control::Pause));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("c", 60.), Transition::Cut)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("d", 60.), Transition::Normal)));
        assert_eq!(fx.take(), ["pause", "load paused c", "play d fade 4"]);

        // Not playing because d has just run out, not because anyone paused:
        // the replacement plays.
        *fx.paused.borrow_mut() = true;
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("e", 60.), Transition::Cut)));
        assert_eq!(fx.take(), ["play e"]);
    }

    /// What the engine says the user's pause holds decides whether the
    /// track replacing a failed, ended or revoked one waits paused.
    #[test]
    fn a_held_request_that_fails_ends_or_is_cut_keeps_the_next_paused() {
        let (mut u, fx) = (ui(), Fx::default());
        let fail =
            |g| AppEvent::Player(PlayerEvent::Failed { generation: g, track_id: "x".into(), message: "gone".into() });
        u.config.audio.crossfade_sec = 4.;
        // Paused during a fade onto x, which then fails to open.
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 60.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("x", 60.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Control(Control::Pause));
        reduce(&mut u, &fx, fail(2));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("b", 60.), Transition::Normal)));
        assert_eq!(fx.take(), ["play a fade 4", "play x fade 4", "pause", "failed give_up=false", "load paused b"]);

        // Resumed before the failure arrives: the replacement plays.
        *fx.held.borrow_mut() = 0;
        reduce(&mut u, &fx, fail(3));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("c", 60.), Transition::Normal)));
        assert_eq!(fx.take(), ["failed give_up=false", "play c fade 4"]);

        // Ended, then restarted and paused before the end arrived.
        reduce(&mut u, &fx, AppEvent::Control(Control::Pause));
        reduce(&mut u, &fx, AppEvent::Player(PlayerEvent::Ended { generation: 4 }));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("d", 60.), Transition::Normal)));
        assert_eq!(fx.take(), ["pause", "ended", "load paused d"]);

        // Paused onto a file that has already failed (the engine has
        // nothing loaded), then revoked before the failure arrives.
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("x", 60.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Control(Control::Pause));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("e", 60.), Transition::Cut)));
        assert_eq!(fx.take(), ["play x fade 4", "pause", "load paused e"]);
    }

    #[test]
    fn the_outgoing_tracks_progress_never_reaches_the_model() {
        let (mut u, fx) = (ui(), Fx::default());
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 300.), Transition::Normal)));
        reduce(&mut u, &fx, progress(1, 120., 300., "a"));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("b", 90.), Transition::Normal)));
        fx.take();

        // During b's fade-in, a's fade-out still reports, as generation 1.
        reduce(&mut u, &fx, progress(1, 121., 300., "a"));
        assert_eq!((u.position, u.duration), (0., 0.), "b's title is not paired with a's timing");
        assert!(fx.take().is_empty(), "nor is the core told");

        reduce(&mut u, &fx, progress(2, 0.5, 90., "b"));
        assert_eq!((u.position, u.duration), (0.5, 90.));
        assert_eq!(fx.take(), ["progress 0.5 true"]);
    }

    #[test]
    fn restarting_the_same_track_ignores_the_old_runs_reports_and_end() {
        let (mut u, fx) = (ui(), Fx::default());
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 300.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 300.), Transition::Normal)));
        fx.take();
        reduce(&mut u, &fx, progress(1, 200., 300., "a"));
        reduce(&mut u, &fx, AppEvent::Player(PlayerEvent::Ended { generation: 1 }));
        assert!(fx.take().is_empty(), "the queue is not advanced past the restart");
        reduce(&mut u, &fx, AppEvent::Player(PlayerEvent::Ended { generation: 2 }));
        assert_eq!(fx.take(), ["ended"]);
    }

    #[test]
    fn reports_after_stop_are_ignored() {
        let (mut u, fx) = (ui(), Fx::default());
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("a", 300.), Transition::Normal)));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::NowPlaying(None)));
        assert_eq!(fx.take(), ["play a", "stop"]);
        reduce(&mut u, &fx, progress(1, 10., 300., "a"));
        assert!(!u.playing && u.track.is_none());
        assert!(fx.take().is_empty());
    }

    #[test]
    fn a_track_that_fails_to_open_is_skipped_then_playback_gives_up() {
        let (mut u, fx) = (ui(), Fx::default());
        let fail =
            |g| AppEvent::Player(PlayerEvent::Failed { generation: g, track_id: "x".into(), message: "gone".into() });
        for (i, id) in ["a", "b", "c"].into_iter().enumerate() {
            reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track(id, 60.), Transition::Normal)));
            reduce(&mut u, &fx, fail(i as u64 + 1));
        }
        assert_eq!(
            fx.take(),
            ["play a", "failed give_up=false", "play b", "failed give_up=false", "play c", "failed give_up=true"]
        );
        assert!(u.notice.as_deref().is_some_and(|n| n.contains("Title c") && n.contains("stopped after 3")));
        assert!(!u.playing);

        // A stale failure (a request since replaced) is not counted.
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("d", 60.), Transition::Normal)));
        fx.take();
        reduce(&mut u, &fx, fail(3));
        assert!(fx.take().is_empty());

        // A track that plays resets the count.
        reduce(&mut u, &fx, fail(4));
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::PlayTrack(track("e", 60.), Transition::Normal)));
        reduce(&mut u, &fx, progress(5, 0.1, 60., "e"));
        assert_eq!(u.open_failures, 0);
    }

    #[test]
    fn scan_completion_and_late_progress() {
        let (mut u, fx) = (ui(), Fx { tracks: vec![track("a", 60.)], ..Default::default() });
        reduce(&mut u, &fx, AppEvent::Rescan);
        assert!(u.scanning);
        reduce(
            &mut u,
            &fx,
            AppEvent::Core(CoreEvent::ScanProgress(ScanProgress {
                scanned: 3,
                total: 10,
                current_path: String::new(),
            })),
        );
        assert!(u.scan_progress.is_some());
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::LibraryChanged));
        reduce(&mut u, &fx, AppEvent::Scanned);
        assert!(!u.scanning && u.scan_progress.is_none());
        assert_eq!(u.tracks().len(), 1);
        // The final progress report arriving after the end must not turn the
        // indicator back on.
        reduce(
            &mut u,
            &fx,
            AppEvent::Core(CoreEvent::ScanProgress(ScanProgress {
                scanned: 10,
                total: 10,
                current_path: String::new(),
            })),
        );
        assert!(!u.scanning && u.scan_progress.is_none());
    }

    #[test]
    fn changing_the_folders_rescans() {
        let (mut u, fx) = (ui(), Fx::default());
        let mut c = u.config.clone();
        c.library_paths.push("/music".into());
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::ConfigChanged(c.clone())));
        assert_eq!(fx.take(), ["scan"]);
        reduce(&mut u, &fx, AppEvent::Core(CoreEvent::ConfigChanged(c)));
        assert!(fx.take().is_empty(), "an unrelated change does not");
    }

    #[test]
    fn the_sleep_timer_pauses_only_if_it_is_still_the_one_set() {
        let (mut u, fx) = (ui(), Fx::default());
        let (old, new) = (Instant::now(), Instant::now() + std::time::Duration::from_secs(60));
        u.playing = true;
        u.sleep_ends_at = Some(new);
        reduce(&mut u, &fx, AppEvent::SleepDue(old));
        assert!(fx.take().is_empty(), "a replaced timer does nothing");
        reduce(&mut u, &fx, AppEvent::SleepDue(new));
        assert_eq!(fx.take(), ["pause"]);
        assert!(u.sleep_ends_at.is_none());

        // Due between one track's end and the next one's first report, when
        // the window shows nothing playing: it still pauses.
        u.playing = false;
        u.sleep_ends_at = Some(new);
        reduce(&mut u, &fx, AppEvent::SleepDue(new));
        assert_eq!(fx.take(), ["pause"]);
    }

    /// Play asks the engine: right after a pause the last report still says
    /// playing, and Play must resume.
    #[test]
    fn play_right_after_a_pause_resumes() {
        let (mut u, fx) = (ui(), Fx::default());
        u.playing = true; // the last report, from before the pause
        *fx.paused.borrow_mut() = true;
        reduce(&mut u, &fx, AppEvent::Control(Control::Play));
        assert_eq!(fx.take(), ["toggle"]);
        *fx.paused.borrow_mut() = false;
        reduce(&mut u, &fx, AppEvent::Control(Control::Play));
        assert!(fx.take().is_empty(), "already playing");
    }

    #[test]
    fn a_late_output_device_clears_only_the_fallback_notice() {
        let (mut u, fx) = (ui(), Fx::default());
        u.config.audio.output_device_id = "Music".into();
        u.notice = Some(DEVICE_FALLBACK.into());
        reduce(&mut u, &fx, AppEvent::OutputAppeared { saved: "Music".into(), id: "Music".into() });
        assert_eq!(fx.take(), ["output Music"]);
        assert!(u.notice.is_none());

        u.notice = Some("something else".into());
        reduce(&mut u, &fx, AppEvent::OutputAppeared { saved: "Music".into(), id: "Music".into() });
        assert_eq!(u.notice.as_deref(), Some("something else"));

        // Carried over from Electron: the setting is rewritten too.
        u.config.audio.output_device_id = "b3199359".into();
        reduce(&mut u, &fx, AppEvent::OutputAppeared { saved: "b3199359".into(), id: "Music".into() });
        assert_eq!(fx.take(), ["output Music", "config", "output Music"]);
    }
}
